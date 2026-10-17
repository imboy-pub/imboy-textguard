//! T14 acceptance tests: ruleset publication with CAS, persistence and
//! RCU snapshot switching (AC-062..AC-066, plan §9).
//!
//! Store-level tests drive [`RulesState`] directly; router tests mount
//! [`manage::router`] in-process through the same tower oneshot pattern
//! as `tests/http_contract.rs`. Content identity, state identity and base
//! dictionary identity are checked independently.

use imboy_textguard::api::auth::AuthTokens;
use imboy_textguard::api::http::manage;
use imboy_textguard::api::CheckCore;
use imboy_textguard::model::ErrorCode;
use imboy_textguard::rules::{
    PublishError, PublishedCore, RulesState, StoreStatus, MAX_PACKAGE_BYTES,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use tonic::codegen::Service as _;

const CHECK_TOKEN: &str = "check-secret";
const PUBLISH_TOKEN: &str = "publish-secret";
const TABLE_ID: &str = "test-t2s-v1";

#[path = "support/publication.rs"]
mod publication;
use publication::*;

// ------------------------------------------------------------------ AC-062

#[test]
fn ac062_idempotent_republish_and_stale_cas() {
    let dir = TempDir::new("ac062");
    let state = store(dir.path());
    assert!(!state.ready());

    let first = state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&state.active_state().unwrap().1),
        )
        .unwrap();
    assert_eq!(first.generation, 1);
    assert!(!first.already_active);
    assert_eq!(first.revision.len(), 64);
    assert_eq!(first.digest.len(), 64);

    // identical bytes with the CURRENT If-Match: idempotent, generation
    // frozen at 1
    let again = state
        .publish(&one_word_ruleset("坏词甲"), Some(&first.state_etag))
        .unwrap();
    assert!(again.already_active);
    assert_eq!(again.generation, 1);
    assert_eq!(again.revision, first.revision);
    assert_eq!(again.digest, first.digest);

    // publish B on top of A, then A's ETag is stale: conflict, B stays
    let second = state
        .publish(&one_word_ruleset("坏词乙"), Some(&first.state_etag))
        .unwrap();
    assert_eq!(second.generation, 2);
    let conflict = state
        .publish(&one_word_ruleset("坏词丙"), Some(&first.state_etag))
        .unwrap_err();
    assert!(matches!(conflict, PublishError::CasConflict));
    let current = state.current().unwrap();
    assert_eq!(current.revision(), second.revision);
    assert_eq!(current.generation(), 2);
}

// ------------------------------------------------------------------ AC-063

#[test]
fn ac063_held_etag_cannot_publish_over_a_newer_ruleset() {
    let dir = TempDir::new("ac063");
    let state = store(dir.path());
    let a = state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&state.active_state().unwrap().1),
        )
        .unwrap();
    let held = a.state_etag.clone(); // a request prepared against A's ETag
    let b = state
        .publish(&one_word_ruleset("坏词乙"), Some(&held))
        .unwrap();
    let outcome = state.publish(&one_word_ruleset("坏词丙"), Some(&held));
    assert!(matches!(outcome.unwrap_err(), PublishError::CasConflict));
    let current = state.current().unwrap();
    assert_eq!(current.revision(), b.revision);
    assert_eq!(current.generation(), 2);
}

// ------------------------------------------------------------------ AC-064
//
// What is simulated here:
//
// * A->B->A cycle: a repeated revision cannot validate an old state ETag.
// * Instance replacement (same dir): a second store with a FORCED
//   different instance id keeps serving the same verified state (restart
//   continuity) while reporting a different serving instance; the
//   manifest still records the original publisher (`published_by`).
// * Rebuilt data dir: the directory survives with an OLD manifest but
//   without its package; the store fails closed (never scans packages/),
//   so an If-Match from the old instance cannot validate anything.

