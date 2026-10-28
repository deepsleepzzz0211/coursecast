//! Seam: the Tauri command layer for wizard steps 4–6 (ticket 13) — voice config,
//! person matte preview, layout + four-segment generate with progress events, film
//! preview/download. Same discipline as tests/commands.rs: the impl functions are the
//! UI's exact code path over cc-core; TTS and matte run the deterministic fake engines
//! while ffmpeg itself is real, so a green suite means the four segments really produce
//! a film.

#![expect(
    clippy::clone_on_ref_ptr,
    reason = "tests clone concrete Arc handles that coerce to Arc<dyn TtsEngine>"
)]

use cc_app::commands::create_project_impl;
use cc_app::pipeline::*;
use cc_core::error::Error;
use cc_core::{FakeMatteEngine, FakeTtsEngine, FallbackChain, MatteEngine};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

fn fixture_pdf() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deck2.pdf")
}

/// Any file works as the "model" for the fake engine — it is only hashed into the
/// fingerprint, never loaded (the real RVM path is exercised by cc-cli's matte tests).
fn dummy_model(dir: &Path) -> PathBuf {
    let path = dir.join("fake-rvm.onnx");
    std::fs::write(&path, b"ticket13-dummy-model").unwrap();
    path
}

fn make_person_video(dir: &Path) {
    std::fs::create_dir_all(dir.join("source")).unwrap();
    let out = SysCommand::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=10:duration=1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(dir.join("source").join("person.mp4"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Steps 1–3 + person video + consent, all through the command layer (the UI path).
fn prepared_project(data_dir: &Path, pages: &[String]) -> (String, PathBuf) {
    let v = create_project_impl(data_dir, "生成测试", Some("1920x1080")).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = data_dir.join("projects").join(&id);
    cc_app::commands::slide_render_impl(data_dir, &id, fixture_pdf().to_str().unwrap()).unwrap();
    cc_app::commands::script_set_impl(data_dir, &id, pages).unwrap();
    make_person_video(&dir);
    voice_consent_set_impl(data_dir, &id, true.into()).unwrap();
    (id, dir)
}

fn collector() -> (Arc<Mutex<Vec<ProgressEvent>>>, EventSink) {
    let events: Arc<Mutex<Vec<ProgressEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let cb: EventSink = Arc::new(move |ev: ProgressEvent| sink.lock().unwrap().push(ev));
    (events, cb)
}

/// (segment, state) pairs of the collected events, progress collapsed to a marker.
fn timeline_of(events: &[ProgressEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            if e.state == "progress" {
                format!("{}:progress", e.segment)
            } else {
                format!("{}:{}", e.segment, e.state)
            }
        })
        .fold(Vec::new(), |mut acc, s| {
            // collapse consecutive progress markers into one
            if s.ends_with(":progress") && acc.last().is_some_and(|l| l == &s) {
                return acc;
            }
            acc.push(s);
            acc
        })
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "test helper selects one of two fixture modes"
)]
fn generate_ok(
    data_dir: &Path,
    id: &str,
    use_matte: bool,
    engine: &mut dyn MatteEngine,
    sink: EventSink,
) -> Value {
    let chain = FallbackChain::single(Arc::new(FakeTtsEngine::new()));
    generate_impl(
        data_dir,
        id,
        &chain,
        use_matte.into(),
        engine,
        &dummy_model(data_dir),
        sink,
    )
    .unwrap()
}

#[test]
fn generate_runs_four_segments_in_order_and_produces_a_film() {
    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(
        tmp.path(),
        &["第一页讲稿内容".into(), "第二页讲稿内容".into()],
    );

    let (events, sink) = collector();
    let v = generate_ok(tmp.path(), &id, true, &mut FakeMatteEngine::new(), sink);

    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["stage"], "done");
    assert_eq!(v["segments"]["tts"]["state"], "done");
    assert_eq!(v["segments"]["timeline"]["state"], "done");
    assert_eq!(v["segments"]["matte"]["state"], "done");
    assert_eq!(v["segments"]["render"]["state"], "done");
    assert_eq!(v["segments"]["render"]["pip"], "alpha");
    let output = dir.join("output/final.mp4");
    assert!(output.is_file(), "final.mp4 must land: {v}");
    assert!(output.metadata().unwrap().len() > 0);

    // event order: the four segments in PRD order, progress events inside tts/matte
    let tl = timeline_of(&events.lock().unwrap());
    assert_eq!(
        tl,
        vec![
            "tts:start",
            "tts:progress",
            "tts:done",
            "timeline:start",
            "timeline:done",
            "matte:start",
            "matte:progress",
            "matte:done",
            "render:start",
            "render:done",
        ],
        "events: {tl:?}"
    );
    // tts progress carries page counts (2 pages), matte progress carries frame counts
    let (tts_count, tts_first_total, matte_frames) = {
        let ev = events.lock().unwrap();
        let tts_progress: Vec<&ProgressEvent> = ev
            .iter()
            .filter(|e| e.segment == "tts" && e.state == "progress")
            .collect();
        let tts_total = tts_progress.first().and_then(|e| e.total);
        let frames = ev
            .iter()
            .filter(|e| e.segment == "matte" && e.state == "progress")
            .map(|e| e.done.unwrap_or(0))
            .max()
            .unwrap_or(0);
        (tts_progress.len(), tts_total, frames)
    };
    assert_eq!(tts_count, 2);
    assert_eq!(tts_first_total, Some(2));
    assert!(matte_frames >= 1);
}

