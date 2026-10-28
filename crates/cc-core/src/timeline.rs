//! Timeline stage: the one and only timeline formula (spec Implementation Decisions
//! 「时间轴唯一公式」) plus the stage runner that turns per-page audio into
//! `work/concat.txt` (image-track concat list, durations per formula) and
//! `work/audio/voice_full.wav` (pages joined by 0.6 s silence, 1.0 s end fade-out).
//!
//! Inputs follow ticket 04's convention: `work/audio/slide_NNN.wav` (1-based, 3-digit).
//! Page count N = contiguous slide wavs starting at 001. concat.txt references
//! `work/slides/slide_NNN.png` (ticket 03's output), which must exist — render (ticket 08)
//! consumes the list as-is.

use crate::error::{Error, Result};
use crate::ffmpeg;
use crate::project::Project;
use crate::stage;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Inter-page silence g (seconds). Picture durations and the audio track share the same g.
pub const GAP_SEC: f64 = 0.6;
/// End-of-track fade-out length (seconds).
pub const FADE_SEC: f64 = 1.0;

/// Per-page audio files (ticket 04) and the joined track.
pub const AUDIO_DIR: &str = "work/audio";
pub const VOICE_FULL: &str = "work/audio/voice_full.wav";
/// Slide images (ticket 03) and the image-track concat list consumed by render (ticket 08).
pub const SLIDE_DIR: &str = "work/slides";
pub const CONCAT_TXT: &str = "work/concat.txt";
/// Key of this stage's record in `project.json` `stages`.
pub const STAGE_KEY: &str = "timeline";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageTiming {
    pub page: u32,
    /// Narration length d_i, parsed from the wav header.
    pub audio_sec: f64,
    /// Picture duration: d_i + g for i < N, plain d_N for the last page.
    pub display_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimelinePlan {
    pub pages: Vec<PageTiming>,
    /// Time page i+1 appears on screen: cumulative d_j + g. Empty for a single page.
    pub turn_points_sec: Vec<f64>,
    /// T = Σd_i + (N-1)·g.
    pub total_sec: f64,
}

/// The one and only timeline formula.
pub fn plan(audio_sec: &[f64]) -> TimelinePlan {
    let n = audio_sec.len();
    let mut pages = Vec::with_capacity(n);
    let mut turn_points_sec = Vec::with_capacity(n.saturating_sub(1));
    let mut t = 0.0;
    for (i, d) in audio_sec.iter().enumerate() {
        let last = i + 1 == n;
        pages.push(PageTiming {
            page: (i + 1) as u32,
            audio_sec: *d,
            display_sec: if last { *d } else { d + GAP_SEC },
        });
        t += d;
        if !last {
            t += GAP_SEC;
            turn_points_sec.push(t);
        }
    }
    TimelinePlan {
        pages,
        turn_points_sec,
        total_sec: t,
    }
}

/// Timeline input fingerprint: all page audio durations + g + page count, hashed.
/// A page whose audio changes length invalidates the whole track downstream.
pub fn fingerprint(audio_sec: &[f64]) -> String {
    let durations: Vec<String> = audio_sec.iter().map(|d| format!("{d:.3}")).collect();
    let canonical = format!(
        "cc-timeline/v1\ngap_sec={GAP_SEC:.3}\npages={}\naudio_sec={}",
        audio_sec.len(),
        durations.join(",")
    );
    stage::sha256_hex(canonical.as_bytes())
}

#[derive(Debug, Clone, Serialize)]
pub struct TimelineSummary {
    pub pages: Vec<PageTiming>,
    pub turn_points_sec: Vec<f64>,
    pub total_sec: f64,
    pub gap_sec: f64,
    pub fade_out_sec: f64,
    pub fingerprint: String,
    /// True when the fingerprint + depends_on matched and the artifacts were intact:
    /// nothing re-ran (ticket 10 失效传播 / 断点续跑).
    pub skipped: bool,
    /// Paths relative to the project directory.
    pub concat_txt: String,
    pub voice_full: String,
}

/// Run the timeline stage: measure per-page audio, write concat.txt + voice_full.wav,
/// record the fingerprint in project.json. Caller must hold the project lock.
pub fn run_locked(project: &mut Project, dir: &Path) -> Result<TimelineSummary> {
    // 1. discover per-page audio: contiguous slide_NNN.wav from 001
    let mut durations = Vec::new();
    let mut page = 1u32;
    loop {
        let path = dir.join(AUDIO_DIR).join(format!("slide_{page:03}.wav"));
        if !path.is_file() {
            break;
        }
        durations.push(crate::wav::parse(&path)?.duration_sec());
        page += 1;
    }
    if durations.is_empty() {
        return Err(Error::AudioMissing {
            dir: dir.join(AUDIO_DIR).display().to_string(),
        });
    }
    let n = durations.len();

    // Every slide needs exactly one page of audio. Without this, a partial TTS run
    // (or a script with fewer pages than the deck) silently produced a shorter film:
    // the old code took the contiguous audio prefix and render happily encoded it.
    let slides = count_slide_images(dir);
    if slides < n as u32 {
        return Err(Error::SlideMissing {
            page: slides + 1,
            path: dir
                .join(SLIDE_DIR)
                .join(format!("slide_{:03}.png", slides + 1))
                .display()
                .to_string(),
        });
    }
    if slides > n as u32 {
        return Err(Error::PageCountMismatch {
            audio: n as u32,
            slides,
        });
    }

    // idempotent skip (ticket 10): the duration fingerprint matching is necessary but
    // not sufficient — depends_on must still agree with the current per-page tts
    // fingerprints, and the landed artifacts must be intact. On skip nothing re-runs,
    // not even the ffmpeg audio concat (断点续跑: 已完成阶段不重跑).
    let fp = fingerprint(&durations);
    let tts_keys: Vec<String> = (1..=n as u32).map(|p| format!("tts:{p}")).collect();
    let tts_refs: Vec<&str> = tts_keys.iter().map(String::as_str).collect();
    if stage::verify(
        project,
        dir,
        &stage::VerifySpec {
            key: STAGE_KEY,
            input_fingerprint: &fp,
            stage_deps: &tts_refs,
            content_deps: &[],
        },
    ) {
        let plan = plan(&durations);
        return Ok(TimelineSummary {
            pages: plan.pages,
            turn_points_sec: plan.turn_points_sec,
            total_sec: plan.total_sec,
            gap_sec: GAP_SEC,
            fade_out_sec: FADE_SEC,
            fingerprint: fp,
            skipped: true,
            concat_txt: CONCAT_TXT.to_string(),
            voice_full: VOICE_FULL.to_string(),
        });
    }

    // 2. plan + concat.txt (image track; render adds `-t` and the repeated-last-image line
    //    is written here so the list is self-contained)
    let plan = plan(&durations);
    let concat_text = build_concat_text(&plan);
    fs::write(dir.join(CONCAT_TXT), &concat_text)?;

    // 4. voice_full.wav: single ffmpeg call, pages joined by g silence, end fade
    let args = build_audio_args(dir, &plan);
    ffmpeg::run(&args)?;

    // 5. fingerprint + stage record into project.json. depends_on binds the track to the
    //    per-page tts fingerprints it was built from (PRD §6.6.1) — a page re-synthesized
    //    later mismatches here and invalidates the track (ticket 10 失效传播).
    let mut depends_on = BTreeMap::new();
    for p in 1..=n as u32 {
        if let Some(record) = project.stages.get(&format!("tts:{p}")) {
            depends_on.insert(format!("tts:{p}"), record.input_fingerprint.clone());
        }
    }
    let outputs = vec![
        stage::hash_artifact(dir, CONCAT_TXT)?,
        stage::hash_artifact(dir, VOICE_FULL)?,
    ];
    let record = stage::success(
        STAGE_KEY,
        fp.clone(),
        format!("cc-core/{}", env!("CARGO_PKG_VERSION")),
        outputs,
        depends_on,
    );
    stage::land(project, dir, STAGE_KEY, record)?;

    Ok(TimelineSummary {
        pages: plan.pages,
        turn_points_sec: plan.turn_points_sec,
        total_sec: plan.total_sec,
        gap_sec: GAP_SEC,
        fade_out_sec: FADE_SEC,
        fingerprint: fp,
        skipped: false,
        concat_txt: CONCAT_TXT.to_string(),
        voice_full: VOICE_FULL.to_string(),
    })
}

/// Image-track concat list: `file` + `duration` per page, last image repeated without a
/// duration (concat demuxer keeps the last frame until `-t` cuts the stream). File paths
/// are relative to concat.txt's own directory (`work/`), how the concat demuxer resolves
/// them.
fn build_concat_text(plan: &TimelinePlan) -> String {
    let n = plan.pages.len();
    let mut text = String::new();
    for pg in &plan.pages {
        text.push_str(&format!("file 'slides/slide_{:03}.png'\n", pg.page));
        text.push_str(&format!("duration {:.3}\n", pg.display_sec));
    }
    text.push_str(&format!("file 'slides/slide_{n:03}.png'\n"));
    text
}

/// ffmpeg argv for the joined audio track — argument array only, never a shell string
/// (spec MCP 权限边界). Inputs: page wavs interleaved with g-length silence, then the
/// concat audio filter (spec 合成: "concat 音频滤镜拼音轨") and the end fade.
fn build_audio_args(dir: &Path, plan: &TimelinePlan) -> Vec<String> {
    let n = plan.pages.len();
    let silence = "aevalsrc=exprs=0:channel_layout=mono:duration=0.600:sample_rate=24000";
    let mut args: Vec<String> = vec!["-y".into(), "-v".into(), "error".into()];
    for (i, pg) in plan.pages.iter().enumerate() {
        args.push("-i".into());
        args.push(
            dir.join(AUDIO_DIR)
                .join(format!("slide_{:03}.wav", pg.page))
                .display()
                .to_string(),
        );
        if i + 1 < n {
            args.push("-f".into());
            args.push("lavfi".into());
            args.push("-i".into());
            args.push(silence.into());
        }
    }
    let segments = 2 * n - 1;
    let labels: String = (0..segments).map(|i| format!("[{i}:a]")).collect();
    let mut filter = format!("{labels}concat=n={segments}:v=0:a=1[cat]");
    if plan.total_sec > FADE_SEC {
        let st = plan.total_sec - FADE_SEC;
        filter.push_str(&format!(
            ";[cat]afade=t=out:st={st:.3}:d={FADE_SEC:.3}[aout]"
        ));
    } else {
        filter.push_str(";[cat]anull[aout]"); // track shorter than the fade: nothing to fade
    }
    args.push("-filter_complex".into());
    args.push(filter);
    args.push("-map".into());
    args.push("[aout]".into());
    args.push("-c:a".into());
    args.push("pcm_s16le".into());
    args.push("-ar".into());
    args.push("24000".into());
    args.push("-ac".into());
    args.push("1".into());
    args.push(dir.join(VOICE_FULL).display().to_string());
    args
}

/// Slide images in `work/slides` numbered contiguously from 001 (the slide_render
/// contract); a gap ends the count so a missing middle page surfaces as a mismatch.
fn count_slide_images(dir: &Path) -> u32 {
    let mut n = 0u32;
    loop {
        let path = dir.join(SLIDE_DIR).join(format!("slide_{:03}.png", n + 1));
        if !path.is_file() {
            return n;
        }
        n += 1;
    }
}
