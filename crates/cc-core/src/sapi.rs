//! Windows SAPI TTS engine (ticket 06): the offline bottom of the fallback chain
//! (PRD §8.3 — edge-tts 已有被阻断的报告，SAPI 是真正底线，不依赖网络). PowerShell
//! System.Speech synthesizes to a temp wav; when the output does not match the 24kHz
//! mono pcm_s16le pipeline format (SAPI writes 16kHz by default), it is normalized with
//! the shared ffmpeg ingest path (ticket 05).
//!
//! Injection discipline (spec 权限边界, extended to powershell): the process is spawned
//! as an argument array — `powershell -NoProfile -NonInteractive -EncodedCommand <b64>`
//! — never a shell string. The script travels as UTF-16LE base64 and the page text is
//! embedded as a single-quoted PowerShell literal (the only escape is `'` → `''`), so no
//! text content can break out or execute.
//!
//! The `TtsEngine` contract says engines must not touch the filesystem; SAPI is the one
//! sanctioned exception: it writes a private temp file (never the caller's artifact
//! paths) and removes it before returning, so a failed attempt still leaves nothing
//! behind.

use crate::error::{Error, Result};
use crate::tts::{TtsEngine, TtsRequest};
use crate::{ffmpeg, wav};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct SapiEngine;

impl SapiEngine {
    pub fn new() -> Self {
        SapiEngine
    }
}

impl Default for SapiEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn engine_failure(message: impl Into<String>) -> Error {
    Error::EngineFailure {
        engine: "sapi".into(),
        message: message.into(),
    }
}

/// Private temp wav file for one synthesis attempt.
fn temp_wav_path() -> PathBuf {
    std::env::temp_dir().join(format!("cc-sapi-{}.wav", uuid::Uuid::new_v4().simple()))
}

/// Embed `s` as a PowerShell single-quoted literal: the only escape inside single quotes
/// is the quote itself, doubled. `$`, backticks and semicolons stay literal.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `-EncodedCommand` payload: the whole script as UTF-16LE base64 (no codepage issues
/// with Chinese text, no way to smuggle a second argument).
fn encoded_command(script: &str) -> String {
    let mut utf16 = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    BASE64.encode(utf16)
}

/// The pipeline format every engine must emit.
fn is_pipeline_format(info: wav::WavInfo) -> bool {
    (info.sample_rate, info.channels, info.bits_per_sample) == (wav::TTS_SAMPLE_RATE, 1, 16)
}

impl SapiEngine {
    /// Speak `text` into `out` via System.Speech with the default installed voice.
    fn speak_to_wav(&self, text: &str, out: &Path) -> Result<()> {
        let script = format!(
            "Add-Type -AssemblyName System.Speech\n$s = New-Object System.Speech.Synthesis.SpeechSynthesizer\n$s.SetOutputToWaveFile({})\n$s.Speak({})\n$s.Dispose()\n",
            ps_quote(&out.display().to_string()),
            ps_quote(text),
        );
        let spawned = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &encoded_command(&script),
            ])
            .output()
            .map_err(|e| engine_failure(format!("failed to spawn powershell: {e}")))?;
        if !spawned.status.success() {
            let stderr = String::from_utf8_lossy(&spawned.stderr);
            let tail: String = stderr
                .lines()
                .rev()
                .take(10)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            return Err(engine_failure(format!("powershell failed: {tail}")));
        }
        Ok(())
    }

    fn synthesize_inner(&self, request: &TtsRequest, raw: &Path) -> Result<Vec<u8>> {
        self.speak_to_wav(&request.text, raw)?;
        let bytes =
            std::fs::read(raw).map_err(|e| engine_failure(format!("SAPI produced no wav: {e}")))?;
        if is_pipeline_format(wav::parse_wav(&bytes)?) {
            return Ok(bytes);
        }
        // SAPI writes its default format (16kHz mono on current Windows): normalize into
        // the pipeline format so the voice-track concat needs no resampling later.
        let norm = temp_wav_path();
        let normalized = ffmpeg::normalize_to_tts_wav(raw, &norm).and_then(|_| {
            let bytes = std::fs::read(&norm)
                .map_err(|e| engine_failure(format!("normalized wav missing: {e}")))?;
            let info = wav::parse_wav(&bytes)?;
            if !is_pipeline_format(info) {
                // defensive: ffmpeg was told exactly this; a mismatch means a broken build
                return Err(engine_failure(format!(
                    "ffmpeg produced {} Hz / {} ch / {} bit, expected 24000 Hz / 1 ch / 16 bit",
                    info.sample_rate, info.channels, info.bits_per_sample
                )));
            }
            Ok(bytes)
        });
        let _ = std::fs::remove_file(&norm);
        normalized
    }
}

impl TtsEngine for SapiEngine {
    fn name(&self) -> &'static str {
        "sapi"
    }

    fn model(&self) -> &'static str {
        "system-speech"
    }

    fn synthesize(&self, request: &TtsRequest) -> Result<Vec<u8>> {
        let raw = temp_wav_path();
        let result = self.synthesize_inner(request, &raw);
        let _ = std::fs::remove_file(&raw); // best effort, also on failure paths
        result
    }
}