#[test]
fn generate_failure_isolates_the_segment_and_retry_reuses_finished_work() {
    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(
        tmp.path(),
        &["第一页讲稿内容".into(), "第二页讲稿内容".into()],
    );

    // matte always fails on the first generate
    struct FailingMatte;
    impl MatteEngine for FailingMatte {
        fn name(&self) -> &'static str {
            "fail"
        }
        fn model(&self) -> &'static str {
            "fail-matte"
        }
        fn infer_size(&self, w: u32, h: u32) -> (u32, u32) {
            (w, h)
        }
        fn infer(&mut self, _: &[f32], _: u32, _: u32) -> cc_core::Result<Vec<f32>> {
            Err(Error::EngineFailure {
                engine: "fail".into(),
                message: "injected".into(),
            })
        }
        fn reset(&mut self) {}
    }

    let (events, sink) = collector();
    let v = generate_ok(tmp.path(), &id, true, &mut FailingMatte, sink);
    assert_eq!(v["ok"], false, "{v}");
    assert_eq!(v["error"]["segment"], "matte");
    assert_eq!(v["error"]["code"], "engine_error");
    assert_eq!(
        v["segments"]["tts"]["state"], "done",
        "earlier segments stay usable"
    );
    assert_eq!(v["segments"]["timeline"]["state"], "done");
    assert_eq!(v["segments"]["matte"]["state"], "failed");
    assert_eq!(
        v["segments"]["render"]["state"], "pending",
        "later segments must not run"
    );
    assert!(!dir.join("output/final.mp4").exists());
    // failure event for the segment the UI attaches the retry button to
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.segment == "matte" && e.state == "failed"));

    // retry with a working engine: tts/timeline fingerprints match → nothing re-runs
    let tts = Arc::new(FakeTtsEngine::new());
    let before = tts.call_count();
    let chain = FallbackChain::single(tts.clone());
    let (events2, sink2) = collector();
    let v2 = generate_impl(
        tmp.path(),
        &id,
        &chain,
        true.into(),
        &mut FakeMatteEngine::new(),
        &dummy_model(tmp.path()),
        sink2,
    )
    .unwrap();
    assert_eq!(v2["ok"], true, "{v2}");
    assert_eq!(v2["segments"]["tts"]["state"], "done");
    assert_eq!(
        tts.call_count(),
        before,
        "已完成的语音段重试时不得重跑（不重复烧额度）"
    );
    assert!(dir.join("output/final.mp4").is_file());
    let tl = timeline_of(&events2.lock().unwrap());
    assert!(
        tl.contains(&"matte:done".to_string()) && tl.contains(&"render:done".to_string()),
        "{tl:?}"
    );
}

