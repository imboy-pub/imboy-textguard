//! Ruleset publication: CAS, fsync persistence and RCU snapshot switching
//! (T14, plan §9).
//!
//! # Storage layout
//!
//! ```text
//! <root>/packages/<sha256-of-published-bytes>.json   immutable packages
//! <root>/manifest.json                               ACTIVE pointer
//! <root>/tmp/                                        staging area
//! ```
//!
//! The manifest is the single authority for what is active
//! (`schema_version, package_digest, generation, instance_id,
//! updated_at_unix, updated_at_rfc3339`). Recovery loads the manifest,
//! re-hashes the package and rebuilds the snapshot; any mismatch (or a
//! missing/unparseable manifest or package) leaves the store NOT READY —
//! recovery never falls back to scanning `packages/` for the newest file
//! (AC-065/AC-070 partial).
//!
//! # Publication semantics
//!
//! * `revision` — the ETag and the CAS token — is the first 16 hex chars
//!   of the SHA-256 of the *exact published bytes* (packages are stored
//!   byte-exact and content-addressed). The engine separately reports
//!   `base_rules_digest` (canonical serialization) in every check
//!   response; for canonically serialized packages the two digests
//!   coincide.
//! * `If-Match` must equal the active revision and may be omitted only
//!   for the initial publish into a store with no active ruleset. An
//!   omitted header while a ruleset is active is a CAS failure (surfaced
//!   as HTTP 428 by the management plane; a mismatched token as 409).
//! * Republishing byte-identical content while it is already active (with
//!   a matching `If-Match`) is an idempotent success: the receipt marks
//!   `already_active = true` and the generation does not change. The CAS
//!   check runs first, so a stale `If-Match` never gets the idempotent
//!   path.
//! * Every real publish increments the generation by exactly one.
//!   Generation is persisted in the manifest, so it survives restarts.
//!   After an A->B->A cycle the content revision repeats but the
//!   (revision, generation) identity never does: CAS never selects
//!   content and recovery never scans, so old state can never be
//!   "resurrected" by a stale ETag (AC-064).
//! * `instance_id` is fresh per process and recorded in the manifest.
//!   [`PublishedState::published_by`] reports the manifest recorder, so a
//!   restarted process serving an inherited manifest is observable: the
//!   serving `instance_id` differs from `published_by`. A rebuilt data
//!   directory (missing or unverifiable manifest/package) fails closed
//!   and an ETag from the old instance cannot validate against it.
//!
//! # Durability
//!
//! Packages and the manifest are staged in `tmp/`, written with
//! `File::sync_all`, renamed into place, then the containing directory is
//! fsynced. Directory fsyncs treat the "unsupported" errno family
//! (EBADF/EINVAL/EOPNOTSUPP/ENOSYS — macOS directory fds) as best-effort
//! skips; that gap is exactly what T22 re-verifies on Linux with fault
//! injection. A failure *before* the manifest rename leaves the previous
//! state and generation fully intact. A real durability failure *after*
//! the rename yields [`PublishError::UnknownState`]: the manifest is
//! already the ACTIVE pointer, so the error never claims "not published"
//! (AC-068 semantics; structure here, injection in T22). Retention keeps
//! at most the active and the previous package file on disk; older files
//! are removed best-effort after the in-memory swap. The retired
//! in-memory snapshot is simply dropped once readers finish — there is no
//! unbounded retention.
//!
//! # Concurrency
//!
//! One `std::sync::Mutex` serializes publications per store object: a
//! concurrent publish fails immediately with [`PublishError::Busy`],
//! there is no queueing. Readers take the current
//! `Arc<PublishedState>` from an `RwLock` and finish their whole request
//! on it, so a swap during a request can never split one response across
//! two rulesets (AC-066). At most one store per directory should publish;
//! cross-process publication into one directory is out of scope for T14.

// The management router's file location is fixed by the T14 task map
// (src/api/http/manage.rs), but the T12 HTTP module (api/http/mod.rs) is
// frozen and cannot declare new submodules, and src/lib.rs only gains
// `pub mod rules;`. The router is therefore compiled as a submodule of
// `rules` through an explicit #[path]:
#[path = "../api/http/manage.rs"]
pub mod manage;

mod store;

