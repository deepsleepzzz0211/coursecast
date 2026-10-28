//! Seam: `cc-cli timeline --json` stdout, the artifacts it writes
//! (`work/concat.txt`, `work/audio/voice_full.wav`) and the `stages.timeline` fingerprint
//! in `project.json` (spec "Testing Decisions": 好测试只测外部行为). Expected values are
//! the timeline spike's worked example: 5 pages 3.0/2.0/4.0/2.5/3.5 s + g=0.6 →
//! T=17.400 s, turn points 3.6/6.2/10.8/13.9 (≤33 ms = 1 frame @30fps).

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

const RATE: f64 = 24000.0;
const GAP: f64 = 0.6;
const DURATIONS: [f64; 5] = [3.0, 2.0, 4.0, 2.5, 3.5];
const TOTAL: f64 = 17.4;
const TURN_POINTS: [f64; 4] = [3.6, 6.2, 10.8, 13.9];

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn sys(bin: &str) -> SysCommand {
    SysCommand::new(bin)
}

fn create_project(data_dir: &Path) -> (String, PathBuf) {
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "project",
            "create",
            "--json",
            "--name",
            "时间轴测试",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    (
        v["project_id"].as_str().unwrap().to_string(),
        PathBuf::from(v["dir"].as_str().unwrap()),
    )
}

