//! T22 process-level fault scenarios (AC-068/AC-069/AC-070): the real
//! binary dies hard at an exact publication commit boundary, restarts on
//! the same data directory, and must recover exactly the on-disk facts.
//!
//! Requires the `fault-injection` feature: the binary under test is
//! started with `TEXTGUARD_FAULT_KILL_AT=manifest_dir_sync` so the very
//! first publication dies between the ACTIVE rename and the directory
//! fsync (the client observes a lost ACK), emulating a hard crash. All
//! restarts use the *plain* binary configuration (no fault activation),
//! proving recovery needs no test-only cooperation from the process.
//!
//! * AC-068: killed after the rename — restart serves the committed
//!   generation, never claims a different uncommitted state.
//! * AC-069: the client never got the success ACK — it *queries* the
//!   actual revision/generation after restart, replays the identical
//!   bytes and receives `already_active` (no blind rollback, no double
//!   generation).
//! * AC-070: after a clean ACK + graceful stop, a restart recovers the
//!   same generation; a tampered ACTIVE leaves the store explicitly NOT
//!   READY even though an orphan newer package exists on disk.

#![cfg(feature = "fault-injection")]

use serde_json::{json, Value};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHECK: &str = "t22-check-canary-0123456789-abcdefg";
const PUBLISH: &str = "t22-publish-canary-0123456789-abcdef";
const RULESET_A: &str = include_str!("fixtures/contract-ruleset.json");

struct Process {
    child: Child,
    http: SocketAddr,
    manage: SocketAddr,
}

