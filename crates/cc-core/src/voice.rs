//! Voice reference provisioning (ticket 05, `voice_configure`): normalize any incoming
//! reference recording (m4a/mp3/wav) to the入库 format every voiceclone request uses —
//! 24kHz / mono / pcm_s16le wav at `source/voice_ref.wav` (spec "TTS 主引擎", PRD §8.2:
//! 与输出采样率一致，音轨拼接无需重采样). ffmpeg is invoked with an argument array only
//! (spec 权限边界). API key storage lives in `credentials`; this module is audio-only.

use crate::error::{Error, Result};
use crate::project::load_project;
use crate::wav;
use crate::ProjectLock;
use serde::Serialize;
use std::path::Path;

/// The landed reference audio, as reported by the shells.
#[derive(Debug, Clone, Serialize)]
pub struct RefAudioInfo {
    /// Path relative to the project directory.
    pub path: String,
    pub bytes: u64,
    pub duration_sec: f64,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Transcode `src` into the project's `source/voice_ref.wav` (atomic rename; a failed
/// transcode never leaves a partial file). Always re-encodes — even an input that is
/// already a wav is normalized to the 24kHz mono format.
pub fn configure_reference_audio(
    data_dir: &Path,
    project_id: &str,
    src: &Path,
) -> Result<RefAudioInfo> {
    let (_project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;

    if !src.is_file() {
        return Err(Error::SourceNotFound(src.display().to_string()));
    }
    let dest = dir.join("source").join("voice_ref.wav");
    std::fs::create_dir_all(dir.join("source"))?;
    let tmp = dest.with_extension("wav.tmp");

    crate::ffmpeg::normalize_to_tts_wav(src, &tmp)?;

    let bytes = std::fs::read(&tmp)?;
    let info = wav::parse_wav(&bytes)?;
    if info.sample_rate != wav::TTS_SAMPLE_RATE || info.channels != 1 || info.bits_per_sample != 16
    {
        // defensive: ffmpeg was told exactly this; a mismatch means a broken build
        std::fs::remove_file(&tmp).ok();
        return Err(Error::InvalidWav(format!(
            "ffmpeg produced {} Hz / {} ch / {} bit, expected 24000 Hz / 1 ch / 16 bit",
            info.sample_rate, info.channels, info.bits_per_sample
        )));
    }
    std::fs::rename(&tmp, &dest)?;

    Ok(RefAudioInfo {
        path: "source/voice_ref.wav".to_string(),
        bytes: bytes.len() as u64,
        duration_sec: info.duration_sec,
        sample_rate: info.sample_rate,
        channels: info.channels,
    })
}
