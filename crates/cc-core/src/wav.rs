//! Wav header parsing + minimal RIFF/WAVE codec (spec 分发: "不打包 ffprobe，时长从
//! wav 头解析"). Reads RIFF/WAVE chunk structure to find `fmt ` and `data`.

use crate::error::{Error, Result};
use std::fs;
use std::path::Path;

/// Sample rate every TTS engine must emit (spec "TTS 主引擎").
pub const TTS_SAMPLE_RATE: u32 = 24_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub data_bytes: u64,
    /// data_size / byte_rate, i.e. what the container claims.
    pub duration_sec: f64,
}

impl WavInfo {
    /// Audio duration in seconds, from the header alone.
    pub fn duration_sec(&self) -> f64 {
        self.data_bytes as f64 / self.byte_rate() as f64
    }

    fn byte_rate(&self) -> u64 {
        self.sample_rate as u64 * self.channels as u64 * (self.bits_per_sample as u64 / 8)
    }
}

/// Encode mono 16-bit PCM samples as a wav container.
pub fn encode_mono_pcm16(sample_rate: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * 2; // mono, 16-bit
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Parse a wav file's header. Errors carry code `wav_invalid`.
pub fn parse(path: &Path) -> Result<WavInfo> {
    let bytes = fs::read(path)?;
    parse_bytes(&bytes, &path.display().to_string())
}

fn parse_bytes(bytes: &[u8], path: &str) -> Result<WavInfo> {
    let bad = |message: String| Error::WavInvalid {
        path: path.to_string(),
        message,
    };
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file".into()));
    }

    let mut fmt: Option<(u32, u16, u16)> = None; // (sample_rate, channels, bits)
    let mut data_bytes: Option<u64> = None;
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            if size < 16 || body + 16 > bytes.len() {
                return Err(bad("fmt chunk too short".into()));
            }
            let channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
            let bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().unwrap());
            fmt = Some((sample_rate, channels, bits));
        } else if id == b"data" {
            if body + size > bytes.len() {
                return Err(bad(format!(
                    "data chunk truncated: header claims {size} bytes, file has {}",
                    bytes.len() - body
                )));
            }
            data_bytes = Some(size as u64);
            break;
        }
        pos = body + size + (size & 1); // chunks are word-aligned
    }

    let (sample_rate, channels, bits) = fmt.ok_or_else(|| bad("missing fmt chunk".into()))?;
    let data_bytes = data_bytes.ok_or_else(|| bad("missing data chunk".into()))?;
    if sample_rate == 0 || channels == 0 || bits == 0 || bits % 8 != 0 {
        return Err(bad(format!(
            "unsupported format: {sample_rate} Hz, {channels} ch, {bits} bit"
        )));
    }
    let byte_rate = sample_rate as u64 * channels as u64 * (bits as u64 / 8);
    Ok(WavInfo {
        sample_rate,
        channels,
        bits_per_sample: bits,
        data_bytes,
        duration_sec: data_bytes as f64 / byte_rate as f64,
    })
}

/// Parse a wav container: walk the RIFF chunks, read fmt + data.
pub fn parse_wav(bytes: &[u8]) -> Result<WavInfo> {
    let malformed = || Error::InvalidWav("not a RIFF/WAVE container".into());
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(malformed());
    }
    // (format, channels, sample_rate, byte_rate, bits)
    let mut fmt: Option<(u16, u16, u32, u32, u16)> = None;
    let mut data_size: Option<u32> = None;
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let id = &bytes[i..i + 4];
        let size = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap());
        let body_start = i + 8;
        let body_end = body_start + size as usize;
        if body_end > bytes.len() {
            return Err(malformed());
        }
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err(malformed());
                }
                let b = &bytes[body_start..body_start + 16];
                fmt = Some((
                    u16::from_le_bytes([b[0], b[1]]),
                    u16::from_le_bytes([b[2], b[3]]),
                    u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
                    u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
                    u16::from_le_bytes([b[14], b[15]]),
                ));
            }
            b"data" => data_size = Some(size),
            _ => {}
        }
        i = body_end + (size as usize % 2); // chunks are word-aligned
    }
    let (format, channels, sample_rate, byte_rate, bits) = fmt.ok_or_else(malformed)?;
    let data_size = data_size.ok_or_else(malformed)?;
    if format != 1 {
        return Err(Error::InvalidWav(format!(
            "unsupported audio format {format}, expected PCM (1)"
        )));
    }
    if channels == 0 || sample_rate == 0 || byte_rate == 0 || bits == 0 {
        return Err(malformed());
    }
    Ok(WavInfo {
        sample_rate,
        channels,
        bits_per_sample: bits,
        data_bytes: data_size as u64,
        duration_sec: data_size as f64 / byte_rate as f64,
    })
}
