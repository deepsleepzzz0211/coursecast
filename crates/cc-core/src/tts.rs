//! TTS engine seam + per-page synthesis orchestration (ticket 04).
//!
//! The engine trait is the seam for the real engines (MiMo/SAPI/edge, tickets 05-07);
//! tests inject the deterministic fake engine so the pipeline runs offline without keys
//! (spec Testing Decisions, 缝 2). The request layer adds per-page timeout + retry on
//! top of any engine; idempotency lives at the orchestration layer via per-page
//! fingerprints recorded in project.json (spec "输入指纹模型" + "幂等性诚实边界").

mod page;

use self::page::{PageOutcome, PageSynthesis};
use crate::edge::EdgeVoice;
use crate::error::{Error, Result};
use crate::project::FallbackEvent;
use crate::stage;
use crate::state::Stage;
use crate::wav;
use crate::{load_project, save_project, ProjectLock};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One page's synthesis request. `style` is the free-form delivery instruction (the
/// user-role message for MiMo, may be empty); `rate` is the speech-rate multiplier
/// (1.0 = normal); `reference_audio` points at the in-project reference wav the engine
/// may require (voiceclone engines do).
#[derive(Debug, Clone)]
pub struct TtsRequest {
    pub text: String,
    pub style: Option<String>,
    pub rate: f32,
    pub reference_audio: Option<PathBuf>,
}

/// Engine seam: `FakeTtsEngine` for tests, MiMo/SAPI/edge (tickets 05-07). Output
/// contract for all engines: 24kHz mono pcm_s16le wav bytes. `Send + Sync` so the request
/// layer can run one synthesis per thread and abandon it on timeout.
pub trait TtsEngine: Send + Sync {
    /// Engine identifier used in fingerprints and tool_version, e.g. "fake", "mimo".
    fn name(&self) -> &'static str;
    /// Model identifier, e.g. "fake-sine-24k", "mimo-v2.5-tts-voiceclone".
    fn model(&self) -> &'static str;
    /// Synthesize one page. Must not touch the caller's artifact paths; the caller owns
    /// artifact storage so failed attempts never leave partial files behind (the SAPI
    /// engine's own private temp file is the one sanctioned exception).
    fn synthesize(&self, request: &TtsRequest) -> Result<Vec<u8>>;
}

/// Deterministic offline engine: sine wav whose duration scales with text length and
/// 1/rate. Same request always produces identical bytes. Counts synthesize() calls so
/// tests can assert idempotency ("不重复调用引擎"); `slow_first_attempts` simulates
/// requests that exceed the per-page timeout.
#[derive(Debug)]
pub struct FakeTtsEngine {
    calls: std::sync::atomic::AtomicU32,
    slow_remaining: std::sync::atomic::AtomicU32,
    slow_delay: std::time::Duration,
    fail_marker: Option<String>,
}

impl FakeTtsEngine {
    pub fn new() -> Self {
        FakeTtsEngine {
            calls: std::sync::atomic::AtomicU32::new(0),
            slow_remaining: std::sync::atomic::AtomicU32::new(0),
            slow_delay: std::time::Duration::ZERO,
            fail_marker: None,
        }
    }

    /// The first `count` synthesize() calls sleep `delay` before answering (the request
    /// layer's timeout abandons them, like a real HTTP client would).
    pub fn slow_first_attempts(self, count: u32, delay: std::time::Duration) -> Self {
        FakeTtsEngine {
            slow_remaining: std::sync::atomic::AtomicU32::new(count),
            slow_delay: delay,
            ..self
        }
    }

    /// Requests whose text contains `marker` always fail — simulates a permanently
    /// broken page (e.g. text the real engine rejects).
    pub fn fail_on_text_contains(self, marker: &str) -> Self {
        FakeTtsEngine {
            fail_marker: Some(marker.to_string()),
            ..self
        }
    }

