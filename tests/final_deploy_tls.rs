//! T23 final verification, part 2 (AC-090): deployment authentication
//! boundaries with the real binary and a real nginx TLS gateway.
//!
//! The service's security model is TLS-gateway: backends only ever bind
//! loopback (a non-loopback bind refuses to start), secrets must be
//! permission-bounded files, and cross-host traffic is terminated by the
//! gateway in `deploy/nginx-tls.conf`. These tests prove, on a real
//! process and a real nginx:
//!
//! * a non-loopback listen address never starts (no plaintext
//!   cross-host path can be configured by accident),
//! * missing secrets / world-readable secrets refuse to start (no
//!   default weak credentials),
//! * the manage port keeps its separate credential scope,
//! * a real nginx gateway with a generated CA terminates TLS and proxies
//!   a check to the loopback backend; plaintext HTTP against the TLS
//!   port is rejected; a client that does not trust the CA refuses the
//!   connection (wrong certificate rejected).
//!
//! Environment requirements (recorded in evidence): `nginx` and
//! `openssl` on PATH — both present on this validation host.

use serde_json::{json, Value};
use std::net::{SocketAddr, TcpListener};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHECK: &str = "t23b-check-canary-0123456789-abcde";
const PUBLISH: &str = "t23b-publish-canary-0123456789-abcdef";
const RULESET: &str = include_str!("fixtures/contract-ruleset.json");

// ---------------------------------------------------------- startup refusals

