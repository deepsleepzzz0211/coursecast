//! 主接缝 e2e (spec Testing Decisions, 缝 1): fingerprint invalidation propagation
//! through the real CLI — create → slide_render (image folder) → voice_synthesize
//! (fake engine) → timeline → render (card branch, no matte), then edit page 2 and
//! walk the chain again: only tts@2 regenerates, the track and the film rebuild, and
//! every stage lands its depends_on record in project.json. Ticket 10.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn run(data_dir: &Path, args: &[&str]) -> (std::process::Output, Value) {
    let mut cmd = cli();
    cmd.arg("--data-dir").arg(data_dir.to_str().unwrap());
    cmd.args(args);
    let out = cmd.output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out, v)
}

fn expect_ok(data_dir: &Path, args: &[&str]) -> Value {
    let (out, v) = run(data_dir, args);
    assert!(
        out.status.success(),
        "command {args:?} failed: {}",
        serde_json::to_string_pretty(&v).unwrap_or_default()
    );
    v
}

fn write_gray_png(path: &Path, gray: u8) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let out = SysCommand::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c=0x{gray:02x}{gray:02x}{gray:02x}:s=320x180"),
        ])
        .args(["-frames:v", "1"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_person_mp4(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let out = SysCommand::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=0x303030:s=200x200:d=1",
        ])
        .args([
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn script(pages: &[&str]) -> String {
    pages.join("\n\n---\n\n")
}

/// Full chain through the CLI, ending at `done` with the card branch (matte skipped).
fn ready_project(tmp: &Path) -> (String, PathBuf) {
    let (out, v) = run(
        tmp,
        &[
            "project",
            "create",
            "--json",
            "--name",
            "失效传播e2e",
            "--resolution",
            "640x360",
        ],
    );
    assert!(out.status.success());
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = PathBuf::from(v["dir"].as_str().unwrap());

    let deck = dir.join("source").join("deck-img");
    fs::create_dir_all(&deck).unwrap();
    for (i, g) in [40u8, 90, 150].iter().enumerate() {
        write_gray_png(&deck.join(format!("幻灯片{}.png", i + 1)), *g);
    }
    expect_ok(
        tmp,
        &[
            "slide_render",
            "--project",
            &id,
            "--source",
            "source/deck-img",
            "--json",
        ],
    );
    fs::write(
        dir.join("script.md"),
        script(&["第一页讲稿", "第二页讲稿", "第三页讲稿"]),
    )
    .unwrap();
    expect_ok(tmp, &["voice_synthesize", "--project", &id, "--json"]);
    expect_ok(tmp, &["timeline", "--project", &id, "--json"]);
    write_person_mp4(&dir.join("source/person.mp4"));
    let v = expect_ok(tmp, &["render", "--project", &id, "--json"]);
    assert_eq!(v["pip"]["source"], "card", "matte was skipped: card branch");
    assert_eq!(v["skipped"], false);
    (id, dir)
}

fn pj(dir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap()
}

#[test]
fn editing_a_page_propagates_through_track_and_film_via_the_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    let before = pj(&dir);
    let old_tts2 = before["stages"]["tts:2"]["input_fingerprint"]
        .as_str()
        .unwrap()
        .to_string();
    let old_tl = before["stages"]["timeline"]["input_fingerprint"]
        .as_str()
        .unwrap()
        .to_string();
    let old_render_fp = before["stages"]["render"]["input_fingerprint"]
        .as_str()
        .unwrap()
        .to_string();

    // 改第 2 页讲稿（更长 → 时长变化）
    fs::write(
        dir.join("script.md"),
        script(&["第一页讲稿", "第二页讲稿现在长了很多", "第三页讲稿"]),
    )
    .unwrap();

    let v = expect_ok(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert_eq!(v["generated"], serde_json::json!([2]));
    assert_eq!(v["skipped"], serde_json::json!([1, 3]));

    let after_tts = pj(&dir);
    assert_ne!(
        after_tts["stages"]["tts:2"]["input_fingerprint"]
            .as_str()
            .unwrap(),
        old_tts2
    );
    assert_eq!(
        after_tts["stages"]["timeline"]["depends_on"]["tts:2"].as_str().unwrap(),
        old_tts2,
        "timeline record still points at the old page fingerprint: the stale link is the propagation record"
    );

    let v = expect_ok(tmp.path(), &["timeline", "--project", &id, "--json"]);
    assert_eq!(v["skipped"], false);
    assert_ne!(v["fingerprint"].as_str().unwrap(), old_tl);

    let v = expect_ok(tmp.path(), &["render", "--project", &id, "--json"]);
    assert_eq!(v["skipped"], false);
    assert_ne!(v["fingerprint"].as_str().unwrap(), old_render_fp);
    assert_eq!(v["stage"], "done");
    assert!(dir.join("output/final.mp4").is_file());

    // idempotent: re-running everything with unchanged inputs skips
    let v = expect_ok(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(v["generated"].as_array().unwrap().is_empty());
    assert_eq!(v["skipped"], serde_json::json!([1, 2, 3]));
    let v = expect_ok(tmp.path(), &["timeline", "--project", &id, "--json"]);
    assert_eq!(v["skipped"], true);
    let v = expect_ok(tmp.path(), &["render", "--project", &id, "--json"]);
    assert_eq!(v["skipped"], true);
}

#[test]
fn changing_layout_through_the_cli_renders_only_the_film() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    let before = pj(&dir);
    let old_tl = before["stages"]["timeline"]["input_fingerprint"]
        .as_str()
        .unwrap()
        .to_string();
    let old_tts1 = before["stages"]["tts:1"]["input_fingerprint"]
        .as_str()
        .unwrap()
        .to_string();

    fs::write(
        dir.join("layout.toml"),
        "position = \"bottom_right\"\nwidth_pct = 25.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n",
    )
    .unwrap();

    let v = expect_ok(tmp.path(), &["render", "--project", &id, "--json"]);
    assert_eq!(v["skipped"], false, "layout change must re-render");
    assert_eq!(v["layout"]["width_pct"], 25.0);
    assert_eq!(v["stage"], "done");

    let after = pj(&dir);
    assert_eq!(
        after["stages"]["timeline"]["input_fingerprint"]
            .as_str()
            .unwrap(),
        old_tl
    );
    assert_eq!(
        after["stages"]["tts:1"]["input_fingerprint"]
            .as_str()
            .unwrap(),
        old_tts1
    );

    // unchanged → skip
    let v = expect_ok(tmp.path(), &["render", "--project", &id, "--json"]);
    assert_eq!(v["skipped"], true);
}
