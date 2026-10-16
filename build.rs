fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/textguard.proto");
    println!("cargo:rerun-if-env-changed=TEXTGUARD_BUILD_ID");
    let build_id =
        std::env::var("TEXTGUARD_BUILD_ID").unwrap_or_else(|_| "development-unreleased".into());
    if build_id.is_empty()
        || build_id.len() > 64
        || !build_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err("invalid TEXTGUARD_BUILD_ID".into());
    }
    println!("cargo:rustc-env=TEXTGUARD_BUILD_ID={build_id}");
    tonic_build::configure().compile_protos(&["proto/textguard.proto"], &["proto"])?;
    Ok(())
}
