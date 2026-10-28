//! Fingerprint invalidation propagation + crash resume (ticket 10; spec Testing
//! Decisions 必测场景「指纹失效传播」「断点续跑」). Seam level with the offline engine
//! doubles; the CLI e2e mirror lives in cc-cli/tests/invalidation.rs.
//!
//! The propagation chain must be assertable in project.json itself (depends_on /
//! fingerprint changes), not just recomputed: editing page 7 leaves the timeline
//! record's `depends_on["tts:7"]` pointing at the OLD fingerprint until the track is
//! rebuilt — that stale link IS the propagation record.

#![expect(
    clippy::clone_on_ref_ptr,
    reason = "tests clone concrete Arc handles that coerce to Arc<dyn TtsEngine>"
)]

use cc_core::matte::{run_matte_locked, STAGE_KEY as MATTE_KEY};
use cc_core::render::{run_locked as run_render, LAYOUT_TOML, STAGE_KEY as RENDER_KEY};
use cc_core::timeline::{run_locked as run_timeline, STAGE_KEY as TIMELINE_KEY, VOICE_FULL};
use cc_core::{
    create_project, load_project, save_project, FakeMatteEngine, FakeTtsEngine, FallbackChain,
    Stage, SynthesisOptions,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

const PAGES: u32 = 8;

fn script_text(pages: &[String]) -> String {
    pages.join("\n\n---\n\n")
}

fn default_pages() -> Vec<String> {
    (1..=PAGES).map(|i| format!("第{i}页讲稿内容")).collect()
}

fn write_gray_png(path: &Path, color: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let out = Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x180"),
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

fn make_person_video(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let out = Command::new("ffmpeg")
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

/// A hashable stand-in for the RVM onnx (the fake engine never loads it).
fn fake_model(data_dir: &Path) -> PathBuf {
    let path = data_dir.join("fake-rvm.onnx");
    std::fs::write(&path, b"not-a-real-model-but-hashable").unwrap();
    path
}

struct Rig {
    id: String,
    dir: PathBuf,
    tts: Arc<FakeTtsEngine>,
    matte: FakeMatteEngine,
    model: PathBuf,
}

/// Full pipeline through the real stage runners with the offline doubles:
/// parsed → tts (fake) → timeline → matte (fake) → render, ending at `Done`.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "test helper selects one of two fixture modes"
)]
fn full_fixture(data_dir: &Path, with_voice_ref: bool) -> Rig {
    let created = create_project(data_dir, "失效传播", Some("640x360")).unwrap();
    let id = created.project.project_id.clone();
    let dir = created.dir;

    std::fs::write(dir.join("script.md"), script_text(&default_pages())).unwrap();
    if with_voice_ref {
        std::fs::write(
            dir.join("source").join("voice_ref.wav"),
            b"reference-voice-v1",
        )
        .unwrap();
    }
    for i in 1..=PAGES {
        write_gray_png(
            &dir.join(format!("work/slides/slide_{i:03}.png")),
            "0x404040",
        );
    }

    let (mut project, dir) = load_project(data_dir, &id).unwrap();
    project.stage = Stage::Parsed;
    save_project(&project, &dir).unwrap();

    let tts = Arc::new(FakeTtsEngine::new());
    cc_core::synthesize_pages(
        data_dir,
        &id,
        &FallbackChain::single(tts.clone()),
        None,
        &SynthesisOptions::default(),
    )
    .unwrap();
    let mut project = reload(data_dir, &id);
    run_timeline(&mut project, &dir).unwrap();

    make_person_video(&dir.join("source/person.mp4"));
    let model = fake_model(data_dir);
    let mut matte = FakeMatteEngine::new();
    let mut project = reload(data_dir, &id);
    run_matte_locked(&mut project, &dir, &mut matte, &model).unwrap();
    let mut project = reload(data_dir, &id);
    run_render(&mut project, &dir).unwrap();

    let (project, _) = load_project(data_dir, &id).unwrap();
    assert_eq!(project.stage, Stage::Done, "fixture must end at done");
    Rig {
        id,
        dir,
        tts,
        matte,
        model,
    }
}

fn reload(data_dir: &Path, id: &str) -> cc_core::Project {
    load_project(data_dir, id).unwrap().0
}

fn pj(data_dir: &Path, id: &str) -> Value {
    let dir = cc_core::project_dir(data_dir, id);
    serde_json::from_str(&std::fs::read_to_string(dir.join("project.json")).unwrap()).unwrap()
}

fn fp<'a>(v: &'a Value, key: &str) -> &'a str {
    v["stages"][key]["input_fingerprint"].as_str().unwrap()
}

