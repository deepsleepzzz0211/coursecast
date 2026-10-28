//! Shared argument-path resolution for commands: as given → project `source/` →
//! project root. Keeps validation at the command boundary without each command
//! re-implementing the same walk.

use std::path::{Path, PathBuf};

pub(super) fn resolve(dir: &Path, arg: &Path) -> PathBuf {
    if arg.is_file() || arg.is_dir() {
        return arg.to_path_buf();
    }
    let in_source = dir.join("source").join(arg);
    if in_source.is_file() || in_source.is_dir() {
        return in_source;
    }
    let in_root = dir.join(arg);
    if in_root.is_file() || in_root.is_dir() {
        return in_root;
    }
    arg.to_path_buf()
}
