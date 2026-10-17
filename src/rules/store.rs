//! Local publication storage: bounded files, fsync, atomic rename and verified recovery.
//! The ACTIVE manifest is authoritative; orphan packages never become active.

use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use sha2::Sha256;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::MANIFEST_SCHEMA_VERSION;

/// One commit boundary in the publication state machine (plan §9).
/// Always defined so the (feature-gated) fault seam shares one type with
/// the always-compiled no-op hook below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// Staging write or fsync while persisting a ruleset package.
    PackageStage,
    /// Rename of the staged package into `packages/`.
    PackageRename,
    /// The `packages/` directory fsync after that rename.
    PackageDirSync,
    /// Creation or fsync of the publication-history marker.
    HistoryMarker,
    /// Staging write or fsync of the manifest body.
    ManifestStage,
    /// Rename of the staged manifest over ACTIVE (before rename: a Pre
    /// failure leaves the previous state fully intact).
    ManifestRename,
    /// The root directory fsync after the manifest rename. A failure here
    /// is post-commit: durability is unknown (AC-068).
    ManifestDirSync,
}

pub(crate) const PACKAGE_DIR: &str = "packages";
pub(crate) const TMP_DIR: &str = "tmp";
pub(crate) const MANIFEST_FILE: &str = "manifest.json";

/// The ACTIVE pointer. `package_digest` names the package file, and
/// `instance_id` records the publishing process so a rebuilt data
/// directory can never be confused with the instance that wrote a
/// surviving manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub package_digest: String,
    pub generation: u64,
    pub instance_id: String,
    pub updated_at_unix: u64,
    pub updated_at_rfc3339: String,
}

/// Failure modes of [`write_manifest`].
pub(crate) enum ManifestError {
    /// Failure before the ACTIVE manifest was replaced: nothing changed
    /// on disk and the previous state is fully intact.
    Pre(io::Error),
    /// The manifest was renamed into place but the follow-up directory
    /// fsync failed with a real error: the publication may or may not be
    /// durable (AC-068 semantics; full injection is T22).
    PostRenameDurability,
}

/// Outcome of [`load_active`].
#[derive(Debug)]
pub(crate) enum LoadOutcome {
    /// No ACTIVE manifest: the store starts not ready with a queryable empty ETag.
    Empty,
    /// Manifest and package both verified against each other.
    Active { manifest: Manifest, bytes: Vec<u8> },
    /// Verification failed; the store must stay NOT READY and never fall
    /// back to scanning `packages/` for the newest file (AC-065/AC-070).
    Corrupt(String),
}

pub(crate) fn ensure_layout(root: &Path) -> io::Result<()> {
    for ancestor in root.ancestors() {
        if fs::symlink_metadata(ancestor).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "publication directory contains a symlink",
            ));
        }
    }
    fs::create_dir_all(root.join(PACKAGE_DIR))?;
    fs::create_dir_all(root.join(TMP_DIR))?;
    for name in [PACKAGE_DIR, TMP_DIR] {
        if fs::symlink_metadata(root.join(name))?
            .file_type()
            .is_symlink()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "publication directory contains a symlink",
            ));
        }
    }
    Ok(())
}

pub(crate) fn load_or_create_instance(root: &Path) -> io::Result<String> {
    let path = root.join("instance-id");
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 64 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid instance identity",
                ));
            }
            let identity = fs::read_to_string(&path)?;
            if identity.len() < 24 || !identity.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid instance identity",
                ));
            }
            return Ok(identity);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if has_history(root)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "instance identity missing from a historical store",
        ));
    }
    let mut random = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let identity: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(identity.as_bytes())?;
    file.sync_all()?;
    sync_dir(root)?;
    Ok(identity)
}