fn synthesize(rig: &Rig, data_dir: &Path, pages: Option<&str>) -> cc_core::tts::SynthesisReport {
    cc_core::synthesize_pages(
        data_dir,
        &rig.id,
        &FallbackChain::single(rig.tts.clone()),
        pages,
        &SynthesisOptions::default(),
    )
    .unwrap()
}

/// AC 1: 改第 7 页文本 → 仅 tts@7 重生成；整轨、成片失效；matte 产物指纹未变。
/// The stale `depends_on` links in project.json are the assertable propagation record.
#[test]
fn editing_page_seven_propagates_minimally_through_track_and_render() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();
    let mut rig = full_fixture(data_dir, false);

    let before = pj(data_dir, &rig.id);
    let (old_tts7, old_tl_fp, old_render_fp) = (
        fp(&before, "tts:7").to_string(),
        fp(&before, "timeline").to_string(),
        fp(&before, RENDER_KEY).to_string(),
    );
    let old_matte_fp = fp(&before, MATTE_KEY).to_string();
    let tts_calls = rig.tts.call_count();
    let matte_calls = rig.matte.call_count();

    // edit page 7 (different length → the audio duration changes with it)
    let mut pages = default_pages();
    pages[6] = "第7页讲稿内容已经被修改得更长了".into();
    std::fs::write(rig.dir.join("script.md"), script_text(&pages)).unwrap();

    let report = synthesize(&rig, data_dir, None);
    assert_eq!(
        report.generated,
        vec![7],
        "only the edited page regenerates"
    );
    assert_eq!(
        report.skipped,
        (1..=PAGES).filter(|p| *p != 7).collect::<Vec<_>>()
    );
    assert_eq!(
        rig.tts.call_count() - tts_calls,
        1,
        "exactly one engine call"
    );
    assert_eq!(report.stage, Stage::TtsDone);

    // assertable propagation records in project.json
    let v = pj(data_dir, &rig.id);
    let new_tts7 = fp(&v, "tts:7").to_string();
    assert_ne!(new_tts7, old_tts7);
    assert_eq!(
        v["stages"][TIMELINE_KEY]["depends_on"]["tts:7"].as_str().unwrap(),
        old_tts7,
        "the timeline record still points at the OLD page fingerprint — the stale link is the propagation record"
    );
    assert_eq!(
        v["stages"][RENDER_KEY]["depends_on"]["timeline"]
            .as_str()
            .unwrap(),
        old_tl_fp,
        "render still points at the old track fingerprint"
    );
    assert_eq!(
        fp(&v, MATTE_KEY),
        old_matte_fp,
        "matte fingerprint must be untouched"
    );
    assert_eq!(v["stage"], "tts_done");

    // the track rebuilds (durations changed → new fingerprint, depends_on re-recorded)
    let mut project = reload(data_dir, &rig.id);
    let ts = run_timeline(&mut project, &rig.dir).unwrap();
    assert!(!ts.skipped);
    assert_ne!(ts.fingerprint, old_tl_fp);
    let v = pj(data_dir, &rig.id);
    assert_eq!(
        v["stages"][TIMELINE_KEY]["depends_on"]["tts:7"]
            .as_str()
            .unwrap(),
        new_tts7
    );

    // the render re-runs
    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    let rs = run_render(&mut project, &dir).unwrap();
    assert!(!rs.skipped);
    assert_ne!(rs.fingerprint, old_render_fp);
    assert_eq!(project.stage, Stage::Done);

    // person_matte: fingerprint unchanged → skip without a single engine call
    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    let mr = run_matte_locked(&mut project, &dir, &mut rig.matte, &rig.model).unwrap();
    assert!(mr.skipped);
    assert_eq!(mr.fingerprint, old_matte_fp);
    assert_eq!(
        rig.matte.call_count(),
        matte_calls,
        "matte must not re-process"
    );
}

