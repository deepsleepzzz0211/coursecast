//! Seam: `cc_core::synthesize_pages` with a fallback chain of injected engine doubles
//! (ticket 06, 三级降级链). Covers the selection logic the CLI e2e cannot observe
//! directly: sliding on failure, sticky engine removal, event recording, structured
//! all-fail errors. Real-engine chain building is covered in `fallback_chain.rs`, the
//! SAPI engine itself in `sapi_engine.rs`.

#![expect(
    clippy::clone_on_ref_ptr,
    reason = "tests clone concrete Arc handles that coerce to Arc<dyn TtsEngine>"
)]

use cc_core::{
    create_project, load_project, save_project, FakeTtsEngine, Stage, TtsEngine, TtsRequest,
};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

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

/// Test double with a chosen engine name and a fixed per-call failure mode
/// (`None` = succeed like a real engine would).
struct StubEngine {
    name: &'static str,
    failure: Option<StubFailure>,
    calls: AtomicU32,
}

enum StubFailure {
    /// Generic engine failure (network down, API 500, …).
    Engine(String),
    /// The structured HTTP 401 signal (ticket 05) the chain matches on.
    Unauthorized(String),
}

impl StubEngine {
    fn failing(name: &'static str, message: &str) -> Arc<Self> {
        Arc::new(StubEngine {
            name,
            failure: Some(StubFailure::Engine(message.to_string())),
            calls: AtomicU32::new(0),
        })
    }

    fn unauthorized(name: &'static str, message: &str) -> Arc<Self> {
        Arc::new(StubEngine {
            name,
            failure: Some(StubFailure::Unauthorized(message.to_string())),
            calls: AtomicU32::new(0),
        })
    }

    fn call_count(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }
}

impl TtsEngine for StubEngine {
    fn name(&self) -> &'static str {
        self.name
    }

    fn model(&self) -> &'static str {
        "stub-model"
    }

    fn synthesize(&self, _request: &TtsRequest) -> cc_core::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.failure {
            Some(StubFailure::Engine(message)) => Err(cc_core::Error::EngineFailure {
                engine: self.name.into(),
                message: message.clone(),
            }),
            Some(StubFailure::Unauthorized(message)) => Err(cc_core::Error::TtsUnauthorized {
                engine: self.name.into(),
                message: message.clone(),
            }),
            None => Ok(vec![]),
        }
    }
}

#[test]
fn head_engine_without_failures_synthesizes_without_degradation_events() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let engine = Arc::new(FakeTtsEngine::new());

    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &cc_core::FallbackChain::single(engine.clone()),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    assert_eq!(report.generated, vec![1, 2]);
    assert!(
        report.fallbacks.is_empty(),
        "no slide, no event: {:?}",
        report.fallbacks
    );
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert!(project.fallbacks.is_empty());
}

#[test]
fn primary_failure_slides_to_fallback_and_records_the_event() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let dir = tmp.path().join("projects").join(&id);
    let primary = StubEngine::failing("mimo", "network unreachable");
    let fallback = Arc::new(FakeTtsEngine::new());

    let chain = cc_core::FallbackChain {
        engines: vec![primary.clone(), fallback.clone()],
        events: Vec::new(),
    };
    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &chain,
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    // every page landed via the fallback engine
    assert_eq!(report.generated, vec![1, 2]);
    assert!(dir.join("work/audio/slide_001.wav").is_file());
    assert!(dir.join("work/audio/slide_002.wav").is_file());

    // the degradation is observable: structured event + user-readable reason
    assert_eq!(report.fallbacks.len(), 1, "{:?}", report.fallbacks);
    assert_eq!(report.fallbacks[0].from, "mimo");
    assert_eq!(report.fallbacks[0].to, "fake");
    assert!(
        report.fallbacks[0].reason.contains("network unreachable"),
        "{report:?}"
    );

    // persisted into project.json (the UI banner data source)
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.fallbacks, report.fallbacks);
    // page records name the engine that actually produced the audio
    let record = project.stages.get("tts:1").unwrap();
    assert_eq!(record.tool_version, "tts-fake/0.1.0+model=fake-sine-24k");

    // sticky slide: the broken primary is not retried on later pages
    assert_eq!(
        primary.call_count(),
        3,
        "only page 1's three attempts hit the primary"
    );
    assert_eq!(
        fallback.call_count(),
        2,
        "one call per page on the fallback"
    );
}

