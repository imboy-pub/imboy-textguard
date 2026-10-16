//! HTTP data-plane contract tests (T12: AC-052, AC-053, AC-054, AC-055).
//!
//! Requests are driven through the router in-process (tower oneshot style:
//! `Service::call` on the `axum::Router`, with the `tower_service::Service`
//! trait reaching the test via tonic's public `codegen` re-export, since the
//! workspace forbids adding `tower`/`http` as direct test dependencies).
//! A mock `CheckCore` is the sanctioned T12 mode; the real core closure and
//! runtime admission are T19's assembly job.

use imboy_textguard::api::auth::AuthTokens;
use imboy_textguard::api::http::{router, MAX_BODY_BYTES};
use imboy_textguard::api::{CheckCore, InfoResponse};
use imboy_textguard::model::{Action, CheckRequest, CheckResponse, ErrorCode};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tonic::codegen::Service as _;

use axum::body::{to_bytes, Body};
use axum::http::StatusCode;
use axum::http::{header, Method, Request};
use axum::Router;

const CHECK_TOKEN: &str = "check-secret";
const PUBLISH_TOKEN: &str = "publish-secret";

struct MockCore {
    error: Option<ErrorCode>,
    ready: bool,
    seen: Mutex<Vec<CheckRequest>>,
}

impl CheckCore for MockCore {
    fn check(&self, request: &CheckRequest) -> Result<CheckResponse, ErrorCode> {
        self.seen.lock().unwrap().push(request.clone());
        if let Some(code) = self.error {
            return Err(code);
        }
        Ok(CheckResponse {
            request_id: request.request_id.clone(),
            ruleset_revision: "rev-1".into(),
            generation: "gen-1".into(),
            normalization_id: "imboy-base-v1".into(),
            t2s_table_id: "opencc-1.1.9-single-v1".into(),
            semantics_id: "compat-v1".into(),
            base_rules_digest: "digest-1".into(),
            decision: Action::Allow,
            scan_complete: true,
            hits_truncated: false,
            hits: Vec::new(),
            fields_checked: request.fields.len() as u32,
            hit_semantics: "literal-all-regex-first-v1".into(),
            build_id: "build-1".into(),
        })
    }

    fn info(&self) -> InfoResponse {
        InfoResponse {
            instance_id: "instance-1".into(),
            ready: self.ready,
            build_id: "build-1".into(),
            ruleset_revision: "rev-1".into(),
            generation: "gen-1".into(),
            normalization_id: "imboy-base-v1".into(),
            t2s_table_id: "opencc-1.1.9-single-v1".into(),
            base_rules_digest: "digest-1".into(),
            canonical_count: 7,
            alias_count: 3,
            regex_count: 2,
            supported_semantics: vec!["compat-v1".into(), "enhanced-v1".into()],
        }
    }

    fn ready(&self) -> bool {
        self.ready
    }
}

struct Fixture {
    router: Router,
    core: Arc<MockCore>,
}

fn fixture_with(error: Option<ErrorCode>, ready: bool) -> Fixture {
    let core = Arc::new(MockCore {
        error,
        ready,
        seen: Mutex::new(Vec::new()),
    });
    Fixture {
        router: router(
            core.clone(),
            Arc::new(AuthTokens::new(CHECK_TOKEN, PUBLISH_TOKEN)),
        ),
        core,
    }
}

fn fixture() -> Fixture {
    fixture_with(None, true)
}