/// AC 2: 换参考音频 → 全部页失效 → 整轨 → 成片。The fake engine folds the reference
/// bytes into its output, so the swap changes every page's audio CONTENT while the
/// durations (and therefore the timeline fingerprint) stay identical — exactly the case
/// the `depends_on` assertions exist for.
#[test]
fn swapping_reference_audio_invalidates_every_page_then_track_then_render() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();
    let mut rig = full_fixture(data_dir, true);

    let before = pj(data_dir, &rig.id);
    let old_page_fps: Vec<String> = (1..=PAGES)
        .map(|p| fp(&before, &format!("tts:{p}")).to_string())
        .collect();
    let old_voice_full = before["stages"][TIMELINE_KEY]["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["path"] == VOICE_FULL)
        .unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let old_render_deps = before["stages"][RENDER_KEY]["depends_on"].clone();
    let tts_calls = rig.tts.call_count();
    let matte_calls = rig.matte.call_count();
    let old_matte_fp = fp(&before, MATTE_KEY).to_string();

    std::fs::write(
        rig.dir.join("source").join("voice_ref.wav"),
        b"a completely different reference voice",
    )
    .unwrap();

    let report = synthesize(&rig, data_dir, None);
    assert_eq!(
        report.generated,
        (1..=PAGES).collect::<Vec<_>>(),
        "every page is invalidated"
    );
    assert!(report.skipped.is_empty());
    assert_eq!(rig.tts.call_count() - tts_calls, PAGES);

    let v = pj(data_dir, &rig.id);
    for (p, old) in (1..=PAGES).zip(&old_page_fps) {
        assert_ne!(
            fp(&v, &format!("tts:{p}")),
            old.as_str(),
            "page {p} fingerprint must change"
        );
        assert_eq!(
            v["stages"][TIMELINE_KEY]["depends_on"][format!("tts:{p}")]
                .as_str()
                .unwrap(),
            old.as_str(),
            "timeline depends_on still records the old page fingerprints"
        );
    }

    // 整轨: rebuilt although the duration fingerprint is identical (same text lengths)
    let mut project = reload(data_dir, &rig.id);
    let ts = run_timeline(&mut project, &rig.dir).unwrap();
    assert!(!ts.skipped, "depends_on mismatch must force the rebuild");
    let v = pj(data_dir, &rig.id);
    let new_voice_full = v["stages"][TIMELINE_KEY]["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["path"] == VOICE_FULL)
        .unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(new_voice_full, old_voice_full, "the track content changed");

    // 成片: re-renders even though its fingerprint inputs are unchanged —
    // depends_on["voice_full"] is the only thing that caught it
    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    let rs = run_render(&mut project, &dir).unwrap();
    assert!(!rs.skipped, "stale voice_full must force the re-render");
    let v = pj(data_dir, &rig.id);
    assert_eq!(
        v["stages"][RENDER_KEY]["depends_on"]["voice_full"]
            .as_str()
            .unwrap(),
        new_voice_full
    );
    assert_ne!(
        v["stages"][RENDER_KEY]["depends_on"], old_render_deps,
        "the render record's dependency hashes must move with the new track"
    );
    assert_eq!(project.stage, Stage::Done);

    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    let mr = run_matte_locked(&mut project, &dir, &mut rig.matte, &rig.model).unwrap();
    assert!(mr.skipped, "matte does not depend on the voice");
    assert_eq!(mr.fingerprint, old_matte_fp);
    assert_eq!(rig.matte.call_count(), matte_calls);
}

/// AC 3: 改 layout.toml → 仅成片失效。
#[test]
fn changing_layout_invalidates_only_the_render() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();
    let rig = full_fixture(data_dir, false);

    let before = pj(data_dir, &rig.id);
    let old_tts: Vec<(String, String)> = (1..=PAGES)
        .map(|p| {
            (
                format!("tts:{p}"),
                fp(&before, &format!("tts:{p}")).to_string(),
            )
        })
        .collect();
    let old_tl_fp = fp(&before, TIMELINE_KEY).to_string();
    let old_tl_deps = before["stages"][TIMELINE_KEY]["depends_on"].clone();
    let old_matte_fp = fp(&before, MATTE_KEY).to_string();
    let old_render_fp = fp(&before, RENDER_KEY).to_string();
    let matte_calls = rig.matte.call_count();

    std::fs::write(
        rig.dir.join(LAYOUT_TOML),
        "position = \"bottom_right\"\nwidth_pct = 25.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n",
    )
    .unwrap();

    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    let rs = run_render(&mut project, &dir).unwrap();
    assert!(!rs.skipped, "layout change must re-render");
    assert_ne!(rs.fingerprint, old_render_fp);
    assert_eq!(project.stage, Stage::Done);

    let v = pj(data_dir, &rig.id);
    for (key, old) in &old_tts {
        assert_eq!(fp(&v, key), old, "{key} untouched");
    }
    assert_eq!(fp(&v, TIMELINE_KEY), old_tl_fp);
    assert_eq!(v["stages"][TIMELINE_KEY]["depends_on"], old_tl_deps);
    assert_eq!(fp(&v, MATTE_KEY), old_matte_fp);

    // idempotent re-entry at done: nothing changed → skip, no illegal transition
    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    let again = run_render(&mut project, &dir).unwrap();
    assert!(again.skipped, "unchanged inputs must skip");
    assert_eq!(project.stage, Stage::Done);
    assert_eq!(rig.matte.call_count(), matte_calls);
}

