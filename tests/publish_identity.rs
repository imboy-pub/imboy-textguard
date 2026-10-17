use imboy_textguard::normalize::table_by_id;
use imboy_textguard::rules::{PublishError, RulesState};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "textguard-publication-identity-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn open(&self) -> RulesState {
        RulesState::load_with_instance(
            &self.0,
            table_by_id("test-t2s-v1").unwrap(),
            "identity-test".to_owned(),
        )
        .unwrap()
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn package(word: &str) -> Vec<u8> {
    let mut value: Value =
        serde_json::from_str(include_str!("fixtures/contract-ruleset.json")).unwrap();
    value["literals"][0]["word"] = Value::String(word.to_owned());
    serde_json::to_vec(&value).unwrap()
}

fn token(state: &RulesState) -> String {
    let current = state.current();
    let generation = current.as_ref().map_or(0, |active| active.generation());
    let revision = current.as_ref().map_or("", |active| active.revision());
    format!(
        "{:x}",
        Sha256::digest(format!("{}\n{generation}\n{revision}", state.instance_id()))
    )
}

#[test]
fn first_publish_requires_the_observed_empty_state_token() {
    let directory = Directory::new();
    let state = directory.open();
    assert!(matches!(
        state.publish(&package("甲词"), None),
        Err(PublishError::CasConflict)
    ));
    state
        .publish(&package("甲词"), Some(&token(&state)))
        .unwrap();
}

#[tokio::test]
async fn unpublished_info_omits_snapshot_fields_on_both_transports() {
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use imboy_textguard::api::auth::AuthTokens;
    use imboy_textguard::api::grpc::TextGuardSvc;
    use imboy_textguard::protocol::text_guard_server::TextGuard as _;
    use imboy_textguard::rules::PublishedCore;
    use std::sync::Arc;
    use tonic::codegen::Service as _;

    let directory = Directory::new();
    let core = Arc::new(PublishedCore::new(Arc::new(directory.open())));
    let tokens = Arc::new(AuthTokens::new("presence-check", "presence-publish"));
    let mut http = imboy_textguard::api::http::router(core.clone(), tokens.clone());
    let request = Request::get("/v1/info")
        .header("authorization", "presence-check")
        .body(Body::empty())
        .unwrap();
    let response = http.call(request).await.unwrap();
    assert_eq!(response.status(), 200);
    let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    let info: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(info["ready"], false);
    assert_eq!(info.as_object().unwrap().len(), 4);

    let mut request = tonic::Request::new(imboy_textguard::protocol::GetInfoRequest {});
    request
        .metadata_mut()
        .insert("authorization", "presence-check".parse().unwrap());
    let info = TextGuardSvc::new(core, tokens)
        .get_info(request)
        .await
        .unwrap()
        .into_inner();
    assert_eq!(info.ready, Some(false));
    assert!(!info.instance_id.unwrap().is_empty());
    assert_eq!(info.ruleset_revision, None);
    assert_eq!(info.generation, None);
    assert_eq!(info.normalization_id, None);
    assert_eq!(info.t2s_table_id, None);
    assert_eq!(info.base_rules_digest, None);
    assert_eq!(info.canonical_count, None);
    assert_eq!(info.alias_count, None);
    assert_eq!(info.regex_count, None);
}

#[test]
fn lost_ack_retries_the_identical_package_without_a_new_generation() {
    let directory = Directory::new();
    let state = directory.open();
    let initial_etag = token(&state);
    let bytes = package("甲词");
    let committed = state.publish(&bytes, Some(&initial_etag)).unwrap();
    let retried = state.publish(&bytes, Some(&initial_etag)).unwrap();
    assert!(retried.already_active);
    assert_eq!(retried.generation, committed.generation);
    assert_eq!(retried.state_etag, committed.state_etag);
    assert!(matches!(
        state.publish(&bytes, None),
        Err(PublishError::CasConflict)
    ));
}

