//! Seam: cc-core project lock (spec "并发": 项目级文件锁、独占句柄、进程退出自动释放).

use cc_core::{Error, ProjectLock};
use std::fs;

fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects").join("abcd1234");
    fs::create_dir_all(&dir).unwrap();
    (tmp, dir)
}

#[test]
fn second_exclusive_acquire_fails_until_released() {
    let (_tmp, dir) = setup();
    {
        let _l1 = ProjectLock::acquire(&dir).unwrap();
        let err = ProjectLock::acquire(&dir).unwrap_err();
        assert!(matches!(err, Error::ProjectLocked { .. }), "got {err:?}");
        // an exclusive holder also blocks shared acquirers
        assert!(ProjectLock::acquire_shared(&dir).is_err());
    }
    // released by drop → can lock again
    let _l2 = ProjectLock::acquire(&dir).unwrap();
}

#[test]
fn shared_locks_coexist_but_block_exclusive() {
    let (_tmp, dir) = setup();
    let _a = ProjectLock::acquire_shared(&dir).unwrap();
    let _b = ProjectLock::acquire_shared(&dir).unwrap();
    assert!(matches!(
        ProjectLock::acquire(&dir).unwrap_err(),
        Error::ProjectLocked { .. }
    ));
}