/// 断点续跑 (AC 4): tts 中途 kill -9 → stage 停在 tts_running、已完成页的记录已在盘 →
/// 重进后已完成页 skipped，未完成页续跑。
#[test]
fn kill_during_tts_resumes_completed_pages_and_finishes_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();
    let created = create_project(data_dir, "断点续跑", None).unwrap();
    let id = created.project.project_id.clone();
    let dir = created.dir;
    std::fs::write(dir.join("script.md"), "一\n\n---\n\n二\n\n---\n\n三\n").unwrap();
    let (mut project, dir) = load_project(data_dir, &id).unwrap();
    project.stage = Stage::Parsed;
    save_project(&project, &dir).unwrap();

    let first = Arc::new(FakeTtsEngine::new());
    cc_core::synthesize_pages(
        data_dir,
        &id,
        &FallbackChain::single(first.clone()),
        Some("1-2"),
        &SynthesisOptions::default(),
    )
    .unwrap();

    // kill -9 residue: the last per-page save left stage at tts_running
    let (mut project, dir) = load_project(data_dir, &id).unwrap();
    project.stage = Stage::TtsRunning;
    save_project(&project, &dir).unwrap();

    let resume = Arc::new(FakeTtsEngine::new());
    let report = cc_core::synthesize_pages(
        data_dir,
        &id,
        &FallbackChain::single(resume.clone()),
        None,
        &SynthesisOptions::default(),
    )
    .unwrap();
    assert_eq!(report.skipped, vec![1, 2], "completed pages skip");
    assert_eq!(report.generated, vec![3], "the interrupted page continues");
    assert_eq!(
        resume.call_count(),
        1,
        "only the unfinished page hits the engine"
    );
    assert_eq!(report.stage, Stage::TtsDone);

    let (project, _) = load_project(data_dir, &id).unwrap();
    assert_eq!(project.stage, Stage::TtsDone);
    assert!(
        project.failed.is_none(),
        "a resumed run clears the crash state"
    );
}

/// kill -9 landed between the last per-page save and the tts_done transition: the
/// re-entry finds every page recorded and completes the interrupted stage.
#[test]
fn kill_after_the_last_page_completes_the_stage_on_reentry() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();
    let created = create_project(data_dir, "僵尸续跑", None).unwrap();
    let id = created.project.project_id.clone();
    let dir = created.dir;
    std::fs::write(dir.join("script.md"), "一\n\n---\n\n二\n").unwrap();
    let (mut project, dir) = load_project(data_dir, &id).unwrap();
    project.stage = Stage::Parsed;
    save_project(&project, &dir).unwrap();

    cc_core::synthesize_pages(
        data_dir,
        &id,
        &FallbackChain::single(Arc::new(FakeTtsEngine::new())),
        None,
        &SynthesisOptions::default(),
    )
    .unwrap();

    let (mut project, dir) = load_project(data_dir, &id).unwrap();
    project.stage = Stage::TtsRunning;
    save_project(&project, &dir).unwrap();

    let engine = Arc::new(FakeTtsEngine::new());
    let report = cc_core::synthesize_pages(
        data_dir,
        &id,
        &FallbackChain::single(engine.clone()),
        None,
        &SynthesisOptions::default(),
    )
    .unwrap();
    assert_eq!(report.skipped, vec![1, 2]);
    assert!(report.generated.is_empty());
    assert_eq!(engine.call_count(), 0, "nothing left to synthesize");
    assert_eq!(
        report.stage,
        Stage::TtsDone,
        "the interrupted stage completes"
    );
}

