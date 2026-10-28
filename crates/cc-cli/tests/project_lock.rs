//! Seam: CLI surfaces a structured error when another process holds the project lock
//! (ticket 01: "并发下第二个进程拿不到锁并得到结构化错误").

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use std::process::Output;

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn run(data_dir: &Path, args: &[&str]) -> (Output, Value) {
    let mut all: Vec<String> = vec!["--data-dir".into(), data_dir.to_str().unwrap().into()];
    all.extend(args.iter().map(|s| s.to_string()));
    let out = cli().args(&all).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out, v)
}

#[test]
fn status_reports_structured_error_while_project_is_locked() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, v) = run(tmp.path(), &["project", "create", "--name", "x", "--json"]);
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = tmp.path().join("projects").join(&id);

    // another process (here: the test itself) holds the exclusive lock, as a stage
    // runner would
    let guard = cc_core::ProjectLock::acquire(&dir).unwrap();

    let (out, v) = run(
        tmp.path(),
        &["project", "status", "--project", &id, "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "project_locked");

    drop(guard);
    let (out, v) = run(
        tmp.path(),
        &["project", "status", "--project", &id, "--json"],
    );
    assert!(out.status.success());
    assert_eq!(v["ok"], true);
}