#[test]
fn missing_or_replaced_identity_cannot_recover_a_historical_active() {
    let directory = Directory::new();
    let table = table_by_id("test-t2s-v1").unwrap();
    let state = RulesState::load(&directory.0, table).unwrap();
    state
        .publish(&package("甲词"), Some(&token(&state)))
        .unwrap();
    let identity = state.instance_id().to_owned();
    drop(state);
    let path = directory.0.join("instance-id");
    std::fs::remove_file(&path).unwrap();
    assert!(RulesState::load(&directory.0, table).is_err());
    assert!(!path.exists());
    let replacement = if identity == "00000000000000000000000000000000" {
        "11111111111111111111111111111111"
    } else {
        "00000000000000000000000000000000"
    };
    std::fs::write(&path, replacement).unwrap();
    assert!(RulesState::load(&directory.0, table).is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), replacement);
}

#[test]
fn missing_active_with_history_cannot_recreate_generation_zero() {
    let directory = Directory::new();
    let table = table_by_id("test-t2s-v1").unwrap();
    let state = RulesState::load(&directory.0, table).unwrap();
    state
        .publish(&package("甲词"), Some(&token(&state)))
        .unwrap();
    drop(state);
    std::fs::remove_file(directory.0.join("manifest.json")).unwrap();
    for entry in std::fs::read_dir(directory.0.join("packages")).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let state = RulesState::load(&directory.0, table).unwrap();
    assert!(!state.ready());
    assert!(matches!(
        state.active_state(),
        Err(PublishError::UnknownState(_))
    ));
    assert!(matches!(
        state.publish(&package("甲词"), Some(&token(&state))),
        Err(PublishError::UnknownState(_))
    ));
}

#[test]
fn unused_directory_can_restart_without_losing_its_initial_identity() {
    let directory = Directory::new();
    let table = table_by_id("test-t2s-v1").unwrap();
    let state = RulesState::load(&directory.0, table).unwrap();
    let identity = state.instance_id().to_owned();
    let etag = state.active_state().unwrap().1;
    drop(state);
    let state = RulesState::load(&directory.0, table).unwrap();
    assert_eq!(state.instance_id(), identity);
    assert_eq!(state.active_state().unwrap().1, etag);
}

#[test]
fn aba_does_not_revalidate_an_old_state_token() {
    let directory = Directory::new();
    let state = directory.open();
    let first = package("甲词");
    state.publish(&first, Some(&token(&state))).unwrap();
    let stale = token(&state);
    state.publish(&package("乙词"), Some(&stale)).unwrap();
    state.publish(&first, Some(&token(&state))).unwrap();
    assert_ne!(token(&state), stale);
    assert!(matches!(
        state.publish(&package("丙词"), Some(&stale)),
        Err(PublishError::CasConflict)
    ));
}

#[test]
fn corrupt_active_cannot_be_overwritten_as_an_empty_store() {
    let directory = Directory::new();
    std::fs::write(directory.0.join("manifest.json"), b"corrupt").unwrap();
    let state = directory.open();
    assert!(!state.ready());
    assert!(matches!(
        state.publish(&package("甲词"), Some(&token(&state))),
        Err(PublishError::UnknownState(_))
    ));
    assert_eq!(
        std::fs::read(directory.0.join("manifest.json")).unwrap(),
        b"corrupt"
    );
}