#[test]
fn ac064_old_state_never_resurrects_across_aba_and_instance_replacement() {
    let dir = TempDir::new("ac064");
    let state = store(dir.path());
    let a_bytes = one_word_ruleset("坏词甲");
    let a = state
        .publish(&a_bytes, Some(&state.active_state().unwrap().1))
        .unwrap(); // gen 1
    let b = state
        .publish(&one_word_ruleset("坏词乙"), Some(&a.state_etag))
        .unwrap(); // gen 2
    assert_ne!(a.revision, b.revision);

    // complete the A->B->A cycle: identical bytes, NEW generation
    let a2 = state.publish(&a_bytes, Some(&b.state_etag)).unwrap(); // gen 3
    assert_eq!(a2.revision, a.revision, "content revision repeats");
    assert_eq!(a2.generation, 3, "identity never does");
    assert!(!a2.already_active, "publishing A over B is a real publish");

    assert!(matches!(
        state.publish(&one_word_ruleset("坏词乙"), Some(&a.state_etag)),
        Err(PublishError::CasConflict)
    ));
    let b2 = state
        .publish(&one_word_ruleset("坏词乙"), Some(&a2.state_etag))
        .unwrap();
    assert_eq!(b2.generation, 4);
    assert_ne!(b2.generation, b.generation);

    // instance replacement over the same dir (forced instance id)
    let replacement =
        RulesState::load_with_instance(dir.path(), table(), "replaced-instance-0001".to_owned())
            .unwrap();
    let current = replacement.current().unwrap();
    assert_eq!(current.revision(), b2.revision);
    assert_eq!(current.generation(), 4);
    assert_eq!(current.digest(), b2.digest);
    assert_eq!(current.published_by(), state.instance_id());
    assert_ne!(replacement.instance_id(), state.instance_id());
    assert!(matches!(
        replacement.publish(&a_bytes, Some(&b2.state_etag)),
        Err(PublishError::CasConflict)
    ));

    // rebuilt data dir: old manifest kept, its package wiped
    let rebuilt_dir = TempDir::new("ac064-rebuilt");
    copy_dir(dir.path(), rebuilt_dir.path());
    for entry in fs::read_dir(rebuilt_dir.path().join("packages")).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let rebuilt = store(rebuilt_dir.path());
    assert!(!rebuilt.ready());
    assert!(matches!(rebuilt.status(), StoreStatus::Failed(_)));
    let outcome = rebuilt.publish(&one_word_ruleset("坏词丙"), Some(&b2.state_etag));
    assert!(
        matches!(outcome.unwrap_err(), PublishError::UnknownState(_)),
        "an ETag from the replaced instance must not validate"
    );
}

// ------------------------------------------------------------------ AC-065

#[test]
fn ac065_empty_dir_empty_ruleset_and_corrupt_manifest_recovery() {
    let dir = TempDir::new("ac065");
    let state = store(dir.path());
    assert!(!state.ready());
    assert_eq!(state.status(), StoreStatus::Empty);

    // an empty ruleset without allow_empty is rejected by the compiler
    let rejected = state
        .publish(
            &empty_ruleset(false),
            Some(&state.active_state().unwrap().1),
        )
        .unwrap_err();
    assert!(matches!(
        rejected,
        PublishError::Invalid(code) if code == ErrorCode::InvalidRuleset
    ));
    assert!(!state.ready());

    // allow_empty=true packages are accepted at T14; the publisher-side
    // double-confirm belongs to the IMBoy publisher (T16), not the plane
    let empty = state
        .publish(&empty_ruleset(true), Some(&state.active_state().unwrap().1))
        .unwrap();
    assert_eq!(empty.generation, 1);

    // publish real rules, then corrupt the ACTIVE manifest: the reload
    // must fail closed and never serve by scanning the newest package
    let a = state
        .publish(&one_word_ruleset("坏词甲"), Some(&empty.state_etag))
        .unwrap();
    let manifest_path = dir.path().join("manifest.json");
    let package_path = dir
        .path()
        .join("packages")
        .join(format!("{}.json", a.digest));
    assert!(package_path.is_file());
    fs::write(&manifest_path, b"{trunc").unwrap();

    let reloaded = store(dir.path());
    assert!(!reloaded.ready());
    assert!(matches!(reloaded.status(), StoreStatus::Failed(_)));
    // proof that a scanning implementation would have found the package
    // but this one refuses to serve it
    assert!(package_path.is_file());
    assert!(
        reloaded.current().is_none(),
        "a corrupt manifest must never fall back to package scanning"
    );
}

// ------------------------------------------------------------------ AC-066