/// In-process oneshot: drive one request through the router and decode the
/// JSON response body.
async fn send(router: Router, request: Request<Body>) -> (StatusCode, Value) {
    let mut router = router;
    let response = router.call(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}

fn post_check(credential: Option<&str>, body: impl Into<Body>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri("/v1/check")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(credential) = credential {
        builder = builder.header(header::AUTHORIZATION, credential);
    }
    builder.body(body.into()).unwrap()
}

fn valid_body() -> String {
    r#"{"request_id":"req-1","scene":"channel_message","fields":[{"field_id":"body","text":"hello"}]}"#.into()
}

fn error_code(value: &Value) -> &str {
    value["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("missing error code in body: {value}"))
}

// AC-052: request shape — explicit text, 1..8 fields, unique bounded ids.

#[tokio::test]
async fn valid_request_round_trips_through_the_core() {
    let fixture = fixture();
    let (status, body) = send(
        fixture.router.clone(),
        post_check(Some(CHECK_TOKEN), valid_body()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["request_id"], json!("req-1"));
    assert_eq!(body["decision"], json!("allow"));
    assert_eq!(body["fields_checked"], json!(1));
    let seen = fixture.core.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].fields[0].field_id, "body");
    assert_eq!(seen[0].fields[0].text, "hello");
}

#[tokio::test]
async fn empty_text_is_accepted_but_missing_or_null_text_is_rejected() {
    let fixture = fixture();
    let empty =
        r#"{"request_id":"r","scene":"profile_field","fields":[{"field_id":"nick","text":""}]}"#;
    let (status, body) = send(fixture.router.clone(), post_check(Some(CHECK_TOKEN), empty)).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(fixture.core.seen.lock().unwrap()[0].fields[0].text, "");

    for body in [
        r#"{"request_id":"r","scene":"profile_field","fields":[{"field_id":"nick"}]}"#,
        r#"{"request_id":"r","scene":"profile_field","fields":[{"field_id":"nick","text":null}]}"#,
    ] {
        let (status, value) =
            send(fixture.router.clone(), post_check(Some(CHECK_TOKEN), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        assert_eq!(error_code(&value), "INVALID_ARGUMENT");
    }
}

#[tokio::test]
async fn field_count_uniqueness_and_identity_bounds_are_enforced() {
    let fixture = fixture();
    let one_field = r#"{"field_id":"f","text":""}"#;
    let cases = [
        // no fields
        r#"{"request_id":"r","scene":"channel_message","fields":[]}"#.to_string(),
        // nine fields
        format!(
            r#"{{"request_id":"r","scene":"channel_message","fields":[{}]}}"#,
            [one_field; 9].join(",")
        ),
        // duplicate field ids
        format!(
            r#"{{"request_id":"r","scene":"channel_message","fields":[{0},{0}]}}"#,
            one_field
        ),
        // request id pattern violations: empty, illegal characters, 65 chars
        format!(r#"{{"request_id":"","scene":"channel_message","fields":[{one_field}]}}"#),
        format!(r#"{{"request_id":"bad id!","scene":"channel_message","fields":[{one_field}]}}"#),
        format!(
            r#"{{"request_id":"{}","scene":"channel_message","fields":[{one_field}]}}"#,
            "a".repeat(65)
        ),
        // field id violations: control character and 65 printable chars
        r#"{"request_id":"r","scene":"channel_message","fields":[{"field_id":"f\nx","text":""}]}"#
            .to_string(),
        format!(
            r#"{{"request_id":"r","scene":"channel_message","fields":[{{"field_id":"{}","text":""}}]}}"#,
            "i".repeat(65)
        ),
    ];
    for body in cases {
        let (status, value) = send(
            fixture.router.clone(),
            post_check(Some(CHECK_TOKEN), body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        assert_eq!(error_code(&value), "INVALID_ARGUMENT", "body: {body}");
    }
    assert!(fixture.core.seen.lock().unwrap().is_empty());
}

// AC-053: strict JSON — duplicates, unknown fields, frozen scenes, types.

#[tokio::test]
async fn duplicate_json_keys_top_level_and_nested_are_rejected() {
    let fixture = fixture();
    let cases = [
        r#"{"request_id":"r","request_id":"r","scene":"channel_message","fields":[{"field_id":"f","text":""}]}"#,
        r#"{"request_id":"r","scene":"channel_message","scene":"profile_field","fields":[{"field_id":"f","text":""}]}"#,
        r#"{"request_id":"r","scene":"channel_message","fields":[{"field_id":"f","field_id":"f","text":""}]}"#,
        r#"{"request_id":"r","scene":"channel_message","fields":[{"field_id":"f","text":"","text":""}]}"#,
    ];
    for body in cases {
        // the documents are well-formed JSON whose last duplicate would win in
        // an untyped parse; the plane must detect the duplication itself
        assert!(serde_json::from_str::<Value>(body).is_ok());
        let (status, value) =
            send(fixture.router.clone(), post_check(Some(CHECK_TOKEN), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code(&value), "INVALID_ARGUMENT");
    }
    assert!(fixture.core.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_fields_bad_scenes_and_bad_types_are_rejected() {
    let fixture = fixture();
    let cases = [
        r#"{"request_id":"r","scene":"channel_message","fields":[{"field_id":"f","text":""}],"extra":1}"#,
        r#"{"request_id":"r","scene":"channel_message","fields":[{"field_id":"f","text":"","note":"x"}]}"#,
        r#"{"request_id":"r","scene":"unknown","fields":[{"field_id":"f","text":""}]}"#,
        r#"{"request_id":"r","scene":"ChannelMessage","fields":[{"field_id":"f","text":""}]}"#,
        r#"{"request_id":"r","scene":7,"fields":[{"field_id":"f","text":""}]}"#,
        r#"{"request_id":"r","scene":"channel_message","fields":{"field_id":"f","text":""}}"#,
        r#"{"request_id":"r","scene":"channel_message","fields":"none"}"#,
        r#"{"request_id":null,"scene":"channel_message","fields":[{"field_id":"f","text":""}]}"#,
        r#"{"request_id":7,"scene":"channel_message","fields":[{"field_id":"f","text":""}]}"#,
        r#"{"request_id":"r","scene":"channel_message","fields":["f"]}"#,
        r#"[{"request_id":"r"}]"#,
        r#"not json"#,
    ];
    for body in cases {
        let (status, value) =
            send(fixture.router.clone(), post_check(Some(CHECK_TOKEN), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "case: {body}");
        assert_eq!(error_code(&value), "INVALID_ARGUMENT", "case: {body}");
    }
    assert!(fixture.core.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_utf8_body_is_rejected_with_invalid_utf8() {
    let fixture = fixture();
    let (status, value) = send(
        fixture.router.clone(),
        post_check(Some(CHECK_TOKEN), b"\xff\xfe{\"request_id\":1}".to_vec()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&value), "INVALID_UTF8");
}

// AC-054: bounded bodies, no decompression.

#[tokio::test]
async fn bodies_over_256kib_are_rejected_with_and_without_content_length() {
    let fixture = fixture();
    let text = "a".repeat(MAX_BODY_BYTES);
    let body = format!(
        r#"{{"request_id":"big","scene":"channel_message","fields":[{{"field_id":"f","text":"{text}"}}]}}"#
    );
    assert!(body.len() > MAX_BODY_BYTES);

    // early rejection through the declared Content-Length
    let with_length = Request::builder()
        .method(Method::POST)
        .uri("/v1/check")
        .header(header::AUTHORIZATION, CHECK_TOKEN)
        .header(header::CONTENT_LENGTH, body.len().to_string())
        .body(Body::from(body.clone()))
        .unwrap();
    let (status, value) = send(fixture.router.clone(), with_length).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error_code(&value), "TEXT_TOO_LARGE");

    // no Content-Length declared: the bounded read still rejects
    let (status, value) = send(fixture.router.clone(), post_check(Some(CHECK_TOKEN), body)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error_code(&value), "TEXT_TOO_LARGE");
    assert!(fixture.core.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unknown_content_encoding_is_rejected_and_identity_accepted() {
    let fixture = fixture();
    let gzip = Request::builder()
        .method(Method::POST)
        .uri("/v1/check")
        .header(header::AUTHORIZATION, CHECK_TOKEN)
        .header(header::CONTENT_ENCODING, "gzip")
        .body(Body::from(valid_body()))
        .unwrap();
    let (status, value) = send(fixture.router.clone(), gzip).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&value), "PROTOCOL_ERROR");
    assert!(fixture.core.seen.lock().unwrap().is_empty());

    let identity = Request::builder()
        .method(Method::POST)
        .uri("/v1/check")
        .header(header::AUTHORIZATION, CHECK_TOKEN)
        .header(header::CONTENT_ENCODING, "identity")
        .body(Body::from(valid_body()))
        .unwrap();
    let (status, body) = send(fixture.router.clone(), identity).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
}

#[tokio::test]
async fn undersized_content_length_is_still_handled_safely() {
    let fixture = fixture();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/check")
        .header(header::AUTHORIZATION, CHECK_TOKEN)
        .header(header::CONTENT_LENGTH, "10")
        .body(Body::from(valid_body()))
        .unwrap();
    let (status, body) = send(fixture.router, request).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["request_id"], json!("req-1"));
}

// AC-055: scoped credentials and unauthenticated health endpoints.

#[tokio::test]
async fn missing_wrong_and_foreign_credentials_are_rejected() {
    let fixture = fixture();
    let (status, value) = send(fixture.router.clone(), post_check(None, valid_body())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&value), "UNAUTHENTICATED");

    let (status, value) = send(
        fixture.router.clone(),
        post_check(Some("wrong-token"), valid_body()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&value), "UNAUTHENTICATED");

    // a valid publish credential is the wrong scope for the check plane
    let (status, value) = send(
        fixture.router.clone(),
        post_check(Some(PUBLISH_TOKEN), valid_body()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&value), "PERMISSION_DENIED");

    // the bearer form of the check credential is accepted
    let (status, body) = send(
        fixture.router.clone(),
        post_check(Some("Bearer check-secret"), valid_body()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(fixture.core.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn health_endpoints_work_without_credentials() {
    let fixture = fixture();
    let (status, body) = send(
        fixture.router.clone(),
        Request::get("/health/live").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("live"));

    let (status, body) = send(
        fixture.router.clone(),
        Request::get("/health/ready").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("ready"));
}

#[tokio::test]
async fn readiness_reports_not_ready_without_credentials() {
    let fixture = fixture_with(None, false);
    let (status, body) = send(
        fixture.router,
        Request::get("/health/ready").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], json!("not_ready"));
}

#[tokio::test]
async fn core_errors_map_to_the_documented_status_table() {
    for (error, status, code) in [
        (
            ErrorCode::TextTooLarge,
            StatusCode::PAYLOAD_TOO_LARGE,
            "TEXT_TOO_LARGE",
        ),
        (
            ErrorCode::Overloaded,
            StatusCode::SERVICE_UNAVAILABLE,
            "OVERLOADED",
        ),
        (
            ErrorCode::NotReady,
            StatusCode::SERVICE_UNAVAILABLE,
            "NOT_READY",
        ),
        (
            ErrorCode::CheckDeadline,
            StatusCode::GATEWAY_TIMEOUT,
            "CHECK_DEADLINE",
        ),
        (
            ErrorCode::MatchBudgetExceeded,
            StatusCode::INTERNAL_SERVER_ERROR,
            "MATCH_BUDGET_EXCEEDED",
        ),
    ] {
        let fixture = fixture_with(Some(error), true);
        let (actual_status, value) =
            send(fixture.router, post_check(Some(CHECK_TOKEN), valid_body())).await;
        assert_eq!(actual_status, status, "code: {code}");
        assert_eq!(error_code(&value), code);
    }
}

#[tokio::test]
async fn info_reports_runtime_state_and_publish_routes_are_absent() {
    let fixture = fixture();
    let info = Request::get("/v1/info")
        .header(header::AUTHORIZATION, CHECK_TOKEN)
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(fixture.router.clone(), info).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["instance_id"], json!("instance-1"));
    assert_eq!(body["canonical_count"], json!(7));

    let anonymous = Request::get("/v1/info").body(Body::empty()).unwrap();
    let (status, value) = send(fixture.router.clone(), anonymous).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&value), "UNAUTHENTICATED");

    // publish endpoints belong to T14 and must not exist here
    let publish = Request::post("/v1/publish")
        .header(header::AUTHORIZATION, PUBLISH_TOKEN)
        .body(Body::from(valid_body()))
        .unwrap();
    let (status, _) = send(fixture.router, publish).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
