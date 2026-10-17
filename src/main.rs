#[tokio::main]
async fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!(
            "imboy-textguard {} {}",
            env!("CARGO_PKG_VERSION"),
            imboy_textguard::BUILD_ID
        );
        return std::process::ExitCode::SUCCESS;
    }
    if std::env::args().nth(1).as_deref() == Some("--healthcheck") {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let probe = async {
            let address: std::net::SocketAddr = std::env::var("TEXTGUARD_HTTP_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8081".to_owned())
                .parse()
                .ok()?;
            if !address.ip().is_loopback() {
                return None;
            }
            let mut socket = tokio::net::TcpStream::connect(address).await.ok()?;
            socket
                .write_all(
                    b"GET /health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .await
                .ok()?;
            let mut response = [0u8; 12];
            socket.read_exact(&mut response).await.ok()?;
            (response == *b"HTTP/1.1 200").then_some(())
        };
        return if matches!(
            tokio::time::timeout(std::time::Duration::from_secs(1), probe).await,
            Ok(Some(()))
        ) {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::from(1)
        };
    }
    std::panic::set_hook(Box::new(|_| eprintln!("INTERNAL_ERROR")));
    #[cfg(feature = "fault-injection")]
    if let Err(error) = activate_fault_rules_from_env() {
        eprintln!("NOT_READY: {error}");
        return std::process::ExitCode::from(2);
    }
    let config = match imboy_textguard::runtime::config::Config::from_env() {
        Ok(config) => config,
        Err(code) => {
            eprintln!("NOT_READY: {code}");
            return std::process::ExitCode::from(2);
        }
    };
    match imboy_textguard::runtime::server::serve(config, shutdown()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(code) => {
            eprintln!("NOT_READY: {code}");
            std::process::ExitCode::from(2)
        }
    }
}

async fn shutdown() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("SIGNAL_SETUP_FAILED");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => (),
        _ = terminate.recv() => (),
    }
}

/// Test-only activation (compiled behind `fault-injection`):
/// `TEXTGUARD_FAULT_KILL_AT=<point>` makes the process die instantly the
/// first time publication reaches that commit boundary. Production
/// builds never read this variable.
#[cfg(feature = "fault-injection")]
fn activate_fault_rules_from_env() -> Result<(), String> {
    let kill_at = std::env::var("TEXTGUARD_FAULT_KILL_AT").unwrap_or_default();
    if kill_at.is_empty() {
        return Ok(());
    }
    let point = imboy_textguard::rules::fault::point_from_name(&kill_at)
        .ok_or_else(|| format!("unknown fault point: {kill_at}"))?;
    imboy_textguard::rules::fault::install(vec![(
        point,
        imboy_textguard::rules::fault::Action::KillProcess,
    )]);
    Ok(())
}