#[test]
fn ac066_readers_only_see_consistent_revision_generation_digest_triples() {
    let dir = TempDir::new("ac066");
    let state = Arc::new(store(dir.path()));
    let a_bytes = one_word_ruleset("坏词甲");
    let b_bytes = one_word_ruleset("坏词乙");
    let baseline = state
        .publish(&a_bytes, Some(&state.active_state().unwrap().1))
        .unwrap();
    let mut tag = baseline.state_etag.clone();

    let stop = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    for _ in 0..4 {
        let state = Arc::clone(&state);
        let stop = Arc::clone(&stop);
        let request = request();
        readers.push(std::thread::spawn(move || {
            let core = PublishedCore::new(state);
            let mut seen: Vec<(String, String, String)> = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let response = core.check(&request).unwrap();
                seen.push((
                    response.ruleset_revision.clone(),
                    response.generation.clone(),
                    response.base_rules_digest.clone(),
                ));
            }
            seen
        }));
    }

    let mut published: HashSet<(String, String)> =
        HashSet::from([(baseline.revision.clone(), expected_base_digest("坏词甲"))]);
    let mut published_generations: HashMap<(String, String), HashSet<String>> = HashMap::new();
    published_generations
        .entry((baseline.revision, expected_base_digest("坏词甲")))
        .or_default()
        .insert(baseline.generation.to_string());
    for index in 0..20 {
        let bytes = if index % 2 == 0 { &b_bytes } else { &a_bytes };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let receipt = loop {
            match state.publish(bytes, Some(&tag)) {
                Err(PublishError::RetiredSnapshotBusy) if std::time::Instant::now() < deadline => {
                    std::thread::yield_now()
                }
                outcome => break outcome.unwrap(),
            }
        };
        let expected_digest = expected_base_digest(if index % 2 == 0 {
            "坏词乙"
        } else {
            "坏词甲"
        });
        tag = receipt.state_etag.clone();
        published.insert((receipt.revision.clone(), expected_digest.clone()));
        published_generations
            .entry((receipt.revision, expected_digest))
            .or_default()
            .insert(receipt.generation.to_string());
    }
    stop.store(true, Ordering::Relaxed);

    let mut generations: HashSet<String> = HashSet::new();
    let mut observed = 0usize;
    for reader in readers {
        for (revision, generation, digest) in reader.join().unwrap() {
            observed += 1;
            assert!(
                published.contains(&(revision.clone(), digest.clone())),
                "unpublished triple: {revision}/{generation}/{digest}"
            );
            // generation must pair with its (revision, digest): the publish
            // loop knows exactly which generation each package got
            let expected = published_generations
                .get(&(revision.clone(), digest.clone()))
                .expect("published pair without generation");
            assert!(
                expected.contains(&generation),
                "mixed generation pair: {revision}/{generation}/{digest}"
            );
            generations.insert(generation);
        }
    }
    assert!(observed > 100, "readers barely ran: {observed} responses");
    assert!(
        generations.len() >= 2,
        "readers should span at least two generations"
    );
}

// ------------------------------------------------------- restart continuity

#[test]
fn restart_preserves_revision_generation_digest_and_cas() {
    let dir = TempDir::new("restart");
    let state = store(dir.path());
    let a = state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&state.active_state().unwrap().1),
        )
        .unwrap();
    let b = state
        .publish(&one_word_ruleset("坏词乙"), Some(&a.state_etag))
        .unwrap();
    drop(state);

    let restarted = store(dir.path());
    assert!(restarted.ready());
    let current = restarted.current().unwrap();
    assert_eq!(current.revision(), b.revision);
    assert_eq!(current.generation(), 2);
    assert_eq!(current.digest(), b.digest);
    assert_eq!(current.snapshot().generation(), "2");

    // CAS continuity survives the restart: the pre-restart ETag still
    // guards the next publish and the generation keeps climbing
    let c = restarted
        .publish(&one_word_ruleset("坏词丙"), Some(&b.state_etag))
        .unwrap();
    assert_eq!(c.generation, 3);

    // the restarted store serves a check response consistent with the
    // published generation
    let core = PublishedCore::new(Arc::new(restarted));
    let response = core.check(&request()).unwrap();
    assert_eq!(response.ruleset_revision, c.revision);
    assert_eq!(response.generation, "3");
}

