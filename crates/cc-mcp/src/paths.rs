//! MCP 权限边界 (spec Implementation Decisions): path arguments are canonicalized
//! and validated before any tool touches them.
//!
//! Two rules, mirroring the CLI's semantics exactly:
//! - **Relative paths** resolve inside the project root; after canonicalization
//!   anything escaping the root (`../../secrets`) is rejected with `path_escape`.
//! - **Absolute paths** are user-provided external inputs (a deck on the desktop, a
//!   phone recording) and stay as usable as `cc-cli --source`/`--audio` — the MCP
//!   tools are thin wrappers over the same calls, not a stricter dialect.

use cc_core::{Error, Result};
use std::path::{Path, PathBuf};

pub(crate) fn resolve_input(project_root: &Path, user_path: &str) -> Result<PathBuf> {
    let raw = Path::new(user_path);
    if raw.is_absolute() {
        return raw
            .canonicalize()
            .map_err(|_| Error::SourceNotFound(user_path.to_string()));
    }
    let root = project_root
        .canonicalize()
        .map_err(|_| Error::SourceNotFound(project_root.display().to_string()))?;
    let joined = root.join(raw);
    let canonical = joined
        .canonicalize()
        .map_err(|_| Error::SourceNotFound(user_path.to_string()))?;
    if canonical != root && !canonical.starts_with(&root) {
        return Err(Error::PathEscape(user_path.to_string()));
    }
    Ok(canonical)
}
