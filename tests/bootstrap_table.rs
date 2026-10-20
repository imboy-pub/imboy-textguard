//! PR #1 must not compile the default-table seed against an alternate table.
//! The default table still bootstraps; an alternate supported table stays live
//! but not ready until an operator publishes a compatible complete package.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct IsolatedProcess {
    child: Child,
    directory: PathBuf,
}

impl Drop for IsolatedProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn ready_status(address: SocketAddr) -> Option<u16> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .ok()?;
    stream
        .write_all(b"GET /health/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut response = String::new();
    stream.take(4096).read_to_string(&mut response).ok()?;
    response.split_whitespace().nth(1)?.parse().ok()
}

#[test]
fn default_seed_and_alternate_table_have_explicit_readiness() {
    for (table, expected) in [
        (imboy_textguard::T2S_TABLE_ID, 200),
        (imboy_textguard::normalize::t2s::FIXTURE_TABLE_ID, 503),
    ] {
        let dir = std::env::temp_dir().join(format!(
            "textguard-bootstrap-table-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        for (name, value) in [
            ("check", "bootstrap-check-test-0123456789abcdef"),
            ("publish", "bootstrap-publish-test-0123456789abcdef"),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, value).unwrap();
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
            .env("TEXTGUARD_DATA_DIR", dir.join("rules"))
            .env("TEXTGUARD_CHECK_SECRET_FILE", dir.join("check"))
            .env("TEXTGUARD_PUBLISH_SECRET_FILE", dir.join("publish"))
            .env("TEXTGUARD_T2S_TABLE_ID", table)
            .env("TEXTGUARD_HTTP_ADDR", addresses[0].to_string())
            .env("TEXTGUARD_GRPC_ADDR", addresses[1].to_string())
            .env("TEXTGUARD_MANAGE_ADDR", addresses[2].to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut process = IsolatedProcess {
            child,
            directory: dir,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let actual = loop {
            assert!(
                process.child.try_wait().unwrap().is_none(),
                "startup exited for supported table {table}"
            );
            if let Some(code) = ready_status(addresses[0]) {
                break code;
            }
            assert!(Instant::now() < deadline, "startup did not listen");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(actual, expected, "table {table}");
    }
}