use crate::api::{CheckCore, InfoResponse};
use crate::engine::{BuildError, Snapshot};
use crate::model::{CheckRequest, CheckResponse, ErrorCode, RuleSet};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard, TryLockError};

/// Maximum accepted ruleset package body: 64 MiB (enforced again by the
/// management router on top of this core-level guard).
pub const MAX_PACKAGE_BYTES: usize = 64 * 1024 * 1024;

/// Schema version of the ACTIVE manifest written by this implementation.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Rule counts surfaced by the management GET endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuleCounts {
    pub canonical: usize,
    pub alias: usize,
    pub regex: usize,
}

impl RuleCounts {
    fn of(ruleset: &RuleSet) -> Self {
        Self {
            canonical: ruleset.literals.len(),
            alias: ruleset.literals.iter().map(|rule| rule.aliases.len()).sum(),
            regex: ruleset.regex_rules.len(),
        }
    }
}

/// One published, immutable serving state. Built only from a verified
/// (manifest, package) pair; shared with readers through `Arc` and never
/// mutated.
pub struct PublishedState {
    snapshot: Snapshot,
    revision: String,
    digest: String,
    generation: u64,
    published_by: String,
    updated_at_unix: u64,
    updated_at_rfc3339: String,
    t2s_table_id: String,
    counts: RuleCounts,
}

impl PublishedState {
    fn assemble(snapshot: Snapshot, ruleset: &RuleSet, manifest: &store::Manifest) -> Self {
        Self {
            snapshot,
            revision: manifest.package_digest[..16].to_owned(),
            digest: manifest.package_digest.clone(),
            generation: manifest.generation,
            published_by: manifest.instance_id.clone(),
            updated_at_unix: manifest.updated_at_unix,
            updated_at_rfc3339: manifest.updated_at_rfc3339.clone(),
            t2s_table_id: ruleset.t2s_table_id.clone(),
            counts: RuleCounts::of(ruleset),
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Publication revision: first 16 hex chars of the package byte
    /// digest; also the ETag and CAS token.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// SHA-256 of the exact published bytes (names the package file).
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Instance that wrote the ACTIVE manifest.
    pub fn published_by(&self) -> &str {
        &self.published_by
    }

    pub fn updated_at_unix(&self) -> u64 {
        self.updated_at_unix
    }

    pub fn updated_at_rfc3339(&self) -> &str {
        &self.updated_at_rfc3339
    }

    pub fn t2s_table_id(&self) -> &str {
        &self.t2s_table_id
    }

    pub fn counts(&self) -> RuleCounts {
        self.counts
    }
}

/// Load-time status of the store (see [`RulesState::status`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreStatus {
    /// An active package is loaded and served.
    Ready,
    /// The data directory holds no ACTIVE manifest (fresh or wiped).
    Empty,
    /// Manifest/package verification failed; the store stays not ready
    /// and never falls back to scanning `packages/`.
    Failed(String),
}

/// Successful publication receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReceipt {
    pub revision: String,
    pub generation: u64,
    pub digest: String,
    pub instance_id: String,
    pub already_active: bool,
}

/// Publication failure. `Io` failures always precede the manifest rename,
/// so the previous state is intact; `UnknownState` means the manifest was
/// renamed but its durability could not be confirmed.
#[derive(Debug)]
pub enum PublishError {
    Busy,
    CasConflict,
    Invalid(ErrorCode),
    Io(io::Error),
    UnknownState(&'static str),
}

impl PublishError {
    /// Stable [`ErrorCode`] for transport mapping.
    pub fn error_code(&self) -> ErrorCode {
        match self {
            Self::Busy => ErrorCode::PublishBusy,
            Self::CasConflict => ErrorCode::CasConflict,
            Self::Invalid(code) => *code,
            Self::Io(_) => ErrorCode::InternalError,
            Self::UnknownState(_) => ErrorCode::PublishStateUnknown,
        }
    }
}

/// The publication state machine plus snapshot store behind one data
/// directory. Clone the `Arc` you pass around; the store itself is shared.
pub struct RulesState {
    dir: PathBuf,
    table: &'static HashMap<char, char>,
    instance_id: String,
    current: RwLock<Option<Arc<PublishedState>>>,
    status: StoreStatus,
    publishing: Mutex<()>,
}

impl RulesState {
    /// Open (and recover) a data directory with a fresh instance id.
    pub fn load(dir: impl Into<PathBuf>, table: &'static HashMap<char, char>) -> io::Result<Self> {
        Self::load_with_instance(dir, table, store::fresh_instance_id())
    }

