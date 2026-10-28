//! Seam: `cc_core::synthesize_pages` with injected engine doubles (spec Testing
//! Decisions, 缝 2). These cover the orchestration behaviors the CLI e2e cannot
//! observe directly: engine call counts, timeouts, per-page failure isolation.

#![expect(
    clippy::clone_on_ref_ptr,
    reason = "tests clone concrete Arc handles that coerce to Arc<dyn TtsEngine>"
)]

use cc_core::{create_project, load_project, save_project, FakeTtsEngine, Stage};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// Project fixture: created + script.md written + stage forced to `Parsed`.
fn fixture(data_dir: &Path, script: &str) -> String {
    let created = create_project(data_dir, "测试项目", None).unwrap();
    let id = created.project.project_id.clone();
    std::fs::write(created.dir.join("script.md"), script).unwrap();
    let (mut project, dir) = load_project(data_dir, &id).unwrap();
    project.stage = Stage::Parsed;
    save_project(&project, &dir).unwrap();
    id
}

fn two_page_script() -> &'static str {
    "第一页讲稿\n\n---\n\n第二页讲稿\n"
}

fn fast_options() -> cc_core::tts::SynthesisOptions {
    cc_core::tts::SynthesisOptions {
        page_timeout: Duration::from_millis(50),
        max_attempts: 3,
    }
}

#[test]
fn rerun_with_same_inputs_skips_all_pages_without_engine_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let engine = Arc::new(FakeTtsEngine::new());

    let first = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(engine.clone()),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();
    assert_eq!(first.generated, vec![1, 2]);
    assert_eq!(engine.call_count(), 2);

    let second = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(engine.clone()),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();
    assert!(
        second.generated.is_empty(),
        "same inputs must regenerate nothing"
    );
    assert_eq!(second.skipped, vec![1, 2]);
    assert_eq!(
        engine.call_count(),
        2,
        "idempotent rerun must not call the engine again"
    );
    assert_eq!(second.stage, Stage::TtsDone);
    assert_eq!(
        second.artifacts.len(),
        2,
        "skipped pages still report their artifacts"
    );
    assert_eq!(second.artifacts[0].path, "work/audio/slide_001.wav");
    // stage record survives unchanged
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::TtsDone);
}

#[test]
fn engine_timeout_triggers_retry_and_page_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), "单页讲稿\n");
    // first attempt sleeps far beyond the 50ms timeout, retry succeeds fast
    let engine = Arc::new(FakeTtsEngine::new().slow_first_attempts(1, Duration::from_millis(500)));

    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(engine.clone()),
        None,
        &fast_options(),
    )
    .unwrap();
    assert_eq!(
        report.generated,
        vec![1],
        "retry after timeout must land the page: {report:?}"
    );
    assert_eq!(
        engine.call_count(),
        2,
        "one timed-out attempt + one successful retry"
    );
    assert!(tmp
        .path()
        .join("projects")
        .join(&id)
        .join("work/audio/slide_001.wav")
        .is_file());
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::TtsDone);
}

#[test]
fn retry_checks_landed_artifacts_and_warns_about_possible_double_billing() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), "单页讲稿\n");
    let dir = tmp.path().join("projects").join(&id);
    // crash window: the wav landed but no stage record was written
    std::fs::create_dir_all(dir.join("work/audio")).unwrap();
    std::fs::write(
        dir.join("work/audio/slide_001.wav"),
        b"stale bytes from a crashed run",
    )
    .unwrap();
    let engine = Arc::new(FakeTtsEngine::new().slow_first_attempts(1, Duration::from_millis(500)));

    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(engine.clone()),
        None,
        &fast_options(),
    )
    .unwrap();
    assert_eq!(
        engine.call_count(),
        2,
        "the pre-placed wav cannot be trusted, engine is called"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("重复计费") && w.contains("slide_001.wav")),
        "retry must surface the honest idempotency boundary: {:?}",
        report.warnings
    );
    assert_eq!(report.generated, vec![1]);
    // the untrustworthy artifact was replaced by a real synthesis
    let bytes = std::fs::read(dir.join("work/audio/slide_001.wav")).unwrap();
    assert!(
        cc_core::wav::parse_wav(&bytes).is_ok(),
        "landed artifact must be a valid wav now"
    );
}