/// Read and verify the ACTIVE pointer plus its package.
pub(crate) fn load_active(root: &Path) -> LoadOutcome {
    let manifest_path = root.join(MANIFEST_FILE);
    match fs::symlink_metadata(&manifest_path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return match has_history(root) {
                Ok(false) => LoadOutcome::Empty,
                _ => LoadOutcome::Corrupt(
                    "ACTIVE missing from a historical or unverifiable store".to_owned(),
                ),
            };
        }
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= 4096 => {}
        _ => return LoadOutcome::Corrupt("ACTIVE is not a regular bounded file".to_owned()),
    }
    let raw = match fs::read(&manifest_path) {
        Ok(raw) => raw,
        Err(error) => return LoadOutcome::Corrupt(format!("manifest unreadable: {error}")),
    };
    let manifest: Manifest = match serde_json::from_slice(&raw) {
        Ok(manifest) => manifest,
        Err(error) => return LoadOutcome::Corrupt(format!("manifest unparseable: {error}")),
    };
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return LoadOutcome::Corrupt(format!(
            "manifest schema {} is not supported",
            manifest.schema_version
        ));
    }
    if !is_sha256_hex(&manifest.package_digest) {
        return LoadOutcome::Corrupt("manifest package digest is malformed".to_owned());
    }
    let package_path = root
        .join(PACKAGE_DIR)
        .join(format!("{}.json", manifest.package_digest));
    match fs::symlink_metadata(&package_path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= super::MAX_PACKAGE_BYTES as u64 => {}
        _ => {
            return LoadOutcome::Corrupt("active package is not a regular bounded file".to_owned())
        }
    }
    let bytes = match fs::read(&package_path) {
        Ok(bytes) => bytes,
        Err(error) => return LoadOutcome::Corrupt(format!("active package unreadable: {error}")),
    };
    if sha256_hex(&bytes) != manifest.package_digest {
        return LoadOutcome::Corrupt("active package digest mismatch".to_owned());
    }
    LoadOutcome::Active { manifest, bytes }
}

/// Write one immutable package: stage, fsync, rename over the digest-named
/// target (idempotent for identical bytes since content addressing makes
/// the target name a function of the content), fsync the directory.
pub(crate) fn write_package(root: &Path, digest: &str, bytes: &[u8]) -> io::Result<()> {
    let packages = root.join(PACKAGE_DIR);
    fs::create_dir_all(&packages)?;
    let target = packages.join(format!("{digest}.json"));
    match fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() != bytes.len() as u64
                || fs::read(&target)? != bytes
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "existing package differs from its content identity",
                ));
            }
            return sync_dir(&packages);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    hook(Point::PackageStage)?;
    let staged = stage(root, &format!("pkg-{digest}"), bytes)?;
    if let Err(error) = hook(Point::PackageRename).and_then(|()| fs::rename(&staged, &target)) {
        let _ = fs::remove_file(&staged);
        return Err(error);
    }
    hook(Point::PackageDirSync)?;
    sync_dir(&packages)
}

/// Replace the ACTIVE manifest: stage, fsync, rename over `manifest.json`.
/// See [`ManifestError`] for the pre/post-rename failure split.
pub(crate) fn write_manifest(root: &Path, manifest: &Manifest) -> Result<(), ManifestError> {
    hook(Point::HistoryMarker).map_err(ManifestError::Pre)?;
    mark_history(root).map_err(ManifestError::Pre)?;
    let bytes = serde_json::to_vec_pretty(manifest).expect("manifest serialization cannot fail");
    hook(Point::ManifestStage).map_err(ManifestError::Pre)?;
    let staged = stage(root, "manifest", &bytes).map_err(ManifestError::Pre)?;
    let target = root.join(MANIFEST_FILE);
    let renamed = hook(Point::ManifestRename).and_then(|()| fs::rename(&staged, &target));
    if let Err(error) = renamed {
        let _ = fs::remove_file(&staged);
        return Err(ManifestError::Pre(error));
    }
    hook(Point::ManifestDirSync).map_err(|_| ManifestError::PostRenameDurability)?;
    match sync_dir(root) {
        Ok(()) => Ok(()),
        Err(_error) => Err(ManifestError::PostRenameDurability),
    }
}

