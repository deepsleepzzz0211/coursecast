//! Seam: `cc-cli project status --json` stdout (spec story 24: AI 查询项目当前阶段与
//! 缺失项，靠返回值决定下一步).

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
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

fn create_project(data_dir: &Path) -> String {
    let (out, v) = run(
        data_dir,
        &["project", "create", "--name", "测试项目", "--json"],
    );
    assert!(out.status.success());
    v["project_id"].as_str().unwrap().to_string()
}

#[test]
fn status_reports_stage_and_missing_inputs() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());

    let (out, v) = run(
        tmp.path(),
        &["project", "status", "--project", &id, "--json"],
    );
    assert!(out.status.success());
    assert_eq!(v["ok"], true);
    assert_eq!(v["project_id"], id.as_str());
    assert_eq!(v["name"], "测试项目");
    assert_eq!(v["stage"], "created");
    assert_eq!(v["failed"], Value::Null);

    let missing: Vec<&str> = v["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert_eq!(missing, vec!["deck", "script", "voice_ref", "person"]);

    assert_eq!(v["inputs"]["deck"]["ready"], false);
    assert_eq!(v["inputs"]["script"]["ready"], false);
    assert_eq!(v["inputs"]["voice_ref"]["ready"], false);
    assert_eq!(v["inputs"]["person"]["ready"], false);

    assert!(
        !v["next"].as_str().unwrap().is_empty(),
        "next suggestion must be present"
    );
}

#[test]
fn status_reflects_files_present_in_project_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    fs::write(dir.join("source").join("deck.pdf"), b"%PDF-1.4 fake").unwrap();
    fs::write(dir.join("script.md"), "# 第 1 页\n讲稿").unwrap();

    let (_, v) = run(
        tmp.path(),
        &["project", "status", "--project", &id, "--json"],
    );
    let missing: Vec<&str> = v["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert_eq!(missing, vec!["voice_ref", "person"]);
    assert_eq!(v["inputs"]["deck"]["ready"], true);
    assert!(v["inputs"]["deck"]["path"]
        .as_str()
        .unwrap()
        .ends_with("deck.pdf"));
    assert_eq!(v["inputs"]["script"]["ready"], true);
}

#[test]
fn deck_pptx_counts_as_deck() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    fs::write(dir.join("source").join("deck.pptx"), b"pptx fake").unwrap();

    let (_, v) = run(
        tmp.path(),
        &["project", "status", "--project", &id, "--json"],
    );
    let missing: Vec<&str> = v["missing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(
        !missing.contains(&"deck"),
        "pptx should satisfy the deck input: {missing:?}"
    );
    assert!(v["inputs"]["deck"]["path"]
        .as_str()
        .unwrap()
        .ends_with("deck.pptx"));
}

#[test]
fn status_with_unknown_project_id_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, v) = run(
        tmp.path(),
        &["project", "status", "--project", "aaaaaaaa", "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "project_not_found");
    assert!(v["error"]["message"].as_str().unwrap().contains("aaaaaaaa"));
}

#[test]
fn status_with_malformed_project_id_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    // path-traversal-shaped id must be rejected by id validation before touching the fs
    let (out, v) = run(
        tmp.path(),
        &["project", "status", "--project", "../../etc", "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "invalid_project_id");
}
