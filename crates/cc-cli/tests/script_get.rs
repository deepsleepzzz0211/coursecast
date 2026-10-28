//! Seam: `cc-cli script_get --json` — the same page-string shape the app and MCP
//! return from the shared deck command.

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn create_project(data_dir: &str) -> String {
    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "project",
            "create",
            "--json",
            "--name",
            "逐字稿测试",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["project_id"].as_str().unwrap().to_string()
}

fn script_get(data_dir: &str, id: &str) -> Value {
    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "script_get",
            "--json",
            "--project",
            id,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn script_get_returns_page_strings_and_handles_a_missing_script() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_str().unwrap();
    let id = create_project(data_dir);

    let empty = script_get(data_dir, &id);
    assert_eq!(empty["ok"], true);
    assert_eq!(empty["exists"], false);
    assert_eq!(empty["pages"], serde_json::json!([]));

    let project_dir = Path::new(data_dir).join("projects").join(&id);
    std::fs::write(
        project_dir.join("script.md"),
        "第一页讲稿\n\n---\n\n第二页讲稿\n",
    )
    .unwrap();
    let got = script_get(data_dir, &id);
    assert_eq!(got["ok"], true);
    assert_eq!(got["exists"], true);
    assert_eq!(
        got["pages"],
        serde_json::json!(["第一页讲稿", "第二页讲稿"])
    );
}

#[test]
fn script_set_round_trips_through_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_str().unwrap();
    let id = create_project(data_dir);
    let pdf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../cc-app/tests/fixtures/deck2.pdf");

    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "slide_render",
            "--json",
            "--project",
            &id,
            "--source",
            pdf.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "script_set",
            "--json",
            "--project",
            &id,
            "--pages",
            r#"[{"page":1,"text":"一"},{"page":2,"text":""}]"#,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let set: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(set["ok"], true);
    assert_eq!(set["page_count"], 2);

    let got = script_get(data_dir, &id);
    assert_eq!(got["pages"], serde_json::json!(["一", ""]));
}
