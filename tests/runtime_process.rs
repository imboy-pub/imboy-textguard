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

const CHECK: &str = "test-check-canary-0123456789-abcdefgh";
const PUBLISH: &str = "test-publish-canary-0123456789-abcdef";

struct Process {
    child: Child,
    directory: PathBuf,
    http: SocketAddr,
    grpc: SocketAddr,
    manage: SocketAddr,
}

impl Process {
    async fn start() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "textguard-process-{}-{}",
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
            assert!(
                process.child.try_wait().unwrap().is_none(),
                "service exited before becoming live"
            );
            if tokio::net::TcpStream::connect(process.http).await.is_ok() {
                return process;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let identity = Command::new("ps")
            .args([
                "-p",
                &process.child.id().to_string(),
                "-o",
                "pid=,stat=,comm=",
            ])
            .output()
            .unwrap();
        let _ = process.child.kill();
        let _ = process.child.wait();
        use std::io::Read;
        let mut error = String::new();
        process
            .child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .unwrap();
        panic!(
            "service did not become live: {} {error}",
            String::from_utf8_lossy(&identity.stdout)
        );
    }

    async fn stop(&mut self) -> Vec<u8> {
        assert!(Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap()
            .success());
        for _ in 0..300 {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                use std::io::Read;
                let mut output = Vec::new();
                self.child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_end(&mut output)
                    .unwrap();
                self.child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_end(&mut output)
                    .unwrap();
                return output;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("graceful shutdown exceeded 3 seconds without active requests");
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

struct RawGrpcBody {
    bytes: Option<axum::body::Bytes>,
    delay: std::pin::Pin<Box<tokio::time::Sleep>>,
}

impl http_body::Body for RawGrpcBody {
    type Data = axum::body::Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        use std::future::Future;
        if self.delay.as_mut().poll(context).is_pending() {
            return std::task::Poll::Pending;
        }
        std::task::Poll::Ready(
            self.bytes
                .take()
                .map(|bytes| Ok(http_body::Frame::data(bytes))),
        )
    }
}

async fn raw_grpc(
    address: SocketAddr,
    path: &str,
    bytes: Vec<u8>,
    delay: Duration,
    headers: &[(&str, &str)],
) -> axum::http::Response<tonic::body::BoxBody> {
    use tonic::codegen::Service;
    let mut channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut request = axum::http::Request::post(path)
        .header("content-type", "application/grpc")
        .header("authorization", CHECK)
        .header("te", "trailers");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = tonic::body::BoxBody::new(RawGrpcBody {
        bytes: Some(bytes.into()),
        delay: Box::pin(tokio::time::sleep(delay)),
    });
    std::future::poll_fn(|context| channel.poll_ready(context))
        .await
        .unwrap();
    channel.call(request.body(body).unwrap()).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn health_watch_explicit_deadline_ends_stream_and_releases_permit() {
    use http_body::Body;
    let mut process = Process::start().await;
    let mut response = raw_grpc(
        process.grpc,
        "/grpc.health.v1.Health/Watch",
        vec![0, 0, 0, 0, 0],
        Duration::ZERO,
        &[("grpc-timeout", "100m")],
    )
    .await;
    let first =
        std::future::poll_fn(|context| std::pin::Pin::new(response.body_mut()).poll_frame(context))
            .await
            .unwrap()
            .unwrap();
    assert!(first.is_data());
    let trailers = tokio::time::timeout(Duration::from_millis(400), async {
        loop {
            let frame = std::future::poll_fn(|context| {
                std::pin::Pin::new(response.body_mut()).poll_frame(context)
            })
            .await
            .unwrap()
            .unwrap();
            if let Ok(trailers) = frame.into_trailers() {
                break trailers;
            }
        }
    })
    .await
    .expect("server must finish Watch even if the client keeps receiving");
    assert_eq!(trailers["grpc-status"], "4");
    assert_eq!(trailers["x-textguard-error-code"], "CHECK_DEADLINE");
    drop(response);
    process.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grpc_codec_and_body_failures_are_counted_once_without_double_counting_handlers() {
    let mut process = Process::start().await;
    for (bytes, delay, headers, expected) in [
        (vec![0, 0, 0, 0, 1, 255], Duration::ZERO, vec![], "13"),
        (vec![0, 0, 4, 0, 1], Duration::ZERO, vec![], "11"),
        (
            vec![0, 0, 0, 0, 0],
            Duration::ZERO,
            vec![("grpc-encoding", "gzip")],
            "12",
        ),
        (
            vec![0, 0, 0, 0, 0],
            Duration::from_millis(1200),
            vec![],
            "4",
        ),
        (vec![0, 0, 0, 0, 0], Duration::ZERO, vec![], "3"),
    ] {
        let response = raw_grpc(
            process.grpc,
            "/imboy.textguard.v1.TextGuard/CheckText",
            bytes,
            delay,
            &headers,
        )
        .await;
        assert_eq!(response.headers()["grpc-status"], expected);
    }
    let (_, metrics) = http_raw(process.manage, "GET", "/metrics", "", None, "").await;
    assert!(
        metrics.contains("textguard_requests_total{scene=\"unknown\",transport=\"grpc\"} 5"),
        "{metrics}"
    );
    for (code, count) in [
        ("PROTOCOL_ERROR", 2),
        ("TEXT_TOO_LARGE", 1),
        ("CHECK_DEADLINE", 1),
        ("INVALID_ARGUMENT", 1),
    ] {
        assert!(metrics.contains(&format!("textguard_errors_total{{scene=\"unknown\",transport=\"grpc\",error_code=\"{code}\"}} {count}")), "{metrics}");
    }
    process.stop().await;
}

async fn http(
    address: SocketAddr,
    method: &str,
    path: &str,
    credential: &str,
    etag: Option<&str>,
    body: &str,
) -> (u16, Value) {
    let (status, body) = http_raw(address, method, path, credential, etag, body).await;
    (
        status,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&body).unwrap()
        },
    )
}

async fn http_raw(
    address: SocketAddr,
    method: &str,
    path: &str,
    credential: &str,
    etag: Option<&str>,
    body: &str,
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let etag = etag.map_or(String::new(), |etag| format!("If-Match: {etag}\r\n"));
    let request = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {credential}\r\n{etag}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    let response = String::from_utf8(response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        body.to_owned(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_binary_publishes_and_serves_both_protocols_then_exits_cleanly() {
    let mut process = Process::start().await;
    assert!(Command::new(env!("CARGO_BIN_EXE_imboy-textguard"))
        .arg("--healthcheck")
        .env("TEXTGUARD_HTTP_ADDR", process.http.to_string())
        .status()
        .unwrap()
        .success());
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{}", process.grpc))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut health = tonic_health::pb::health_client::HealthClient::new(channel);
    let mut readiness = health
        .watch(tonic_health::pb::HealthCheckRequest {
            service: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(readiness.message().await.unwrap().unwrap().status, 2);
    assert_eq!(
        http(process.http, "GET", "/health/ready", "", None, "")
            .await
            .0,
        503
    );
    assert_eq!(
        http(
            process.http,
            "PUT",
            "/internal/v1/rulesets/active",
            PUBLISH,
            None,
            "{}"
        )
        .await
        .0,
        404
    );
    assert_eq!(
        http(
            process.manage,
            "GET",
            "/internal/v1/rulesets/active",
            CHECK,
            None,
            ""
        )
        .await
        .0,
        403
    );
    let (_, active) = http(
        process.manage,
        "GET",
        "/internal/v1/rulesets/active",
        PUBLISH,
        None,
        "",
    )
    .await;
    let etag = active["state_etag"].as_str().unwrap();
    assert_eq!(
        http(
            process.manage,
            "PUT",
            "/internal/v1/rulesets/active",
            PUBLISH,
            Some(etag),
            include_str!("fixtures/contract-ruleset.json")
        )
        .await
        .0,
        200
    );
    let text = "process-body-canary-unique-测试词";
    let request = json!({"request_id":"process-1","scene":"channel_message","fields":[{"field_id":"body","text":text}]}).to_string();
    let (status, checked) = http(process.http, "POST", "/v1/check", CHECK, None, &request).await;
    assert_eq!(status, 200);
    assert_eq!(checked["decision"], "block");
    tokio::time::timeout(Duration::from_secs(1), async {
        for _ in 0..8 {
            if readiness.message().await.unwrap().unwrap().status == 1 {
                return;
            }
        }
        panic!("health did not report the published ready snapshot");
    })
    .await
    .unwrap();
    drop(readiness);
    drop(health);
    let mut client = TextGuardClient::connect(format!("http://{}", process.grpc))
        .await
        .unwrap();
    let mut request = tonic::Request::new(CheckTextRequest {
        request_id: Some("process-1".to_owned()),
        scene: Some(1),
        fields: vec![TextField {
            field_id: Some("body".to_owned()),
            text: Some(text.to_owned()),
        }],
    });
    request
        .metadata_mut()
        .insert("authorization", CHECK.parse().unwrap());
    request.set_timeout(Duration::from_millis(150));
    let checked_grpc = client.check_text(request).await.unwrap().into_inner();
    assert_eq!(checked_grpc.decision, Some(3));
    assert_eq!(
        checked_grpc.ruleset_revision.as_deref(),
        checked["ruleset_revision"].as_str()
    );
    drop(client);
    let (status, metrics) = http_raw(process.manage, "GET", "/metrics", "", None, "").await;
    assert_eq!(status, 200);
    assert!(metrics.contains("textguard_publish_success_total 1"));
    assert!(metrics.contains("scene=\"channel_message\",transport=\"http\"} 1"));
    assert!(metrics.contains("scene=\"channel_message\",transport=\"grpc\"} 1"));
    for canary in [CHECK, PUBLISH, text, "process-1", "测试词"] {
        assert!(!metrics.contains(canary));
    }
    assert_eq!(
        http_raw(process.http, "GET", "/metrics", "", None, "")
            .await
            .0,
        404
    );
    let logs = process.stop().await;
    let logs = String::from_utf8(logs).unwrap();
    for canary in [CHECK, PUBLISH, text, "测试词"] {
        assert!(!logs.contains(canary));
    }
}

#[tokio::test]
async fn incomplete_http2_preface_releases_the_socket() {
    let mut process = Process::start().await;
    let mut socket = tokio::net::TcpStream::connect(process.grpc).await.unwrap();
    socket.write_all(b"P").await.unwrap();
    let mut byte = [0u8; 1];
    let result = tokio::time::timeout(Duration::from_millis(1600), socket.read(&mut byte)).await;
    assert!(
        matches!(result, Ok(Ok(0)) | Ok(Err(_))),
        "partial HTTP/2 preface retained the connection beyond its header budget"
    );
    drop(socket);
    process.stop().await;
}

#[test]
fn startup_without_secrets_fails_closed_without_echoing_environment() {
    let output = Command::new(env!("CARGO_BIN_EXE_imboy-textguard"))
        .env_remove("TEXTGUARD_CHECK_SECRET_FILE")
        .env_remove("TEXTGUARD_PUBLISH_SECRET_FILE")
        .env("UNRELATED_CANARY", "environment-canary-must-not-leak")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = String::from_utf8(output.stderr).unwrap();
    assert!(output.contains("NOT_READY"));
    assert!(!output.contains("environment-canary-must-not-leak"));
}

#[test]
fn secret_loader_rejects_permissive_symlink_and_weak_files() {
    let directory = std::env::temp_dir().join(format!("textguard-secret-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("secret");
    std::fs::write(&path, CHECK).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(imboy_textguard::runtime::config::secret(&path).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        imboy_textguard::runtime::config::secret(&path).unwrap(),
        CHECK
    );
    std::os::unix::fs::symlink(&path, directory.join("link")).unwrap();
    assert!(imboy_textguard::runtime::config::secret(directory.join("link")).is_err());
    std::fs::write(&path, format!("{}\nextra", "a".repeat(4096))).unwrap();
    assert!(imboy_textguard::runtime::config::secret(&path).is_err());
    std::fs::write(&path, "weak").unwrap();
    assert!(imboy_textguard::runtime::config::secret(&path).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
