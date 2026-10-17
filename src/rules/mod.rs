//! Publication state, full-byte revision, persistent instance identity and generation CAS.
//! A single build lock and bounded retired snapshot protect publication resources.
//! An unverified ACTIVE or post-rename durability error fails closed until recovery.

mod store;

use crate::api::{CheckCore, InfoResponse};
use crate::engine::{BuildError, Snapshot};
use crate::model::{CheckRequest, CheckResponse, ErrorCode, RuleSet, Scene};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard, TryLockError, Weak};

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
    scene_counts: [usize; 3],
}

impl PublishedState {
    fn assemble(snapshot: Snapshot, ruleset: &RuleSet, manifest: &store::Manifest) -> Self {
        Self {
            snapshot,
            revision: manifest.package_digest.clone(),
            digest: manifest.package_digest.clone(),
            generation: manifest.generation,
            published_by: manifest.instance_id.clone(),
            updated_at_unix: manifest.updated_at_unix,
            updated_at_rfc3339: manifest.updated_at_rfc3339.clone(),
            t2s_table_id: ruleset.t2s_table_id.clone(),
            counts: RuleCounts::of(ruleset),
            scene_counts: [
                Scene::ChannelMessage,
                Scene::MomentPost,
                Scene::ProfileField,
            ]
            .map(|scene| {
                ruleset
                    .literals
                    .iter()
                    .filter(|rule| rule.scenes.contains(&scene))
                    .count()
                    + ruleset
                        .regex_rules
                        .iter()
                        .filter(|rule| rule.scenes.contains(&scene))
                        .count()
            }),
        }
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Publication revision: full SHA-256 of the package byte
    /// digest; state ETags additionally bind instance and generation.
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

    pub fn scene_counts(&self) -> [usize; 3] {
        self.scene_counts
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
    pub state_etag: String,
    pub scene_counts: [usize; 3],
}

/// Publication failure. `Io` failures always precede the manifest rename,
/// so the previous state is intact; `UnknownState` means the manifest was
/// renamed but its durability could not be confirmed.
#[derive(Debug)]
pub enum PublishError {
    Busy,
    CasConflict,
    RetiredSnapshotBusy,
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
            Self::RetiredSnapshotBusy => ErrorCode::RetiredSnapshotBusy,
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
    uncertain: AtomicBool,
    retired: Mutex<Weak<PublishedState>>,
}

impl RulesState {
    /// Open (and recover) a data directory with its persistent instance id.
    pub fn load(dir: impl Into<PathBuf>, table: &'static HashMap<char, char>) -> io::Result<Self> {
        let dir = dir.into();
        store::ensure_layout(&dir)?;
        let instance_id = store::load_or_create_instance(&dir)?;
        let state = Self::load_with_instance(dir, table, instance_id)?;
        if let Some(active) = state.current() {
            if active.published_by() != state.instance_id() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "ACTIVE instance identity mismatch",
                ));
            }
            store::mark_history(state.dir())?;
        }
        Ok(state)
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
            uncertain: AtomicBool::new(false),
            retired: Mutex::new(Weak::new()),
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
        if self.uncertain.load(Ordering::Acquire) {
            return None;
        }
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
        self.current().is_some()
    }

    /// Load-time outcome (recomputed to `Ready` once a publish succeeds).
    pub fn status(&self) -> StoreStatus {
        if self.uncertain.load(Ordering::Acquire) {
            StoreStatus::Failed("publication durability is unknown".to_owned())
        } else if self.ready() {
            StoreStatus::Ready
        } else {
            self.status.clone()
        }
    }

    pub fn active_state(&self) -> Result<(Option<Arc<PublishedState>>, String), PublishError> {
        if matches!(self.status, StoreStatus::Failed(_)) || self.uncertain.load(Ordering::Acquire) {
            return Err(PublishError::UnknownState("ACTIVE cannot be verified"));
        }
        let active = self.current();
        if self.uncertain.load(Ordering::Acquire) {
            return Err(PublishError::UnknownState(
                "publication durability is unknown",
            ));
        }
        let generation = active.as_ref().map_or(0, |value| value.generation());
        let revision = active.as_ref().map_or("", |value| value.revision());
        let etag = self.etag(generation, revision);
        Ok((active, etag))
    }

    fn etag(&self, generation: u64, revision: &str) -> String {
        store::sha256_hex(format!("{}\n{generation}\n{revision}", self.instance_id).as_bytes())
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
        // single-flight publication: a concurrent publish fails fast
        let _guard = match self.publishing.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(PublishError::Busy),
        };

        // CAS against the manifest-backed active state
        let (current, etag) = self.active_state()?;
        let supplied_etag = if_match.ok_or(PublishError::CasConflict)?;
        let digest = store::sha256_hex(bytes);
        if let Some(active) = &current {
            if active.digest() == digest {
                return Ok(PublishReceipt {
                    revision: active.revision().to_owned(),
                    generation: active.generation(),
                    digest,
                    instance_id: self.instance_id.clone(),
                    already_active: true,
                    state_etag: etag,
                    scene_counts: active.scene_counts(),
                });
            }
        }
        if supplied_etag != etag {
            return Err(PublishError::CasConflict);
        }

        let mut retired = self
            .retired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if retired.strong_count() > 0 {
            return Err(PublishError::RetiredSnapshotBusy);
        }
        let generation = current
            .as_ref()
            .map_or(0, |active| active.generation())
            .checked_add(1)
            .ok_or(PublishError::Invalid(ErrorCode::InvalidRuleset))?;
        let text = std::str::from_utf8(bytes)
            .map_err(|_| PublishError::Invalid(ErrorCode::InvalidUtf8))?;
        let ruleset: RuleSet = serde_json::from_str(text)
            .map_err(|_| PublishError::Invalid(ErrorCode::InvalidRuleset))?;
        let compiled = Snapshot::build(&ruleset, self.table)
            .map_err(|error| PublishError::Invalid(build_error_code(error)))?;

        // persist: package first, then the ACTIVE manifest, then swap
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
        match store::write_manifest(&self.dir, &manifest) {
            Ok(()) => {}
            Err(store::ManifestError::Pre(error)) => return Err(PublishError::Io(error)),
            Err(store::ManifestError::PostRenameDurability) => {
                self.uncertain.store(true, Ordering::Release);
                return Err(PublishError::UnknownState(
                    "ACTIVE rename completed but directory fsync failed",
                ));
            }
        }

        // RCU swap: readers hold the old Arc until they finish
        let previous = current.as_ref().map(|active| active.digest().to_owned());
        let snapshot = compiled.with_publication(&digest, generation);
        let state = Arc::new(PublishedState::assemble(snapshot, &ruleset, &manifest));
        let scene_counts = state.scene_counts();
        *retired = current.as_ref().map_or_else(Weak::new, Arc::downgrade);
        *self.write_current() = Some(state);
        store::cleanup_packages(
            &self.dir,
            &[digest.as_str(), previous.as_deref().unwrap_or("")],
        );
        Ok(PublishReceipt {
            revision: manifest.package_digest.clone(),
            generation,
            digest,
            instance_id: self.instance_id.clone(),
            already_active: false,
            state_etag: self.etag(generation, &manifest.package_digest),
            scene_counts,
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
        snapshot.with_publication(&manifest.package_digest, manifest.generation),
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
        self.check_cancellable(request, &|| false)
    }

    fn check_cancellable(
        &self,
        request: &CheckRequest,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<CheckResponse, ErrorCode> {
        let current = self.state.current().ok_or(ErrorCode::NotReady)?;
        current.snapshot().check(request, cancelled)
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
