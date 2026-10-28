//! Seam: `cc-cli voice_synthesize --json` stdout + project.json + wav artifacts
//! (spec stories 25/26/27: AI 单步调用合成工具，靠返回值串流程，重跑结果一致).
//!
//! Fixture: stage is advanced by writing project.json directly (slide_render, which
//! normally drives parsed, is ticket 03 and not a dependency of this ticket).

use assert_cmd::Command;
use cc_core::Stage;
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

fn create_project(data_dir: &Path) -> String {
    let (out, v) = run(
        data_dir,
        &["project", "create", "--name", "测试项目", "--json"],
    );
    assert!(out.status.success());
    v["project_id"].as_str().unwrap().to_string()
}

fn set_stage(data_dir: &Path, id: &str, stage: Stage) {
    let (mut project, dir) = cc_core::load_project(data_dir, id).unwrap();
    project.stage = stage;
    cc_core::save_project(&project, &dir).unwrap();
}

/// Write a 3-page script.
fn write_script(dir: &Path) {
    std::fs::write(
        dir.join("script.md"),
        "# 第 1 页\n这是第一页的讲稿内容\n\n---\n\n第二页讲稿\n\n---\n\n第三页讲稿\n",
    )
    .unwrap();
}

/// Duration of a wav written by our 24kHz mono pcm_s16le encoder, derived from the
/// container contract (44-byte header, 48000 bytes/sec) — not from cc-core's parser.
fn wav_duration_from_size(bytes: u64) -> f64 {
    (bytes - 44) as f64 / 48_000.0
}