fn write_sine_wav(path: &Path, samples: usize, freq: f64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut pcm = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        let v = (2.0 * std::f64::consts::PI * freq * i as f64 / RATE).sin() * 12000.0;
        pcm.extend_from_slice(&(v as i16).to_le_bytes());
    }
    let mut b = Vec::with_capacity(44 + pcm.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&(RATE as u32).to_le_bytes());
    b.extend_from_slice(&((RATE as u32) * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(&pcm);
    fs::write(path, b).unwrap();
}

fn write_gray_png(path: &Path, color: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let out = sys("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={color}:s=320x180"),
            "-frames:v",
            "1",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Spike setup: 5 sine wavs + 5 gray-ladder slide PNGs (gray levels make page turns
/// detectable by scene change, the lesson recorded in the timeline spike).
fn setup_pages(dir: &Path) {
    let colors = ["0x101010", "0x404040", "0x808080", "0xC0C0C0", "0xF0F0F0"];
    for (i, d) in DURATIONS.iter().enumerate() {
        write_sine_wav(
            &dir.join("work/audio")
                .join(format!("slide_{:03}.wav", i + 1)),
            (d * RATE) as usize,
            300.0 + 80.0 * i as f64,
        );
        write_gray_png(
            &dir.join("work/slides")
                .join(format!("slide_{:03}.png", i + 1)),
            colors[i],
        );
    }
}

fn run_timeline(data_dir: &Path, id: &str) -> (std::process::Output, Value) {
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "timeline",
            "--project",
            id,
            "--json",
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out, v)
}

/// Test-side PCM reader (mirror of the header rules, kept independent of cc-core internals).
fn read_wav_pcm(path: &Path) -> (u32, Vec<i16>) {
    let b = fs::read(path).unwrap();
    assert_eq!(&b[0..4], b"RIFF");
    assert_eq!(&b[8..12], b"WAVE");
    let (mut pos, mut rate, mut data) = (12usize, 0u32, None);
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = pos + 8;
        if id == b"fmt " {
            rate = u32::from_le_bytes(b[body + 4..body + 8].try_into().unwrap());
        } else if id == b"data" {
            data = Some(&b[body..body + size]);
            break;
        }
        pos = body + size + (size & 1);
    }
    let samples: Vec<i16> = data
        .unwrap()
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    (rate, samples)
}

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn timeline_outputs_artifacts_json_and_fingerprint() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);

    let (out, v) = run_timeline(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["ok"], true);

    // per-page durations: audio_sec = d_i, display_sec = d_i + g (last page plain d_N)
    let pages = v["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 5);
    for (i, pg) in pages.iter().enumerate() {
        assert_eq!(pg["page"], (i + 1) as u64);
        assert!(
            approx(pg["audio_sec"].as_f64().unwrap(), DURATIONS[i], 1e-6),
            "page {}",
            i + 1
        );
        let want = if i == 4 {
            DURATIONS[i]
        } else {
            DURATIONS[i] + GAP
        };
        assert!(
            approx(pg["display_sec"].as_f64().unwrap(), want, 1e-6),
            "page {}",
            i + 1
        );
    }

    // turn points and total from the spike's worked example
    let turns = v["turn_points_sec"].as_array().unwrap();
    assert_eq!(turns.len(), 4);
    for (got, want) in turns.iter().zip(TURN_POINTS) {
        assert!(
            approx(got.as_f64().unwrap(), want, 0.005),
            "turn {got} != {want}"
        );
    }
    assert!(approx(v["total_sec"].as_f64().unwrap(), TOTAL, 0.005));
    assert!(approx(v["gap_sec"].as_f64().unwrap(), 0.6, 1e-9));

    // artifacts on disk
    let concat = dir.join("work/concat.txt");
    let voice = dir.join("work/audio/voice_full.wav");
    assert!(concat.is_file());
    assert!(voice.is_file());
    assert_eq!(v["artifacts"]["concat_txt"], "work/concat.txt");
    assert_eq!(v["artifacts"]["voice_full"], "work/audio/voice_full.wav");

    // concat.txt: per-page display durations per the formula, last image repeated
    let text = fs::read_to_string(&concat).unwrap();
    for (i, want) in [3.6, 2.6, 4.6, 3.1, 3.5].iter().enumerate() {
        assert!(
            text.contains(&format!("file 'slides/slide_{:03}.png'", i + 1)),
            "{text}"
        );
        assert!(
            text.contains(&format!("duration {want:.3}")),
            "duration {} missing: {text}",
            want
        );
    }
    assert_eq!(
        text.matches("file 'slides/slide_005.png'").count(),
        2,
        "last image repeated: {text}"
    );

    // voice_full: exactly Σd + 4g seconds of 24 kHz mono PCM
    let (rate, samples) = read_wav_pcm(&voice);
    assert_eq!(rate, 24000);
    let expected_samples = (TOTAL * RATE).round() as usize; // 417600
    assert_eq!(
        samples.len(),
        expected_samples,
        "voice_full must be sample-exact"
    );

    // fingerprint + stage record in project.json; main state untouched
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    let record = &pj["stages"]["timeline"];
    assert_eq!(record["stage"], "timeline");
    assert_eq!(record["state"], "success");
    let fp = record["input_fingerprint"].as_str().unwrap();
    let hex = fp.strip_prefix("sha256:").unwrap();
    assert_eq!(hex.len(), 64);
    let outputs = record["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 2);
    for o in outputs {
        assert!(o["sha256"].as_str().unwrap().starts_with("sha256:"));
        assert!(o["bytes"].as_u64().unwrap() > 0);
    }
    assert_eq!(
        pj["stage"], "created",
        "timeline records a stage fingerprint, not a state"
    );
}

#[test]
fn rendered_video_turn_points_within_one_frame_and_duration_consistent() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    let (out, _) = run_timeline(tmp.path(), &id);
    assert!(out.status.success());

    // consume the artifacts the way render (ticket 08) will: concat image track + voice
    let mp4 = dir.join("work/check.mp4");
    let render = sys("ffmpeg")
        .args(["-y", "-v", "error", "-f", "concat", "-safe", "0"])
        .arg("-i")
        .arg(dir.join("work/concat.txt"))
        .arg("-i")
        .arg(dir.join("work/audio/voice_full.wav"))
        .args([
            "-vf",
            "scale=320:180:force_original_aspect_ratio=decrease,pad=320:180:(ow-iw)/2:(oh-ih)/2:color=white,setsar=1,fps=30,format=yuv420p",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "20",
            "-c:a", "aac", "-b:a", "96k",
            "-t", "17.4", // hard lock at the formula total, as the spec's 合成 prescribes
            "-movflags", "+faststart",
        ])
        .arg(&mp4)
        .output()
        .unwrap();
    assert!(
        render.status.success(),
        "{}",
        String::from_utf8_lossy(&render.stderr)
    );

    // container / audio track / video track all agree with the concat expectation
    let mut durations = Vec::new();
    for sel in [
        vec!["-show_entries", "format=duration"],
        vec!["-select_streams", "a:0", "-show_entries", "stream=duration"],
        vec!["-select_streams", "v:0", "-show_entries", "stream=duration"],
    ] {
        let out = sys("ffprobe")
            .args(["-v", "error"])
            .args(&sel)
            .args(["-of", "csv=p=0"])
            .arg(&mp4)
            .output()
            .unwrap();
        let value: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        durations.push(value);
    }
    for d in &durations {
        assert!(
            approx(*d, TOTAL, 0.05),
            "duration {d} != {TOTAL} (container/audio/video)"
        );
    }

    // page turns: scene-change detection, tolerance 1 frame @30fps (33 ms)
    let detect = sys("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(&mp4)
        .args([
            "-filter:v",
            "select='gt(scene,0.02)',showinfo",
            "-f",
            "null",
            "-",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&detect.stderr);
    let mut detected = Vec::new();
    for line in stderr.lines() {
        if let Some(idx) = line.find("pts_time:") {
            let rest = &line[idx + "pts_time:".len()..];
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(t) = num.parse::<f64>() {
                detected.push(t);
            }
        }
    }
    assert_eq!(
        detected.len(),
        4,
        "expected exactly 4 page turns, got {detected:?}"
    );
    for (got, want) in detected.iter().zip(TURN_POINTS) {
        assert!(
            approx(*got, want, 0.034),
            "turn point {got} vs expected {want}: error {} ms exceeds 1 frame @30fps",
            ((got - want).abs() * 1000.0).round()
        );
    }
}

#[test]
fn voice_full_gaps_are_silent_and_tail_fades_out() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    let (out, _) = run_timeline(tmp.path(), &id);
    assert!(out.status.success());

    let (rate, samples) = read_wav_pcm(&dir.join("work/audio/voice_full.wav"));
    assert_eq!(rate, 24000);
    let to_idx = |sec: f64| (sec * RATE).round() as usize;

    // gap k sits after page k's audio: [Σ_{j<=k} d_j + (k-1)g, +0.6]
    let mut t = 0.0;
    for (k, d) in DURATIONS.iter().enumerate() {
        t += d;
        if k == 4 {
            break;
        }
        let gap_start = to_idx(t);
        let gap_end = to_idx(t + GAP);
        // gap interior: digital silence
        let interior_max = samples[gap_start + 480..gap_end - 480]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap();
        assert!(
            interior_max < 8,
            "gap {} not silent: max amplitude {interior_max}",
            k + 1
        );
        // audio present right before the gap and right after it (gap is where it belongs)
        let before_max = samples[gap_start - 480..gap_start - 48]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap();
        let after_max = samples[gap_end + 48..gap_end + 480]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap();
        assert!(before_max > 1000, "no audio before gap {}", k + 1);
        assert!(after_max > 1000, "no audio after gap {}", k + 1);
        t += GAP;
    }

    // 1.0 s fade-out: the last 0.1 s is far quieter than the pre-fade tail of page 5
    let rms = |a: usize, b: usize| {
        (samples[a..b]
            .iter()
            .map(|s| (*s as f64) * (*s as f64))
            .sum::<f64>()
            / (b - a) as f64)
            .sqrt()
    };
    let pre_fade = rms(to_idx(16.0), to_idx(16.4));
    let fade_end = rms(to_idx(17.3), to_idx(17.4));
    assert!(
        fade_end < 0.25 * pre_fade,
        "no audible fade-out: {fade_end} vs {pre_fade}"
    );
    assert!(fade_end > 0.0, "audio vanished before the fade");
}

#[test]
fn timeline_without_audio_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, _) = create_project(tmp.path());
    let (out, v) = run_timeline(tmp.path(), &id);
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "audio_missing");
}