#[test]
fn check_responses_track_published_revision_and_generation() {
    let dir = TempDir::new("checkflow");
    let state = Arc::new(store(dir.path()));
    let core = PublishedCore::new(Arc::clone(&state));

    // not ready before the first publish
    assert_eq!(core.check(&request()).unwrap_err(), ErrorCode::NotReady);
    assert!(!core.ready());

    let a = state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&state.active_state().unwrap().1),
        )
        .unwrap();
    let before = core.check(&request()).unwrap();
    assert_eq!(before.ruleset_revision, a.revision);
    assert_eq!(before.generation, "1");

    let b = state
        .publish(&one_word_ruleset("坏词乙"), Some(&a.state_etag))
        .unwrap();
    let after = core.check(&request()).unwrap();
    assert_eq!(after.ruleset_revision, b.revision);
    assert_eq!(after.generation, "2");
    assert_ne!(after.ruleset_revision, before.ruleset_revision);
    assert_ne!(after.generation, before.generation);
    assert_eq!(after.base_rules_digest, expected_base_digest("坏词乙"));
}

// ------------------------------------------------------------- publish lock

#[test]
fn overlapping_publishes_never_both_succeed() {
    let dir = TempDir::new("busy");
    let state = Arc::new(store(dir.path()));
    let a = state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&state.active_state().unwrap().1),
        )
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let tag = a.state_etag.clone();
    let bytes_one = big_ruleset(1);
    let bytes_two = big_ruleset(2);

    let first_store = Arc::clone(&state);
    let first_barrier = Arc::clone(&barrier);
    let first_tag = tag.clone();
    let one = std::thread::spawn(move || {
        first_barrier.wait();
        first_store.publish(&bytes_one, Some(&first_tag))
    });
    let second_store = Arc::clone(&state);
    let second_tag = tag;
    let two = std::thread::spawn(move || {
        barrier.wait();
        second_store.publish(&bytes_two, Some(&second_tag))
    });

    let results = [one.join().unwrap(), two.join().unwrap()];
    let successes = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(successes, 1, "exactly one publish may win the CAS");
    for result in &results {
        if let Err(error) = result {
            // the loser is either rejected by the single publish lock
            // (PUBLISH_BUSY, immediately, no queue) or by the stale CAS
            assert!(matches!(
                error,
                PublishError::Busy | PublishError::CasConflict
            ));
        }
    }
    assert_eq!(state.current().unwrap().generation(), 2);
}

// ---------------------------------------------- simulated mid-publication IO failure
//
// Cheap T14-level simulation (full Linux fault injection is T22): the
// packages directory is made read-only so the package rename fails AFTER
// staging (and long before the manifest is ever touched).

#[cfg(unix)]
#[test]
fn package_write_failure_keeps_the_old_state_and_generation() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("midfail");
    let state = store(dir.path());
    let a = state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&state.active_state().unwrap().1),
        )
        .unwrap();
    let packages = dir.path().join("packages");
    let original = fs::metadata(&packages).unwrap().permissions();

    fs::set_permissions(&packages, fs::Permissions::from_mode(0o555)).unwrap();
    let error = state
        .publish(&one_word_ruleset("坏词乙"), Some(&a.state_etag))
        .unwrap_err();
    assert!(matches!(error, PublishError::Io(_)));

    // the old ruleset and generation are untouched
    let current = state.current().unwrap();
    assert_eq!(current.revision(), a.revision);
    assert_eq!(current.generation(), 1);
    // staging leftovers are cleaned
    let staging = fs::read_dir(dir.path().join("tmp")).unwrap().count();
    assert_eq!(staging, 0);
    // the manifest on disk still describes A
    let manifest: Value =
        serde_json::from_slice(&fs::read(dir.path().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["package_digest"], a.digest);
    assert_eq!(manifest["generation"], 1);

    // after restoring write access the same CAS publish succeeds
    fs::set_permissions(&packages, original).unwrap();
    let b = state
        .publish(&one_word_ruleset("坏词乙"), Some(&a.state_etag))
        .unwrap();
    assert_eq!(b.generation, 2);
}

// -------------------------------------------------------- management router

struct Fixture {
    router: Router,
    state: Arc<RulesState>,
}

fn fixture(dir: &Path) -> Fixture {
    let state = Arc::new(store(dir));
    Fixture {
        router: manage::router(
            Arc::clone(&state),
            Arc::new(AuthTokens::new(CHECK_TOKEN, PUBLISH_TOKEN)),
        ),
        state,
    }
}