    /// Load with an explicit instance id. Test/recovery seam that
    /// simulates a replaced process observing a directory published by
    /// its predecessor; production code uses [`RulesState::load`].
    pub fn load_with_instance(
        dir: impl Into<PathBuf>,
        table: &'static HashMap<char, char>,
        instance_id: String,
    ) -> io::Result<Self> {
        let dir = dir.into();
        store::ensure_layout(&dir)?;
        let (current, status) = match store::load_active(&dir) {
            store::LoadOutcome::Empty => (None, StoreStatus::Empty),
            store::LoadOutcome::Corrupt(reason) => (None, StoreStatus::Failed(reason)),
            store::LoadOutcome::Active { manifest, bytes } => {
                match restore(&bytes, &manifest, table) {
                    Some(state) => (Some(Arc::new(state)), StoreStatus::Ready),
                    None => (
                        None,
                        StoreStatus::Failed("active package no longer compiles".to_owned()),
                    ),
                }
            }
        };
        Ok(Self {
            dir,
            table,
            instance_id,
            current: RwLock::new(current),
            status,
            publishing: Mutex::new(()),
        })
    }

    /// Data directory backing this store.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// This process's instance id (recorded in the manifest on publish).
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// The active published state, if one is loaded.
    pub fn current(&self) -> Option<Arc<PublishedState>> {
        // lock poisoning never invalidates a whole-state pointer: the
        // guarded Option is either complete or absent, so readers recover
        // instead of cascading a panic into every request
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// True when an active package is loaded and served.
    pub fn ready(&self) -> bool {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// Load-time outcome (recomputed to `Ready` once a publish succeeds).
    pub fn status(&self) -> StoreStatus {
        if self.ready() {
            StoreStatus::Ready
        } else {
            self.status.clone()
        }
    }

    /// Publish one ruleset package (plan §9). `bytes` are stored exactly
    /// as received; `if_match` is the CAS token. The exact semantics live
    /// in the module documentation.
    pub fn publish(
        &self,
        bytes: &[u8],
        if_match: Option<&str>,
    ) -> Result<PublishReceipt, PublishError> {
        if bytes.len() > MAX_PACKAGE_BYTES {
            return Err(PublishError::Invalid(ErrorCode::TextTooLarge));
        }
        // parse and compile up front: an invalid package must never touch
        // the publish lock or the disk
        let text = std::str::from_utf8(bytes)
            .map_err(|_| PublishError::Invalid(ErrorCode::InvalidUtf8))?;
        let ruleset: RuleSet = serde_json::from_str(text)
            .map_err(|_| PublishError::Invalid(ErrorCode::InvalidRuleset))?;
        let compiled = Snapshot::build(&ruleset, self.table)
            .map_err(|error| PublishError::Invalid(build_error_code(error)))?;

        // single-flight publication: a concurrent publish fails fast
        let _guard = match self.publishing.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(PublishError::Busy),
        };

        // CAS against the manifest-backed active state
        let current = self.current();
        match (&current, if_match) {
            (None, None) => {}
            (None, Some(_)) => return Err(PublishError::CasConflict),
            (Some(active), Some(token)) if token == active.revision() => {}
            (Some(_), _) => return Err(PublishError::CasConflict),
        }
        let digest = store::sha256_hex(bytes);
        if let Some(active) = &current {
            if active.digest() == digest {
                return Ok(PublishReceipt {
                    revision: active.revision().to_owned(),
                    generation: active.generation(),
                    digest,
                    instance_id: self.instance_id.clone(),
                    already_active: true,
                });
            }
        }

        // persist: package first, then the ACTIVE manifest, then swap
        let generation = current.as_ref().map_or(1, |active| active.generation() + 1);
        store::write_package(&self.dir, &digest, bytes).map_err(PublishError::Io)?;
        let updated_at_unix = store::unix_now();
        let manifest = store::Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            package_digest: digest.clone(),
            generation,
            instance_id: self.instance_id.clone(),
            updated_at_unix,
            updated_at_rfc3339: store::rfc3339(updated_at_unix),
        };
        let mut durability_unknown = false;
        match store::write_manifest(&self.dir, &manifest) {
            Ok(()) => {}
            Err(store::ManifestError::Pre(error)) => return Err(PublishError::Io(error)),
            Err(store::ManifestError::PostRenameDurability) => durability_unknown = true,
        }

        // RCU swap: readers hold the old Arc until they finish
        let previous = current.as_ref().map(|active| active.digest().to_owned());
        let snapshot = compiled.with_generation(&generation.to_string());
        let state = Arc::new(PublishedState::assemble(snapshot, &ruleset, &manifest));
        *self.write_current() = Some(state);
        store::cleanup_packages(
            &self.dir,
            &[digest.as_str(), previous.as_deref().unwrap_or("")],
        );
        if durability_unknown {
            return Err(PublishError::UnknownState(
                "manifest renamed but its directory fsync failed: the publication may or may not be durable",
            ));
        }
        Ok(PublishReceipt {
            revision: manifest.package_digest[..16].to_owned(),
            generation,
            digest,
            instance_id: self.instance_id.clone(),
            already_active: false,
        })
    }