/// Bounded retention: keep at most the ACTIVE and the previous package
/// (content addressing may collapse both into one file). Best-effort — a
/// cleanup failure never fails a publication.
pub(crate) fn cleanup_packages(root: &Path, keep: &[&str]) {
    let Ok(entries) = fs::read_dir(root.join(PACKAGE_DIR)) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(digest) = name.strip_suffix(".json") else {
            continue;
        };
        if !keep.contains(&digest) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn has_history(root: &Path) -> io::Result<bool> {
    for name in [MANIFEST_FILE, "publication-history"] {
        match fs::symlink_metadata(root.join(name)) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    for name in [PACKAGE_DIR, TMP_DIR] {
        if fs::read_dir(root.join(name))?.next().transpose()?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn mark_history(root: &Path) -> io::Result<()> {
    let path = root.join("publication-history");
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() == 2
                && fs::read(&path)? == b"1\n" =>
        {
            return sync_dir(root)
        }
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid publication history marker",
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    if let Err(error) = file.write_all(b"1\n").and_then(|()| file.sync_all()) {
        drop(file);
        fs::remove_file(path)?;
        return Err(error);
    }
    sync_dir(root)
}

fn stage(root: &Path, tag: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    stage_file(root, tag, |file| {
        file.write_all(bytes)?;
        file.sync_all()
    })
}

fn stage_file(
    root: &Path,
    tag: &str,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let tmp_dir = root.join(TMP_DIR);
    fs::create_dir_all(&tmp_dir)?;
    // one publishing store per directory per process; the pid still keeps
    // the name distinct if operators ever share a root by mistake
    let path = tmp_dir.join(format!("{tag}-{}.stage", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    if let Err(error) = write(&mut file) {
        drop(file);
        fs::remove_file(&path)?;
        return Err(error);
    }
    Ok(path)
}

fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(feature = "fault-injection")]
fn hook(point: Point) -> io::Result<()> {
    super::fault::trigger(point)
}

#[cfg(not(feature = "fault-injection"))]
fn hook(_point: Point) -> io::Result<()> {
    Ok(())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Fresh per-process instance id (no new dependencies): publish-time
/// nanoseconds, the pid and a process-local counter in hex.
#[cfg(test)]
pub(crate) fn fresh_instance_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:016x}{:08x}{sequence:08x}", std::process::id())
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// RFC 3339 UTC rendering of a unix timestamp without a time crate.
pub(crate) fn rfc3339(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let seconds_of_day = unix_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3_600,
        (seconds_of_day % 3_600) / 60,
        seconds_of_day % 60
    )
}

/// Days-since-epoch to (year, month, day); Howard Hinnant's
/// `civil_from_days`, valid for the whole u64 second range.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_reference_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rfc3339_renders_known_timestamps() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1), "1970-01-01T00:00:01Z");
        // leap day
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_760_000_000), "2025-10-09T08:53:20Z");
    }

    #[test]
    fn instance_ids_never_repeat_within_a_process() {
        assert_ne!(fresh_instance_id(), fresh_instance_id());
        assert!(fresh_instance_id().len() >= 24);
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            package_digest: "ab".repeat(32),
            generation: 7,
            instance_id: "instance".to_owned(),
            updated_at_unix: 5,
            updated_at_rfc3339: rfc3339(5),
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.generation, 7);
        assert_eq!(parsed.package_digest, manifest.package_digest);
    }

    #[test]
    fn load_active_is_empty_missing_corrupt_or_verified() {
        let root = std::env::temp_dir().join(format!(
            "textguard-store-unit-{}-{}",
            std::process::id(),
            fresh_instance_id()
        ));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        ensure_layout(&root).unwrap();
        assert!(matches!(load_active(&root), LoadOutcome::Empty));

        // truncated manifest
        fs::write(root.join(MANIFEST_FILE), b"{trunc").unwrap();
        assert!(matches!(load_active(&root), LoadOutcome::Corrupt(_)));

        // well-formed manifest whose package is gone
        let digest = sha256_hex(b"package");
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            package_digest: digest.clone(),
            generation: 2,
            instance_id: "instance".to_owned(),
            updated_at_unix: 1,
            updated_at_rfc3339: rfc3339(1),
        };
        fs::write(
            root.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(matches!(load_active(&root), LoadOutcome::Corrupt(_)));

        // package present and verified
        fs::write(
            root.join(PACKAGE_DIR).join(format!("{digest}.json")),
            b"package",
        )
        .unwrap();
        match load_active(&root) {
            LoadOutcome::Active { manifest, bytes } => {
                assert_eq!(manifest.generation, 2);
                assert_eq!(bytes, b"package");
            }
            _ => panic!("expected a verified Active outcome"),
        }

        // package tampered: digest mismatch
        fs::write(
            root.join(PACKAGE_DIR).join(format!("{digest}.json")),
            b"tampered",
        )
        .unwrap();
        assert!(matches!(load_active(&root), LoadOutcome::Corrupt(_)));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn failed_stage_write_or_sync_is_removed_before_same_process_retry() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("textguard-stage-{}", fresh_instance_id()));
        ensure_layout(&root).unwrap();
        for kind in [io::ErrorKind::StorageFull, io::ErrorKind::Other] {
            let result = stage_file(&root, "manifest", |file| {
                file.write_all(b"partial")?;
                Err(io::Error::new(kind, "injected write or sync failure"))
            });
            assert_eq!(result.unwrap_err().kind(), kind);
            assert_eq!(fs::read_dir(root.join(TMP_DIR)).unwrap().count(), 0);
            let staged = stage(&root, "manifest", b"complete").unwrap();
            assert_eq!(fs::read(&staged).unwrap(), b"complete");
            fs::remove_file(staged).unwrap();
        }
        fs::remove_dir_all(root).unwrap();
    }
}