#[test]
fn generate_without_matte_renders_the_card_fallback() {
    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(
        tmp.path(),
        &["第一页讲稿内容".into(), "第二页讲稿内容".into()],
    );

    let (events, sink) = collector();
    let v = generate_ok(tmp.path(), &id, false, &mut FakeMatteEngine::new(), sink);

    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["segments"]["matte"]["state"], "skipped");
    assert_eq!(v["segments"]["render"]["pip"], "card");
    assert!(dir.join("output/final.mp4").is_file());
    // the skipped matte segment emits no matte events at all
    assert!(!events.lock().unwrap().iter().any(|e| e.segment == "matte"));
}

#[test]
fn person_matte_preview_runs_before_tts_and_the_pipeline_skips_it() {
    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(
        tmp.path(),
        &["第一页讲稿内容".into(), "第二页讲稿内容".into()],
    );

    // the wizard mats at its person step — TTS has not run, stage is still `parsed`
    let (events, sink) = collector();
    let mut engine = FakeMatteEngine::new();
    let frames_before = engine.call_count();
    let v = person_matte_impl(tmp.path(), &id, &mut engine, &dummy_model(tmp.path()), sink);
    let v = v.unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(
        v["stage"], "parsed",
        "preview must not move the state machine"
    );
    assert_eq!(v["skipped"], false);
    assert!(dir.join("work/person/alpha.mov").is_file());
    assert!(
        dir.join("work/person/preview.png").is_file(),
        "棋盘格预览必须落盘"
    );
    assert!(engine.call_count() > frames_before);
    assert!(events
        .lock()
        .unwrap()
        .iter()
        .any(|e| e.segment == "matte" && e.state == "progress" && e.done.unwrap_or(0) >= 1));
    let preview_abs = PathBuf::from(v["preview_abs"].as_str().unwrap());
    assert!(
        preview_abs.is_file(),
        "preview_abs must be an absolute usable path: {v}"
    );

    // generate afterwards: the tts segment runs first (parsed → tts_done), then the
    // matte segment skips through the preview's fingerprint record — no second pass
    let mut engine2 = FakeMatteEngine::new();
    let v2 = generate_ok(tmp.path(), &id, true, &mut engine2, collector().1);
    assert_eq!(v2["ok"], true, "{v2}");
    assert_eq!(
        v2["segments"]["matte"]["state"], "skipped",
        "preview 的产物必须被复用: {v2}"
    );
    assert_eq!(engine2.call_count(), 0, "幂等：不得重新推理");
    assert_eq!(v2["segments"]["render"]["pip"], "alpha");
    assert!(dir.join("output/final.mp4").is_file());
}

