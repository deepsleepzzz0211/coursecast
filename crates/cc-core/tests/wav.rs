//! Seam: `cc_core::wav::parse` — duration must come from the wav header alone
//! (spec 分发: "不打包 ffprobe，时长从 wav 头解析").

use cc_core::wav;
use std::fs;
use std::path::Path;

/// Canonical 44-byte-header PCM s16le mono wav.
fn write_wav(path: &Path, sample_rate: u32, samples: &[i16]) {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + samples.len() * 2);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    b.extend_from_slice(&2u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    fs::write(path, b).unwrap();
}

#[test]
fn parses_duration_sample_rate_and_channels_from_header() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.wav");
    // 3.0 s at 24 kHz mono s16 = 72000 samples (timeline spike's first page)
    write_wav(&path, 24000, &[0i16; 72000]);

    let info = wav::parse(&path).unwrap();
    assert_eq!(info.sample_rate, 24000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.bits_per_sample, 16);
    assert_eq!(info.data_bytes, 144000);
    assert!((info.duration_sec() - 3.0).abs() < 1e-9);
}

#[test]
fn duration_reflects_actual_header_values_not_assumptions() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("b.wav");
    write_wav(&path, 8000, &[0i16; 8000]); // 1.0 s at 8 kHz
    let info = wav::parse(&path).unwrap();
    assert!((info.duration_sec() - 1.0).abs() < 1e-9);
}

#[test]
fn walks_metadata_chunks_before_data() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("c.wav");
    // LIST/INFO chunk placed between fmt and data (ffmpeg writes these sometimes)
    let mut b: Vec<u8> = Vec::new();
    b.extend_from_slice(b"RIFF");
    let riff_size_pos = 4;
    b.extend_from_slice(&0u32.to_le_bytes()); // patched below
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24000u32.to_le_bytes());
    b.extend_from_slice(&48000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"LIST");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(b"INFO"); // 4-byte payload
    b.extend_from_slice(b"data");
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&[0u8; 4]); // 2 samples
    let riff_size = (b.len() - 8) as u32;
    b[riff_size_pos..riff_size_pos + 4].copy_from_slice(&riff_size.to_le_bytes());
    fs::write(&path, b).unwrap();

    let info = wav::parse(&path).unwrap();
    assert_eq!(info.data_bytes, 4);
    assert!((info.duration_sec() - 2.0 / 24000.0).abs() < 1e-12);
}

#[test]
fn rejects_garbage_with_wav_invalid_code() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("d.wav");
    fs::write(&path, b"this is not a wav file at all").unwrap();
    let err = wav::parse(&path).unwrap_err();
    assert_eq!(err.code(), "wav_invalid");
    assert!(err.to_string().contains(path.to_string_lossy().as_ref()));
}

#[test]
fn rejects_truncated_data_chunk() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("e.wav");
    // header claims 72000 samples but the file ends after 5
    let mut b: Vec<u8> = Vec::new();
    b.extend_from_slice(b"RIFF");
    let size_pos = 4;
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24000u32.to_le_bytes());
    b.extend_from_slice(&48000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&144000u32.to_le_bytes()); // claims 72000 samples
    b.extend_from_slice(&[0u8; 10]); // but only 5 samples present
    let riff_size = (b.len() - 8) as u32;
    b[size_pos..size_pos + 4].copy_from_slice(&riff_size.to_le_bytes());
    fs::write(&path, b).unwrap();

    assert_eq!(wav::parse(&path).unwrap_err().code(), "wav_invalid");
}
