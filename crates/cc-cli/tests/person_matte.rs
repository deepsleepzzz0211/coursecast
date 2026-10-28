//! Seam: `cc-cli person_matte --json` stdout and the artifacts it lands
//! (spec Testing Decisions: CLI e2e is the main seam). Uses the real RVM model; the
//! project lifecycle runs through the CLI like the AI/user path does.

use assert_cmd::Command;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command as SysCommand;

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

/// vendor/ is gitignored, so worktrees don't carry it — walk up to the main checkout
/// (same resolution rule as cc-core/tests/matte_inference.rs).
fn model_path() -> PathBuf {
    for anc in PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors() {
        let cand = anc.join("vendor/models/rvm_mobilenetv3_fp32.onnx");
        if cand.is_file() {
            return cand;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/models/rvm_mobilenetv3_fp32.onnx")
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
            "人像测试",
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

/// tts_done through the CLI (fake engine wired by the binary, ticket 04). The stage
/// is advanced by writing project.json directly — slide_render (ticket 03), which
/// would produce `parsed`, is not wired yet (same fixture trick as voice_synthesize.rs).
fn reach_tts_done(data_dir: &str, id: &str, project_dir: &std::path::Path) {
    std::fs::write(
        project_dir.join("script.md"),
        "第一页讲稿\n\n---\n\n第二页讲稿\n",
    )
    .unwrap();
    let (mut project, dir) = cc_core::load_project(std::path::Path::new(data_dir), id).unwrap();
    project.stage = cc_core::Stage::Parsed;
    cc_core::save_project(&project, &dir).unwrap();
    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "voice_synthesize",
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
}

fn make_person_video(path: &std::path::Path) {
    let out = SysCommand::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=10:duration=1",
        ])
        .args(["-pix_fmt", "yuv420p"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn person_matte_runs_to_matte_done_and_reruns_as_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let data_dir = data_dir.to_str().unwrap();
    let id = create_project(data_dir);
    let project_dir = tmp.path().join("data").join("projects").join(&id);
    reach_tts_done(data_dir, &id, &project_dir);

    make_person_video(&project_dir.join("source").join("person.mp4"));

    let out = cli()
        .env("CC_RVM_MODEL", model_path())
        .args([
            "--data-dir",
            data_dir,
            "person_matte",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "matte_done");
    assert_eq!(v["skipped"], false);
    assert!(
        (v["frames"].as_u64().unwrap() as f64 - 10.0).abs() <= 2.0,
        "1s@10fps: {}",
        v["frames"]
    );
    assert_eq!(v["width"], 320);
    assert_eq!(v["height"], 240);
    assert!(v["fingerprint"].as_str().unwrap().starts_with("sha256:"));
    // alpha mov + preview on disk, paths relative to the project
    let artifacts = v["artifacts"].as_array().unwrap();
    let paths: Vec<&str> = artifacts
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        vec!["work/person/alpha.mov", "work/person/preview.png"]
    );
    for a in artifacts {
        let abs = project_dir.join(a["path"].as_str().unwrap());
        assert!(abs.is_file(), "{} missing", a["path"]);
        assert_eq!(
            std::fs::metadata(&abs).unwrap().len(),
            a["bytes"].as_u64().unwrap()
        );
    }
    assert_eq!(v["preview"], "work/person/preview.png");
    assert!(project_dir.join("work/person/preview.png").is_file());
    assert!(v["next"].as_str().unwrap().contains("render"));

    // rerun: fingerprint unchanged → skip, no re-inference
    let out = cli()
        .env("CC_RVM_MODEL", model_path())
        .args([
            "--data-dir",
            data_dir,
            "person_matte",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["skipped"], true, "unchanged inputs must skip: {v}");
    assert_eq!(v["stage"], "matte_done");
}

#[test]
fn person_matte_without_video_fails_with_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let data_dir = data_dir.to_str().unwrap();
    let id = create_project(data_dir);
    let project_dir = tmp.path().join("data").join("projects").join(&id);
    reach_tts_done(data_dir, &id, &project_dir);

    let out = cli()
        .env("CC_RVM_MODEL", model_path())
        .args([
            "--data-dir",
            data_dir,
            "person_matte",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "person_video_missing");
    // state untouched: no half-entered matte_running
    let status = cli()
        .args([
            "--data-dir",
            data_dir,
            "project",
            "status",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    let s: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(s["stage"], "tts_done");
}

/// 打包形态靠 CC_FFMPEG 定位随包 ffmpeg；matte 的探测/编码/解码也必须走这条缝
/// （票 16 复审：裸 "ffmpeg" 会让干净安装的 Step 5 直接失败）。
#[test]
fn person_matte_honours_the_cc_ffmpeg_seam() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let data_dir = data_dir.to_str().unwrap();
    let id = create_project(data_dir);
    let project_dir = tmp.path().join("data").join("projects").join(&id);
    reach_tts_done(data_dir, &id, &project_dir);
    make_person_video(&project_dir.join("source").join("person.mp4"));

    let out = cli()
        .env("CC_RVM_MODEL", model_path())
        .env("CC_FFMPEG", "Z:/no-such-dir/ffmpeg.exe")
        .args([
            "--data-dir",
            data_dir,
            "person_matte",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "ffmpeg_failed");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("ffmpeg"),
        "the spawn error must name ffmpeg: {v}"
    );
}

#[test]
fn person_matte_honours_the_fake_engine_seam() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let data_dir = data_dir.to_str().unwrap();
    let id = create_project(data_dir);
    let project_dir = tmp.path().join("data").join("projects").join(&id);
    reach_tts_done(data_dir, &id, &project_dir);
    make_person_video(&project_dir.join("source").join("person.mp4"));

    let out = cli()
        .env("CC_MATTE_ENGINE", "fake")
        .args([
            "--data-dir",
            data_dir,
            "person_matte",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "matte_done");
    assert_eq!(v["skipped"], false);
    assert!(v["frames"].as_u64().unwrap() > 0);
}

#[test]
fn person_video_set_ingests_a_clip_via_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let data_dir = data_dir.to_str().unwrap();
    let id = create_project(data_dir);
    let project_dir = tmp.path().join("data").join("projects").join(&id);
    reach_tts_done(data_dir, &id, &project_dir);
    let clip = tmp.path().join("clip.mp4");
    make_person_video(&clip);

    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "person_video_set",
            "--json",
            "--project",
            &id,
            "--source",
            clip.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["path"], "source/person.mp4");
    assert!(project_dir.join("source").join("person.mp4").is_file());

    let out = cli()
        .env("CC_MATTE_ENGINE", "fake")
        .args([
            "--data-dir",
            data_dir,
            "person_matte",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["stage"], "matte_done");
}

#[test]
fn person_status_and_skip_are_cli_visible() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let data_dir = data_dir.to_str().unwrap();
    let id = create_project(data_dir);
    let project_dir = tmp.path().join("data").join("projects").join(&id);
    reach_tts_done(data_dir, &id, &project_dir);
    make_person_video(&project_dir.join("source").join("person.mp4"));

    let out = cli()
        .env("CC_MATTE_ENGINE", "fake")
        .args([
            "--data-dir",
            data_dir,
            "person_matte",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "person_status",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["has_video"], true);
    assert_eq!(status["matte_done"], true);

    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "person_matte_skip",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let skipped: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(skipped["matte_done"], false);

    let out = cli()
        .args([
            "--data-dir",
            data_dir,
            "person_status",
            "--json",
            "--project",
            &id,
        ])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(status["matte_done"], false);
}
