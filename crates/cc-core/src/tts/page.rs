//! Per-page TTS policy (ticket 03): silence, fallback-chain walk, retry, idempotency,
//! atomic landing and failure isolation for one page. `synthesize_pages` owns the page
//! loop, final stage transitions and the report; this module owns what happens to a page.

use super::{PageArtifact, PageFailure, SynthesisOptions, TtsEngine, TtsRequest};
use crate::error::{Error, Result};
use crate::project::{save_project, FallbackEvent, OutputArtifact, Project};
use crate::stage;
use crate::state::Stage;
use crate::wav;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

pub(super) enum PageOutcome {
    Generated(PageArtifact),
    Skipped(PageArtifact),
    Failed(PageFailure),
    /// A deterministic auth failure with no engine left: the project is already marked
    /// failed and the caller must return this error.
    Fatal(Error),
}

/// Fingerprint of one page's synthesis inputs (spec "输入指纹模型"): 该页文本 + 引擎 +
/// 模型名 + 参考音频哈希 + 风格指令 + 语速参数.
#[derive(Serialize)]
struct FingerprintInput<'a> {
    engine: &'a str,
    model: &'a str,
    text: &'a str,
    reference_audio_sha256: &'a str,
    style: Option<&'a str>,
    rate: f32,
}

fn page_fingerprint(engine: &dyn TtsEngine, request: &TtsRequest, ref_hash: &str) -> String {
    let input = FingerprintInput {
        engine: engine.name(),
        model: engine.model(),
        text: &request.text,
        reference_audio_sha256: ref_hash,
        style: request.style.as_deref(),
        rate: request.rate,
    };
    stage::sha256_hex(
        serde_json::to_string(&input)
            .expect("serializing a plain struct cannot fail")
            .as_bytes(),
    )
}

/// Fingerprint of a silent page (empty script text): independent of every engine and
/// of the reference audio — silence has no voice, so swapping the reference cannot
/// invalidate it (ticket 10's 换参考音频全部页失效 only bites spoken pages).
fn silence_fingerprint() -> String {
    stage::sha256_hex(br#"{"engine":"silence","model":"5s","sample_rate":24000,"channels":1}"#)
}

fn tool_version(engine: &dyn TtsEngine) -> String {
    format!(
        "tts-{}/{}+model={}",
        engine.name(),
        env!("CARGO_PKG_VERSION"),
        engine.model()
    )
}

/// Per-page artifact path inside the project: `work/audio/slide_NNN.wav` (0-padded).
fn artifact_rel_path(page: u32) -> String {
    format!("work/audio/slide_{page:03}.wav")
}

fn page_key(page: u32) -> String {
    format!("tts:{page}")
}

/// A page whose (re)synthesis failed must not leave its previous artifact or success
/// record behind: the old audio belongs to different inputs (otherwise it would have
/// been skipped), and a stale record would let timeline/render certify the old take
/// as if it matched the new script (spec story 22).
fn drop_stale_page(project: &mut Project, dir: &Path, page: u32) -> Result<()> {
    project.stages.remove(&page_key(page));
    let abs = dir.join(artifact_rel_path(page));
    let _ = std::fs::remove_file(&abs);
    let _ = std::fs::remove_file(abs.with_extension("wav.tmp"));
    save_project(project, dir)
}

/// One engine call with a hard timeout: run it on a worker thread and stop waiting when
/// the timeout hits (like an HTTP client abandoning a request). The abandoned thread's
/// result is dropped — the caller owns artifact storage, so nothing lands on disk.
fn call_with_timeout(
    engine: &Arc<dyn TtsEngine>,
    request: &TtsRequest,
    timeout: std::time::Duration,
) -> Result<Vec<u8>> {
    let engine_name = engine.name();
    let engine = Arc::clone(engine);
    let request = request.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(engine.synthesize(&request));
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(Error::EngineTimeout {
            engine: engine_name.into(),
            timeout: timeout.as_secs_f64().to_string(),
        }),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(Error::EngineFailure {
            engine: engine_name.into(),
            message: "engine thread panicked".into(),
        }),
    }
}

