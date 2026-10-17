//! gRPC contract tests (T13, AC-056..AC-059).
//!
//! In-process transport: a real tonic server over a loopback TCP listener,
//! exercised with a real tonic client. The `CheckCore` is a mock defined in
//! this file (sanctioned T13 mode; the real core is assembled in T19).
//!
//! Empirically documented here and in `src/api/grpc/mod.rs`:
//! - unknown enum wire values (scene 0 / 99) decode into the raw `i32` field
//!   and are rejected by the handler with `INVALID_ARGUMENT`;
//! - `grpc-encoding: gzip` is rejected by the tonic codec (`UNIMPLEMENTED`,
//!   "Content is compressed with `gzip` which isn't supported") because no
//!   compression feature is compiled in;
//! - messages above 256 KiB are rejected by the framing layer with
//!   `OUT_OF_RANGE` before any handler runs.
//!
//! The deadline race is proven at two levels: end-to-end through the transport
//! (server clamps the 150ms requested deadline to a 60ms budget, so the
//! server-sent `DEADLINE_EXCEEDED` always beats both the client-side and
//! transport-side `grpc-timeout` timers — this is also what the real core does
//! with its 80ms `CHECK_BUDGET`), and directly against the service trait with
//! an unclamped deadline. Both use a mock core that sleeps 2s; neither sleeps
//! on the client side.

use imboy_textguard::api::auth::AuthTokens;
use imboy_textguard::api::grpc::{service_with_limits, TextGuardSvc, MAX_MESSAGE_BYTES};
use imboy_textguard::api::{CheckCore, InfoResponse};
use imboy_textguard::model::{self, ErrorCode};
use imboy_textguard::protocol::text_guard_client::TextGuardClient;
use imboy_textguard::protocol::text_guard_server::TextGuard as TextGuardService;
use imboy_textguard::protocol::{CheckTextRequest, GetInfoRequest, TextField};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tonic::Code;

const CHECK_TOKEN: &str = "grpc-check-secret";
const PUBLISH_TOKEN: &str = "grpc-publish-secret";

/// Serves the limited gRPC service on an ephemeral loopback port and returns
/// the bound address. Uses a hand-rolled listener stream because tokio-stream
/// is not a direct dependency of this crate.
async fn spawn_server(core: Arc<MockCore>) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let service = service_with_limits(core, Arc::new(AuthTokens::new(CHECK_TOKEN, PUBLISH_TOKEN)));
    tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming(Incoming(listener))
            .await
            .unwrap();
    });
    address
}

struct Incoming(tokio::net::TcpListener);

impl tonic::codegen::tokio_stream::Stream for Incoming {
    type Item = std::io::Result<tokio::net::TcpStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0
            .poll_accept(cx)
            .map(|accepted| Some(accepted.map(|(stream, _address)| stream)))
    }
}

async fn client(address: SocketAddr) -> TextGuardClient<tonic::transport::Channel> {
    let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}")).unwrap();
    TextGuardClient::new(endpoint.connect().await.unwrap())
}

/// Test double for the whole core: records requests, sleeps to simulate scan
/// work, then returns the programmed error or a full sample response. The
/// optional budget clamps `check_deadline` exactly like the real core's
/// `CHECK_BUDGET` does.
struct MockCore {
    error: Mutex<Option<ErrorCode>>,
    seen: Mutex<Vec<model::CheckRequest>>,
    sleep: Duration,
    budget: Option<Duration>,
    started: AtomicBool,
    finished: AtomicBool,
}

impl MockCore {
    fn with_parts(
        error: Option<ErrorCode>,
        sleep: Duration,
        budget: Option<Duration>,
    ) -> Arc<Self> {
        Arc::new(Self {
            error: Mutex::new(error),
            seen: Mutex::new(Vec::new()),
            sleep,
            budget,
            started: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        })
    }

    fn quick() -> Arc<Self> {
        Self::with_parts(None, Duration::ZERO, None)
    }

    fn failing(code: ErrorCode) -> Arc<Self> {
        Self::with_parts(Some(code), Duration::ZERO, None)
    }

    /// A core whose `check` blocks for `sleep` and whose deadline budget is
    /// clamped to `budget` when requested deadlines arrive.
    fn sleeping(sleep: Duration, budget: Option<Duration>) -> Arc<Self> {
        Self::with_parts(None, sleep, budget)
    }

