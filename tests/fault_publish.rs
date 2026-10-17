//! T22 acceptance tests: injected publication faults at every commit
//! boundary (AC-067..AC-071, plan §9 / task card T22).
//!
//! These tests require the `fault-injection` feature and drive
//! [`RulesState`] directly, mirroring the store-level style of
//! `tests/rules_publish.rs`. Process-level kill/restart scenarios live in
//! `tests/fault_process.rs`; this file proves the in-process state
//! machine response at each boundary:
//!
//! * pre-commit failures leave generation, revision and the on-disk
//!   ACTIVE manifest completely unchanged and retryable (AC-067),
//! * a post-rename durability failure flips the store to an explicit
//!   unknown state that only a reload can resolve (AC-068),
//! * at least 100 successful updates plus one injection at *every*
//!   commit boundary (T22 verification requirement),
//! * a corrupted or tampered ACTIVE never recovers by scanning
//!   `packages/` for the newest file (AC-070),
//! * bounded retention and explicit RetiredSnapshotBusy while a long
//!   in-flight request holds the previous snapshot (AC-071).

#![cfg(feature = "fault-injection")]

use imboy_textguard::model::ErrorCode;
use imboy_textguard::rules::fault::{self, Action, Point};
use imboy_textguard::rules::{PublishError, RulesState, StoreStatus};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

/// The fault seam is process-global; tests in this binary must not
/// install rules concurrently. Every test takes this lock for its whole
/// body (a minimal in-file serial_test).
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const TABLE_ID: &str = "test-t2s-v1";

#[path = "support/publication.rs"]
#[allow(dead_code)]
mod publication;
use publication::*;

struct TempDirGuard(std::path::PathBuf);