#[test]
fn generation_overflow_is_rejected_before_any_durable_change() {
    let directory = Directory::new();
    let state = directory.open();
    state
        .publish(&package("甲词"), Some(&token(&state)))
        .unwrap();
    drop(state);
    let path = directory.0.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["generation"] = Value::from(u64::MAX);
    let before = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(&path, &before).unwrap();
    let state = directory.open();
    assert!(state
        .publish(&package("乙词"), Some(&token(&state)))
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(state.current().unwrap().generation(), u64::MAX);
}

#[test]
fn a_retained_reader_bounds_the_number_of_retired_generations() {
    let directory = Directory::new();
    let state = directory.open();
    state
        .publish(&package("甲词"), Some(&token(&state)))
        .unwrap();
    let reader = state.current().unwrap();
    state
        .publish(&package("乙词"), Some(&token(&state)))
        .unwrap();
    assert!(state
        .publish(&package("丙词"), Some(&token(&state)))
        .is_err());
    assert_eq!(state.current().unwrap().generation(), 2);
    drop(reader);
    state
        .publish(&package("丙词"), Some(&token(&state)))
        .unwrap();
    assert_eq!(state.current().unwrap().generation(), 3);
}

#[test]
fn restart_preserves_directory_identity_and_exact_package_revision() {
    let directory = Directory::new();
    let table = table_by_id("test-t2s-v1").unwrap();
    let state = RulesState::load(&directory.0, table).unwrap();
    let identity = state.instance_id().to_owned();
    let bytes = package("甲词");
    let first = state.publish(&bytes, Some(&token(&state))).unwrap();
    assert_eq!(first.revision, format!("{:x}", Sha256::digest(&bytes)));
    let base_digest = state.current().unwrap().snapshot().digest().to_owned();
    drop(state);
    let state = RulesState::load(&directory.0, table).unwrap();
    assert_eq!(state.instance_id(), identity);
    assert_eq!(state.active_state().unwrap().1, first.state_etag);
    let mut different_bytes = bytes;
    different_bytes.push(b'\n');
    let second = state
        .publish(&different_bytes, Some(&first.state_etag))
        .unwrap();
    assert_ne!(first.revision, second.revision);
    assert_eq!(
        second.revision,
        format!("{:x}", Sha256::digest(&different_bytes))
    );
    assert_eq!(state.current().unwrap().snapshot().digest(), base_digest);
}

#[test]
fn corrupt_content_addressed_package_is_never_replaced_or_activated() {
    let directory = Directory::new();
    let state = directory.open();
    let bytes = package("甲词");
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let path = directory.0.join("packages").join(format!("{digest}.json"));
    std::fs::write(&path, b"corrupt").unwrap();
    assert!(matches!(
        state.publish(&bytes, Some(&token(&state))),
        Err(PublishError::Io(_))
    ));
    assert_eq!(std::fs::read(path).unwrap(), b"corrupt");
    assert!(!state.ready());
    assert!(!directory.0.join("manifest.json").exists());
}

#[tokio::test]
async fn empty_publication_requires_both_package_flag_and_confirmation_header() {
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use imboy_textguard::api::auth::AuthTokens;
    use imboy_textguard::api::http::manage;
    use std::sync::Arc;
    use tonic::codegen::Service as _;

    let directory = Directory::new();
    let state = Arc::new(directory.open());
    let mut router = manage::router(
        Arc::clone(&state),
        Arc::new(AuthTokens::new("check-secret", "publish-secret")),
    );
    for (allow_empty, confirm, expected) in [
        (false, false, StatusCode::BAD_REQUEST),
        (true, false, StatusCode::BAD_REQUEST),
        (false, true, StatusCode::BAD_REQUEST),
        (true, true, StatusCode::OK),
    ] {
        let mut value: Value = serde_json::from_slice(&package("甲词")).unwrap();
        value["literals"] = serde_json::json!([]);
        value["regex_rules"] = serde_json::json!([]);
        value["allow_empty"] = Value::Bool(allow_empty);
        let mut builder = Request::builder()
            .method("PUT")
            .uri("/internal/v1/rulesets/active")
            .header("authorization", "publish-secret")
            .header("if-match", token(&state));
        if confirm {
            builder = builder.header("x-textguard-confirm-empty", "true");
        }
        let response = router
            .call(
                builder
                    .body(Body::from(serde_json::to_vec(&value).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        if expected == StatusCode::OK {
            assert_eq!(body["ready"], true);
            assert_eq!(body["scene_counts"]["channel_message"], 0);
        } else {
            assert_eq!(body["error"]["code"], "INVALID_RULESET");
            assert_eq!(body["error"]["request_id"], "");
            assert!(body["error"]["retryable"].is_boolean());
            assert_eq!(body["error"].as_object().unwrap().len(), 3);
            assert_eq!(body.as_object().unwrap().len(), 1);
        }
    }
}
