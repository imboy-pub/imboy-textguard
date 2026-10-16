fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!(
            "imboy-textguard {} {}",
            env!("CARGO_PKG_VERSION"),
            imboy_textguard::BUILD_ID
        );
        return std::process::ExitCode::SUCCESS;
    }
    eprintln!("NOT_READY: data-plane implementation is not available in this skeleton");
    std::process::ExitCode::from(2)
}