#[test]
fn unauthorized_slides_instead_of_stopping_when_a_fallback_remains() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let primary = StubEngine::unauthorized("mimo", "invalid api key");
    let fallback = Arc::new(FakeTtsEngine::new());

    let chain = cc_core::FallbackChain {
        engines: vec![primary.clone(), fallback.clone()],
        events: Vec::new(),
    };
    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &chain,
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    assert_eq!(
        report.generated,
        vec![1, 2],
        "the run must not stop on a 401 with a fallback: {report:?}"
    );
    assert_eq!(report.fallbacks[0].from, "mimo");
    assert_eq!(report.fallbacks[0].to, "fake");
    assert!(
        report.fallbacks[0].reason.contains("401"),
        "{:?}",
        report.fallbacks
    );
    // a 401 never heals: no retry even on the page that discovered it, no later pages
    assert_eq!(primary.call_count(), 1);
    assert_eq!(fallback.call_count(), 2);
}

#[test]
fn all_engines_failing_yields_a_structured_page_failure_naming_every_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let first = StubEngine::failing("mimo", "network unreachable");
    let second = StubEngine::failing("sapi", "no voices installed");

    let chain = cc_core::FallbackChain {
        engines: vec![first.clone(), second.clone()],
        events: Vec::new(),
    };
    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &chain,
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    assert!(report.generated.is_empty());
    assert_eq!(report.failed_pages.len(), 2);
    for failure in &report.failed_pages {
        assert!(
            failure.error.contains("mimo") && failure.error.contains("sapi"),
            "the structured failure must name every engine tried: {}",
            failure.error
        );
        assert!(failure.error.contains("network unreachable"));
        assert!(failure.error.contains("no voices installed"));
    }
    assert_eq!(report.stage, Stage::Failed);
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::Failed);
    assert!(
        !dir_check(tmp.path(), &id, "work/audio/slide_001.wav"),
        "no artifact for a failed page"
    );
    // the slide from the first to the second engine is still recorded
    assert_eq!(project.fallbacks.len(), 1);
    assert_eq!(project.fallbacks[0].from, "mimo");
    assert_eq!(project.fallbacks[0].to, "sapi");
}

fn dir_check(data_dir: &Path, id: &str, rel: &str) -> bool {
    data_dir.join("projects").join(id).join(rel).is_file()
}

#[test]
fn rerun_after_fallback_skips_without_calling_the_broken_primary_again_beyond_one_page() {
    let tmp = tempfile::tempdir().unwrap();
    let id = fixture(tmp.path(), two_page_script());
    let primary = StubEngine::failing("mimo", "network unreachable");
    let fallback = Arc::new(FakeTtsEngine::new());
    let chain = cc_core::FallbackChain {
        engines: vec![primary.clone(), fallback.clone()],
        events: Vec::new(),
    };
    cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &chain,
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    // rerun with the same broken primary: page 1 proves it is still broken, then the
    // fallback fingerprint matches and the page is skipped without an engine call
    let primary2 = StubEngine::failing("mimo", "network unreachable");
    let fallback2 = Arc::new(FakeTtsEngine::new());
    let chain2 = cc_core::FallbackChain {
        engines: vec![primary2.clone(), fallback2.clone()],
        events: Vec::new(),
    };
    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &chain2,
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();
    assert_eq!(report.skipped, vec![1, 2], "{report:?}");
    assert!(report.generated.is_empty());
    assert_eq!(
        fallback2.call_count(),
        0,
        "the fallback fingerprint matches: no engine call"
    );
    assert_eq!(
        primary2.call_count(),
        3,
        "only page 1 re-probes the broken primary"
    );
    assert_eq!(report.stage, Stage::TtsDone);
}
