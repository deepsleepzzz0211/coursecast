//! Project-level file lock (spec "并发": 独占句柄、进程退出自动释放).
//!
//! Advisory byte-range lock via std `File::try_lock*` (LockFileEx on Windows): the OS
//! releases the lock when the handle closes, so a crashed process never leaves a stale
//! lock behind. Only the `.lock` file itself persists. Mutating commands take the lock
//! at the command boundary in exclusive mode; stage runners assume the caller holds it.
//! Read-only commands (project status) take it shared so they can run alongside each
//! other.

use crate::error::{Error, Result};
use std::fs::{self, TryLockError};
use std::path::Path;

#[derive(Debug)]
pub struct ProjectLock {
    /// Holding the handle open holds the lock; dropping it releases.
    _file: fs::File,
}

impl ProjectLock {
    pub const LOCK_FILE: &'static str = ".lock";

    /// Exclusive lock: no other holder (shared or exclusive) may exist.
    pub fn acquire(project_dir: &Path) -> Result<ProjectLock> {
        Self::acquire_with(project_dir, false)
    }

    /// Shared lock: coexists with other shared holders, conflicts with exclusive.
    pub fn acquire_shared(project_dir: &Path) -> Result<ProjectLock> {
        Self::acquire_with(project_dir, true)
    }

    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "private helper; callers are the named acquire/acquire_shared wrappers"
    )]
    fn acquire_with(project_dir: &Path, shared: bool) -> Result<ProjectLock> {
        let path = project_dir.join(Self::LOCK_FILE);
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        // std File::try_lock() is exclusive; try_lock_shared coexists with shared holders.
        let locked = if shared {
            file.try_lock_shared()
        } else {
            file.try_lock()
        };
        match locked {
            Ok(()) => Ok(ProjectLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(Error::ProjectLocked {
                path: path.display().to_string(),
            }),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn exclusive_lock_rejects_a_second_exclusive_holder() {
        let dir = TempDir::new().unwrap();
        let _held = ProjectLock::acquire(dir.path()).unwrap();
        assert!(matches!(
            ProjectLock::acquire(dir.path()).unwrap_err(),
            Error::ProjectLocked { .. }
        ));
    }

    #[test]
    fn shared_locks_coexist_and_block_an_exclusive_holder() {
        let dir = TempDir::new().unwrap();
        let first = ProjectLock::acquire_shared(dir.path()).unwrap();
        let second = ProjectLock::acquire_shared(dir.path()).unwrap();
        assert!(matches!(
            ProjectLock::acquire(dir.path()).unwrap_err(),
            Error::ProjectLocked { .. }
        ));
        drop(second);
        drop(first);
        let _exclusive = ProjectLock::acquire(dir.path()).unwrap();
    }

    #[test]
    fn exclusive_lock_blocks_shared_holders() {
        let dir = TempDir::new().unwrap();
        let exclusive = ProjectLock::acquire(dir.path()).unwrap();
        assert!(matches!(
            ProjectLock::acquire_shared(dir.path()).unwrap_err(),
            Error::ProjectLocked { .. }
        ));
        drop(exclusive);
        let _shared = ProjectLock::acquire_shared(dir.path()).unwrap();
    }
}