    /// Number of synthesize() invocations so far.
    pub fn call_count(&self) -> u32 {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Default for FakeTtsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TtsEngine for FakeTtsEngine {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn model(&self) -> &'static str {
        "fake-sine-24k"
    }

    fn synthesize(&self, request: &TtsRequest) -> Result<Vec<u8>> {
        use std::sync::atomic::Ordering;
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self
            .slow_remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            std::thread::sleep(self.slow_delay);
        }
        if let Some(marker) = &self.fail_marker {
            if request.text.contains(marker.as_str()) {
                return Err(Error::EngineFailure {
                    engine: "fake".into(),
                    message: format!("injected failure (text contains {marker:?})"),
                });
            }
        }
        let chars = request.text.chars().count() as f64;
        let duration = (0.2 + 0.01 * chars) / request.rate.max(0.1) as f64;
        // A voiceclone engine speaks with the reference voice, so a swapped reference
        // changes the audio CONTENT while the duration stays the same (ticket 10's
        // depends_on assertions ride on exactly that). The double folds the reference
        // bytes into the tone; reading the input file mirrors the real mimo engine.
        let ref_tone: u32 = match &request.reference_audio {
            Some(path) => std::fs::read(path)
                .map(|bytes| {
                    bytes
                        .iter()
                        .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(*b as u32))
                })
                .unwrap_or(0),
            None => 0,
        };
        let freq = 440.0 + (ref_tone % 160) as f64;
        let n = (wav::TTS_SAMPLE_RATE as f64 * duration).round() as usize;
        let samples: Vec<i16> = (0..n)
            .map(|i| {
                let t = i as f64 / wav::TTS_SAMPLE_RATE as f64;
                let v = 0.3 * (2.0 * std::f64::consts::PI * freq * t).sin();
                (v * i16::MAX as f64) as i16
            })
            .collect();
        Ok(wav::encode_mono_pcm16(wav::TTS_SAMPLE_RATE, &samples))
    }
}

/// One page's landed artifact, as reported by the shells.
#[derive(Debug, Clone, Serialize)]
pub struct PageArtifact {
    pub page: u32,
    /// Path relative to the project directory.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub duration_sec: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageFailure {
    pub page: u32,
    pub error: String,
}

/// Per-page synthesis report (spec story 26: structured JSON with artifact paths).
#[derive(Debug, Clone, Serialize)]
pub struct SynthesisReport {
    pub generated: Vec<u32>,
    pub skipped: Vec<u32>,
    pub failed_pages: Vec<PageFailure>,
    pub artifacts: Vec<PageArtifact>,
    pub total_duration_sec: f64,
    pub warnings: Vec<String>,
    /// Fallback-chain degradation events of this run (ticket 06): construction-time
    /// slides plus runtime slides, each with a user-readable reason (提示条数据源).
    pub fallbacks: Vec<FallbackEvent>,
    pub next: String,
    /// Pipeline position after this run.
    pub stage: Stage,
}

/// Parse a `--pages` spec (`3`, `1-5`, `1-3,7`) into 1-based page numbers, validated
/// against `page_count`. Sorted + deduplicated.
pub fn parse_pages(spec: &str, page_count: u32) -> Result<Vec<u32>> {
    let invalid = || Error::InvalidPagesSpec(spec.to_string());
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if let Some((a, b)) = part.split_once('-') {
            let (a, b) = (a.trim(), b.trim());
            if a.is_empty() || b.is_empty() {
                return Err(invalid());
            }
            let (a, b): (u32, u32) = match (a.parse(), b.parse()) {
                (Ok(a), Ok(b)) => (a, b),
                _ => return Err(invalid()),
            };
            if a == 0 || b < a {
                return Err(invalid());
            }
            if b > page_count {
                return Err(Error::PageOutOfRange {
                    page: b,
                    pages: page_count,
                });
            }
            out.extend(a..=b);
        } else {
            let n: u32 = part.parse().map_err(|_| invalid())?;
            if n == 0 {
                return Err(invalid());
            }
            if n > page_count {
                return Err(Error::PageOutOfRange {
                    page: n,
                    pages: page_count,
                });
            }
            out.push(n);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

const NEXT_TTS_DONE: &str = "提供出镜视频 source/person.mp4，随后运行 person_matte";

/// Request-layer policy (spec "TTS 主引擎": 每页请求带超时 120 s + 重试).
#[derive(Debug, Clone)]
pub struct SynthesisOptions {
    /// Timeout for one engine call (one page).
    pub page_timeout: std::time::Duration,
    /// Total attempts per page, including the first.
    pub max_attempts: u32,
}

impl Default for SynthesisOptions {
    fn default() -> Self {
        SynthesisOptions {
            page_timeout: std::time::Duration::from_secs(120),
            max_attempts: 3,
        }
    }
}

/// The TTS engine choices every shell exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TtsEngineKind {
    /// Offline deterministic engine (tests and the default AI-tooling path).
    Fake,
    /// MiMo voice clone, head of the real fallback chain.
    Mimo,
    /// Edge read-aloud, middle level of the real fallback chain.
    Edge,
    /// Windows SAPI, the offline bottom level.
    Sapi,
}

impl TtsEngineKind {
    /// Every engine a shell may request, in help-text order.
    pub const ALL: [TtsEngineKind; 4] = [
        TtsEngineKind::Fake,
        TtsEngineKind::Mimo,
        TtsEngineKind::Edge,
        TtsEngineKind::Sapi,
    ];

    /// Stable identifier used on the command line, in JSON and in error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            TtsEngineKind::Fake => "fake",
            TtsEngineKind::Mimo => "mimo",
            TtsEngineKind::Edge => "edge",
            TtsEngineKind::Sapi => "sapi",
        }
    }
}