/// One page through the request layer: bounded retries, and the honest idempotency
/// check before every retry — an already-landed artifact we cannot tie to the current
/// inputs means a retry may double-bill (spec "幂等性诚实边界").
fn synthesize_with_retry(
    engine: &Arc<dyn TtsEngine>,
    request: &TtsRequest,
    artifact_rel: &str,
    artifact_abs: &Path,
    page: u32,
    options: &SynthesisOptions,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>> {
    let mut last_err: Option<Error> = None;
    for attempt in 1..=options.max_attempts {
        if attempt > 1 && artifact_abs.is_file() {
            warnings.push(format!(
                "第 {page} 页重试前检测到已落盘产物 {artifact_rel}，但无法确认其对应本次输入；重试可能重复计费"
            ));
        }
        match call_with_timeout(engine, request, options.page_timeout) {
            Ok(bytes) => return Ok(bytes),
            // auth failure never heals within a run — retrying would only repeat the 401
            Err(err @ Error::TtsUnauthorized { .. }) => return Err(err),
            // a 403 block is deterministic inside the token window; the engine already
            // did its one jittered clock-skew retry, the request layer must not hammer it
            Err(err @ Error::EngineBlocked { .. }) => return Err(err),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.expect("at least one attempt was made"))
}

/// The mutable state that lives across pages: the sticky engine chain, degradation
/// events, warnings, and whether the stage has already entered `tts_running`.
pub(super) struct PageSynthesis<'a> {
    dir: &'a Path,
    options: &'a SynthesisOptions,
    ref_path: &'a Path,
    ref_hash: &'a str,
    chain_engines: &'a [Arc<dyn TtsEngine>],
    live: Vec<Arc<dyn TtsEngine>>,
    fallbacks: Vec<FallbackEvent>,
    warnings: Vec<String>,
    chain_exhausted: Option<String>,
    entered_running: bool,
}

impl<'a> PageSynthesis<'a> {
    pub(super) fn new(
        dir: &'a Path,
        options: &'a SynthesisOptions,
        ref_path: &'a Path,
        ref_hash: &'a str,
        chain_engines: &'a [Arc<dyn TtsEngine>],
        initial_fallbacks: Vec<FallbackEvent>,
    ) -> Self {
        Self {
            dir,
            options,
            ref_path,
            ref_hash,
            chain_engines,
            live: chain_engines.to_vec(),
            fallbacks: initial_fallbacks,
            warnings: Vec::new(),
            chain_exhausted: None,
            entered_running: false,
        }
    }

    /// Synthesize (or reuse) one page. Persists artifacts and stage records; returns a
    /// page outcome for the caller's report.
    pub(super) fn synthesize(
        &mut self,
        project: &mut Project,
        page: u32,
        text: &str,
    ) -> Result<PageOutcome> {
        let request = TtsRequest {
            text: text.to_string(),
            style: None,
            rate: 1.0,
            reference_audio: self.ref_path.is_file().then(|| self.ref_path.to_path_buf()),
        };
        if request.text.trim().is_empty() {
            self.synthesize_silence(project, page)
        } else {
            self.synthesize_spoken(project, page, &request)
        }
    }

    pub(super) fn fallbacks(&self) -> &[FallbackEvent] {
        &self.fallbacks
    }

    pub(super) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub(super) fn entered_running(&self) -> bool {
        self.entered_running
    }

    fn ensure_running(&mut self, project: &mut Project) -> Result<()> {
        if self.entered_running {
            return Ok(());
        }
        // resume (ticket 10): re-entering at tts_running IS the resume — the stage is
        // already there, so only transition from a different state.
        if project.stage != Stage::TtsRunning {
            project.transition(Stage::TtsRunning)?;
        }
        project.failed = None;
        project.fallbacks = self.fallbacks.clone();
        save_project(project, self.dir)?;
        std::fs::create_dir_all(self.dir.join("work").join("audio"))?;
        self.entered_running = true;
        Ok(())
    }

    /// Verify a recorded page through the shared stage protocol. No warning here:
    /// callers decide whether a near miss is worth reporting.
    fn reuse_recorded(
        &self,
        project: &Project,
        page: u32,
        rel: &str,
        abs: &Path,
        fingerprint: &str,
    ) -> Result<Option<PageArtifact>> {
        let key = page_key(page);
        if !stage::verify(
            project,
            self.dir,
            &stage::VerifySpec {
                key: &key,
                input_fingerprint: fingerprint,
                stage_deps: &[],
                content_deps: &[],
            },
        ) {
            return Ok(None);
        }
        let record = project.stages.get(&key).expect("verified record exists");
        let bytes = std::fs::read(abs)?;
        let info = wav::parse_wav(&bytes)?;
        Ok(Some(PageArtifact {
            page,
            path: rel.to_string(),
            sha256: record.outputs[0].sha256.clone(),
            bytes: bytes.len() as u64,
            duration_sec: info.duration_sec,
        }))
    }

    /// Reuse the recorded artifact for this page when the fingerprint matches and the
    /// file still has the recorded content. A success record whose artifact vanished or
    /// changed is reported honestly (and the page regenerates).
    fn reuse(
        &mut self,
        project: &Project,
        page: u32,
        rel: &str,
        abs: &Path,
        fingerprint: &str,
        warning: &str,
    ) -> Result<Option<PageArtifact>> {
        if let Some(artifact) = self.reuse_recorded(project, page, rel, abs, fingerprint)? {
            return Ok(Some(artifact));
        }
        let matching = project
            .stages
            .get(&page_key(page))
            .is_some_and(|r| r.state == "success" && r.input_fingerprint == fingerprint);
        if matching {
            self.warnings.push(warning.to_string());
        }
        Ok(None)
    }

    /// Land one page's bytes atomically and record success. The caller supplies the
    /// fingerprint/tool version; silence and spoken pages share this path.
    #[expect(
        clippy::fn_params_excessive_bools,
        clippy::too_many_arguments,
        reason = "page landing carries the full artifact context; set_fallbacks selects the fallbacks update"
    )]
    fn land_page(
        &mut self,
        project: &mut Project,
        page: u32,
        rel: &str,
        abs: &Path,
        fingerprint: String,
        tool_version: String,
        bytes: &[u8],
        set_fallbacks: bool,
    ) -> Result<PageArtifact> {
        let info = wav::parse_wav(bytes)?;
        let hash = stage::sha256_hex(bytes);
        stage::write_atomic(abs, bytes)?;
        let record = stage::success(
            "tts",
            fingerprint,
            tool_version,
            vec![OutputArtifact {
                path: rel.to_string(),
                sha256: hash.clone(),
                bytes: bytes.len() as u64,
            }],
            BTreeMap::new(),
        );
        if set_fallbacks {
            project.fallbacks = self.fallbacks.clone();
        }
        stage::land(project, self.dir, &page_key(page), record)?;
        Ok(PageArtifact {
            page,
            path: rel.to_string(),
            sha256: hash,
            bytes: bytes.len() as u64,
            duration_sec: info.duration_sec,
        })
    }

    fn synthesize_silence(&mut self, project: &mut Project, page: u32) -> Result<PageOutcome> {
        let rel = artifact_rel_path(page);
        let abs = self.dir.join(&rel);
        let fingerprint = silence_fingerprint();

        if let Some(artifact) = self.reuse(
            project,
            page,
            &rel,
            &abs,
            &fingerprint,
            &format!("第 {page} 页静默页有成功记录但产物缺失或不一致，将重新生成"),
        )? {
            return Ok(PageOutcome::Skipped(artifact));
        }

        self.ensure_running(project)?;
        let bytes = wav::encode_mono_pcm16(24_000, &vec![0i16; 5 * 24_000]);
        let artifact = self.land_page(
            project,
            page,
            &rel,
            &abs,
            fingerprint,
            "silence/5s-24000-mono".into(),
            &bytes,
            false,
        )?;
        Ok(PageOutcome::Generated(artifact))
    }

    fn synthesize_spoken(
        &mut self,
        project: &mut Project,
        page: u32,
        request: &TtsRequest,
    ) -> Result<PageOutcome> {
        let rel = artifact_rel_path(page);
        let abs = self.dir.join(&rel);
        let mut attempts: Vec<String> = Vec::new();

        while let Some(engine) = self.live.first().cloned() {
            let fingerprint = page_fingerprint(engine.as_ref(), request, self.ref_hash);

            // Idempotency per chain position: the most-preferred available engine whose
            // fingerprint matches reuses its artifact; a lower position only gets its
            // turn once the engines above it have failed and been removed.
            if let Some(artifact) = self.reuse(
                project,
                page,
                &rel,
                &abs,
                &fingerprint,
                &format!("第 {page} 页有成功记录但产物缺失或不一致，将重新合成（可能重复计费）"),
            )? {
                return Ok(PageOutcome::Skipped(artifact));
            }

            self.ensure_running(project)?;

            match synthesize_with_retry(
                &engine,
                request,
                &rel,
                &abs,
                page,
                self.options,
                &mut self.warnings,
            ) {
                Ok(bytes) => {
                    let artifact = self.land_page(
                        project,
                        page,
                        &rel,
                        &abs,
                        fingerprint,
                        tool_version(engine.as_ref()),
                        &bytes,
                        true,
                    )?;
                    return Ok(PageOutcome::Generated(artifact));
                }
                Err(err) => {
                    let is_auth = matches!(err, Error::TtsUnauthorized { .. });
                    let had_next = self.live.len() > 1;
                    attempts.push(format!("{}: {}", engine.name(), err));
                    if had_next {
                        // slide down (ticket 06): the next level takes over, the slide is
                        // recorded for project.json and the CLI JSON hint field
                        self.fallbacks.push(FallbackEvent {
                            from: engine.name().to_string(),
                            to: self.live[1].name().to_string(),
                            reason: err.to_string(),
                        });
                    } else if is_auth {
                        // deterministic auth failure with nothing left to fall back to:
                        // no page can succeed, so stop the run here and return the error
                        // itself — the CLI surfaces code `tts_unauthorized` (ticket 05)
                        project.fallbacks = self.fallbacks.clone();
                        drop_stale_page(project, self.dir, page)?;
                        project.fail(err.to_string())?;
                        save_project(project, self.dir)?;
                        return Ok(PageOutcome::Fatal(err));
                    }
                    self.live.remove(0); // the next engine becomes the head
                }
            }
        }

        if attempts.is_empty() {
            // The chain was already exhausted before this page. A previous result from
            // any original engine is still perfectly valid (e.g. another page burned
            // the only engine) — reuse it instead of reporting a phantom failure and
            // deleting good audio.
            if let Some(artifact) = self.recorded_artifact(project, page, request)? {
                return Ok(PageOutcome::Skipped(artifact));
            }
            // genuinely nothing recorded: report the accumulated reasons, no stale
            // artifact to drop (we never attempted this page in this run)
            let error = self
                .chain_exhausted
                .clone()
                .unwrap_or_else(|| "没有可用引擎".to_string());
            return Ok(PageOutcome::Failed(PageFailure { page, error }));
        }

        // this page was attempted and failed: its old audio belongs to different
        // inputs, so drop the record + artifact before reporting the failure
        drop_stale_page(project, self.dir, page)?;
        let composed = attempts.join("; ");
        if self.live.is_empty() {
            self.chain_exhausted = Some(composed.clone());
        }
        Ok(PageOutcome::Failed(PageFailure {
            page,
            error: composed,
        }))
    }

    /// Reuse a page's recorded artifact when no engine is left to synthesize it in this
    /// run, matching any engine from the original chain (the engine that produced it may
    /// already have been removed after failing on an earlier page).
    fn recorded_artifact(
        &self,
        project: &Project,
        page: u32,
        request: &TtsRequest,
    ) -> Result<Option<PageArtifact>> {
        let rel = artifact_rel_path(page);
        let abs = self.dir.join(&rel);
        let Some(record) = project.stages.get(&page_key(page)) else {
            return Ok(None);
        };
        if record.state != "success" {
            return Ok(None);
        }
        let matching_engine = self.chain_engines.iter().any(|engine| {
            page_fingerprint(engine.as_ref(), request, self.ref_hash) == record.input_fingerprint
        });
        if !matching_engine {
            return Ok(None);
        }
        let fingerprint = record.input_fingerprint.clone();
        self.reuse_recorded(project, page, &rel, &abs, &fingerprint)
    }
}