impl Process {
    /// Start the real binary reusing `directory` (secrets already in
    /// place from the first start) with optional extra environment.
    async fn start(directory: &Path, extra_env: &[(&str, &str)]) -> Self {
        let listeners: Vec<_> = (0..2)
            .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        let addresses: Vec<_> = listeners
            .iter()
            .map(|listener| listener.local_addr().unwrap())
            .collect();
        drop(listeners);
        let mut command = Command::new(env!("CARGO_BIN_EXE_imboy-textguard"));
        command
            .env("TEXTGUARD_DATA_DIR", directory.join("rules"))
            .env("TEXTGUARD_CHECK_SECRET_FILE", directory.join("check"))
            .env("TEXTGUARD_PUBLISH_SECRET_FILE", directory.join("publish"))
            .env("TEXTGUARD_T2S_TABLE_ID", "test-t2s-v1")
            .env("TEXTGUARD_HTTP_ADDR", addresses[0].to_string())
            .env("TEXTGUARD_GRPC_ADDR", "127.0.0.1:0")
            .env("TEXTGUARD_MANAGE_ADDR", addresses[1].to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in extra_env {
            command.env(key, value);
        }
        let child = command.spawn().unwrap();
        let mut process = Self {
            child,
            http: addresses[0],
            manage: addresses[1],
        };
        for _ in 0..600 {
            if process.child.try_wait().unwrap().is_some() {
                panic!("service exited before becoming live");
            }
            if tokio::net::TcpStream::connect(process.http).await.is_ok() {
                return process;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("service did not become live");
    }

    /// Fresh data directory with secrets written once for reuse across
    /// restarts.
    fn fresh_directory(tag: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("textguard-t22-proc-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let directory = directory.canonicalize().unwrap();
        for (name, secret) in [("check", CHECK), ("publish", PUBLISH)] {
            let path = directory.join(name);
            std::fs::write(&path, secret).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        directory
    }
}

/// SIGTERM and wait; returns the exit status.
fn graceful_stop(process: &mut Process) -> std::process::ExitStatus {
    unsafe {
        libc::kill(process.child.id() as i32, libc::SIGTERM);
    }
    process.child.wait().unwrap()
}

/// Send a request; tolerate the connection dying without a response
/// (the injected SIGKILL scenario). JSON bodies are parsed for the
/// caller; non-JSON bodies arrive as `Value::Null`.
async fn http_tolerant(
    address: SocketAddr,
    method: &str,
    path: &str,
    credential: &str,
    etag: Option<&str>,
    body: &str,
) -> Option<(u16, Value)> {
    let mut stream = tokio::net::TcpStream::connect(address).await.ok()?;
    let etag = etag.map_or(String::new(), |etag| format!("If-Match: {etag}\r\n"));
    let request = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {credential}\r\n{etag}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut response = Vec::new();
    let read =
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    let Ok(Ok(_bytes)) = read else {
        return None;
    };
    if response.is_empty() {
        return None; // hard-crashed before responding (lost ACK)
    }
    let response = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = response.split_once("\r\n\r\n")?;
    let code: u16 = head.split_whitespace().nth(1)?.parse().ok()?;
    let value = serde_json::from_str(body).unwrap_or(Value::Null);
    Some((code, value))
}

async fn wait_live(process: &Process) {
    for _ in 0..600 {
        if tokio::net::TcpStream::connect(process.http).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("restart did not become live");
}

// ------------------------------------------------- AC-068 + AC-069 combined

/// The first publication dies between the ACTIVE rename and the root
/// directory fsync (hard SIGKILL at the exact commit boundary). The
/// client sees a dead connection (AC-069 lost ACK). A plain restart
/// must serve exactly the on-disk facts (AC-068): generation 1, and the
/// replayed identical bytes come back `already_active` — the client
/// recovered the truth by querying, never by blindly rolling back or
/// double-publishing.
#[tokio::test]
async fn ac068_ac069_kill_after_rename_restart_recovers_facts() {
    let directory = Process::fresh_directory("kill");

    // phase 1: process dies at the commit boundary of the FIRST publish
    let mut victim = Process::start(
        &directory,
        &[("TEXTGUARD_FAULT_KILL_AT", "manifest_dir_sync")],
    )
    .await;
    let outcome = http_tolerant(
        victim.manage,
        "PUT",
        "/internal/v1/rulesets/active",
        PUBLISH,
        Some(&placeholder_etag(&victim).await),
        RULESET_A,
    )
    .await;
    // the ACK is lost: either an empty reply or no reply at all
    assert!(
        outcome.is_none(),
        "the hard-crashing publish must not deliver a success ACK, got {outcome:?}"
    );
    let status = victim.child.wait().unwrap();
    assert!(
        !status.success(),
        "victim must have died from the injected SIGKILL"
    );
    // on disk: the manifest was already renamed into place
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(directory.join("rules/manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["generation"], 1);

    // phase 2: plain restart recovers the committed facts
    let mut survivor = Process::start(&directory, &[]).await;
    wait_live(&survivor).await;
    let (code, active) = http_tolerant(
        survivor.manage,
        "GET",
        "/internal/v1/rulesets/active",
        PUBLISH,
        None,
        "",
    )
    .await
    .unwrap();
    assert_eq!(code, 200);
    assert_eq!(
        active["generation"],
        json!("1"),
        "recovery must serve gen 1"
    );

    // phase 3 (AC-069): the client replays the identical bytes with the
    // freshly queried state etag — already_active, no new generation
    let etag = active["state_etag"].as_str().unwrap().to_owned();
    let (code, replay) = http_tolerant(
        survivor.manage,
        "PUT",
        "/internal/v1/rulesets/active",
        PUBLISH,
        Some(&etag),
        RULESET_A,
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "replay body: {replay:?}");
    assert_eq!(replay["status"], json!("already_active"));
    assert_eq!(replay["generation"], json!("1"));

    // the data plane serves under the recovered snapshot
    let request = json!({"request_id":"t22-1","scene":"channel_message","fields":[{"field_id":"body","text":"unique-测试词-canary"}]}).to_string();
    let (code, checked) = http_tolerant(survivor.http, "POST", "/v1/check", CHECK, None, &request)
        .await
        .unwrap();
    assert_eq!(code, 200);
    assert_eq!(checked["decision"], "block");

    assert!(graceful_stop(&mut survivor).success());
    let _ = std::fs::remove_dir_all(&directory);
}

async fn placeholder_etag(process: &Process) -> String {
    let (code, active) = http_tolerant(
        process.manage,
        "GET",
        "/internal/v1/rulesets/active",
        PUBLISH,
        None,
        "",
    )
    .await
    .unwrap();
    assert_eq!(code, 200);
    active["state_etag"].as_str().unwrap().to_owned()
}

// ------------------------------------------------------------------ AC-070

/// A cleanly ACKed publication survives a graceful restart (same
/// generation/revision). After tampering with the ACTIVE manifest, the
/// restart stays explicitly NOT READY — even though an orphan newer
/// package sits in `packages/`, recovery never scans for the newest file.
#[tokio::test]
async fn ac070_ack_restart_recovers_then_tampered_manifest_stays_not_ready() {
    let directory = Process::fresh_directory("tamper");

    // phase 1: normal publish with a clean ACK
    let mut first = Process::start(&directory, &[]).await;
    let (code, _) = http_tolerant(
        first.manage,
        "PUT",
        "/internal/v1/rulesets/active",
        PUBLISH,
        Some(&placeholder_etag(&first).await),
        RULESET_A,
    )
    .await
    .unwrap();
    assert_eq!(code, 200);
    assert!(graceful_stop(&mut first).success());

    // phase 2: graceful restart recovers the ACKed generation unchanged
    let mut second = Process::start(&directory, &[]).await;
    wait_live(&second).await;
    let (code, active) = http_tolerant(
        second.manage,
        "GET",
        "/internal/v1/rulesets/active",
        PUBLISH,
        None,
        "",
    )
    .await
    .unwrap();
    assert_eq!(code, 200);
    assert_eq!(active["generation"], json!("1"));
    assert!(graceful_stop(&mut second).success());

    // phase 3: plant an orphan "newer" package, truncate the manifest,
    // restart: must stay NOT READY and never adopt the orphan
    let orphan = br#"{"schema_version":1,"orphan":true}"#;
    let digest = {
        use std::fmt::Write as _;
        let mut hex = String::new();
        for byte in sha2_orphan(orphan) {
            write!(hex, "{byte:02x}").unwrap();
        }
        hex
    };
    std::fs::write(
        directory
            .join("rules/packages")
            .join(format!("{digest}.json")),
        orphan,
    )
    .unwrap();
    let manifest_path = directory.join("rules/manifest.json");
    let raw = std::fs::read(&manifest_path).unwrap();
    std::fs::write(&manifest_path, &raw[..raw.len() / 2]).unwrap();

    let mut third = Process::start(&directory, &[]).await;
    wait_live(&third).await;
    // ready endpoint reports NOT READY
    let (code, body) = http_tolerant(third.http, "GET", "/health/ready", CHECK, None, "")
        .await
        .unwrap();
    assert_eq!(code, 503, "tampered store must not be ready: {body:?}");
    // the management query reports the unknown/unverified state explicitly
    let (code, active) = http_tolerant(
        third.manage,
        "GET",
        "/internal/v1/rulesets/active",
        PUBLISH,
        None,
        "",
    )
    .await
    .unwrap();
    assert_ne!(code, 200, "tampered ACTIVE must not serve 200: {active:?}");
    // the data plane refuses to check against an unverifiable base
    let request = json!({"request_id":"t22-2","scene":"channel_message","fields":[{"field_id":"body","text":"x"}]}).to_string();
    let (code, body) = http_tolerant(third.http, "POST", "/v1/check", CHECK, None, &request)
        .await
        .unwrap();
    assert_ne!(code, 200, "checks must fail closed: {body:?}");
    assert!(graceful_stop(&mut third).success());
    let _ = std::fs::remove_dir_all(&directory);
}

/// Minimal SHA-256 for the orphan package name (content addressing) so
/// this test does not depend on a hash crate of its own.
fn sha2_orphan(bytes: &[u8]) -> [u8; 32] {
    // The store names packages by imboy_textguard's own sha256_hex; reuse
    // it through the public rules API to keep one hash definition.
    let hex = imboy_textguard::rules::sha256_hex(bytes);
    let mut digest = [0u8; 32];
    for (index, slot) in digest.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap();
    }
    digest
}
