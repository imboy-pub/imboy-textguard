//! Test-only fault injection seam for T22 publication fault testing.
//!
//! Compiled exclusively behind the `fault-injection` cargo feature.
//! Production builds never enable the feature, so this module and every
//! hook call site compile away entirely. The seam is a process-global
//! ordered rule list: the first rule matching a commit boundary fires
//! once and is consumed, which keeps multi-publication tests explicit.

pub use super::store::Point;
use std::io;
use std::sync::Mutex;

/// What happens when a rule fires at its point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The hooked operation fails with this error kind.
    Fail(io::ErrorKind),
    /// The whole process dies instantly at this exact commit boundary
    /// (SIGKILL: nothing can intercept or flush).
    KillProcess,
}

static RULES: Mutex<Vec<(Point, Action)>> = Mutex::new(Vec::new());

/// Install an ordered rule list, replacing any previous one.
pub fn install(rules: Vec<(Point, Action)>) {
    *RULES.lock().unwrap() = rules;
}

/// Remove all rules.
pub fn clear() {
    RULES.lock().unwrap().clear();
}

/// Hook invoked at every commit boundary. The first rule registered for
/// this point fires (and is consumed); otherwise the call is a no-op.
pub(crate) fn trigger(point: Point) -> io::Result<()> {
    let mut rules = RULES.lock().unwrap();
    let Some(index) = rules.iter().position(|(at, _)| *at == point) else {
        return Ok(());
    };
    let (_, action) = rules.remove(index);
    match action {
        Action::Fail(kind) => Err(io::Error::new(
            kind,
            format!("injected failure at {point:?}"),
        )),
        Action::KillProcess => {
            // A hard crash at the boundary: no destructors, no flush, no
            // chance for the publisher to report anything.
            unsafe { libc::raise(libc::SIGKILL) };
            unreachable!("SIGKILL cannot be intercepted");
        }
    }
}

/// Map a textual point name (used by the binary's test env activation)
/// to a [`Point`].
pub fn point_from_name(name: &str) -> Option<Point> {
    match name {
        "package_stage" => Some(Point::PackageStage),
        "package_rename" => Some(Point::PackageRename),
        "package_dir_sync" => Some(Point::PackageDirSync),
        "history_marker" => Some(Point::HistoryMarker),
        "manifest_stage" => Some(Point::ManifestStage),
        "manifest_rename" => Some(Point::ManifestRename),
        "manifest_dir_sync" => Some(Point::ManifestDirSync),
        _ => None,
    }
}