#[test]
fn generate_reports_the_fallback_chain_degradation_as_the_ui_banner_source() {
    let tmp = tempdir().unwrap();
    let (id, _dir) = prepared_project(
        tmp.path(),
        &["第一页讲稿内容".into(), "第二页讲稿内容".into()],
    );

    // 降级链形状（真实 build_fallback_chain 的行为在 cc-core 已测；这里测 generate
    // 编排对链的透传）：首引擎必败 → 次引擎接管出片，fallbacks 上报给 UI 提示条
    struct FailingTts;
    impl cc_core::TtsEngine for FailingTts {
        fn name(&self) -> &'static str {
            "mimo"
        }
        fn model(&self) -> &'static str {
            "mimo-v2.5-tts-voiceclone"
        }
        fn synthesize(&self, _: &cc_core::TtsRequest) -> cc_core::Result<Vec<u8>> {
            Err(Error::EngineFailure {
                engine: "mimo".into(),
                message: "injected".into(),
            })
        }
    }
    let chain = cc_core::FallbackChain {
        engines: vec![Arc::new(FailingTts), Arc::new(FakeTtsEngine::new())],
        events: vec![cc_core::FallbackEvent {
            from: "mimo".to_string(),
            to: "fake".to_string(),
            reason: "无 Key（测试注入）".to_string(),
        }],
    };

    let v = generate_impl(
        tmp.path(),
        &id,
        &chain,
        false.into(),
        &mut FakeMatteEngine::new(),
        &dummy_model(tmp.path()),
        collector().1,
    )
    .unwrap();
    assert_eq!(v["ok"], true, "{v}");
    let tts = &v["segments"]["tts"];
    let tool = tts["tool_version"].as_str().unwrap_or("");
    assert!(tool.starts_with("tts-fake/"), "次引擎产物: {tool}");
    let fallbacks = tts["fallbacks"].as_array().unwrap();
    assert!(
        fallbacks.iter().any(|f| f["from"] == "mimo"),
        "降级事件必须上报: {fallbacks:?}"
    );
    // 整个结果也携带 project.fallbacks（UI 提示条数据源）
    assert!(v["fallbacks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["from"] == "mimo"));
    assert!(dir_has_film(tmp.path(), &id));
}

fn dir_has_film(data_dir: &Path, id: &str) -> bool {
    data_dir
        .join("projects")
        .join(id)
        .join("output/final.mp4")
        .is_file()
}

#[test]
fn generate_refuses_without_the_voice_consent_acknowledgement() {
    let tmp = tempdir().unwrap();
    let v = create_project_impl(tmp.path(), "合规测试", Some("1920x1080")).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = tmp.path().join("projects").join(&id);
    cc_app::commands::slide_render_impl(tmp.path(), &id, fixture_pdf().to_str().unwrap()).unwrap();
    cc_app::commands::script_set_impl(tmp.path(), &id, &["a".into(), "b".into()]).unwrap();
    make_person_video(&dir);

    let err = {
        let chain = FallbackChain::single(Arc::new(FakeTtsEngine::new()));
        generate_impl(
            tmp.path(),
            &id,
            &chain,
            true.into(),
            &mut FakeMatteEngine::new(),
            &dummy_model(tmp.path()),
            collector().1,
        )
        .unwrap_err()
    };
    assert!(err.to_string().contains("声音授权确认"), "{err}");
    assert!(!dir.join("work/audio").exists(), "未授权不得产生任何合成");

    // granted → generate proceeds
    voice_consent_set_impl(tmp.path(), &id, true.into()).unwrap();
    let granted = voice_consent_get_impl(tmp.path(), &id).unwrap();
    assert_eq!(granted["granted"], true);
    let v = generate_ok(
        tmp.path(),
        &id,
        true,
        &mut FakeMatteEngine::new(),
        collector().1,
    );
    assert_eq!(v["ok"], true, "{v}");
}

#[test]
fn empty_script_page_flows_through_generate_as_five_seconds_of_silence() {
    let tmp = tempdir().unwrap();
    // page 1 left blank (a leading empty page is representable in script.md)
    let (id, dir) = prepared_project(tmp.path(), &["".into(), "第二页讲稿内容".into()]);

    let v = generate_ok(
        tmp.path(),
        &id,
        false,
        &mut FakeMatteEngine::new(),
        collector().1,
    );
    assert_eq!(v["ok"], true, "{v}");

    let silence = std::fs::read(dir.join("work/audio/slide_001.wav")).unwrap();
    let info = cc_core::wav::parse_wav(&silence).unwrap();
    assert!(
        (info.duration_sec - 5.0).abs() < 1e-6,
        "空页 = 5s 静音: {info:?}"
    );
    assert!(silence[44..].iter().all(|&b| b == 0));
    let spoken = std::fs::read(dir.join("work/audio/slide_002.wav")).unwrap();
    assert!(!spoken[44..].iter().all(|&b| b == 0));

    // the timeline formula counts the 5 s as d_1: total = 5 + d_2 + g
    let spoken_sec = cc_core::wav::parse_wav(&spoken).unwrap().duration_sec;
    let total = v["segments"]["timeline"]["total_sec"].as_f64().unwrap();
    assert!(
        (total - (5.0 + spoken_sec + 0.6)).abs() < 0.01,
        "total={total}"
    );
}

#[test]
fn voice_preview_lands_the_fixed_sample_sentence() {
    let tmp = tempdir().unwrap();
    let v = create_project_impl(tmp.path(), "试听测试", None).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = tmp.path().join("projects").join(&id);

    // the fake engine reads the reference when present; no reference configured yet
    let v = voice_preview_impl(tmp.path(), &id, &FakeTtsEngine::new()).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["text"], SAMPLE_TEXT);
    let abs = PathBuf::from(v["path_abs"].as_str().unwrap());
    assert!(abs.is_file());
    let info = cc_core::wav::parse_wav(&std::fs::read(&abs).unwrap()).unwrap();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (24_000, 1, 16)
    );
    assert!((v["duration_sec"].as_f64().unwrap() - info.duration_sec).abs() < 1e-9);
    assert!(dir.join("work/voice_preview.wav").is_file());

    // a failing engine surfaces as a structured error, nothing lands
    struct FailingTts;
    impl cc_core::TtsEngine for FailingTts {
        fn name(&self) -> &'static str {
            "fail"
        }
        fn model(&self) -> &'static str {
            "fail-tts"
        }
        fn synthesize(&self, _: &cc_core::TtsRequest) -> cc_core::Result<Vec<u8>> {
            Err(Error::EngineFailure {
                engine: "fail".into(),
                message: "injected".into(),
            })
        }
    }
    let err = voice_preview_impl(tmp.path(), &id, &FailingTts).unwrap_err();
    assert!(matches!(err, Error::EngineFailure { .. }));
}

