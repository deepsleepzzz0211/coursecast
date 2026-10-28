//! API key storage (ticket 05): the MiMo key lives in the Windows Credential Manager
//! (keyring crate), per spec 权限边界 — never in project.json, logs or command output.
//!
//! Test seam: setting `CC_CREDENTIALS_FILE` redirects storage to that plaintext file.
//! Tests only — integration tests need the key to survive across child processes, which
//! the in-process OS store API cannot offer a hermetic substitute for. Production never
//! sets the variable and always lands in the credential manager.

use crate::error::{Error, Result};
use std::path::PathBuf;

const SERVICE: &str = "CourseCast";
const USER: &str = "mimo-api-key";

fn credential_store_error(e: keyring::Error) -> Error {
    Error::CredentialStore(e.to_string())
}

/// Test seam target, if set.
fn file_seam() -> Option<PathBuf> {
    std::env::var_os("CC_CREDENTIALS_FILE").map(PathBuf::from)
}

/// One credential handle per process. In production this is just a handle into the OS
/// store (reads always hit the real store); it also makes the keyring *mock* backend
/// usable in tests, since the mock keeps its data inside the entry object itself.
fn entry() -> Result<&'static keyring::Entry> {
    static ENTRY: std::sync::OnceLock<std::result::Result<keyring::Entry, String>> =
        std::sync::OnceLock::new();
    ENTRY
        .get_or_init(|| keyring::Entry::new(SERVICE, USER).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| Error::CredentialStore(e.clone()))
}

/// Store (or overwrite) the MiMo API key.
pub fn store_api_key(key: &str) -> Result<()> {
    if let Some(path) = file_seam() {
        return std::fs::write(path, key).map_err(|e| Error::CredentialStore(e.to_string()));
    }
    entry()?.set_password(key).map_err(credential_store_error)
}

/// The stored key, or `None` when none was configured yet.
pub fn read_api_key() -> Result<Option<String>> {
    if let Some(path) = file_seam() {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::CredentialStore(e.to_string())),
        };
        return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
    }
    match entry()?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(credential_store_error(e)),
    }
}