#[test]
fn failed_page_is_isolated_from_other_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), "第一页讲稿\n\n---\n\n坏页：这页会失败\n");
    let dir = tmp.path().join("projects").join(&id);
    let engine = Arc::new(FakeTtsEngine::new().fail_on_text_contains("坏页"));

    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(engine.clone()),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    // healthy page landed, failing page is reported independently
    assert_eq!(report.generated, vec![1]);
    assert_eq!(report.failed_pages.len(), 1);
    assert_eq!(report.failed_pages[0].page, 2);
    assert!(
        report.failed_pages[0].error.contains("engine"),
        "{:?}",
        report.failed_pages
    );
    assert_eq!(
        engine.call_count(),
        4,
        "page 1 once + page 2 through all 3 attempts"
    );
    assert!(dir.join("work/audio/slide_001.wav").is_file());
    assert!(
        !dir.join("work/audio/slide_002.wav").exists(),
        "failed page must not land an artifact"
    );
    assert_eq!(report.stage, Stage::Failed);

    // project state: failed at tts, success record only for the healthy page
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::Failed);
    let failed = project
        .failed
        .as_ref()
        .expect("failed info must be recorded");
    assert_eq!(failed.stage, Stage::TtsRunning);
    assert!(failed.error.contains('2'));
    assert!(project.stages.contains_key("tts:1"));
    assert!(
        !project.stages.contains_key("tts:2"),
        "no success record for the failed page"
    );
}

#[test]
fn retry_after_page_failure_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), "第一页讲稿\n\n---\n\n坏页：这页会失败\n");
    let dir = tmp.path().join("projects").join(&id);

    let bad = Arc::new(FakeTtsEngine::new().fail_on_text_contains("坏页"));
    cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(bad),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    // healthy engine: failed page regenerates, healthy page is skipped (fingerprint match)
    let good = Arc::new(FakeTtsEngine::new());
    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(good.clone()),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();
    assert_eq!(report.generated, vec![2]);
    assert_eq!(
        report.skipped,
        vec![1],
        "the healthy page must not be re-synthesized"
    );
    assert_eq!(good.call_count(), 1);
    assert!(dir.join("work/audio/slide_002.wav").is_file());
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::TtsDone);
    assert!(
        project.failed.is_none(),
        "successful retry clears the failure record"
    );
}

/// Test double for the deterministic auth failure of a real engine (ticket 05): every
/// request is rejected with the structured 401 signal.
struct AlwaysUnauthorized(std::sync::atomic::AtomicU32);

impl cc_core::TtsEngine for AlwaysUnauthorized {
    fn name(&self) -> &'static str {
        "mimo"
    }

    fn model(&self) -> &'static str {
        "mimo-v2.5-tts-voiceclone"
    }

    fn synthesize(&self, _request: &cc_core::TtsRequest) -> cc_core::Result<Vec<u8>> {
        use std::sync::atomic::Ordering;
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(cc_core::Error::TtsUnauthorized {
            engine: "mimo".into(),
            message: "invalid api key".into(),
        })
    }
}

#[test]
fn auth_failure_stops_the_run_and_surfaces_the_structured_signal() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let dir = tmp.path().join("projects").join(&id);
    let engine = Arc::new(AlwaysUnauthorized(std::sync::atomic::AtomicU32::new(0)));

    let err = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(engine.clone()),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap_err();
    assert_eq!(
        err.code(),
        "tts_unauthorized",
        "frontend guidance matches on this code: {err}"
    );
    assert_eq!(
        engine.0.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "one attempt on page 1 only — a 401 never heals within a run, page 2 is not sent"
    );

    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::Failed);
    let failed = project.failed.as_ref().expect("failure must be recorded");
    assert_eq!(failed.stage, Stage::TtsRunning);
    assert!(
        failed.error.contains("401"),
        "the auth failure is in the project record: {failed:?}"
    );
    assert!(
        !dir.join("work/audio/slide_001.wav").exists(),
        "no artifact lands for a rejected page"
    );
}

/// 重配失败不能留下旧音频冒充新稿（spec story 22）：失败页的旧产物与成功记录
/// 必须一起消失，否则 timeline/render 会把旧配音当成当前输入。
#[test]
fn a_failed_regeneration_drops_the_old_audio_and_record() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let good = Arc::new(FakeTtsEngine::new());
    cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(good),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    let (_, dir) = load_project(tmp.path(), &id).unwrap();
    std::fs::write(dir.join("script.md"), "改过的第一页\n\n---\n\n第二页讲稿\n").unwrap();
    let bad = Arc::new(FakeTtsEngine::new().fail_on_text_contains("改过的第一页"));
    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(bad),
        None,
        &fast_options(),
    )
    .unwrap();

    assert_eq!(report.failed_pages.len(), 1);
    assert_eq!(report.failed_pages[0].page, 1);
    assert_eq!(report.skipped, vec![2], "unchanged page stays reused");

    let (project, dir) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::Failed);
    assert_eq!(project.failed.as_ref().unwrap().stage, Stage::TtsRunning);
    assert!(
        !project.stages.contains_key("tts:1"),
        "stale page record must be gone"
    );
    assert!(
        !dir.join("work/audio/slide_001.wav").exists(),
        "stale audio must be gone"
    );
    assert!(
        project.stages.contains_key("tts:2"),
        "the untouched page keeps its record"
    );
    assert!(dir.join("work/audio/slide_002.wav").exists());
}