    fn seen_requests(&self) -> Vec<model::CheckRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl CheckCore for MockCore {
    fn check(&self, request: &model::CheckRequest) -> Result<model::CheckResponse, ErrorCode> {
        self.started.store(true, Ordering::SeqCst);
        self.seen.lock().unwrap().push(request.clone());
        std::thread::sleep(self.sleep);
        self.finished.store(true, Ordering::SeqCst);
        match *self.error.lock().unwrap() {
            Some(code) => Err(code),
            None => Ok(sample_response(&request.request_id)),
        }
    }

    fn info(&self) -> InfoResponse {
        InfoResponse {
            instance_id: "instance-1".into(),
            ready: true,
            build_id: "build-1".into(),
            ruleset_revision: "rev-1".into(),
            generation: "gen-1".into(),
            normalization_id: "imboy-base-v1".into(),
            t2s_table_id: "opencc-1.1.9-single-v1".into(),
            base_rules_digest: "digest-1".into(),
            canonical_count: 7,
            alias_count: 5,
            regex_count: 3,
            supported_semantics: vec!["compat-v1".into(), "enhanced-v1".into()],
        }
    }

    fn ready(&self) -> bool {
        true
    }

    fn check_deadline(&self, requested: Option<Instant>) -> Instant {
        match (self.budget, requested) {
            (Some(budget), _) => requested
                .unwrap_or_else(Instant::now)
                .min(Instant::now() + budget),
            (None, Some(requested)) => requested,
            (None, None) => Instant::now() + imboy_textguard::runtime::CHECK_BUDGET,
        }
    }
}

fn sample_response(request_id: &str) -> model::CheckResponse {
    model::CheckResponse {
        request_id: request_id.into(),
        ruleset_revision: "rev-1".into(),
        generation: "gen-1".into(),
        normalization_id: "imboy-base-v1".into(),
        t2s_table_id: "opencc-1.1.9-single-v1".into(),
        semantics_id: "compat-v1".into(),
        base_rules_digest: "digest-1".into(),
        decision: model::Action::Review,
        scan_complete: true,
        hits_truncated: false,
        hits: vec![model::Hit {
            field_id: "body".into(),
            rule_id: "r-1".into(),
            variant_id: "v-1".into(),
            variant_type: model::VariantType::Pinyin,
            word: "bad".into(),
            category: "abuse".into(),
            severity: model::Severity::Low,
            effective_action: model::RuleAction::Review,
            start_byte: 0,
            end_byte: 3,
            match_methods: vec![model::MatchMethod::Literal, model::MatchMethod::T2s],
        }],
        fields_checked: 1,
        hit_semantics: "literal-all-regex-first-v1".into(),
        build_id: "build-1".into(),
    }
}

fn field(field_id: Option<&str>, text: Option<&str>) -> TextField {
    TextField {
        field_id: field_id.map(str::to_owned),
        text: text.map(str::to_owned),
    }
}

fn authorized_request(
    request_id: Option<&str>,
    scene: Option<i32>,
    fields: Vec<TextField>,
) -> tonic::Request<CheckTextRequest> {
    let mut request = tonic::Request::new(CheckTextRequest {
        request_id: request_id.map(str::to_owned),
        scene,
        fields,
    });
    request
        .metadata_mut()
        .insert("authorization", CHECK_TOKEN.parse().unwrap());
    request
}

fn error_code_of(status: &tonic::Status) -> String {
    status
        .metadata()
        .get("x-textguard-error-code")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

async fn expect_invalid_argument(
    client: &mut TextGuardClient<tonic::transport::Channel>,
    request: tonic::Request<CheckTextRequest>,
) {
    let error = client.check_text(request).await.unwrap_err();
    assert_eq!(error.code(), Code::InvalidArgument, "{error:?}");
    assert_eq!(error_code_of(&error), "INVALID_ARGUMENT");
}

// ---- AC-056: presence -------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_request_id_is_invalid_argument() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    expect_invalid_argument(
        &mut client,
        authorized_request(None, Some(1), vec![field(Some("body"), Some("hello"))]),
    )
    .await;
    assert!(core.seen_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_scene_is_invalid_argument() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    expect_invalid_argument(
        &mut client,
        authorized_request(
            Some("req-1"),
            None,
            vec![field(Some("body"), Some("hello"))],
        ),
    )
    .await;
    assert!(core.seen_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_fields_is_invalid_argument() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    expect_invalid_argument(
        &mut client,
        authorized_request(Some("req-1"), Some(1), vec![]),
    )
    .await;
    assert!(core.seen_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_field_id_or_text_is_invalid_argument() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    expect_invalid_argument(
        &mut client,
        authorized_request(Some("req-1"), Some(1), vec![field(None, Some("hello"))]),
    )
    .await;
    expect_invalid_argument(
        &mut client,
        authorized_request(Some("req-1"), Some(1), vec![field(Some("body"), None)]),
    )
    .await;
    assert!(core.seen_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_field_ids_and_more_than_eight_fields_are_invalid_argument() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    expect_invalid_argument(
        &mut client,
        authorized_request(
            Some("req-1"),
            Some(1),
            vec![
                field(Some("body"), Some("a")),
                field(Some("body"), Some("b")),
            ],
        ),
    )
    .await;
    let nine: Vec<_> = (0..9)
        .map(|index| field(Some(&format!("f{index}")), Some("x")))
        .collect();
    expect_invalid_argument(
        &mut client,
        authorized_request(Some("req-1"), Some(1), nine),
    )
    .await;
    // eight distinct fields remain valid
    let eight: Vec<_> = (0..8)
        .map(|index| field(Some(&format!("f{index}")), Some("x")))
        .collect();
    assert!(client
        .check_text(authorized_request(Some("req-1"), Some(1), eight))
        .await
        .is_ok());
    assert_eq!(core.seen_requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_empty_text_is_valid_and_reaches_the_core() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    let response = client
        .check_text(authorized_request(
            Some("req-empty"),
            Some(3),
            vec![field(Some("nickname"), Some(""))],
        ))
        .await
        .unwrap()
        .into_inner();
    let seen = core.seen_requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].fields[0].text, "");
    assert_eq!(response.request_id.as_deref(), Some("req-empty"));
}

// ---- AC-057: unknown enums and full response presence -------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scene_zero_and_unknown_scene_values_are_invalid_argument() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    // prost decodes optional enums as raw i32, so 0 and 99 reach the handler
    // and must be rejected there (never defaulted, never a decode crash).
    for scene in [0, 99, -7] {
        expect_invalid_argument(
            &mut client,
            authorized_request(
                Some("req-1"),
                Some(scene),
                vec![field(Some("body"), Some("x"))],
            ),
        )
        .await;
    }
    assert!(core.seen_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn decoded_response_keeps_scan_complete_and_decision_set() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    let response = client
        .check_text(authorized_request(
            Some("req-1"),
            Some(1),
            vec![field(Some("body"), Some("hello"))],
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(response.scan_complete, Some(true));
    assert_eq!(response.decision, Some(2)); // Decision::Review
                                            // every response field is set after the wire round-trip
    assert_eq!(response.request_id.as_deref(), Some("req-1"));
    assert_eq!(response.ruleset_revision.as_deref(), Some("rev-1"));
    assert_eq!(response.generation.as_deref(), Some("gen-1"));
    assert_eq!(response.normalization_id.as_deref(), Some("imboy-base-v1"));
    assert_eq!(
        response.t2s_table_id.as_deref(),
        Some("opencc-1.1.9-single-v1")
    );
    assert_eq!(response.semantics_id.as_deref(), Some("compat-v1"));
    assert_eq!(response.base_rules_digest.as_deref(), Some("digest-1"));
    assert_eq!(response.hits_truncated, Some(false));
    assert_eq!(response.fields_checked, Some(1));
    assert_eq!(
        response.hit_semantics.as_deref(),
        Some("literal-all-regex-first-v1")
    );
    assert_eq!(response.build_id.as_deref(), Some("build-1"));
    assert_eq!(response.hits.len(), 1);
    let hit = &response.hits[0];
    assert_eq!(hit.field_id.as_deref(), Some("body"));
    assert_eq!(hit.rule_id.as_deref(), Some("r-1"));
    assert_eq!(hit.variant_id.as_deref(), Some("v-1"));
    assert_eq!(hit.variant_type, Some(2)); // VariantType::Pinyin
    assert_eq!(hit.word.as_deref(), Some("bad"));
    assert_eq!(hit.category.as_deref(), Some("abuse"));
    assert_eq!(hit.severity, Some(1)); // Severity::Low
    assert_eq!(hit.effective_action, Some(2)); // Decision::Review
    assert_eq!(hit.start_byte, Some(0));
    assert_eq!(hit.end_byte, Some(3));
    assert_eq!(hit.match_methods, vec![1, 2]); // LITERAL, T2S
}

// ---- AC-058: error mapping ----------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overloaded_and_text_too_large_share_resource_exhausted_but_keep_distinct_codes() {
    for (code, name) in [
        (ErrorCode::Overloaded, "OVERLOADED"),
        (ErrorCode::TextTooLarge, "TEXT_TOO_LARGE"),
    ] {
        let core = MockCore::failing(code);
        let address = spawn_server(core).await;
        let mut client = client(address).await;
        let error = client
            .check_text(authorized_request(
                Some("req-1"),
                Some(1),
                vec![field(Some("body"), Some("hello"))],
            ))
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::ResourceExhausted, "{error:?}");
        assert_eq!(error.message(), name);
        assert_eq!(error_code_of(&error), name);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credential_failures_map_to_unauthenticated_and_permission_denied() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;

    let mut missing =
        authorized_request(Some("req-1"), Some(1), vec![field(Some("body"), Some("x"))]);
    missing.metadata_mut().remove("authorization");
    let error = client.check_text(missing).await.unwrap_err();
    assert_eq!(error.code(), Code::Unauthenticated);
    assert_eq!(error_code_of(&error), "UNAUTHENTICATED");

    let mut wrong =
        authorized_request(Some("req-1"), Some(1), vec![field(Some("body"), Some("x"))]);
    wrong
        .metadata_mut()
        .insert("authorization", "garbage".parse().unwrap());
    let error = client.check_text(wrong).await.unwrap_err();
    assert_eq!(error.code(), Code::Unauthenticated);
    assert_eq!(error_code_of(&error), "UNAUTHENTICATED");

    let mut publish =
        authorized_request(Some("req-1"), Some(1), vec![field(Some("body"), Some("x"))]);
    publish
        .metadata_mut()
        .insert("authorization", PUBLISH_TOKEN.parse().unwrap());
    let error = client.check_text(publish).await.unwrap_err();
    assert_eq!(error.code(), Code::PermissionDenied);
    assert_eq!(error_code_of(&error), "PERMISSION_DENIED");

    // Bearer-prefixed raw token is accepted, for HTTP-plane parity
    let mut bearer =
        authorized_request(Some("req-1"), Some(1), vec![field(Some("body"), Some("x"))]);
    bearer.metadata_mut().insert(
        "authorization",
        format!("Bearer {CHECK_TOKEN}").parse().unwrap(),
    );
    assert!(client.check_text(bearer).await.is_ok());
    assert_eq!(core.seen_requests().len(), 1);

    // repeated credentials are treated as absent, for HTTP-plane parity
    let mut repeated =
        authorized_request(Some("req-1"), Some(1), vec![field(Some("body"), Some("x"))]);
    repeated
        .metadata_mut()
        .append("authorization", CHECK_TOKEN.parse().unwrap());
    let error = client.check_text(repeated).await.unwrap_err();
    assert_eq!(error.code(), Code::Unauthenticated);
    assert_eq!(error_code_of(&error), "UNAUTHENTICATED");
    assert_eq!(core.seen_requests().len(), 1);
}

// ---- AC-059: limits, deadline and compression ---------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_message_is_rejected_by_framing_before_the_handler() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    let oversized = "a".repeat(MAX_MESSAGE_BYTES + 4096);
    let error = client
        .check_text(authorized_request(
            Some("req-big"),
            Some(1),
            vec![field(Some("body"), Some(&oversized))],
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::OutOfRange, "{error:?}");
    assert!(error.message().contains("decoded message length too large"));
    assert!(core.seen_requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grpc_timeout_deadline_aborts_while_the_core_is_still_working() {
    // 2s core work vs a 150ms grpc-timeout. The mock clamps the effective
    // deadline to a 60ms budget (the real core clamps to CHECK_BUDGET), so the
    // server-sent DEADLINE_EXCEEDED status deterministically beats both the
    // client-side and transport-side grpc-timeout timers.
    let core = MockCore::sleeping(Duration::from_secs(2), Some(Duration::from_millis(60)));
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    let mut request = authorized_request(
        Some("req-slow"),
        Some(1),
        vec![field(Some("body"), Some("hello"))],
    );
    request
        .metadata_mut()
        .insert("grpc-timeout", "150m".parse().unwrap());
    let started = Instant::now();
    let error = client.check_text(request).await.unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.code(), Code::DeadlineExceeded, "{error:?}");
    // the server sent this status: only it sets the error-code metadata
    assert_eq!(error_code_of(&error), "CHECK_DEADLINE");
    assert!(elapsed < Duration::from_secs(1), "abort took {elapsed:?}");
    assert!(core.started.load(Ordering::SeqCst));
    // the core is still working when the RPC aborts...
    assert!(!core.finished.load(Ordering::SeqCst));
    assert_eq!(core.seen_requests().len(), 1);
    // ...and finishes on its own later, without the RPC waiting for it
    let deadline = Instant::now() + Duration::from_secs(4);
    while !core.finished.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "core never finished");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn service_race_honors_an_unclamped_grpc_timeout() {
    // Direct trait call: no transport layers race the deadline here, so the
    // requested 150ms is used verbatim and the abort is purely ours.
    let core = MockCore::sleeping(Duration::from_secs(2), None);
    let service = TextGuardSvc::new(
        core.clone(),
        Arc::new(AuthTokens::new(CHECK_TOKEN, PUBLISH_TOKEN)),
    );
    let mut request = authorized_request(
        Some("req-slow"),
        Some(1),
        vec![field(Some("body"), Some("hello"))],
    );
    request
        .metadata_mut()
        .insert("grpc-timeout", "150m".parse().unwrap());
    let started = Instant::now();
    let error = TextGuardService::check_text(&service, request)
        .await
        .unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error.code(), Code::DeadlineExceeded, "{error:?}");
    assert_eq!(error_code_of(&error), "CHECK_DEADLINE");
    assert!(elapsed < Duration::from_secs(1), "abort took {elapsed:?}");
    assert!(
        elapsed >= Duration::from_millis(100),
        "abort took {elapsed:?}"
    );
    assert!(!core.finished.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gzip_compressed_request_is_rejected_without_decompression() {
    // No compression feature is compiled in: the tonic codec rejects the
    // grpc-encoding header itself (the body below is not even gzipped — the
    // server must refuse before ever looking at the payload).
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;
    let mut request = authorized_request(
        Some("req-gzip"),
        Some(1),
        vec![field(Some("body"), Some("hello"))],
    );
    request
        .metadata_mut()
        .insert("grpc-encoding", "gzip".parse().unwrap());
    let error = client.check_text(request).await.unwrap_err();
    assert_eq!(error.code(), Code::Unimplemented, "{error:?}");
    assert!(
        error.message().contains("gzip") && error.message().contains("isn't supported"),
        "{:?}",
        error.message()
    );
    assert!(core.seen_requests().is_empty());
}

// ---- GetInfo -------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_info_maps_every_field_and_requires_credentials() {
    let core = MockCore::quick();
    let address = spawn_server(core.clone()).await;
    let mut client = client(address).await;

    let mut request = tonic::Request::new(GetInfoRequest {});
    request
        .metadata_mut()
        .insert("authorization", CHECK_TOKEN.parse().unwrap());
    let info = client.get_info(request).await.unwrap().into_inner();
    assert_eq!(info.instance_id.as_deref(), Some("instance-1"));
    assert_eq!(info.ready, Some(true));
    assert_eq!(info.build_id.as_deref(), Some("build-1"));
    assert_eq!(info.ruleset_revision.as_deref(), Some("rev-1"));
    assert_eq!(info.generation.as_deref(), Some("gen-1"));
    assert_eq!(info.normalization_id.as_deref(), Some("imboy-base-v1"));
    assert_eq!(info.t2s_table_id.as_deref(), Some("opencc-1.1.9-single-v1"));
    assert_eq!(info.base_rules_digest.as_deref(), Some("digest-1"));
    assert_eq!(info.canonical_count, Some(7));
    assert_eq!(info.alias_count, Some(5));
    assert_eq!(info.regex_count, Some(3));
    assert_eq!(
        info.supported_semantics,
        vec!["compat-v1".to_owned(), "enhanced-v1".to_owned()]
    );

    let error = client
        .get_info(tonic::Request::new(GetInfoRequest {}))
        .await
        .unwrap_err();
    assert_eq!(error.code(), Code::Unauthenticated);
    assert_eq!(error_code_of(&error), "UNAUTHENTICATED");
}
