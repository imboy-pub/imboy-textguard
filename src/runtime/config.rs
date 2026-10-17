use crate::api::auth::AuthTokens;
use std::fs::OpenOptions;
use std::io::Read;
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Config {
    pub http: SocketAddr,
    pub grpc: SocketAddr,
    pub manage: SocketAddr,
    pub data_dir: PathBuf,
    pub tokens: Arc<AuthTokens>,
    pub table_id: String,
}

impl Config {
    pub fn from_env() -> Result<Self, &'static str> {
        let check = secret(&required("TEXTGUARD_CHECK_SECRET_FILE")?)?;
        let publish = secret(&required("TEXTGUARD_PUBLISH_SECRET_FILE")?)?;
        if check == publish {
            return Err("SECRET_SCOPES_MUST_DIFFER");
        }
        let table_id = std::env::var("TEXTGUARD_T2S_TABLE_ID")
            .unwrap_or_else(|_| crate::T2S_TABLE_ID.to_owned());
        if crate::normalize::table_by_id(&table_id).is_none() {
            return Err("UNSUPPORTED_RULESET");
        }
        let data_dir = PathBuf::from(required("TEXTGUARD_DATA_DIR")?);
        if !data_dir.is_absolute() {
            return Err("DATA_DIRECTORY_MUST_BE_ABSOLUTE");
        }
        Ok(Self {
            http: address("TEXTGUARD_HTTP_ADDR", "127.0.0.1:8081")?,
            grpc: address("TEXTGUARD_GRPC_ADDR", "127.0.0.1:8082")?,
            manage: address("TEXTGUARD_MANAGE_ADDR", "127.0.0.1:8083")?,
            data_dir,
            tokens: Arc::new(AuthTokens::new(&check, &publish)),
            table_id,
        })
    }
}

fn required(name: &str) -> Result<String, &'static str> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or("MISSING_CONFIGURATION")
}

fn address(name: &str, default: &str) -> Result<SocketAddr, &'static str> {
    let address: SocketAddr = std::env::var(name)
        .unwrap_or_else(|_| default.to_owned())
        .parse()
        .map_err(|_| "INVALID_LISTEN_ADDRESS")?;
    if !address.ip().is_loopback() {
        return Err("TLS_GATEWAY_REQUIRES_LOOPBACK_BACKEND");
    }
    Ok(address)
}

pub fn secret(path: impl AsRef<Path>) -> Result<String, &'static str> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "SECRET_FILE_UNREADABLE")?;
    let metadata = file.metadata().map_err(|_| "SECRET_FILE_UNREADABLE")?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err("SECRET_FILE_PERMISSIONS");
    }
    if metadata.len() > 4098 {
        return Err("INVALID_SECRET");
    }
    let mut bytes = Vec::new();
    file.take(4099)
        .read_to_end(&mut bytes)
        .map_err(|_| "SECRET_FILE_UNREADABLE")?;
    if bytes.len() > 4098 {
        return Err("INVALID_SECRET");
    }
    let value = String::from_utf8(bytes).map_err(|_| "INVALID_SECRET")?;
    let value = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(&value);
    if !(32..=4096).contains(&value.len()) || !value.bytes().all(|byte| (33..=126).contains(&byte))
    {
        return Err("INVALID_SECRET");
    }
    Ok(value.to_owned())
}
