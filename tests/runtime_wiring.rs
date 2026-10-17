use imboy_textguard::api::auth::AuthTokens;
use imboy_textguard::api::{CheckCore, InfoResponse};
use imboy_textguard::model::{CheckRequest, CheckResponse, ErrorCode};
use imboy_textguard::protocol::text_guard_server::TextGuard as _;
use imboy_textguard::protocol::{CheckTextRequest, TextField};
use imboy_textguard::runtime::Admission;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tonic::codegen::Service as _;

struct HeldCore {
    started: AtomicUsize,
    release: AtomicBool,
}

impl CheckCore for HeldCore {
    fn check(&self, _: &CheckRequest) -> Result<CheckResponse, ErrorCode> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.release.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(ErrorCode::NotReady)
    }
    fn ready(&self) -> bool {
        true
    }
    fn info(&self) -> InfoResponse {
        panic!("info is outside this check-only test")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_and_grpc_share_workers_after_callers_cancel() {
    let core = Arc::new(HeldCore {
        started: AtomicUsize::new(0),
        release: AtomicBool::new(false),
    });
    let gate = Admission::default();
    let tokens = Arc::new(AuthTokens::new("check", "publish"));
    let mut http =
        imboy_textguard::api::http::router_with_runtime(core.clone(), tokens.clone(), gate.clone());
    let grpc =
        imboy_textguard::api::grpc::TextGuardSvc::with_runtime(core.clone(), tokens, gate.clone());
    let http_request = axum::http::Request::post("/v1/check").header("authorization", "check")
        .body(axum::body::Body::from(r#"{"request_id":"shared-http","scene":"channel_message","fields":[{"field_id":"body","text":"hold"}]}"#)).unwrap();
    let http_request = tokio::spawn(async move { http.call(http_request).await });
    let mut grpc_request = tonic::Request::new(CheckTextRequest {
        request_id: Some("shared-grpc".to_owned()),
        scene: Some(1),
        fields: vec![TextField {
            field_id: Some("body".to_owned()),
            text: Some("hold".to_owned()),
        }],
    });
    grpc_request
        .metadata_mut()
        .insert("authorization", "check".parse().unwrap());
    let grpc_request = tokio::spawn(async move { grpc.check_text(grpc_request).await });
    for _ in 0..100 {
        if core.started.load(Ordering::SeqCst) == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(core.started.load(Ordering::SeqCst), 2);
    http_request.abort();
    grpc_request.abort();
    let _ = http_request.await;
    let _ = grpc_request.await;
    assert_eq!(gate.counts().running, 2);
    assert_eq!(gate.counts().cancelled_still_running, 2);
    assert_eq!(
        gate.execute(
            Instant::now() + Duration::from_millis(80),
            |_| -> Result<(), ErrorCode> { panic!("third worker must not start") }
        )
        .await,
        Err(ErrorCode::Overloaded)
    );
    core.release.store(true, Ordering::SeqCst);
    for _ in 0..100 {
        if gate.counts().running == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(gate.counts().running, 0);
}

#[tokio::test]
async fn an_expired_absolute_transport_deadline_never_starts_the_core() {
    let core = Arc::new(HeldCore {
        started: AtomicUsize::new(0),
        release: AtomicBool::new(true),
    });
    let grpc = imboy_textguard::api::grpc::TextGuardSvc::new(
        core.clone(),
        Arc::new(AuthTokens::new("check", "publish")),
    );
    let mut request = tonic::Request::new(CheckTextRequest {
        request_id: Some("expired-before-decode".into()),
        scene: Some(1),
        fields: vec![TextField {
            field_id: Some("body".into()),
            text: Some("hold".into()),
        }],
    });
    request
        .metadata_mut()
        .insert("authorization", "check".parse().unwrap());
    request
        .metadata_mut()
        .insert("grpc-timeout", "5m".parse().unwrap());
    request
        .extensions_mut()
        .insert(Instant::now() - Duration::from_millis(1));
    assert_eq!(
        grpc.check_text(request).await.unwrap_err().code(),
        tonic::Code::DeadlineExceeded
    );
    assert_eq!(core.started.load(Ordering::SeqCst), 0);
}