/// Each misconfiguration must refuse to start with exit code 2 and an
/// explicit NOT_READY reason: no fallback, no default credentials.
#[tokio::test]
async fn ac090_misconfigured_startup_always_refuses() {
    let base = temp_dir("ac090-startup");
    for (name, env) in [
        (
            "non_loopback_http",
            vec![
                ("TEXTGUARD_HTTP_ADDR", "0.0.0.0:9871".to_owned()),
                ("TEXTGUARD_GRPC_ADDR", "127.0.0.1:0".to_owned()),
                ("TEXTGUARD_MANAGE_ADDR", "127.0.0.1:0".to_owned()),
            ],
        ),
        (
            "non_loopback_grpc",
            vec![
                ("TEXTGUARD_HTTP_ADDR", "127.0.0.1:0".to_owned()),
                ("TEXTGUARD_GRPC_ADDR", "10.0.0.1:9872".to_owned()),
                ("TEXTGUARD_MANAGE_ADDR", "127.0.0.1:0".to_owned()),
            ],
        ),
        (
            "non_loopback_manage",
            vec![
                ("TEXTGUARD_HTTP_ADDR", "127.0.0.1:0".to_owned()),
                ("TEXTGUARD_GRPC_ADDR", "127.0.0.1:0".to_owned()),
                ("TEXTGUARD_MANAGE_ADDR", "0.0.0.0:9873".to_owned()),
            ],
        ),
    ] {
        let output = run_binary(&base, &env, None).await;
        assert_eq!(
            output.0.code(),
            Some(2),
            "{name}: must refuse to start, got {:?}",
            output.0
        );
        assert!(
            output.1.contains("TLS_GATEWAY_REQUIRES_LOOPBACK_BACKEND"),
            "{name}: reason must name the loopback policy: {}",
            output.1
        );
    }

    // missing secret file
    let output = run_binary(
        &base,
        &[
            ("TEXTGUARD_HTTP_ADDR", "127.0.0.1:0".to_owned()),
            ("TEXTGUARD_GRPC_ADDR", "127.0.0.1:0".to_owned()),
            ("TEXTGUARD_MANAGE_ADDR", "127.0.0.1:0".to_owned()),
        ],
        Some("missing"),
    )
    .await;
    assert_eq!(output.0.code(), Some(2));
    assert!(
        output.1.contains("SECRET_FILE_UNREADABLE"),
        "missing secret reason: {}",
        output.1
    );

    // world-readable secret file
    let world = base.join("world-secret");
    std::fs::write(&world, "0123456789abcdef0123456789abcdef").unwrap();
    std::fs::set_permissions(&world, std::fs::Permissions::from_mode(0o644)).unwrap();
    let output = run_binary(
        &base,
        &[
            ("TEXTGUARD_HTTP_ADDR", "127.0.0.1:0".to_owned()),
            ("TEXTGUARD_GRPC_ADDR", "127.0.0.1:0".to_owned()),
            ("TEXTGUARD_MANAGE_ADDR", "127.0.0.1:0".to_owned()),
        ],
        Some("world"),
    )
    .await;
    assert_eq!(output.0.code(), Some(2));
    assert!(
        output.1.contains("SECRET_FILE_PERMISSIONS"),
        "weak secret reason: {}",
        output.1
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// The management plane keeps its own credential: the check credential
/// cannot publish, the publish credential is refused on the data plane.
#[tokio::test]
async fn ac090_manage_credentials_stay_scoped_on_real_process() {
    let mut process = Process::start("ac090-scoped").await;
    // check token on manage publish → forbidden
    let (code, _) = process
        .http(
            process.manage,
            "PUT",
            "/internal/v1/rulesets/active",
            CHECK,
            None,
            RULESET,
        )
        .await
        .unwrap();
    assert_eq!(code, 403);
    // publish token on the data plane → unauthorized/forbidden
    let (code, _) = process
        .http(
            process.http,
            "POST",
            "/v1/check",
            PUBLISH,
            None,
            &json!({"request_id":"x","scene":"channel_message","fields":[{"field_id":"body","text":"y"}]}).to_string(),
        )
        .await
        .unwrap();
    assert!(
        code == 401 || code == 403,
        "data plane must reject the manage credential: {code}"
    );
    // no credential at all → unauthorized
    let (code, _) = process
        .http(process.http, "POST", "/v1/check", "", None, "{}")
        .await
        .unwrap();
    assert_eq!(code, 401);
    process.stop().await;
}

// ------------------------------------------------------------- TLS gateway

/// A real nginx TLS gateway in front of the real service: a trusted CA
/// proxies an encrypted check end to end; plaintext against the TLS
/// port is refused; a client that does not trust the certificate
/// rejects it (verify error) instead of continuing.
#[tokio::test]
async fn ac090_real_nginx_tls_gateway_boundaries() {
    let dir = temp_dir("ac090-tls");
    // 1. generate a throwaway CA + server certificate
    let ca_key = dir.join("ca.key");
    let ca_cert = dir.join("ca.crt");
    let server_key = dir.join("server.key");
    let server_csr = dir.join("server.csr");
    let server_cert = dir.join("server.crt");
    assert!(
        Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                ca_key.to_str().unwrap(),
                "-out",
                ca_cert.to_str().unwrap(),
                "-subj",
                "/CN=t23-test-ca",
                "-days",
                "2"
            ])
            .output()
            .unwrap()
            .status
            .success(),
        "openssl CA generation failed"
    );
    assert!(Command::new("openssl")
        .args([
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            server_key.to_str().unwrap(),
            "-out",
            server_csr.to_str().unwrap(),
            "-subj",
            "/CN=localhost"
        ])
        .output()
        .unwrap()
        .status
        .success());
    assert!(
        Command::new("openssl")
            .args([
                "x509",
                "-req",
                "-in",
                server_csr.to_str().unwrap(),
                "-CA",
                ca_cert.to_str().unwrap(),
                "-CAkey",
                ca_key.to_str().unwrap(),
                "-CAcreateserial",
                "-out",
                server_cert.to_str().unwrap(),
                "-days",
                "2",
                "-extfile",
                "/dev/stdin"
            ])
            .stdin(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(b"subjectAltName=DNS:localhost,IP:127.0.0.1\n")?;
                child.wait_with_output()
            })
            .unwrap()
            .status
            .success(),
        "server certificate signing failed"
    );

    // 2. start the real backend on loopback with a published ruleset
    let mut process = Process::start("ac090-tls").await;
    let (code, active) = process
        .http(
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
    let etag = serde_json::from_str::<Value>(&active).unwrap()["state_etag"]
        .as_str()
        .unwrap()
        .to_owned();
    let (code, body) = process
        .http(
            process.manage,
            "PUT",
            "/internal/v1/rulesets/active",
            PUBLISH,
            Some(&etag),
            RULESET,
        )
        .await
        .unwrap();
    assert_eq!(code, 200, "publish: {body}");

    // 3. nginx gateway: TLS on a free port, proxy /v1/ to the backend
    let tls_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tls_port = tls_listener.local_addr().unwrap().port();
    drop(tls_listener);
    let conf = dir.join("nginx.conf");
    std::fs::write(
        &conf,
        format!(
            "worker_processes 1;\n\
             pid {pid};\n\
             error_log {err} warn;\n\
             daemon off;\n\
             events {{ worker_connections 128; }}\n\
             http {{\n\
               access_log off;\n\
               client_body_temp_path {tmp}/body;\n\
               proxy_temp_path {tmp}/proxy;\n\
               ssl_certificate {cert};\n\
               ssl_certificate_key {key};\n\
               ssl_protocols TLSv1.2 TLSv1.3;\n\
               server {{\n\
                 listen {port} ssl;\n\
                 client_max_body_size 256k;\n\
                 location /v1/ {{\n\
                   proxy_connect_timeout 20ms;\n\
                   proxy_read_timeout 2s;\n\
                   proxy_pass http://{backend};\n\
                 }}\n\
                 location / {{ return 404; }}\n\
               }}\n\
             }}\n",
            pid = dir.join("nginx.pid").display(),
            err = dir.join("nginx-error.log").display(),
            tmp = dir.display(),
            cert = server_cert.display(),
            key = server_key.display(),
            port = tls_port,
            backend = process.http,
        ),
    )
    .unwrap();
    let nginx = Command::new("nginx")
        .args(["-c", conf.to_str().unwrap(), "-p", dir.to_str().unwrap()])
        .spawn()
        .expect("nginx must be available on this host");
    // a Drop guard: even if an assertion below panics, the host nginx and
    // its temp directory are still torn down (mirrors the Process guard)
    struct NginxGuard {
        child: std::process::Child,
        conf: PathBuf,
        prefix: PathBuf,
    }
    impl NginxGuard {
        fn running(&mut self) -> bool {
            self.child.try_wait().ok().flatten().is_none()
        }
        fn finish(mut self) {
            let _ = Command::new("nginx")
                .args([
                    "-c",
                    self.conf.to_str().unwrap(),
                    "-p",
                    self.prefix.to_str().unwrap(),
                    "-s",
                    "stop",
                ])
                .output();
            let _ = self.child.wait();
        }
    }
    impl Drop for NginxGuard {
        fn drop(&mut self) {
            if self.running() {
                let _ = Command::new("nginx")
                    .args([
                        "-c",
                        self.conf.to_str().unwrap(),
                        "-p",
                        self.prefix.to_str().unwrap(),
                        "-s",
                        "stop",
                    ])
                    .output();
                let _ = self.child.wait();
            }
        }
    }
    let mut nginx = NginxGuard {
        child: nginx,
        conf,
        prefix: dir.clone(),
    };
    // wait for the TLS listener
    let gateway = format!("127.0.0.1:{tls_port}");
    let mut live = false;
    for _ in 0..200 {
        if std::net::TcpStream::connect(&gateway).is_ok() {
            live = true;
            break;
        }
        assert!(nginx.running(), "nginx exited early");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(live, "nginx TLS gateway did not come up");

    // 4. trusted client: TLS handshake + proxied check succeeds
    let body = json!({"request_id":"t23-tls","scene":"channel_message","fields":[{"field_id":"body","text":"TLS网关测试词"}]}).to_string();
    let proxied = tls_http(&gateway, &ca_cert, "POST", "/v1/check", CHECK, &body).await;
    assert!(
        proxied.contains("HTTP/1.1 200"),
        "trusted TLS client must reach the backend: {proxied}"
    );
    assert!(proxied.contains("\"decision\":\"block\""), "{proxied}");

    // 5. plaintext HTTP against the TLS port is refused by nginx
    let plain = plain_http(&gateway, "GET", "/v1/check").await;
    assert!(
        plain.contains("400") || plain.is_empty(),
        "plaintext must not be served on the TLS port: {plain}"
    );

    // 6. a client that does not trust the CA rejects the certificate
    //    (verify error) — the wrong-certificate boundary holds on the
    //    client side; the gateway never downgrades to plaintext.
    let distrust = Command::new("openssl")
        .args([
            "s_client",
            "-connect",
            &gateway,
            "-CAfile",
            "/dev/null",
            "-verify_return_error",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.as_mut().unwrap().write_all(b"Q\n")?;
            child.wait_with_output()
        })
        .unwrap();
    let distrust_text = distrust.stdout.clone();
    let distrust_text = String::from_utf8_lossy(&distrust_text).into_owned();
    assert!(
        distrust_text.contains("verify error")
            || distrust_text.contains("verification failed")
            || !distrust.status.success(),
        "untrusting client must fail verification: {}",
        &distrust_text[..distrust_text.len().min(400)]
    );

    // cleanup (the guard also covers panic paths)
    nginx.finish();
    process.stop().await;
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ helpers

fn temp_dir(tag: &str) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "textguard-t23b-{tag}-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

/// Run the binary to completion with the given environment; `secret_mode`
/// prepares check/publish secret files ("ok", "missing", "world").
async fn run_binary(
    base: &Path,
    env: &[(&str, String)],
    secret_mode: Option<&str>,
) -> (std::process::ExitStatus, String) {
    let dir = temp_dir("ac090-run");
    for (name, secret) in [("check", CHECK), ("publish", PUBLISH)] {
        let path = dir.join(name);
        std::fs::write(&path, secret).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let check_file = match secret_mode {
        Some("missing") => dir.join("absent").display().to_string(),
        Some("world") => base.join("world-secret").display().to_string(),
        _ => dir.join("check").display().to_string(),
    };
    let publish_file = match secret_mode {
        Some("missing") | Some("world") => dir.join("publish").display().to_string(),
        _ => dir.join("publish").display().to_string(),
    };
    let mut command = Command::new(env!("CARGO_BIN_EXE_imboy-textguard"));
    command
        .env("TEXTGUARD_DATA_DIR", dir.join("rules"))
        .env("TEXTGUARD_CHECK_SECRET_FILE", &check_file)
        .env("TEXTGUARD_PUBLISH_SECRET_FILE", &publish_file)
        .env("TEXTGUARD_T2S_TABLE_ID", "test-t2s-v1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    // the process is expected to exit immediately; wait it out
    let output = command.output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status, text)
    // `dir` intentionally left for inspection until the caller cleans base
}

struct Process {
    child: std::process::Child,
    directory: PathBuf,
    http: SocketAddr,
    manage: SocketAddr,
}

impl Process {
    async fn start(tag: &str) -> Self {
        let directory = temp_dir(tag);
        for (name, secret) in [("check", CHECK), ("publish", PUBLISH)] {
            let path = directory.join(name);
            std::fs::write(&path, secret).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let listeners: Vec<_> = (0..2)
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
            .env("TEXTGUARD_GRPC_ADDR", "127.0.0.1:0")
            .env("TEXTGUARD_MANAGE_ADDR", addresses[1].to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut process = Self {
            child,
            directory,
            http: addresses[0],
            manage: addresses[1],
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

    async fn http(
        &self,
        address: SocketAddr,
        method: &str,
        path: &str,
        credential: &str,
        etag: Option<&str>,
        body: &str,
    ) -> Option<(u16, String)> {
        let mut stream = tokio::net::TcpStream::connect(address).await.ok()?;
        let etag = etag.map_or(String::new(), |etag| format!("If-Match: {etag}\r\n"));
        let request = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {credential}\r\n{etag}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut response = Vec::new();
        let read =
            tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
        let Ok(Ok(_)) = read else { return None };
        let text = String::from_utf8_lossy(&response).into_owned();
        let (head, body) = text.split_once("\r\n\r\n")?;
        Some((
            head.split_whitespace().nth(1)?.parse().ok()?,
            body.to_owned(),
        ))
    }

    async fn stop(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        for _ in 0..300 {
            if self.child.try_wait().unwrap().is_some() {
                let _ = std::fs::remove_dir_all(&self.directory);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("graceful shutdown exceeded");
    }
}

/// One HTTPS request through `openssl s_client` (no TLS client crate in
/// the dependency tree): write the raw HTTP request, read everything.
async fn tls_http(
    gateway: &str,
    ca_cert: &Path,
    method: &str,
    path: &str,
    credential: &str,
    body: &str,
) -> String {
    let request = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: {credential}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}\nQ\n", body.len());
    let gateway = gateway.to_owned();
    let ca = ca_cert.to_owned();
    let output = tokio::task::spawn_blocking(move || {
        let mut child = Command::new("openssl")
            .args([
                "s_client",
                "-connect",
                &gateway,
                "-CAfile",
                ca.to_str().unwrap(),
                "-quiet",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write as _;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(request.as_bytes())
            .unwrap();
        let _ = child.stdin.as_mut().unwrap().flush();
        std::thread::sleep(Duration::from_millis(600));
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    })
    .await
    .unwrap();
    output
}

async fn plain_http(gateway: &str, method: &str, path: &str) -> String {
    let mut stream = match tokio::net::TcpStream::connect(gateway).await {
        Ok(stream) => stream,
        Err(_) => return String::new(),
    };
    let request =
        format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).await.is_err() {
        return String::new();
    }
    let mut response = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut response)).await;
    String::from_utf8_lossy(&response).into_owned()
}
