//! ffmpeg invocation. Spec MCP 权限边界: "ffmpeg 一律参数数组调用，绝不拼 shell 字符串" —
//! `Command::new("ffmpeg").args(...)` never goes through a shell.

use crate::error::{Error, Result};
use std::path::Path;
use std::process::Command;

/// The ffmpeg binary to spawn: `CC_FFMPEG` (absolute path) overrides the PATH
/// lookup — packaged builds pin their bundled minimal build this way, dev
/// checkouts keep plain `ffmpeg` (same pattern as CC_RVM_MODEL / CC_PDFIUM_DLL).
pub(crate) fn ffmpeg_exe() -> std::ffi::OsString {
    std::env::var_os("CC_FFMPEG")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "ffmpeg".into())
}

/// Transcode any audio file into the pipeline wav format: 24kHz / mono / pcm_s16le
/// (spec "TTS 主引擎" — 与输出采样率一致，音轨拼接无需重采样). Shared by the
/// reference-audio ingest (ticket 05) and the SAPI output normalization (ticket 06).
/// Argument array only, never a shell string (spec 权限边界).
pub fn normalize_to_tts_wav(src: &Path, dest: &Path) -> Result<()> {
    let args: Vec<String> = [
        "-y",
        "-i",
        src.to_str().expect("input paths are valid unicode"),
        "-vn",
        "-ac",
        "1",
        "-ar",
        "24000",
        "-acodec",
        "pcm_s16le",
        "-f",
        "wav",
        dest.to_str().expect("output paths are valid unicode"),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    run(&args)
}

/// Run `ffmpeg` with the given argument array. On failure the error carries the tail of
/// stderr; on success stderr is discarded.
pub fn run(args: &[String]) -> Result<()> {
    let out = Command::new(ffmpeg_exe())
        .args(args)
        .output()
        .map_err(|e| Error::Ffmpeg {
            stderr: format!("failed to spawn ffmpeg: {e}"),
        })?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(10).collect();
        let tail: String = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(Error::Ffmpeg { stderr: tail });
    }
    Ok(())
}

/// ffmpeg version token parsed from the first line of `ffmpeg -version`, e.g.
/// `8.1.1-full_build-www.gyan.dev`. Part of the render input fingerprint
/// (PRD §6.6.1: render 指纹含 ffmpeg 版本).
pub fn version() -> Result<String> {
    let out = Command::new(ffmpeg_exe())
        .arg("-version")
        .output()
        .map_err(|e| Error::Ffmpeg {
            stderr: format!("failed to spawn ffmpeg: {e}"),
        })?;
    if !out.status.success() {
        return Err(Error::Ffmpeg {
            stderr: "ffmpeg -version failed".into(),
        });
    }
    let first = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let tokens: Vec<&str> = first.split_whitespace().collect();
    if tokens.len() > 2 && tokens[0] == "ffmpeg" && tokens[1] == "version" {
        Ok(tokens[2].to_string())
    } else {
        Ok(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CC_FFMPEG pins the spawned binary: a nonexistent path surfaces the path in
    /// the spawn error — packaged builds rely on this seam (ticket 14).
    #[test]
    fn cc_ffmpeg_env_pins_the_binary() {
        std::env::set_var("CC_FFMPEG", "Z:/definitely/not/ffmpeg.exe");
        let err = run(&["-version".to_string()]).unwrap_err();
        std::env::remove_var("CC_FFMPEG");
        assert!(err.to_string().contains("failed to spawn"), "{err}"); // 到达 Err 即证明缝生效：PATH 里的真 ffmpeg 会返回 Ok
    }
}