    fn write_current(&self) -> RwLockWriteGuard<'_, Option<Arc<PublishedState>>> {
        self.current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Rebuild a serving state from verified package bytes plus its manifest.
fn restore(
    bytes: &[u8],
    manifest: &store::Manifest,
    table: &'static HashMap<char, char>,
) -> Option<PublishedState> {
    let ruleset: RuleSet = serde_json::from_slice(bytes).ok()?;
    let snapshot = Snapshot::build(&ruleset, table).ok()?;
    Some(PublishedState::assemble(
        snapshot.with_generation(&manifest.generation.to_string()),
        &ruleset,
        manifest,
    ))
}

fn build_error_code(error: BuildError) -> ErrorCode {
    match error {
        BuildError::Matchers(error) => ErrorCode::from(error),
        BuildError::Regex(code) => code,
    }
}

/// [`CheckCore`] adapter over the publication store so T19 can mount the
/// data-plane transports directly: checks run against the active frozen
/// snapshot (or fail `NotReady`), info mirrors the published state.
pub struct PublishedCore {
    /// The publication store serving the active snapshot.
    pub state: Arc<RulesState>,
    /// The t2s table the store compiles with, retained so integrators can
    /// build side snapshots the same way the store does.
    pub table: &'static HashMap<char, char>,
}

impl PublishedCore {
    pub fn new(state: Arc<RulesState>) -> Self {
        Self {
            table: state.table,
            state,
        }
    }
}

impl CheckCore for PublishedCore {
    fn check(&self, request: &CheckRequest) -> Result<CheckResponse, ErrorCode> {
        let current = self.state.current().ok_or(ErrorCode::NotReady)?;
        current.snapshot().check(request, &|| false)
    }

    fn info(&self) -> InfoResponse {
        let mut info = InfoResponse {
            instance_id: self.state.instance_id().to_owned(),
            ready: false,
            build_id: crate::BUILD_ID.to_owned(),
            ruleset_revision: String::new(),
            generation: String::new(),
            normalization_id: crate::NORMALIZATION_ID.to_owned(),
            t2s_table_id: String::new(),
            base_rules_digest: String::new(),
            canonical_count: 0,
            alias_count: 0,
            regex_count: 0,
            supported_semantics: vec!["compat-v1".to_owned(), "enhanced-v1".to_owned()],
        };
        if let Some(current) = self.state.current() {
            info.ready = true;
            // the publication revision (ETag identity); equals the engine
            // content revision for canonically serialized packages
            info.ruleset_revision = current.revision().to_owned();
            info.generation = current.generation().to_string();
            info.t2s_table_id = current.t2s_table_id().to_owned();
            info.base_rules_digest = current.snapshot().digest().to_owned();
            info.canonical_count = current.counts().canonical as u32;
            info.alias_count = current.counts().alias as u32;
            info.regex_count = current.counts().regex as u32;
        }
        info
    }

    fn ready(&self) -> bool {
        self.state.ready()
    }
}