#[test]
fn synthesizes_all_pages_to_work_audio_with_json_report() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&dir);

    let (out, v) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(out.status.success(), "stdout: {}", v);
    assert_eq!(v["ok"], true);
    assert_eq!(v["project_id"], id.as_str());
    assert_eq!(v["stage"], "tts_done");

    let generated: Vec<u32> = v["generated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(generated, vec![1, 2, 3]);
    assert_eq!(v["skipped"].as_array().unwrap().len(), 0);
    assert_eq!(v["failed_pages"].as_array().unwrap().len(), 0);

    // artifacts: one wav per page, 24kHz mono pcm_s16le, durations consistent
    let artifacts = v["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 3);
    let mut total = 0.0;
    for (i, a) in artifacts.iter().enumerate() {
        let page = (i + 1) as u32;
        assert_eq!(a["page"], page);
        let path = a["path"].as_str().unwrap();
        assert_eq!(path, format!("work/audio/slide_{page:03}.wav"));
        let abs = dir.join(path);
        let bytes = std::fs::read(&abs).unwrap_or_else(|e| panic!("artifact {path} missing: {e}"));
        assert_eq!(a["bytes"], bytes.len() as u64);
        let info = cc_core::wav::parse_wav(&bytes).unwrap();
        assert_eq!(info.sample_rate, 24_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        let expected = wav_duration_from_size(bytes.len() as u64);
        assert!((a["duration_sec"].as_f64().unwrap() - expected).abs() < 1e-9);
        assert!(a["sha256"].as_str().unwrap().starts_with("sha256:"));
        total += expected;
    }
    assert!((v["total_duration_sec"].as_f64().unwrap() - total).abs() < 1e-9);
    assert!(v["next"].as_str().unwrap().contains("person_matte"));

    // per-page success records in project.json (spec 输入指纹模型: tts 按页指纹)
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::TtsDone);
    for page in 1..=3 {
        let key = format!("tts:{page}");
        let record = project
            .stages
            .get(&key)
            .unwrap_or_else(|| panic!("missing {key}"));
        assert_eq!(record.stage, "tts");
        assert_eq!(record.state, "success");
        assert!(
            record.input_fingerprint.starts_with("sha256:"),
            "{record:?}"
        );
        assert_eq!(record.tool_version, "tts-fake/0.1.0+model=fake-sine-24k");
        assert_eq!(
            record.outputs[0].path,
            format!("work/audio/slide_{page:03}.wav")
        );
        assert_eq!(
            record.outputs[0].bytes as usize,
            std::fs::read(dir.join(&record.outputs[0].path))
                .unwrap()
                .len()
        );
    }
}

#[test]
fn rerun_with_same_inputs_reports_all_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&dir);

    let (_, first) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert_eq!(first["skipped"].as_array().unwrap().len(), 0);
    let first_hashes: Vec<&str> = first["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["sha256"].as_str().unwrap())
        .collect();

    let (out, second) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(out.status.success());
    let generated: Vec<u32> = second["generated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    assert!(
        generated.is_empty(),
        "same inputs must regenerate nothing: {second}"
    );
    let skipped: Vec<u32> = second["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(skipped, vec![1, 2, 3]);
    let second_hashes: Vec<&str> = second["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["sha256"].as_str().unwrap())
        .collect();
    assert_eq!(
        first_hashes, second_hashes,
        "artifacts must be untouched by a skipped rerun"
    );
    assert_eq!(second["stage"], "tts_done");
}

#[test]
fn editing_one_page_text_regenerates_only_that_page() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&dir);

    let (_, first) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    let sha = |v: &Value, page: u32| -> String {
        v["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["page"] == page)
            .unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // rewrite page 2 only (spec story 22: 改第 7 页讲稿，成片里第 7 页必须重配)
    std::fs::write(
        dir.join("script.md"),
        "# 第 1 页\n这是第一页的讲稿内容\n\n---\n\n第二页讲稿，改过了\n\n---\n\n第三页讲稿\n",
    )
    .unwrap();

    let (out, second) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(
        out.status.success(),
        "rerun after text edit failed: {second}"
    );
    let generated: Vec<u32> = second["generated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    let skipped: Vec<u32> = second["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(
        generated,
        vec![2],
        "only the edited page regenerates: {second}"
    );
    assert_eq!(skipped, vec![1, 3]);

    assert_ne!(sha(&first, 2), sha(&second, 2), "page 2 audio must change");
    assert_eq!(
        sha(&first, 1),
        sha(&second, 1),
        "untouched pages keep their artifacts"
    );
    assert_eq!(sha(&first, 3), sha(&second, 3));
    assert_eq!(second["stage"], "tts_done");
}

#[test]
fn voice_synthesize_requires_a_stage_that_may_enter_tts() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    write_script(&dir);

    // created: parsing (slide_render, ticket 03) has not happened yet
    let (out, v) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "invalid_transition");
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert_eq!(
        project.stage,
        Stage::Created,
        "failed gate must leave state unchanged"
    );
    assert!(!dir.join("work/audio").exists());

    // done: invalidation re-entry (ticket 10) — a completed project legally re-enters
    // tts when its inputs changed; with no tts records the pages re-synthesize
    set_stage(tmp.path(), &id, Stage::Done);
    let (out, v) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(out.status.success());
    assert_eq!(v["generated"].as_array().unwrap().len(), 3);
    assert_eq!(v["stage"], "tts_done");
}

#[test]
fn missing_or_empty_script_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);

    let (out, v) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "script_not_found");
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert_eq!(
        project.stage,
        Stage::Parsed,
        "stage must not move without a script"
    );

    // a script made only of dividers is three blank pages: every page becomes 5 s
    // of silence (spec story 12 — blank pages never block the flow)
    std::fs::write(dir.join("script.md"), "---\n\n---\n").unwrap();
    let (out, v) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(out.status.success(), "{v}");
    assert_eq!(v["generated"], serde_json::json!([1, 2, 3]));
    for a in v["artifacts"].as_array().unwrap() {
        let d = a["duration_sec"].as_f64().unwrap();
        assert!((d - 5.0).abs() < 1e-6, "blank page = 5 s silence: {a}");
    }

    // a literally empty file has no page structure at all → script_empty
    std::fs::write(dir.join("script.md"), "").unwrap();
    let (out, v) = run(
        tmp.path(),
        &["voice_synthesize", "--project", &id, "--json"],
    );
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "script_empty");
}

#[test]
fn pages_subset_synthesizes_only_requested_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&dir);

    let (out, v) = run(
        tmp.path(),
        &[
            "voice_synthesize",
            "--project",
            &id,
            "--pages",
            "2-3",
            "--json",
        ],
    );
    assert!(out.status.success(), "stdout: {v}");
    let generated: Vec<u32> = v["generated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(generated, vec![2, 3]);

    let artifacts = v["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    assert!(
        !dir.join("work/audio/slide_001.wav").exists(),
        "page 1 was out of scope"
    );
    let mut total = 0.0;
    for a in artifacts {
        let bytes = std::fs::read(dir.join(a["path"].as_str().unwrap())).unwrap();
        total += wav_duration_from_size(bytes.len() as u64);
    }
    assert!((v["total_duration_sec"].as_f64().unwrap() - total).abs() < 1e-9);

    // stage is tts_done even though page 1 was never synthesized (it has no record yet)
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::TtsDone);
    assert!(project.stages.contains_key("tts:2") && project.stages.contains_key("tts:3"));
    assert!(!project.stages.contains_key("tts:1"));
}