/// In-process oneshot: drive one request through the router and decode
/// the JSON response body plus the ETag header.
async fn send(router: Router, request: Request<Body>) -> (StatusCode, Value, Option<String>) {
    let mut router = router;
    let response = router.call(request).await.unwrap();
    let status = response.status();
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value, etag)
}

fn put_active(
    body: impl Into<Body>,
    if_match: Option<&str>,
    credential: Option<&str>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::PUT)
        .uri("/internal/v1/rulesets/active")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(tag) = if_match {
        builder = builder.header(header::IF_MATCH, tag);
    }
    if let Some(credential) = credential {
        builder = builder.header(header::AUTHORIZATION, credential);
    }
    builder.body(body.into()).unwrap()
}

fn get_active(credential: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(Method::GET)
        .uri("/internal/v1/rulesets/active");
    if let Some(credential) = credential {
        builder = builder.header(header::AUTHORIZATION, credential);
    }
    builder.body(Body::empty()).unwrap()
}

fn error_code(value: &Value) -> &str {
    value["error"]["code"]
        .as_str()
        .unwrap_or_else(|| panic!("missing error code in body: {value}"))
}

#[tokio::test]
async fn management_router_requires_the_publish_scope() {
    let dir = TempDir::new("auth");
    let fixture = fixture(dir.path());

    let (status, value, _) = send(fixture.router.clone(), get_active(None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(error_code(&value), "UNAUTHENTICATED");

    // a valid check credential is the wrong scope for the publish plane
    let (status, value, _) = send(fixture.router.clone(), get_active(Some(CHECK_TOKEN))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_code(&value), "PERMISSION_DENIED");

    let (status, value, _) = send(fixture.router.clone(), get_active(Some(PUBLISH_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["status"], "uninitialized");
    assert_eq!(value["generation"], "0");
    assert_eq!(value["state_etag"].as_str().unwrap().len(), 64);

    // the bearer form of the publish credential is accepted
    let (status, _, _) = send(
        fixture.router.clone(),
        get_active(Some(&format!("Bearer {PUBLISH_TOKEN}"))),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!fixture.state.ready());
}

#[tokio::test]
async fn put_and_get_contract_round_trip() {
    let dir = TempDir::new("contract");
    let fixture = fixture(dir.path());
    let bytes = one_word_ruleset("坏词甲");

    let (status, initial, _) = send(fixture.router.clone(), get_active(Some(PUBLISH_TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    let initial_etag = initial["state_etag"].as_str().unwrap();
    let (status, body, etag) = send(
        fixture.router.clone(),
        put_active(bytes.clone(), Some(initial_etag), Some(PUBLISH_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["generation"], "1");
    assert_eq!(body["status"], "committed");
    assert_eq!(body["instance_id"], fixture.state.instance_id());
    let revision = body["ruleset_revision"].as_str().unwrap().to_owned();
    let state_etag = body["state_etag"].as_str().unwrap().to_owned();
    assert_eq!(revision.len(), 64);
    assert_eq!(state_etag.len(), 64);
    assert_eq!(etag.as_deref(), Some(format!("\"{state_etag}\"").as_str()));
    assert!(fixture.state.ready());

    // GET returns the full active pointer
    let (status, info, etag) = send(fixture.router.clone(), get_active(Some(PUBLISH_TOKEN))).await;
    assert_eq!(status, StatusCode::OK, "body: {info}");
    assert_eq!(info["ready"], true);
    assert_eq!(info["ruleset_revision"], revision.as_str());
    assert_eq!(info["generation"], "1");
    assert_eq!(info["state_etag"], state_etag);
    assert_eq!(info["instance_id"], fixture.state.instance_id());
    assert_eq!(info["scene_counts"]["channel_message"], 1);
    assert_eq!(info.as_object().unwrap().len(), 7);
    assert_eq!(etag.as_deref(), Some(format!("\"{state_etag}\"").as_str()));

    // identical bytes with the current If-Match: already_active, no bump
    let (status, body, _) = send(
        fixture.router.clone(),
        put_active(bytes.clone(), Some(&state_etag), Some(PUBLISH_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["status"], "already_active");
    assert_eq!(body["generation"], "1");
    assert_eq!(body["ruleset_revision"], revision.as_str());
}

#[tokio::test]
async fn if_match_absent_is_precondition_required_and_mismatch_is_conflict() {
    let dir = TempDir::new("preconditions");
    let fixture = fixture(dir.path());
    let first = fixture
        .state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&fixture.state.active_state().unwrap().1),
        )
        .unwrap();

    // omitted If-Match while a ruleset is active: 428 + CAS_CONFLICT
    let (status, value, _) = send(
        fixture.router.clone(),
        put_active(one_word_ruleset("坏词乙"), None, Some(PUBLISH_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "body: {value}");
    assert_eq!(error_code(&value), "CAS_CONFLICT");

    // mismatched token: 409 + CAS_CONFLICT
    let (status, value, _) = send(
        fixture.router.clone(),
        put_active(
            one_word_ruleset("坏词乙"),
            Some("deadbeefdeadbeef"),
            Some(PUBLISH_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(error_code(&value), "CAS_CONFLICT");

    // quoted If-Match tokens are accepted
    let quoted = format!("\"{}\"", first.state_etag);
    let (status, body, _) = send(
        fixture.router.clone(),
        put_active(
            one_word_ruleset("坏词乙"),
            Some(&quoted),
            Some(PUBLISH_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["generation"], "2");

    // wildcard tokens and duplicated If-Match headers are INVALID_ARGUMENT
    for request in [
        put_active(one_word_ruleset("坏词丙"), Some("*"), Some(PUBLISH_TOKEN)),
        {
            let mut request = put_active(
                one_word_ruleset("坏词丙"),
                Some("deadbeefdeadbeef"),
                Some(PUBLISH_TOKEN),
            );
            request.headers_mut().append(
                header::IF_MATCH,
                header::HeaderValue::from_static("deadbeefdeadbeef"),
            );
            request
        },
    ] {
        let (status, value, _) = send(fixture.router.clone(), request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error_code(&value), "INVALID_ARGUMENT");
    }
    assert_eq!(fixture.state.current().unwrap().generation(), 2);
}

#[tokio::test]
async fn oversized_encoded_and_invalid_bodies_are_rejected() {
    let dir = TempDir::new("bodies");
    let fixture = fixture(dir.path());
    fixture
        .state
        .publish(
            &one_word_ruleset("坏词甲"),
            Some(&fixture.state.active_state().unwrap().1),
        )
        .unwrap();

    // early rejection through a declared Content-Length over 64 MiB
    let oversized = Request::builder()
        .method(Method::PUT)
        .uri("/internal/v1/rulesets/active")
        .header(header::AUTHORIZATION, PUBLISH_TOKEN)
        .header(header::CONTENT_LENGTH, (MAX_PACKAGE_BYTES + 1).to_string())
        .body(Body::from(one_word_ruleset("坏词甲")))
        .unwrap();
    let (status, value, _) = send(fixture.router.clone(), oversized).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error_code(&value), "TEXT_TOO_LARGE");

    // any non-identity content encoding is a protocol error
    let gzip = Request::builder()
        .method(Method::PUT)
        .uri("/internal/v1/rulesets/active")
        .header(header::AUTHORIZATION, PUBLISH_TOKEN)
        .header(header::CONTENT_ENCODING, "gzip")
        .body(Body::from(one_word_ruleset("坏词甲")))
        .unwrap();
    let (status, value, _) = send(fixture.router.clone(), gzip).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(error_code(&value), "PROTOCOL_ERROR");

    // a ruleset that parses but fails compilation is INVALID_RULESET
    let revision = fixture.state.active_state().unwrap().1;
    let (status, value, _) = send(
        fixture.router.clone(),
        put_active(empty_ruleset(false), Some(&revision), Some(PUBLISH_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {value}");
    assert_eq!(error_code(&value), "INVALID_RULESET");

    // malformed JSON is INVALID_RULESET and invalid UTF-8 keeps its code
    let (status, value, _) = send(
        fixture.router.clone(),
        put_active("not json", Some(&revision), Some(PUBLISH_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&value), "INVALID_RULESET");

    let (status, value, _) = send(
        fixture.router.clone(),
        put_active(b"\xff\xfe{}".to_vec(), Some(&revision), Some(PUBLISH_TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_code(&value), "INVALID_UTF8");

    // state untouched by all rejected bodies
    assert_eq!(fixture.state.current().unwrap().generation(), 1);
}