#[test]
fn layout_commands_roundtrip_and_reject_bad_values() {
    let tmp = tempdir().unwrap();
    let v = create_project_impl(tmp.path(), "版式测试", None).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = tmp.path().join("projects").join(&id);

    let defaults = layout_get_impl(tmp.path(), &id).unwrap();
    assert_eq!(defaults["layout"]["position"], "bottom_right");
    assert_eq!(defaults["layout"]["width_pct"], 20.0);
    assert!(!dir.join("layout.toml").is_file(), "默认值不落盘");

    let set = layout_set_impl(
        tmp.path(),
        &id,
        cc_core::command::render::LayoutPatch {
            position: Some("right_bar"),
            width_pct: Some(15.0),
            shape: Some("circle"),
            transition: Some("fade"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(set["layout"]["position"], "right_bar");
    let again = layout_get_impl(tmp.path(), &id).unwrap();
    assert_eq!(again["layout"]["shape"], "circle");
    assert_eq!(again["layout"]["transition"], "fade");
    assert!(dir.join("layout.toml").is_file(), "文件即真相");

    let free = layout_set_impl(
        tmp.path(),
        &id,
        cc_core::command::render::LayoutPatch {
            x_pct: Some(30.0),
            y_pct: Some(20.0),
            width_pct: Some(20.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(free["layout"]["mode"], "free");
    assert_eq!(free["layout"]["x_pct"], 30.0);
    assert_eq!(free["layout"]["y_pct"], 20.0);

    let err = layout_set_impl(
        tmp.path(),
        &id,
        cc_core::command::render::LayoutPatch {
            position: Some("middle"),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("position"), "{err}");

    let err = layout_set_impl(
        tmp.path(),
        &id,
        cc_core::command::render::LayoutPatch {
            position: Some("bottom_right"),
            x_pct: Some(10.0),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("不能同时"), "{err}");
}

#[test]
fn final_video_and_download_commands() {
    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(
        tmp.path(),
        &["第一页讲稿内容".into(), "第二页讲稿内容".into()],
    );

    // not generated yet: done=false, download is a structured error
    let st = final_video_impl(tmp.path(), &id).unwrap();
    assert_eq!(st["done"], false);
    let dl_dir = tmp.path().join("dl");
    let err = download_video_impl(tmp.path(), &id, &dl_dir).unwrap_err();
    assert!(err.to_string().contains("尚未生成"), "{err}");

    generate_ok(
        tmp.path(),
        &id,
        false,
        &mut FakeMatteEngine::new(),
        collector().1,
    );

    let st = final_video_impl(tmp.path(), &id).unwrap();
    assert_eq!(st["done"], true);
    assert!(PathBuf::from(st["path_abs"].as_str().unwrap()).is_file());

    let dl = download_video_impl(tmp.path(), &id, &dl_dir).unwrap();
    let saved = PathBuf::from(dl["saved"].as_str().unwrap());
    assert!(saved.is_file(), "{dl}");
    assert!(saved
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with(".mp4"));
    // byte-identical copy of the film
    let sha = |p: &Path| {
        let mut h = Sha256::new();
        h.update(std::fs::read(p).unwrap());
        format!("{:x}", h.finalize())
    };
    assert_eq!(sha(&saved), sha(&dir.join("output/final.mp4")));
    // a second download does not clobber the first
    let dl2 = download_video_impl(tmp.path(), &id, &dl_dir).unwrap();
    let saved2 = PathBuf::from(dl2["saved"].as_str().unwrap());
    assert_ne!(saved, saved2, "同名不覆盖: {dl2}");
    assert!(saved2.is_file());
}

#[test]
fn person_video_set_copies_only_supported_formats() {
    let tmp = tempdir().unwrap();
    let v = create_project_impl(tmp.path(), "出镜测试", None).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = tmp.path().join("projects").join(&id);

    make_person_video(tmp.path());
    let src = tmp.path().join("source").join("person.mp4");
    let moved = tmp.path().join("my-video.MOV");
    std::fs::rename(&src, &moved).unwrap();

    let bad = person_video_set_impl(
        tmp.path(),
        &id,
        tmp.path().join("notes.txt").to_str().unwrap(),
    );
    assert!(bad.is_err(), "txt must be rejected");

    let ok = person_video_set_impl(tmp.path(), &id, moved.to_str().unwrap()).unwrap();
    assert_eq!(ok["ok"], true, "{ok}");
    assert!(dir.join("source/person.mp4").is_file());

    let status = person_status_impl(tmp.path(), &id).unwrap();
    assert_eq!(status["has_video"], true);
    assert!(PathBuf::from(status["video_abs"].as_str().unwrap()).is_file());
    assert_eq!(status["matte_done"], false);
}

/// Real-MiMo acceptance (ticket checklist: 真实路径各出片一次). Reads the key the user
/// previously stored via voice_configure from the OS credential manager — the key is
/// never printed, logged or written anywhere. Run with:
/// `cargo test -p cc-app --test pipeline real_mimo -- --ignored --test-threads=1`
/// (CC_MATTE_ENGINE is irrelevant here: matte uses the fake engine by construction.)
#[test]
#[ignore = "runs the real MiMo API; needs a stored key and network"]
fn real_mimo_full_generate() {
    std::env::remove_var("CC_CREDENTIALS_FILE"); // the real store, not the test seam
    match cc_core::credentials::read_api_key() {
        // presence only; the key value is never read, printed or stored anywhere
        Ok(Some(_)) => {}
        Ok(None) => {
            eprintln!("真实路径未验，原因：无 Key（凭据存储中没有 MiMo API Key）");
            return;
        }
        Err(err) => panic!("cannot read the credential store: {err}"),
    }

    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(
        tmp.path(),
        &[
            "大家好，欢迎来到本课程的第一讲。".into(),
            "第二页：今天我们讲两个要点。".into(),
        ],
    );

    // voiceclone needs a reference clip; synthesize 3 s of speech-shaped wav via ffmpeg
    let ref_src = tmp.path().join("ref.wav");
    let out = SysCommand::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=220:duration=3",
        ])
        .args(["-ar", "24000", "-ac", "1", "-c:a", "pcm_s16le"])
        .arg(&ref_src)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    cc_app::pipeline::voice_configure_impl(
        tmp.path(),
        &id,
        None,
        Some(ref_src.to_str().unwrap()),
        None,
        None,
    )
    .unwrap();

    let chain = cc_core::build_fallback_chain(cc_core::TtsEngineKind::Mimo).unwrap();
    let v = generate_impl(
        tmp.path(),
        &id,
        &chain,
        true.into(),
        &mut FakeMatteEngine::new(),
        &dummy_model(tmp.path()),
        collector().1,
    )
    .unwrap();
    assert_eq!(v["ok"], true, "真实 MiMo 全流程: {v}");
    assert_eq!(v["segments"]["tts"]["state"], "done");
    let tool = v["segments"]["tts"]["tool_version"].as_str().unwrap_or("");
    assert!(tool.contains("mimo"), "真实路径必须走 MiMo 引擎: {tool}");
    assert!(
        !v["segments"]["tts"]["fallbacks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["from"] == "mimo"),
        "真实 Key 不应降级: {tool}"
    );
    let output = dir.join("output/final.mp4");
    assert!(output.is_file() && output.metadata().unwrap().len() > 0);
    for page in 1..=2 {
        let info = cc_core::wav::parse_wav(
            &std::fs::read(dir.join(format!("work/audio/slide_{page:03}.wav"))).unwrap(),
        )
        .unwrap();
        assert!(info.duration_sec > 1.0, "克隆语音应有真实时长: {info:?}");
    }
}

/// 跳过抠像必须是服务端持久化动作：清掉旧 alpha + 记录，生成本次走卡片兜底；
/// 重新进入项目也不会又捡起上一段视频的抠像。
#[test]
fn skipping_matte_drops_the_alpha_and_generates_with_the_card() {
    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(tmp.path(), &["第一页讲稿".into(), "第二页讲稿".into()]);
    let mut engine = FakeMatteEngine::new();
    let v = person_matte_impl(
        tmp.path(),
        &id,
        &mut engine,
        &dummy_model(tmp.path()),
        collector().1,
    )
    .unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert!(dir.join("work/person/alpha.mov").is_file());

    let skipped = person_matte_skip_impl(tmp.path(), &id).unwrap();
    assert_eq!(skipped["ok"], true);
    assert_eq!(skipped["matte_done"], false);
    assert!(
        !dir.join("work/person/alpha.mov").exists(),
        "alpha must be dropped"
    );
    assert!(!dir.join("work/person/preview.png").exists());
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert!(
        !project.stages.contains_key("matte"),
        "matte record must be dropped"
    );

    let v = generate_ok(
        tmp.path(),
        &id,
        false,
        &mut FakeMatteEngine::new(),
        collector().1,
    );
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["segments"]["render"]["pip"], "card");
}

/// 换一段出镜视频 → 旧 alpha 立即失效（预览还没跑也不许复用上一段的抠像）。
#[test]
fn replacing_the_person_video_drops_the_previous_matte() {
    let tmp = tempdir().unwrap();
    let (id, dir) = prepared_project(tmp.path(), &["第一页讲稿".into(), "第二页讲稿".into()]);
    let mut engine = FakeMatteEngine::new();
    person_matte_impl(
        tmp.path(),
        &id,
        &mut engine,
        &dummy_model(tmp.path()),
        collector().1,
    )
    .unwrap();
    assert!(dir.join("work/person/alpha.mov").is_file());

    let other = tmp.path().join("other.mp4");
    std::fs::copy(dir.join("source/person.mp4"), &other).unwrap();
    let v = person_video_set_impl(tmp.path(), &id, other.to_str().unwrap()).unwrap();
    assert_eq!(v["ok"], true);
    assert!(!dir.join("work/person/alpha.mov").exists());
    assert!(!dir.join("work/person/preview.png").exists());
    let (project, _) = cc_core::load_project(tmp.path(), &id).unwrap();
    assert!(!project.stages.contains_key("matte"));
}
