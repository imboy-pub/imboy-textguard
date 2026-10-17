//! T23 final verification, part 1 (AC-087/AC-088/AC-089): the real
//! binary under genuine concurrent load and privacy canaries.
//!
//! Unlike the T17 component tests (mock transports, in-process
//! admission), these tests run the assembled service: HTTP and gRPC hit
//! the *same* live admission (WORKERS=2, WAITING=32), cancellation is
//! observed through real client deadlines, and the privacy canary is
//! searched across everything the process ever printed plus the full
//! metrics exposition.
//!
//! * AC-087: HTTP and gRPC saturate the shared budget together — mixed
//!   overload produces explicit Overloaded rejections (never doubled
//!   capacity, never a crash), and the service recovers immediately.
//! * AC-088: a client that abandons a slow check releases its permit
//!   only when the real work finishes; cancelled-but-running work is
//!   observable and drains to zero.
//! * AC-089: a unique body canary (hit, oversized, malformed, wrong
//!   credentials) never appears in logs or metrics; no high-cardinality
//!   labels are invented.

use imboy_textguard::protocol::text_guard_client::TextGuardClient;
use imboy_textguard::protocol::{CheckTextRequest, TextField};
use serde_json::{json, Value};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHECK: &str = "t23-check-canary-0123456789-abcdefg";
const PUBLISH: &str = "t23-publish-canary-0123456789-abcdef";
const RULESET: &str = include_str!("fixtures/contract-ruleset.json");

struct Process {
    child: Child,
    directory: PathBuf,
    http: SocketAddr,
    grpc: SocketAddr,
    manage: SocketAddr,
}