#[test]
fn timeline_without_slides_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    for (i, d) in DURATIONS.iter().enumerate() {
        write_sine_wav(
            &dir.join("work/audio")
                .join(format!("slide_{:03}.wav", i + 1)),
            (d * RATE) as usize,
            300.0,
        );
    }
    let (out, v) = run_timeline(tmp.path(), &id);
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "slide_missing");
}

#[test]
fn timeline_with_corrupt_wav_returns_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    fs::create_dir_all(dir.join("work/audio")).unwrap();
    fs::create_dir_all(dir.join("work/slides")).unwrap();
    fs::write(dir.join("work/audio/slide_001.wav"), b"garbage not a wav").unwrap();
    write_gray_png(&dir.join("work/slides/slide_001.png"), "0x101010");
    let (out, v) = run_timeline(tmp.path(), &id);
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "wav_invalid");
}

#[test]
fn fingerprint_changes_when_page_audio_duration_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    let (_, v1) = run_timeline(tmp.path(), &id);
    let fp1 = v1["fingerprint"].as_str().unwrap().to_string();

    // page 2 audio becomes longer -> only that page's timing and the fingerprint change
    write_sine_wav(
        &dir.join("work/audio/slide_002.wav"),
        (2.5 * RATE) as usize,
        380.0,
    );
    let (out, v2) = run_timeline(tmp.path(), &id);
    assert!(out.status.success());
    assert_ne!(v2["fingerprint"].as_str().unwrap(), fp1);
    assert!(approx(
        v2["pages"][1]["audio_sec"].as_f64().unwrap(),
        2.5,
        1e-6
    ));
    assert!(approx(v2["total_sec"].as_f64().unwrap(), 17.9, 0.005));
}

/// 音频比课件页少（TTS 缺页/尾页空稿被丢）必须报错，不能把连续音频前缀悄悄
/// 渲染成缩短的成片（票 16 复审）。
#[test]
fn timeline_refuses_a_page_count_mismatch_instead_of_truncating() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    fs::remove_file(dir.join("work/audio/slide_005.wav")).unwrap();

    let (out, v) = run_timeline(tmp.path(), &id);
    assert!(!out.status.success(), "4 wavs vs 5 slides must fail");
    assert_eq!(v["error"]["code"], "page_count_mismatch");
    assert!(
        !dir.join("work/concat.txt").exists(),
        "no concat.txt may be written on mismatch"
    );
}