#[cfg(test)]
#[expect(
    clippy::clone_on_ref_ptr,
    reason = "typed Arc fixtures are cloned into the trait-object chain"
)]
mod tests {
    use super::*;
    use crate::project::{create_project, save_project};
    use crate::state::Stage;
    use crate::tts::{FakeTtsEngine, SynthesisOptions, TtsEngine, TtsRequest};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn project_at(stage: Stage) -> (TempDir, Project, std::path::PathBuf) {
        let data = TempDir::new().unwrap();
        let created = create_project(data.path(), "page", None).unwrap();
        let mut project = created.project;
        project.stage = stage;
        save_project(&project, &created.dir).unwrap();
        (data, project, created.dir)
    }

    fn options() -> SynthesisOptions {
        SynthesisOptions {
            page_timeout: std::time::Duration::from_secs(5),
            max_attempts: 2,
        }
    }

    struct FlakyEngine {
        calls: AtomicU32,
        inner: FakeTtsEngine,
    }

    impl TtsEngine for FlakyEngine {
        fn name(&self) -> &'static str {
            "flaky"
        }

        fn model(&self) -> &'static str {
            "flaky-once"
        }

        fn synthesize(&self, request: &TtsRequest) -> Result<Vec<u8>> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(Error::EngineFailure {
                    engine: "flaky".into(),
                    message: "first attempt fails".into(),
                });
            }
            self.inner.synthesize(request)
        }
    }

    #[test]
    fn silence_page_is_five_seconds_and_skips_every_engine() {
        let (_data, mut project, dir) = project_at(Stage::Parsed);
        let options = options();
        let engine = Arc::new(FakeTtsEngine::new());
        let chain: Vec<Arc<dyn TtsEngine>> = vec![engine.clone()];
        let ref_path = dir.join("source/voice_ref.wav");
        let mut synthesis = PageSynthesis::new(&dir, &options, &ref_path, "none", &chain, vec![]);

        let outcome = synthesis.synthesize(&mut project, 1, "").unwrap();
        match outcome {
            PageOutcome::Generated(artifact) => {
                assert!((artifact.duration_sec - 5.0).abs() < 1e-9);
            }
            _ => panic!("silence must generate"),
        }
        assert_eq!(engine.call_count(), 0, "silence must never call an engine");
        assert_eq!(project.stage, Stage::TtsRunning);
        assert!(project.stages.contains_key("tts:1"));
    }

    #[test]
    fn sticky_fallback_survives_to_later_pages() {
        let (_data, mut project, dir) = project_at(Stage::Parsed);
        let options = options();
        let primary = Arc::new(FakeTtsEngine::new().fail_on_text_contains("fail"));
        let secondary = Arc::new(FakeTtsEngine::new());
        let chain: Vec<Arc<dyn TtsEngine>> = vec![primary.clone(), secondary.clone()];
        let ref_path = dir.join("source/voice_ref.wav");
        let mut synthesis = PageSynthesis::new(&dir, &options, &ref_path, "none", &chain, vec![]);

        assert!(matches!(
            synthesis.synthesize(&mut project, 1, "fail here").unwrap(),
            PageOutcome::Generated(_)
        ));
        assert_eq!(synthesis.fallbacks().len(), 1);
        assert_eq!(primary.call_count(), 2, "retry budget spent once");
        assert_eq!(secondary.call_count(), 1);

        assert!(matches!(
            synthesis.synthesize(&mut project, 2, "good page").unwrap(),
            PageOutcome::Generated(_)
        ));
        assert_eq!(primary.call_count(), 2, "failed primary is sticky");
        assert_eq!(secondary.call_count(), 2);
    }

    #[test]
    fn retry_recovers_a_flaky_page() {
        let (_data, mut project, dir) = project_at(Stage::Parsed);
        let options = options();
        let flaky = Arc::new(FlakyEngine {
            calls: AtomicU32::new(0),
            inner: FakeTtsEngine::new(),
        });
        let chain: Vec<Arc<dyn TtsEngine>> = vec![flaky.clone()];
        let ref_path = dir.join("source/voice_ref.wav");
        let mut synthesis = PageSynthesis::new(&dir, &options, &ref_path, "none", &chain, vec![]);

        assert!(matches!(
            synthesis.synthesize(&mut project, 1, "retry me").unwrap(),
            PageOutcome::Generated(_)
        ));
        assert_eq!(flaky.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stale_artifact_is_reported_and_regenerated() {
        let (_data, mut project, dir) = project_at(Stage::Parsed);
        let options = options();
        let engine = Arc::new(FakeTtsEngine::new());
        let chain: Vec<Arc<dyn TtsEngine>> = vec![engine.clone()];
        let ref_path = dir.join("source/voice_ref.wav");

        {
            let mut synthesis =
                PageSynthesis::new(&dir, &options, &ref_path, "none", &chain, vec![]);
            assert!(matches!(
                synthesis.synthesize(&mut project, 1, "hello").unwrap(),
                PageOutcome::Generated(_)
            ));
        }
        let audio = dir.join("work/audio/slide_001.wav");
        std::fs::write(&audio, b"tampered").unwrap();

        let mut synthesis = PageSynthesis::new(&dir, &options, &ref_path, "none", &chain, vec![]);
        assert!(matches!(
            synthesis.synthesize(&mut project, 1, "hello").unwrap(),
            PageOutcome::Generated(_)
        ));
        assert!(
            synthesis
                .warnings()
                .iter()
                .any(|w| w.contains("产物缺失或不一致")),
            "stale artifact must be reported: {:?}",
            synthesis.warnings()
        );
        assert_eq!(engine.call_count(), 2);
    }

    #[test]
    fn exhausted_chain_reuses_a_recorded_artifact() {
        let (_data, mut project, dir) = project_at(Stage::Parsed);
        let options = options();
        let ok = Arc::new(FakeTtsEngine::new());
        let chain: Vec<Arc<dyn TtsEngine>> = vec![ok.clone()];
        let ref_path = dir.join("source/voice_ref.wav");

        {
            let mut first = PageSynthesis::new(&dir, &options, &ref_path, "none", &chain, vec![]);
            assert!(matches!(
                first.synthesize(&mut project, 2, "page two").unwrap(),
                PageOutcome::Generated(_)
            ));
        }

        let failing = Arc::new(FakeTtsEngine::new().fail_on_text_contains("fail"));
        let failing_chain: Vec<Arc<dyn TtsEngine>> = vec![failing.clone()];
        let mut second =
            PageSynthesis::new(&dir, &options, &ref_path, "none", &failing_chain, vec![]);
        assert!(matches!(
            second.synthesize(&mut project, 1, "fail this").unwrap(),
            PageOutcome::Failed(_)
        ));
        assert!(matches!(
            second.synthesize(&mut project, 2, "page two").unwrap(),
            PageOutcome::Skipped(_)
        ));
        assert_eq!(
            failing.call_count(),
            2,
            "page 2 must not re-probe the engine"
        );
    }

    #[test]
    fn resume_from_tts_running_keeps_the_stage_until_the_orchestrator_finishes() {
        let (_data, mut project, dir) = project_at(Stage::TtsRunning);
        let options = options();
        let engine = Arc::new(FakeTtsEngine::new());
        let chain: Vec<Arc<dyn TtsEngine>> = vec![engine];
        let ref_path = dir.join("source/voice_ref.wav");
        let mut synthesis = PageSynthesis::new(&dir, &options, &ref_path, "none", &chain, vec![]);

        assert!(matches!(
            synthesis.synthesize(&mut project, 1, "resume").unwrap(),
            PageOutcome::Generated(_)
        ));
        assert_eq!(project.stage, Stage::TtsRunning);
        assert!(synthesis.entered_running());
    }
}
