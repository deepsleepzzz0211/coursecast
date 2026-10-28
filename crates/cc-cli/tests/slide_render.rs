//! Seam: `cc-cli slide_render --json` stdout + the artifacts it writes (ticket 03).
//! Uses an image-folder source: the stage behavior (ordering, fingerprints) is spec'd at
//! the cc-core seam; here we pin the CLI contract — args, JSON shape, paths, exit codes.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn create_project(data_dir: &Path) -> String {
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "project",
            "create",
            "--json",
            "--name",
            "t",
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["project_id"].as_str().unwrap().to_string()
}

fn slide_render(data_dir: &Path, project: &str, source: &str) -> (std::process::Output, Value) {
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "slide_render",
            "--project",
            project,
            "--source",
            source,
            "--json",
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out, v)
}

/// Two solid-gray PNGs in a folder inside the project's source/.
fn write_image_folder(project_dir: &Path, name: &str, gray_levels: &[u8]) -> PathBuf {
    let folder = project_dir.join("source").join(name);
    fs::create_dir_all(&folder).unwrap();
    for (i, g) in gray_levels.iter().enumerate() {
        image::RgbaImage::from_pixel(64, 48, image::Rgba([*g, *g, *g, 255]))
            .save(folder.join(format!("幻灯片{}.png", i + 1)))
            .unwrap();
    }
    folder
}

#[test]
fn slide_render_reports_pages_and_paths_and_persists_state() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let pj_path = tmp.path().join("projects").join(&id).join("project.json");
    write_image_folder(
        &tmp.path().join("projects").join(&id),
        "deck-img",
        &[30, 220],
    );

    let (out, v) = slide_render(tmp.path(), &id, "source/deck-img");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["ok"], true);
    assert_eq!(v["project_id"], id.as_str());
    assert_eq!(v["page_count"], 2);
    assert_eq!(v["skipped"], false);
    assert!(v["fingerprint"].as_str().unwrap().starts_with("sha256:"));
    assert_eq!(v["stage"], "parsed");
    assert!(!v["next"].as_str().unwrap().is_empty());
    let slides: Vec<&str> = v["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(
        slides,
        vec!["work/slides/slide_001.png", "work/slides/slide_002.png"]
    );

    let project_dir = tmp.path().join("projects").join(&id);
    for rel in &slides {
        assert!(project_dir.join(rel).is_file(), "missing artifact {rel}");
    }
    let pj: Value = serde_json::from_str(&fs::read_to_string(&pj_path).unwrap()).unwrap();
    assert_eq!(pj["stage"], "parsed");
    assert_eq!(pj["stages"]["slide_render"]["state"], "success");
}

#[test]
fn slide_render_rerun_skips_with_unchanged_source() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    write_image_folder(
        &tmp.path().join("projects").join(&id),
        "deck-img",
        &[30, 220],
    );

    let (_, first) = slide_render(tmp.path(), &id, "source/deck-img");
    let (_, second) = slide_render(tmp.path(), &id, "source/deck-img");

    assert_eq!(second["skipped"], true);
    assert_eq!(second["fingerprint"], first["fingerprint"].clone());
    assert_eq!(second["page_count"], 2);
}

#[test]
fn slide_render_missing_source_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());

    let (out, v) = slide_render(tmp.path(), &id, "no-such-deck.pdf");

    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "source_not_found");
}
