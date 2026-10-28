//! Seam: `cc-cli project create --json` stdout + the `project.json` it writes (spec "文件即真相").

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn run_create(data_dir: &Path, extra: &[&str]) -> (std::process::Output, Value) {
    let mut args: Vec<String> = vec![
        "--data-dir".into(),
        data_dir.to_str().unwrap().into(),
        "project".into(),
        "create".into(),
        "--json".into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let out = cli().args(&args).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out, v)
}

#[test]
fn create_writes_project_json_and_reports_id_and_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, v) = run_create(tmp.path(), &["--name", "药品包装"]);

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["ok"], true);

    let id = v["project_id"].as_str().unwrap();
    assert!(
        (8..=32).contains(&id.len())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        "project_id `{id}` must match [a-z0-9]{{8,32}}"
    );

    let dir = PathBuf::from(v["dir"].as_str().unwrap());
    assert!(dir.is_dir(), "reported dir {dir:?} does not exist");
    assert!(
        dir.ends_with(Path::new("projects").join(id)),
        "dir {dir:?} not under projects/{id}"
    );

    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert_eq!(pj["schema_version"], 1);
    assert_eq!(pj["project_id"], id);
    assert_eq!(pj["name"], "药品包装");
    assert_eq!(pj["resolution"], "1920x1080", "default resolution");
    assert_eq!(pj["stage"], "created");
    assert!(
        pj["created_at"].as_str().unwrap().len() >= 20,
        "created_at should be RFC3339"
    );

    assert!(dir.join("source").is_dir());
    assert!(dir.join("work").is_dir());
    assert!(dir.join("output").is_dir());
}

#[test]
fn create_honors_resolution_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, v) = run_create(tmp.path(), &["--name", "x", "--resolution", "1280x720"]);
    assert!(out.status.success());
    let dir = PathBuf::from(v["dir"].as_str().unwrap());
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert_eq!(pj["resolution"], "1280x720");
}

#[test]
fn create_with_empty_name_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, v) = run_create(tmp.path(), &["--name", "   "]);
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "invalid_name");
    assert!(v["error"]["message"].as_str().unwrap().contains("name"));
}

#[test]
fn create_with_bad_resolution_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, v) = run_create(tmp.path(), &["--name", "x", "--resolution", "1920X1080"]);
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "invalid_resolution");
}