impl std::str::FromStr for TtsEngineKind {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self> {
        match name {
            "fake" => Ok(TtsEngineKind::Fake),
            "mimo" => Ok(TtsEngineKind::Mimo),
            "edge" => Ok(TtsEngineKind::Edge),
            "sapi" => Ok(TtsEngineKind::Sapi),
            other => Err(Error::InvalidArgument(format!(
                "unknown tts engine `{other}`: expected `fake`, `mimo`, `edge` or `sapi`"
            ))),
        }
    }
}

impl std::fmt::Display for TtsEngineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The TTS fallback chain (ticket 06 三级降级链): engines in preference order plus the
/// degradation events already observed while building it. Every page tries the engines
/// in order; an engine that cannot even be constructed (no stored MiMo key) is simply
/// absent from `engines` and shows up in `events`.
pub struct FallbackChain {
    /// Engines in preference order; index 0 is tried first for every page.
    pub engines: Vec<Arc<dyn TtsEngine>>,
    /// Construction-time degradation events (e.g. no API key → the next level takes over).
    pub events: Vec<FallbackEvent>,
}

impl std::fmt::Debug for FallbackChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackChain")
            .field(
                "engines",
                &self.engines.iter().map(|e| e.name()).collect::<Vec<_>>(),
            )
            .field("events", &self.events)
            .finish()
    }
}

impl FallbackChain {
    /// A chain with exactly one engine: no fallback possible, behavior identical to the
    /// pre-chain orchestration (this is what `--engine fake` and engine-injecting tests use).
    pub fn single(engine: Arc<dyn TtsEngine>) -> Self {
        FallbackChain {
            engines: vec![engine],
            events: Vec::new(),
        }
    }
}

const REAL_REGISTRY: [TtsEngineKind; 3] = [
    TtsEngineKind::Mimo,
    TtsEngineKind::Edge,
    TtsEngineKind::Sapi,
];

fn build_engine(kind: TtsEngineKind, edge_voice: Option<EdgeVoice>) -> Result<Arc<dyn TtsEngine>> {
    match kind {
        TtsEngineKind::Mimo => Ok(Arc::new(crate::MimoEngine::from_credential_store()?)),
        TtsEngineKind::Edge => Ok(Arc::new(match edge_voice {
            Some(voice) => crate::EdgeEngine::with_voice(voice),
            None => crate::EdgeEngine::from_env()?,
        })),
        TtsEngineKind::Sapi => Ok(Arc::new(crate::SapiEngine::new())),
        TtsEngineKind::Fake => Ok(Arc::new(FakeTtsEngine::new())),
    }
}

/// Build the fallback chain for `preferred` with the default Edge voice.
pub fn build_fallback_chain(preferred: TtsEngineKind) -> Result<FallbackChain> {
    build_fallback_chain_with_voice(preferred, None)
}

/// Build the fallback chain for `preferred`: the preferred engine first, then every later
/// registry engine in order. `Fake` yields its single offline engine; `edge_voice`
/// overrides the Edge level (project preference) and defaults to `CC_EDGE_VOICE`/Xiaoxiao.
/// An engine that cannot be constructed in this environment (no stored MiMo key) records
/// a degradation event and is skipped, so a resolvable preference always ends at SAPI.
///
/// `runtime::tts_chain` is the shell-facing entry point: it also honors the
/// `CC_TTS_ENGINE=fake` test seam and the fake delay knob.
pub fn build_fallback_chain_with_voice(
    preferred: TtsEngineKind,
    edge_voice: Option<EdgeVoice>,
) -> Result<FallbackChain> {
    if preferred == TtsEngineKind::Fake {
        return Ok(FallbackChain::single(Arc::new(FakeTtsEngine::new())));
    }
    let start = REAL_REGISTRY
        .iter()
        .position(|kind| *kind == preferred)
        .expect("every non-fake TtsEngineKind is in REAL_REGISTRY");
    let mut engines = Vec::new();
    let mut events = Vec::new();
    for (i, kind) in REAL_REGISTRY.iter().enumerate().skip(start) {
        match build_engine(*kind, edge_voice) {
            Ok(engine) => engines.push(engine),
            Err(err) => match REAL_REGISTRY.get(i + 1) {
                Some(next) => events.push(FallbackEvent {
                    from: kind.as_str().to_string(),
                    to: next.as_str().to_string(),
                    reason: err.to_string(),
                }),
                // the last level itself is unavailable: nothing can take over
                None => return Err(err),
            },
        }
    }
    Ok(FallbackChain { engines, events })
}