impl TempDirGuard {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "textguard-t22-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        fault::clear();
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fresh_store(tag: &str) -> (TempDirGuard, RulesState) {
    let dir = TempDirGuard::new(tag);
    let state = RulesState::load(dir.path(), table()).unwrap();
    (dir, state)
}

/// Publish one generation holding a valid CAS token.
fn publish_next(
    state: &RulesState,
    seed: usize,
) -> Result<imboy_textguard::rules::PublishReceipt, PublishError> {
    let etag = state.active_state().unwrap().1;
    state.publish(&big_ruleset(seed), Some(&etag))
}

fn manifest_on_disk(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap()
}

fn package_file_count(dir: &Path) -> usize {
    fs::read_dir(dir.join("packages")).unwrap().count()
}

// ------------------------------------------------------------------ AC-067

/// Every pre-commit boundary fails closed: generation and the on-disk
/// ACTIVE stay at the previous publication, staging is cleaned, and the
/// very same CAS request succeeds once the fault clears.
#[test]
fn ac067_each_pre_commit_boundary_keeps_previous_state_retryable() {
    let _serial = serial();
    let pre_points = [
        Point::PackageStage,
        Point::PackageRename,
        Point::PackageDirSync,
        Point::HistoryMarker,
        Point::ManifestStage,
        Point::ManifestRename,
    ];
    for point in pre_points {
        let (dir, state) = fresh_store("ac067");
        let first = publish_next(&state, 1).unwrap();
        assert_eq!(first.generation, 1);

        fault::install(vec![(point, Action::Fail(std::io::ErrorKind::StorageFull))]);
        let error = publish_next(&state, 2).unwrap_err();
        assert!(
            matches!(error, PublishError::Io(ref io) if io.kind() == std::io::ErrorKind::StorageFull),
            "point {point:?} must surface as a pre-commit Io failure, got {error:?}"
        );

        // previous publication is fully intact
        let current = state.current().expect("previous snapshot still served");
        assert_eq!(current.generation(), 1);
        assert_eq!(current.revision(), first.revision);
        let manifest = manifest_on_disk(dir.path());
        assert_eq!(manifest["generation"], 1);
        assert_eq!(manifest["package_digest"], first.digest);
        // no staging leftovers from the failed attempt
        assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
        // the store is still Ready for checks
        assert!(state.ready());

        // retry the identical CAS request once the fault is consumed
        let second = publish_next(&state, 2).unwrap();
        assert_eq!(second.generation, 2);
        assert!(!second.already_active);
        assert_eq!(manifest_on_disk(dir.path())["generation"], 2);
    }
}

/// A failure injected while another boundary already consumed its rule
/// proves rules are consumed exactly once (the second publish succeeds).
#[test]
fn ac067_injection_consumed_once_then_normal_service_resumes() {
    let _serial = serial();
    let (dir, state) = fresh_store("ac067-once");
    publish_next(&state, 1).unwrap();
    fault::install(vec![(
        Point::ManifestStage,
        Action::Fail(std::io::ErrorKind::PermissionDenied),
    )]);
    assert!(publish_next(&state, 2).is_err());
    // the rule is gone: two further publishes succeed normally
    assert_eq!(publish_next(&state, 2).unwrap().generation, 2);
    assert_eq!(publish_next(&state, 3).unwrap().generation, 3);
    assert_eq!(manifest_on_disk(dir.path())["generation"], 3);
    assert_eq!(package_file_count(dir.path()), 2); // current + previous retention
}

// ------------------------------------------------------------------ AC-068

/// A post-rename directory-fsync failure is reported as UnknownState and
/// the store fails closed: no snapshot is served, status is Failed, and
/// even the management active query refuses to claim a known state. The
/// on-disk manifest *has* moved to the new generation (rename happened);
/// only a fresh load (process restart) can resolve the uncertainty.
#[test]
fn ac068_post_rename_durability_failure_fails_closed_until_reload() {
    let _serial = serial();
    let (dir, state) = fresh_store("ac068");
    publish_next(&state, 1).unwrap();

    fault::install(vec![(
        Point::ManifestDirSync,
        Action::Fail(std::io::ErrorKind::Other),
    )]);
    let error = publish_next(&state, 2).unwrap_err();
    match &error {
        PublishError::UnknownState(reason) => {
            assert!(reason.contains("fsync") || reason.contains("durability"));
        }
        other => panic!("expected UnknownState, got {other:?}"),
    }
    assert_eq!(error.error_code(), ErrorCode::PublishStateUnknown);

    // fail closed: nothing is served, status says why
    assert!(state.current().is_none());
    assert!(!state.ready());
    assert!(
        matches!(state.status(), StoreStatus::Failed(ref reason) if reason.contains("unknown"))
    );
    assert!(matches!(
        state.active_state(),
        Err(PublishError::UnknownState(_))
    ));
    // a further publish cannot even CAS from an unknown state
    assert!(matches!(
        state.publish(&big_ruleset(3), Some("any")),
        Err(PublishError::UnknownState(_))
    ));

    // the rename did land on disk: the manifest already describes gen 2.
    // Recovery is a reload (same process identity), which must resolve to
    // exactly the on-disk facts — no rollback, no duplicated generation.
    let manifest = manifest_on_disk(dir.path());
    assert_eq!(manifest["generation"], 2);
    let reloaded = RulesState::load_with_instance(
        dir.path().to_owned(),
        table(),
        state.instance_id().to_owned(),
    )
    .unwrap();
    let current = reloaded.current().expect("reload resolves the uncertainty");
    assert_eq!(current.generation(), 2);
    assert!(reloaded.ready());
    // the next publish continues from the recovered generation
    let etag = reloaded.active_state().unwrap().1;
    let next = reloaded.publish(&big_ruleset(3), Some(&etag)).unwrap();
    assert_eq!(next.generation, 3);
    assert_eq!(manifest_on_disk(dir.path())["generation"], 3);
}

// ------------------------------------------------------------------ AC-070

/// A tampered ACTIVE (truncated JSON, foreign digest, or mismatched
/// package bytes) keeps the store NOT READY after a restart-style reload
/// even though newer packages exist on disk: recovery never scans
/// `packages/` for the newest file.
#[test]
fn ac070_tampered_active_never_recovers_by_scanning_packages() {
    let _serial = serial();
    for tamper in ["truncate", "wrong_digest", "package_bytes"] {
        let (dir, state) = fresh_store("ac070");
        publish_next(&state, 1).unwrap();
        let second = publish_next(&state, 2).unwrap();
        // keep an *orphan* newer package on disk that a scanner might pick
        let orphan = big_ruleset(99);
        let orphan_digest = imboy_textguard::rules::sha256_hex(&orphan);
        fs::write(
            dir.path()
                .join("packages")
                .join(format!("{orphan_digest}.json")),
            &orphan,
        )
        .unwrap();

        match tamper {
            "truncate" => {
                let raw = fs::read(dir.path().join("manifest.json")).unwrap();
                fs::write(dir.path().join("manifest.json"), &raw[..raw.len() / 2]).unwrap();
            }
            "wrong_digest" => {
                let mut manifest = manifest_on_disk(dir.path());
                manifest["package_digest"] = serde_json::json!(format!("64{}", "0".repeat(62)));
                fs::write(
                    dir.path().join("manifest.json"),
                    serde_json::to_vec(&manifest).unwrap(),
                )
                .unwrap();
            }
            "package_bytes" => {
                // flip the stored package so it no longer matches its digest
                let package = dir
                    .path()
                    .join("packages")
                    .join(format!("{}.json", second.digest));
                let raw = fs::read(&package).unwrap();
                let mut flipped = raw.clone();
                flipped[..8].copy_from_slice(b"tampered");
                fs::write(&package, flipped).unwrap();
            }
            _ => unreachable!(),
        }

        let reloaded = RulesState::load_with_instance(
            dir.path().to_owned(),
            table(),
            state.instance_id().to_owned(),
        )
        .unwrap();
        assert!(
            !reloaded.ready(),
            "{tamper}: store must stay NOT READY, got {:?}",
            reloaded.status()
        );
        assert!(matches!(reloaded.status(), StoreStatus::Failed(_)));
        // the orphan package never became active
        assert!(reloaded.current().is_none());
        // and the store still refuses to publish from an unverified base
        assert!(reloader_publish_rejected(&reloaded));
    }
}

fn reloader_publish_rejected(state: &RulesState) -> bool {
    matches!(
        state.publish(&big_ruleset(50), Some("any-etag")),
        Err(PublishError::UnknownState(_))
    )
}

// ------------------------------------------------------------------ AC-071

/// A long in-flight request holds the gen-1 snapshot across a gen-2
/// publication (RCU: readers finish on the old snapshot). While gen-1 is
/// still strongly held, the *next* publication is rejected explicitly
/// with RetiredSnapshotBusy — no unbounded chain of retired generations;
/// once the holder finishes, publication resumes and retention keeps at
/// most the active and previous packages.
#[test]
fn ac071_retired_snapshot_busy_and_bounded_retention() {
    let _serial = serial();
    let (dir, state) = fresh_store("ac071");
    publish_next(&state, 1).unwrap();

    // long in-flight check grabs the gen-1 snapshot
    let held = state.current().unwrap();
    assert_eq!(held.generation(), 1);

    // gen-2 publishes fine while the reader still runs on gen-1 (RCU)
    assert_eq!(publish_next(&state, 2).unwrap().generation, 2);
    assert_eq!(held.generation(), 1); // the reader keeps its consistent view
    assert_eq!(state.current().unwrap().generation(), 2);

    // gen-3 must be rejected while the retired gen-1 is still held:
    // otherwise retired generations would chain without bound
    let error = publish_next(&state, 3).unwrap_err();
    assert!(matches!(error, PublishError::RetiredSnapshotBusy));
    assert_eq!(error.error_code(), ErrorCode::RetiredSnapshotBusy);
    assert_eq!(state.current().unwrap().generation(), 2); // unchanged

    drop(held); // in-flight request completes
    assert_eq!(publish_next(&state, 3).unwrap().generation, 3);

    // retention bound: only current + previous packages survive
    assert!(
        package_file_count(dir.path()) <= 2,
        "retention must stay bounded"
    );
    assert_eq!(manifest_on_disk(dir.path())["generation"], 3);
}

/// One hundred successful updates with a rotating fault injected at every
/// commit boundary (T22: "at least 100 successful updates and an
/// injection at every commit boundary"): each fault fails once, is
/// retried, and the sequence reaches generation 100 with bounded storage.
#[test]
fn t22_hundred_successful_updates_with_boundary_injections() {
    let _serial = serial();
    let (dir, state) = fresh_store("t22-hundred");
    let boundaries = [
        Point::PackageStage,
        Point::PackageRename,
        Point::PackageDirSync,
        Point::HistoryMarker,
        Point::ManifestStage,
        Point::ManifestRename,
    ];
    // seed generation 1 so CAS always has a base
    assert_eq!(publish_next(&state, 0).unwrap().generation, 1);

    let mut injected_at = vec![];
    let mut generation = 1u64;
    for step in 1..=100usize {
        let point = boundaries[step % boundaries.len()];
        fault::install(vec![(point, Action::Fail(std::io::ErrorKind::Other))]);
        // the faulted attempt must fail without advancing anything
        let before = manifest_on_disk(dir.path());
        if publish_next(&state, step).is_err() {
            injected_at.push(point);
            let after = manifest_on_disk(dir.path());
            assert_eq!(before, after, "fault at {point:?} changed the manifest");
            assert!(state.ready());
        }
        // the retry (fault consumed) succeeds and advances exactly once
        let receipt = publish_next(&state, step).unwrap();
        assert_eq!(receipt.generation, generation + 1, "step {step}");
        generation += 1;
    }
    assert_eq!(generation, 101); // seed + 100 updates
                                 // every boundary was exercised by at least one injection
    for point in boundaries {
        assert!(
            injected_at.contains(&point),
            "boundary {point:?} never fired an injection"
        );
    }
    // bounded retention after 100 publications
    assert!(package_file_count(dir.path()) <= 2);
    assert_eq!(manifest_on_disk(dir.path())["generation"], 101);
    // final state serves checks under the last published revision
    let current = state.current().unwrap();
    assert_eq!(current.generation(), 101);
}