#[test]
fn engine_sapi_synthesizes_offline_without_degradation() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&dir);

    // the chain's bottom level requested directly: real PowerShell System.Speech, no key
    let (out, v) = run(
        tmp.path(),
        &[
            "voice_synthesize",
            "--project",
            &id,
            "--engine",
            "sapi",
            "--json",
        ],
    );
    assert!(out.status.success(), "stdout: {v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "tts_done");
    assert_eq!(
        v["fallbacks"].as_array().unwrap().len(),
        0,
        "direct SAPI synthesis is not a degradation: {v}"
    );

    for a in v["artifacts"].as_array().unwrap() {
        let bytes = std::fs::read(dir.join(a["path"].as_str().unwrap())).unwrap();
        let info = cc_core::wav::parse_wav(&bytes).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.bits_per_sample),
            (24_000, 1, 16)
        );
        assert!(info.duration_sec > 0.3, "spoken page audio: {info:?}");
    }

    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert!(project.fallbacks.is_empty());
    assert!(project
        .stages
        .get("tts:1")
        .unwrap()
        .tool_version
        .starts_with("tts-sapi/"));
}

#[test]
fn engine_edge_blocked_slides_to_sapi() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&dir);

    // ticket 07: the middle level requested directly, pointed at a dead local port —
    // a blocked unofficial service is a NORMAL case: one degradation event, real SAPI
    // audio on every page, the run never stops
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let out = cli()
        .env("CC_EDGE_ENDPOINT", format!("ws://127.0.0.1:{port}/tc"))
        .args([
            "--data-dir",
            tmp.path().to_str().unwrap(),
            "voice_synthesize",
            "--project",
            &id,
            "--engine",
            "edge",
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
    assert!(out.status.success(), "a block must slide, not stop: {v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "tts_done");

    let fallbacks = v["fallbacks"].as_array().unwrap();
    assert_eq!(fallbacks.len(), 1, "{v}");
    assert_eq!(fallbacks[0]["from"], "edge");
    assert_eq!(fallbacks[0]["to"], "sapi");
    assert!(
        fallbacks[0]["reason"]
            .as_str()
            .unwrap()
            .contains("connect failed"),
        "{fallbacks:?}"
    );

    for a in v["artifacts"].as_array().unwrap() {
        let bytes = std::fs::read(dir.join(a["path"].as_str().unwrap())).unwrap();
        let info = cc_core::wav::parse_wav(&bytes).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.bits_per_sample),
            (24_000, 1, 16)
        );
        assert!(info.duration_sec > 0.3, "spoken page audio: {info:?}");
    }
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert!(project
        .stages
        .get("tts:1")
        .unwrap()
        .tool_version
        .starts_with("tts-sapi/"));
}

#[test]
fn pages_out_of_range_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    let dir = tmp.path().join("projects").join(&id);
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&dir);

    let (out, v) = run(
        tmp.path(),
        &[
            "voice_synthesize",
            "--project",
            &id,
            "--pages",
            "7",
            "--json",
        ],
    );
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "page_out_of_range");
    // nothing was synthesized and the stage is untouched
    assert!(!dir.join("work/audio").exists());
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::Parsed);
}

#[test]
fn malformed_pages_spec_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let id = create_project(tmp.path());
    set_stage(tmp.path(), &id, Stage::Parsed);
    write_script(&tmp.path().join("projects").join(&id));

    for spec in ["abc", "2-", "-4", "0", "3-1"] {
        // `=` form so clap does not swallow the leading-dash case before cc-core sees it
        let (out, v) = run(
            tmp.path(),
            &[
                "voice_synthesize",
                "--project",
                &id,
                &format!("--pages={spec}"),
                "--json",
            ],
        );
        assert!(!out.status.success(), "spec {spec} should fail");
        assert_eq!(v["error"]["code"], "invalid_pages", "spec {spec}");
    }
}