/// Convenience wrapper for tests and standalone callers: acquires the project lock and
/// delegates to `synthesize_pages_locked`. Commands should acquire the lock themselves
/// and call the runner.
pub fn synthesize_pages(
    data_dir: &Path,
    project_id: &str,
    chain: &FallbackChain,
    pages: Option<&str>,
    options: &SynthesisOptions,
) -> Result<SynthesisReport> {
    let (_, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    synthesize_pages_locked(data_dir, project_id, chain, pages, options)
}

/// `voice_synthesize` stage runner (ticket 04): synthesize every requested page, land
/// `work/audio/slide_NNN.wav`, record per-page fingerprints, and move the project
/// parsed → tts_running → tts_done. Caller must hold the project lock.
///
/// Fallback (ticket 06): each page walks `chain` in preference order. An engine that
/// fails after its per-page retry budget is removed for the rest of the run (re-probing
/// it on every later page would multiply the latency for no gain) and the next level
/// takes over; every slide records a `FallbackEvent` into project.json and the report.
pub fn synthesize_pages_locked(
    data_dir: &Path,
    project_id: &str,
    chain: &FallbackChain,
    pages: Option<&str>,
    options: &SynthesisOptions,
) -> Result<SynthesisReport> {
    let (mut project, dir) = load_project(data_dir, project_id)?;

    let script_pages = crate::script::read_script_pages(&dir)?;
    if script_pages.is_empty() {
        return Err(Error::ScriptEmpty {
            path: dir.join(crate::script::SCRIPT_FILE).display().to_string(),
        });
    }
    let page_count = script_pages.len() as u32;
    let scope = match pages {
        Some(spec) => parse_pages(spec, page_count)?,
        None => (1..=page_count).collect(),
    };

    // Reference audio hash feeds every page fingerprint; absent ref -> "none" so the
    // fingerprint still changes the day one appears.
    let ref_path = dir.join("source").join("voice_ref.wav");
    let ref_hash = if ref_path.is_file() {
        stage::sha256_hex(&std::fs::read(&ref_path)?)
    } else {
        "none".to_string()
    };

    let mut generated = Vec::new();
    let mut skipped = Vec::new();
    let mut failed_pages = Vec::new();
    let mut artifacts = Vec::new();
    // A kill -9 mid-run leaves stage at tts_running with the completed pages' records
    // already saved. Re-entering at tts_running IS the resume (断点续跑): no transition
    // needed, the finished pages skip below and the rest continue.
    let resuming = project.stage == Stage::TtsRunning;
    let mut page_synthesis = PageSynthesis::new(
        &dir,
        options,
        &ref_path,
        &ref_hash,
        &chain.engines,
        chain.events.clone(),
    );

    for page in scope {
        let text = &script_pages[(page - 1) as usize];
        match page_synthesis.synthesize(&mut project, page, text)? {
            PageOutcome::Generated(artifact) => {
                generated.push(page);
                artifacts.push(artifact);
            }
            PageOutcome::Skipped(artifact) => {
                skipped.push(page);
                artifacts.push(artifact);
            }
            PageOutcome::Failed(failure) => failed_pages.push(failure),
            PageOutcome::Fatal(err) => return Err(err),
        }
    }

    let entered_running = page_synthesis.entered_running();
    let fallbacks = page_synthesis.fallbacks().to_vec();
    let warnings = page_synthesis.warnings().to_vec();

    if entered_running
        || (resuming && (1..=page_count).all(|p| project.stages.contains_key(&format!("tts:{p}"))))
    {
        // fresh work, or a kill -9 zombie whose every page is now recorded: the stage
        // completes. A partially-resumed scope stays at tts_running for the next entry.
        project.fallbacks = fallbacks.clone();
        if failed_pages.is_empty() {
            project.transition(Stage::TtsDone)?;
        } else {
            let summary = failed_pages
                .iter()
                .map(|f| format!("第{}页 {}", f.page, f.error))
                .collect::<Vec<_>>()
                .join("; ");
            project.fail(format!("{} 页合成失败: {}", failed_pages.len(), summary))?;
        }
        save_project(&project, &dir)?;
    }

    let total_duration_sec = artifacts.iter().map(|a| a.duration_sec).sum();
    let next = if !failed_pages.is_empty() {
        format!(
            "{} 页合成失败：修复后重跑 voice_synthesize，已完成页会自动跳过",
            failed_pages.len()
        )
    } else if !generated.is_empty() && project.stages.contains_key(crate::timeline::STAGE_KEY) {
        // invalidation: the track was already built once — its depends_on now points at
        // the old page fingerprints, so the rebuild is the next step (ticket 10)
        "音频已更新：运行 timeline 重建整轨，然后 render 重渲成片".to_string()
    } else {
        NEXT_TTS_DONE.to_string()
    };
    Ok(SynthesisReport {
        generated,
        skipped,
        failed_pages,
        artifacts,
        total_duration_sec,
        warnings,
        fallbacks,
        next,
        stage: project.stage,
    })
}