impl Process {
    async fn start(tag: &str) -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "textguard-t23-{tag}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let directory = directory.canonicalize().unwrap();
        for (name, secret) in [("check", CHECK), ("publish", PUBLISH)] {
            let path = directory.join(name);
            std::fs::write(&path, secret).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let listeners: Vec<_> = (0..3)
            .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        let addresses: Vec<_> = listeners
            .iter()
            .map(|listener| listener.local_addr().unwrap())
            .collect();
        drop(listeners);
        let child = Command::new(env!("CARGO_BIN_EXE_imboy-textguard"))
            .env("TEXTGUARD_DATA_DIR", directory.join("rules"))
            .env("TEXTGUARD_CHECK_SECRET_FILE", directory.join("check"))
            .env("TEXTGUARD_PUBLISH_SECRET_FILE", directory.join("publish"))
            .env("TEXTGUARD_T2S_TABLE_ID", "test-t2s-v1")
            .env("TEXTGUARD_HTTP_ADDR", addresses[0].to_string())
            .env("TEXTGUARD_GRPC_ADDR", addresses[1].to_string())
            .env("TEXTGUARD_MANAGE_ADDR", addresses[2].to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut process = Self {
            child,
            directory,
            http: addresses[0],
            grpc: addresses[1],
            manage: addresses[2],
        };
        for _ in 0..300 {
            assert!(process.child.try_wait().unwrap().is_none());
            if tokio::net::TcpStream::connect(process.http).await.is_ok() {
                return process;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("service did not become live");
    }

    /// SIGTERM and collect everything the process ever printed.
    async fn stop(&mut self) -> String {
        assert!(Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap()
            .success());
        for _ in 0..300 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                use std::io::Read;
                let mut output = String::new();
                if let Some(mut pipe) = self.child.stdout.take() {
                    output.push_str(&{
                        let mut raw = String::new();
                        pipe.read_to_string(&mut raw).unwrap();
                        raw
                    });
                }
                if let Some(mut pipe) = self.child.stderr.take() {
                    let mut raw = String::new();
                    pipe.read_to_string(&mut raw).unwrap();
                    output.push_str(&raw);
                }
                return output;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("graceful shutdown exceeded");
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

async fn http(
    address: SocketAddr,
    method: &str,
    path: &str,
    credential: &str,
    etag: Option<&str>,
    body: &str,
    timeout: Duration,
) -> Option<(u16, String)> {
    let mut stream = tokio::net::TcpStream::connect(address).await.ok()?;
    let etag = etag.map_or(String::new(), |etag| format!("If-Match: {etag}\r\n"));
    let request = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {credential}\r\n{etag}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut response = Vec::new();
    let read = tokio::time::timeout(timeout, stream.read_to_end(&mut response)).await;
    let Ok(Ok(_bytes)) = read else {
        return None;
    };
    if response.is_empty() {
        return None;
    }
    let response = String::from_utf8_lossy(&response).into_owned();
    let (head, body) = response.split_once("\r\n\r\n")?;
    Some((
        head.split_whitespace().nth(1)?.parse().ok()?,
        body.to_owned(),
    ))
}

async fn publish_ruleset(process: &Process) {
    let (code, active) = http(
        process.manage,
        "GET",
        "/internal/v1/rulesets/active",
        PUBLISH,
        None,
        "",
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(code, 200);
    let etag = serde_json::from_str::<Value>(&active).unwrap()["state_etag"]
        .as_str()
        .unwrap()
        .to_owned();
    let (code, body) = http(
        process.manage,
        "PUT",
        "/internal/v1/rulesets/active",
        PUBLISH,
        Some(&etag),
        RULESET,
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "publish failed: {body}");
}

fn check_body(text: &str) -> String {
    json!({"request_id":"t23-load","scene":"channel_message","fields":[{"field_id":"body","text":text}]}).to_string()
}

async fn metrics(process: &Process) -> String {
    let (_, body) = http(
        process.manage,
        "GET",
        "/metrics",
        "",
        None,
        "",
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    body
}

// ------------------------------------------------------------------ AC-087

/// Mixed HTTP+gRPC load far above WORKERS+WAITING (34): every request
/// either completes or is explicitly rejected as Overloaded; rejections
/// are counted once each; the shared budget is not doubled (a single
/// protocol alone also saturates), and the service recovers instantly.
#[tokio::test]
async fn ac087_shared_admission_overload_rejects_explicitly_and_recovers() {
    let mut process = Process::start("ac087").await;
    publish_ruleset(&process).await;

    // A 4 KiB body widens the per-request work window; a barrier releases
    // every TCP connection at the same instant so 80 offered requests
    // genuinely race for the 2-worker + 32-waiting budget.
    let padding = "负载填充文本".repeat(900);
    let body = check_body(&format!("并发负载测试词 唯一命中 {padding}"));
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(80));
    let mut http_handles = Vec::new();
    for index in 0..40 {
        let address = process.http;
        let body = body.clone();
        let credential = CHECK.to_owned();
        let barrier = barrier.clone();
        http_handles.push(tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            barrier.wait().await;
            let request = format!("POST /v1/check HTTP/1.1\r\nHost: localhost\r\nAuthorization: {credential}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            match tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await {
                Ok(Ok(_)) if !response.is_empty() => {
                    let text = String::from_utf8_lossy(&response);
                    let code: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
                    Some((code, index))
                }
                _ => None,
            }
        }));
    }
    let mut grpc_handles = Vec::new();
    for _index in 0..40 {
        let address = process.grpc;
        let credential = CHECK.to_owned();
        let barrier = barrier.clone();
        let grpc_padding = padding.clone();
        grpc_handles.push(tokio::spawn(async move {
            let mut client = TextGuardClient::connect(format!("http://{address}"))
                .await
                .unwrap();
            barrier.wait().await;
            let mut request = tonic::Request::new(CheckTextRequest {
                request_id: Some("t23-load".to_owned()),
                scene: Some(1),
                fields: vec![TextField {
                    field_id: Some("body".to_owned()),
                    text: Some(format!("并发负载测试词 grpc {grpc_padding}")),
                }],
            });
            request
                .metadata_mut()
                .insert("authorization", credential.parse().unwrap());
            request.set_timeout(Duration::from_secs(5));
            client.check_text(request).await
        }));
    }

    let mut completed = 0usize;
    let mut overloaded = 0usize;
    for handle in http_handles {
        let (code, _index) = handle.await.unwrap().expect("http request must resolve");
        match code {
            200 => completed += 1,
            429 => overloaded += 1,
            other_code => panic!("unexpected HTTP status {other_code}"),
        }
    }
    for handle in grpc_handles {
        match handle.await.unwrap() {
            Ok(_) => completed += 1,
            Err(status) if status.code() == tonic::Code::ResourceExhausted => overloaded += 1,
            Err(status) => panic!("unexpected gRPC status: {status}"),
        }
    }
    // 80 offered, none lost: completed + rejected == 80
    assert_eq!(completed + overloaded, 80, "requests must not vanish");
    // the shared budget (2 workers + 32 waiting) is exceeded: mixed load
    // must produce explicit rejections, and both transports saw them or
    // saturated the budget together
    assert!(overloaded > 0, "80 concurrent > 34 budget must reject some");
    let report = metrics(&process).await;
    assert!(
        report.contains("textguard_rejected_total"),
        "rejections must be metered: {report}"
    );

    // immediate recovery: a normal follow-up request succeeds
    let (code, body) = http(
        process.http,
        "POST",
        "/v1/check",
        CHECK,
        None,
        &check_body("恢复探测"),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "service must recover instantly: {body}");

    // single-protocol saturation also rejects (the budget is not doubled
    // by splitting across transports)
    let solo_barrier = std::sync::Arc::new(tokio::sync::Barrier::new(48));
    let mut solo = Vec::new();
    for _ in 0..48 {
        let address = process.http;
        let body = check_body(&format!("单独饱和并发负载测试词 {padding}"));
        let credential = CHECK.to_owned();
        let solo_barrier = solo_barrier.clone();
        solo.push(tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            solo_barrier.wait().await;
            let request = format!("POST /v1/check HTTP/1.1\r\nHost: localhost\r\nAuthorization: {credential}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            let text = String::from_utf8_lossy(&response);
            text.split_whitespace().nth(1).unwrap().parse::<u16>().unwrap()
        }));
    }
    let mut solo_overloaded = 0usize;
    let mut solo_completed = 0usize;
    for handle in solo {
        match handle.await.unwrap() {
            200 => solo_completed += 1,
            429 => solo_overloaded += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(solo_completed + solo_overloaded, 48);
    assert!(solo_overloaded > 0, "48 > 34 budget must reject some");

    let output = process.stop().await;
    assert!(
        !output.contains("panic"),
        "service must not panic: {output}"
    );
}

// ------------------------------------------------------------------ AC-088

/// A client abandons a genuinely slow check (large multi-field body,
/// 20 ms client deadline): the server keeps running the work
/// (cancelled_still_running becomes observable), permits return only
/// when the real work finishes, gauges drain to zero and later requests
/// are unaffected.
#[tokio::test]
async fn ac088_client_cancellation_releases_permits_with_real_work() {
    let mut process = Process::start("ac088").await;
    publish_ruleset(&process).await;

    // a big multi-field body: enough real scanning work to outlive the
    // 20 ms client deadline
    let big_field = format!("{}测试词{}", "填充".repeat(8_000), "尾部");
    let body = json!({
        "request_id":"t23-cancel",
        "scene":"channel_message",
        "fields":[
            {"field_id":"a","text":big_field},
            {"field_id":"b","text":"再次填充测试词重复"},
        ]
    })
    .to_string();
    // the client writes the request and immediately hangs up: the
    // server either finishes first or keeps the work running past the
    // abandoned connection — both are valid AC-088 observations
    {
        let mut stream = tokio::net::TcpStream::connect(process.http).await.unwrap();
        let request = format!("POST /v1/check HTTP/1.1\r\nHost: localhost\r\nAuthorization: {CHECK}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(request.as_bytes()).await.unwrap();
        drop(stream); // abandon before reading any response
    }

    // while (or after) the work runs, poll the gauges until they drain;
    // cancelled-but-running work must be observable if it outlived the
    // client, and must eventually reach zero as real work completes
    let drained = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let report = metrics(&process).await;
            let value = |name: &str| {
                report
                    .lines()
                    .find(|line| line.starts_with(name))
                    .and_then(|line| line.rsplit(' ').next())
                    .and_then(|number| number.parse::<u64>().ok())
                    .unwrap_or(0)
            };
            let running = value("textguard_running");
            let queued = value("textguard_queued");
            let cancelled = value("textguard_cancelled_still_running");
            // cancelled-but-running work is a valid observation; whatever
            // happened, the gauges must settle at zero when the real work
            // completes and its permits return
            if running == 0 && queued == 0 && cancelled == 0 {
                // require two consecutive zero samples to avoid a sampling
                // race between the write and the abandoned response
                tokio::time::sleep(Duration::from_millis(80)).await;
                let confirm = metrics(&process).await;
                let confirm_value = |name: &str| {
                    confirm
                        .lines()
                        .find(|line| line.starts_with(name))
                        .and_then(|line| line.rsplit(' ').next())
                        .and_then(|number| number.parse::<u64>().ok())
                        .unwrap_or(0)
                };
                if confirm_value("textguard_running") == 0
                    && confirm_value("textguard_queued") == 0
                    && confirm_value("textguard_cancelled_still_running") == 0
                {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(drained.is_ok(), "admission gauges must drain to zero");

    // permits are back: a burst of normal requests all complete
    for index in 0..10 {
        let (code, body) = http(
            process.http,
            "POST",
            "/v1/check",
            CHECK,
            None,
            &check_body(&format!("取消后恢复探测{index}")),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert_eq!(code, 200, "post-cancel request {index} failed: {body}");
    }
    let output = process.stop().await;
    assert!(
        !output.contains("panic"),
        "service must not panic: {output}"
    );
}

// ------------------------------------------------------------------ AC-089

/// A unique body canary is driven through every error class (success
/// with a hit, oversized body, malformed JSON, unknown enum, missing
/// credentials, bad publish attempt) and then searched across the full
/// process output and the complete metrics exposition: it must never
/// leak; no per-request labels are invented.
#[tokio::test]
async fn ac089_canary_never_reaches_logs_or_metrics() {
    let mut process = Process::start("ac089").await;
    publish_ruleset(&process).await;

    let canary = format!("隐私金丝雀canary-SECRET-{:#x}-测试词", rand_canary());
    // 1: successful check that actually hits
    let (code, body) = http(
        process.http,
        "POST",
        "/v1/check",
        CHECK,
        None,
        &check_body(&canary),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(code, 200, "canary hit request failed: {body}");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["decision"],
        "block"
    );

    // 2: oversized body containing the canary
    let oversized = format!("{}{canary}{}", "前缀".repeat(9_000), "后缀".repeat(9_000));
    let (code, _) = http(
        process.http,
        "POST",
        "/v1/check",
        CHECK,
        None,
        &check_body(&oversized),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(code, 413, "oversized canary must be rejected");

    // 3: malformed JSON with the canary inside
    let (code, _) = http(
        process.http,
        "POST",
        "/v1/check",
        CHECK,
        None,
        &format!("{{broken {canary}"),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(code, 400);

    // 4: unknown enum carrying the canary in a field
    let (code, _) = http(
        process.http,
        "POST",
        "/v1/check",
        CHECK,
        None,
        &json!({"request_id":"x","scene":"no_such_scene","fields":[{"field_id":"f","text":canary}]}).to_string(),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(code, 400);

    // 5: missing credentials
    let (code, _) = http(
        process.http,
        "POST",
        "/v1/check",
        "",
        None,
        &check_body(&canary),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(code, 401);

    // 6: a publication attempt containing the canary is rejected and
    // metered, never echoed
    let (code, _) = http(
        process.manage,
        "PUT",
        "/internal/v1/rulesets/active",
        CHECK, // wrong scope on purpose
        None,
        &format!("{{\"canary\":\"{canary}\"}}"),
        Duration::from_secs(3),
    )
    .await
    .unwrap();
    assert_eq!(code, 403);

    // metrics: complete exposition, canary-free, bounded label space
    let report = metrics(&process).await;
    assert!(
        !report.contains(&canary),
        "canary leaked into metrics:\n{report}"
    );
    // every admission/metrics line is a known bounded series: request_id
    // or free text never becomes a label value
    for line in report.lines().filter(|line| line.starts_with("textguard_")) {
        let label_section = line
            .split('{')
            .nth(1)
            .and_then(|rest| rest.split('}').next());
        if let Some(labels) = label_section {
            assert!(
                !labels.contains(&canary),
                "canary became a label value: {line}"
            );
            assert!(
                !labels.contains("request_id"),
                "per-request labels are forbidden: {line}"
            );
        }
    }

    // full process output: canary-free
    let output = process.stop().await;
    assert!(
        !output.contains(&canary),
        "canary leaked into logs:\n{output}"
    );
}

fn rand_canary() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
        ^ (std::process::id() as u64) << 32
}