/// AC 5: 失败阶段重试 — 单阶段重试命令，不从全片重跑。
#[test]
fn failed_render_retries_alone_without_rerunning_upstream() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();
    let rig = full_fixture(data_dir, false);

    let voice_full = rig.dir.join(VOICE_FULL);
    let good = std::fs::read(&voice_full).unwrap();
    let before = pj(data_dir, &rig.id);
    let old_tl_fp = fp(&before, TIMELINE_KEY).to_string();
    let old_matte_fp = fp(&before, MATTE_KEY).to_string();
    let old_render_fp = fp(&before, RENDER_KEY).to_string();
    let old_tts: Vec<(String, String)> = (1..=PAGES)
        .map(|p| {
            (
                format!("tts:{p}"),
                fp(&before, &format!("tts:{p}")).to_string(),
            )
        })
        .collect();
    let matte_calls = rig.matte.call_count();

    // break the render's input → the skip check catches the stale voice_full hash and
    // the render fails (and only the render)
    std::fs::write(&voice_full, b"corrupted, not a wav").unwrap();
    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    assert!(
        run_render(&mut project, &dir).is_err(),
        "corrupt track must fail the render"
    );
    let (project, _) = load_project(data_dir, &rig.id).unwrap();
    assert_eq!(project.stage, Stage::Failed);
    let failed = project.failed.as_ref().unwrap();
    assert_eq!(failed.stage, Stage::Rendering);
    assert_eq!(
        project.stages.get(RENDER_KEY).unwrap().input_fingerprint,
        old_render_fp,
        "the failed run must not touch the previous success record"
    );

    // repair and retry the single stage: upstream records stay untouched
    std::fs::write(&voice_full, good).unwrap();
    let (mut project, dir) = load_project(data_dir, &rig.id).unwrap();
    let rs = run_render(&mut project, &dir).unwrap();
    assert!(!rs.skipped);
    assert_eq!(project.stage, Stage::Done);

    let v = pj(data_dir, &rig.id);
    for (key, old) in &old_tts {
        assert_eq!(
            fp(&v, key),
            old.as_str(),
            "{key} untouched by the render retry"
        );
    }
    assert_eq!(fp(&v, TIMELINE_KEY), old_tl_fp);
    assert_eq!(fp(&v, MATTE_KEY), old_matte_fp);
    assert_eq!(
        rig.matte.call_count(),
        matte_calls,
        "matte never re-runs for a render retry"
    );
}

/// 无变更重跑：timeline 的指纹与 depends_on 都匹配、产物完整 → 整轨跳过。
#[test]
fn unchanged_inputs_skip_the_timeline_rebuild() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path();
    let rig = full_fixture(data_dir, false);

    let before = pj(data_dir, &rig.id);
    let old_fp = fp(&before, TIMELINE_KEY).to_string();
    let old_voice_full = before["stages"][TIMELINE_KEY]["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["path"] == VOICE_FULL)
        .unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_string();

    let mut project = reload(data_dir, &rig.id);
    let ts = run_timeline(&mut project, &rig.dir).unwrap();
    assert!(ts.skipped, "unchanged inputs must skip the track rebuild");
    assert_eq!(ts.fingerprint, old_fp);

    let v = pj(data_dir, &rig.id);
    let voice_full = v["stages"][TIMELINE_KEY]["outputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["path"] == VOICE_FULL)
        .unwrap()["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        voice_full, old_voice_full,
        "skipped rebuild leaves the artifacts untouched"
    );
}
