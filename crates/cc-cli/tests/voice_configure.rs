//! Seam: `cc-cli voice_configure --json` stdout + the landed `source/voice_ref.wav` +
//! the credential store (ticket 05). The API key must never leak into stdout, stderr,
//! non-JSON output or project.json; reference audio is normalized to 24kHz mono
//! pcm_s16le wav (spec "TTS 主引擎", PRD §8.2 工程要点).
//!
//! Key storage goes through the `CC_CREDENTIALS_FILE` test seam (plaintext file) so the
//! real Windows Credential Manager is never touched; the store/read path is the same.

use assert_cmd::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

const KEY: &str = "sk-test-mimo-SUPERSECRET-9f3a";

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn sys(bin: &str) -> SysCommand {
    SysCommand::new(bin)
}

fn run_ok(command: &mut SysCommand) -> Vec<u8> {
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "command failed: {:?}\n{}",
        command,
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn run_cli_ok(command: &mut Command) -> Vec<u8> {
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "cc-cli failed: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

struct Project {
    data_dir: PathBuf,
    id: String,
    dir: PathBuf,
}

fn create_project(tmp: &Path) -> Project {
    let data_dir = tmp.join("data");
    let out = run_cli_ok(cli().args([
        "--data-dir",
        data_dir.to_str().unwrap(),
        "project",
        "create",
        "--name",
        "语音配置测试",
        "--json",
    ]));
    let v: Value = serde_json::from_slice(&out).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    Project {
        dir: data_dir.join("projects").join(&id),
        data_dir,
        id,
    }
}

/// `voice_configure --json` with the credential file seam pointed at `key_file`.
fn configure(
    project: &Project,
    key_file: &Path,
    audio: Option<&Path>,
    api_key: Option<&str>,
) -> (std::process::Output, Value) {
    let mut cmd = cli();
    cmd.env("CC_CREDENTIALS_FILE", key_file)
        .args(["--data-dir", project.data_dir.to_str().unwrap()])
        .args(["voice_configure", "--project", &project.id, "--json"]);
    if let Some(a) = audio {
        cmd.arg("--audio").arg(a);
    }
    if let Some(k) = api_key {
        cmd.arg("--api-key").arg(k);
    }
    let out = cmd.output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out, v)
}

#[test]
fn m4a_becomes_24k_mono_wav_and_the_key_never_leaks() {
    let tmp = tempfile::tempdir().unwrap();
    let project = create_project(tmp.path());

    // fixtures from real ffmpeg: a 48kHz stereo wav and its m4a encode (same duration)
    let fixture = tmp.path().join("ref48.wav");
    let m4a = tmp.path().join("ref.m4a");
    run_ok(sys("ffmpeg").args([
        "-y",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
        "-ar",
        "48000",
        "-ac",
        "2",
        fixture.to_str().unwrap(),
    ]));
    run_ok(sys("ffmpeg").args([
        "-y",
        "-i",
        fixture.to_str().unwrap(),
        "-c:a",
        "aac",
        m4a.to_str().unwrap(),
    ]));
    let stereo_bytes = fixture.metadata().unwrap().len();

    let key_file = tmp.path().join("mimo.key");
    let (out, v) = configure(&project, &key_file, Some(&m4a), Some(KEY));
    assert!(
        out.status.success(),
        "stdout: {v}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["ok"], true);
    assert_eq!(
        v["api_key_stored"], true,
        "the key was accepted into the store"
    );

    // the normalized reference wav: 24kHz / mono / 16-bit, ~4x smaller than the 48kHz
    // stereo original (spec 工程要点: 入库统一转 24kHz 单声道)
    let audio = &v["audio"];
    assert_eq!(audio["path"], "source/voice_ref.wav");
    assert_eq!(audio["sample_rate"], 24_000);
    assert_eq!(audio["channels"], 1);
    assert_eq!(
        audio["bytes"],
        fs_size(&project.dir.join("source/voice_ref.wav"))
    );
    let info = cc_core::wav::parse(&project.dir.join("source/voice_ref.wav")).unwrap();
    assert_eq!(info.sample_rate, 24_000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.bits_per_sample, 16);
    assert!(
        (info.duration_sec - 2.0).abs() < 0.1,
        "duration preserved: {}",
        info.duration_sec
    );
    let ratio = stereo_bytes as f64 / audio["bytes"].as_u64().unwrap() as f64;
    assert!(
        (3.6..4.4).contains(&ratio),
        "24kHz mono must be ~4x smaller than 48kHz stereo: {stereo_bytes} vs {}",
        audio["bytes"]
    );

    // the key never leaks: stdout (JSON), stderr, and project.json
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!stdout.contains(KEY), "key leaked into stdout JSON");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains(KEY), "key leaked into stderr");
    let project_json = std::fs::read_to_string(project.dir.join("project.json")).unwrap();
    assert!(!project_json.contains(KEY), "key leaked into project.json");

    // the stored key actually landed in the credential store (file seam)
    assert_eq!(std::fs::read_to_string(&key_file).unwrap(), KEY);

    let next = v["next"].as_str().unwrap();
    assert!(
        next.contains("voice_synthesize"),
        "next guides to synthesis: {next}"
    );
}

fn fs_size(path: &Path) -> u64 {
    path.metadata()
        .unwrap_or_else(|e| panic!("{} missing: {e}", path.display()))
        .len()
}

#[test]
fn voice_configure_with_nothing_to_configure_fails_structured() {
    let tmp = tempfile::tempdir().unwrap();
    let project = create_project(tmp.path());
    let key_file = tmp.path().join("mimo.key");

    let (out, v) = configure(&project, &key_file, None, None);
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "invalid_argument");
}

#[test]
fn missing_audio_file_is_a_structured_error() {
    let tmp = tempfile::tempdir().unwrap();
    let project = create_project(tmp.path());
    let key_file = tmp.path().join("mimo.key");

    let (out, v) = configure(
        &project,
        &key_file,
        Some(tmp.path().join("nope.m4a").as_path()),
        None,
    );
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "source_not_found");
}

#[test]
fn voice_configure_persists_the_edge_voice_preference() {
    let tmp = tempfile::tempdir().unwrap();
    let project = create_project(tmp.path());
    let key_file = tmp.path().join("mimo.key");

    let out = run_cli_ok(
        cli()
            .env("CC_CREDENTIALS_FILE", &key_file)
            .args(["--data-dir", project.data_dir.to_str().unwrap()])
            .args([
                "voice_configure",
                "--project",
                &project.id,
                "--engine",
                "edge",
                "--edge-voice",
                "yunxi",
                "--json",
            ]),
    );
    let v: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["tts"]["engine"], "edge");
    assert_eq!(v["tts"]["edge_voice"], "yunxi");

    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(project.dir.join("project.json")).unwrap())
            .unwrap();
    assert_eq!(saved["tts"]["engine"], "edge");
    assert_eq!(saved["tts"]["edge_voice"], "yunxi");
}
