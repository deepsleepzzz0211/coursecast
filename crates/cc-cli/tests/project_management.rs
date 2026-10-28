//! Seam: `cc-cli project list` / `project delete` — AI-first project management
//! (spec F1: 创建/列表/删除; list gives the agent the project IDs it needs).

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

fn create_project(data_dir: &Path, name: &str) -> (String, String) {
    let (out, v) = run(data_dir, &["project", "create", "--name", name, "--json"]);
    assert!(out.status.success(), "create failed: {v}");
    (
        v["project_id"].as_str().unwrap().to_string(),
        v["dir"].as_str().unwrap().to_string(),
    )
}

#[test]
fn list_reports_each_project_with_resume_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let (first, _) = create_project(tmp.path(), "第一门");
    let (second, _) = create_project(tmp.path(), "第二门");

    let (out, v) = run(tmp.path(), &["project", "list", "--json"]);
    assert!(out.status.success());
    assert_eq!(v["ok"], true);
    let projects = v["projects"].as_array().expect("projects array");
    assert_eq!(projects.len(), 2);
    for id in [&first, &second] {
        let entry = projects
            .iter()
            .find(|p| p["project_id"] == id.as_str())
            .expect("created project is listed");
        assert_eq!(entry["progress"]["slides"], false);
        assert_eq!(entry["progress"]["script"], false);
        assert_eq!(entry["progress"]["person"], false);
        assert_eq!(entry["progress"]["film"], false);
    }
}

#[test]
fn list_is_empty_before_the_first_project() {
    let tmp = tempfile::tempdir().unwrap();
    let (out, v) = run(tmp.path(), &["project", "list", "--json"]);
    assert!(out.status.success());
    assert_eq!(v["projects"], serde_json::json!([]));
}

#[test]
fn delete_requires_confirm_and_then_removes_the_project() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path(), "测试垃圾项目");
    let dir = std::path::PathBuf::from(dir);

    let (out, v) = run(
        tmp.path(),
        &["project", "delete", "--project", &id, "--json"],
    );
    assert!(!out.status.success(), "unconfirmed delete must fail");
    assert_eq!(v["error"]["code"], "invalid_argument");
    assert!(
        dir.is_dir(),
        "unconfirmed delete must not touch the directory"
    );

    let (out, v) = run(
        tmp.path(),
        &["project", "delete", "--project", &id, "--confirm", "--json"],
    );
    assert!(out.status.success(), "delete failed: {v}");
    assert_eq!(v["deleted"], true);
    assert_eq!(v["name"], "测试垃圾项目");
    assert!(!dir.exists(), "project directory is gone");

    let (out, v) = run(
        tmp.path(),
        &["project", "status", "--project", &id, "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "project_not_found");
}
