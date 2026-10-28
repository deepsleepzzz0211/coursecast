//! Seam: `cc-cli voice_synthesize --engine mimo --json` end to end against a local mock
//! HTTP server (ticket 05). Covers what the engine-level tests cannot see: the key
//! flowing from the credential store into the `api-key` header, the reply payload
//! landing as per-page wav artifacts, and the 401 surfacing as the structured
//! `tts_unauthorized` error the fallback chain (ticket 06) matches on.
//!
//! The mock server runs on its own runtime thread so the tests stay synchronous and the
//! cc-cli child process (which blocks on real HTTP) cannot deadlock the runtime.

use assert_cmd::Command;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use cc_core::wav;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-e2e-mimo-KEY-77c1";

#[derive(Debug, Clone)]
struct Captured {
    path: String,
    api_key: Option<String>,
    body: Value,
}

/// Mock server handle: base URL + everything it received.
struct MockApi {
    base_url: String,
    requests: Arc<Mutex<Vec<Captured>>>,
}

/// Start a mock that answers every POST with `status`/`body`, on a dedicated thread.
fn start_mock(status: u16, body: Value) -> MockApi {
    let requests: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(Vec::new()));
    let (uri_tx, uri_rx) = std::sync::mpsc::channel();
    let capture = Arc::clone(&requests);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let server = MockServer::start().await;
            let responder = move |req: &wiremock::Request| {
                capture.lock().unwrap().push(Captured {
                    path: req.url.path().to_string(),
                    api_key: req
                        .headers
                        .get("api-key")
                        .and_then(|v| v.to_str().ok())
                        .map(String::from),
                    body: req.body_json::<Value>().unwrap_or(Value::Null),
                });
                ResponseTemplate::new(status).set_body_json(body.clone())
            };
            server
                .register(Mock::given(method("POST")).respond_with(responder))
                .await;
            uri_tx.send(server.uri()).unwrap();
            // serve until the test process exits (MockServer must outlive the test)
            std::future::pending::<()>().await;
        });
    });
    MockApi {
        base_url: uri_rx.recv().unwrap(),
        requests,
    }
}

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

struct Project {
    data_dir: PathBuf,
    id: String,
    dir: PathBuf,
    key_file: PathBuf,
    /// Mock endpoint prefix (set per test; empty until assigned).
    mock_base_url: String,
    /// Edge engine endpoint override (ticket 07): set to a dead local port so the
    /// chain's middle level fails deterministically and fast, whatever the real
    /// Microsoft service would do today.
    edge_endpoint: String,
}

fn fixture(tmp: &Path) -> Project {
    let data_dir = tmp.join("data");
    let key_file = tmp.join("mimo.key");
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "project",
            "create",
            "--name",
            "MiMo合成测试",
            "--json",
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = data_dir.join("projects").join(&id);
    std::fs::write(dir.join("script.md"), "第一页讲稿\n\n---\n\n第二页讲稿\n").unwrap();
    // advance the stage directly (slide_render is ticket 03, not this ticket's seam)
    let (mut project, pdir) = cc_core::load_project(&data_dir, &id).unwrap();
    project.stage = cc_core::Stage::Parsed;
    cc_core::save_project(&project, &pdir).unwrap();
    // reference audio the engine will inline (any valid wav bytes)
    std::fs::create_dir_all(dir.join("source")).unwrap();
    std::fs::write(
        dir.join("source/voice_ref.wav"),
        wav::encode_mono_pcm16(wav::TTS_SAMPLE_RATE, &[90i16; 1200]),
    )
    .unwrap();
    // clone runs require the compliance acknowledgement (spec 合规); the dedicated
    // gate test below revokes it and asserts the refusal
    cc_core::set_voice_consent(&dir, true.into()).unwrap();
    Project {
        data_dir,
        id,
        dir,
        key_file,
        mock_base_url: String::new(),
        edge_endpoint: String::new(),
    }
}

/// A local port that refuses connections: bind, read the port, drop the listener.
fn dead_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

/// Run a cc-cli subcommand with the credential file seam + mock endpoint env.
fn run(project: &Project, args: &[&str]) -> (std::process::Output, Value) {
    let mut cmd = cli();
    cmd.env("CC_CREDENTIALS_FILE", &project.key_file)
        .env("CC_MIMO_BASE_URL", &project.mock_base_url);
    if !project.edge_endpoint.is_empty() {
        cmd.env("CC_EDGE_ENDPOINT", &project.edge_endpoint);
    }
    let out = cmd
        .args(["--data-dir", project.data_dir.to_str().unwrap()])
        .args(args)
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

#[test]
fn mimo_synthesis_stores_key_configures_then_lands_decoded_wav_per_page() {
    let tmp = tempfile::tempdir().unwrap();
    let project = fixture(tmp.path());

    // the reply payload: one 24kHz mono wav, base64-wrapped like the real API
    let reply_wav = wav::encode_mono_pcm16(wav::TTS_SAMPLE_RATE, &[42i16; 6000]);
    let mock = start_mock(
        200,
        json!({ "choices": [{ "message": { "audio": { "data": BASE64.encode(&reply_wav) } } }] }),
    );
    let mut project = project;
    project.mock_base_url = format!("{}/v1", mock.base_url);

    // key enters the credential store through voice_configure
    let (out, v) = run(
        &project,
        &[
            "voice_configure",
            "--project",
            &project.id,
            "--api-key",
            KEY,
            "--json",
        ],
    );
    assert!(out.status.success(), "voice_configure failed: {v}");
    assert_eq!(v["api_key_stored"], true);

    let (out, v) = run(
        &project,
        &[
            "voice_synthesize",
            "--project",
            &project.id,
            "--engine",
            "mimo",
            "--json",
        ],
    );
    assert!(out.status.success(), "stdout: {v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "tts_done");
    let generated: Vec<u32> = v["generated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(generated, vec![1, 2]);

    // each page's artifact is exactly the decoded reply payload
    for page in 1..=2 {
        let bytes =
            std::fs::read(project.dir.join(format!("work/audio/slide_{page:03}.wav"))).unwrap();
        assert_eq!(
            bytes, reply_wav,
            "page {page} must be the decoded mock payload"
        );
        let info = wav::parse_wav(&bytes).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.bits_per_sample),
            (24_000, 1, 16)
        );
    }
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains(KEY),
        "the key must never appear in command output"
    );

    // the stored key flowed through the credential store into the api-key header
    let reqs = mock.requests.lock().unwrap();
    assert_eq!(reqs.len(), 2, "one request per page");
    assert!(reqs.iter().all(|r| r.api_key.as_deref() == Some(KEY)));
    assert!(reqs.iter().all(|r| r.path == "/v1/chat/completions"));
    assert!(reqs
        .iter()
        .all(|r| r.body["model"] == "mimo-v2.5-tts-voiceclone"));

    // project.json records the mimo engine (fingerprint/tool_version) and no key
    let project_json = std::fs::read_to_string(project.dir.join("project.json")).unwrap();
    assert!(project_json.contains("tts-mimo/0.1.0+model=mimo-v2.5-tts-voiceclone"));
    assert!(
        !project_json.contains(KEY),
        "the key must never appear in project.json"
    );
}

#[test]
fn mimo_401_slides_to_sapi_for_the_rest_of_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    let project = fixture(tmp.path());
    let mock = start_mock(
        401,
        json!({ "error": { "message": "quota or key invalid" } }),
    );
    let mut project = project;
    project.mock_base_url = format!("{}/v1", mock.base_url);
    // ticket 07: the chain is mimo → edge → sapi; point edge at a dead local port so
    // its level fails deterministically and the run observably lands on SAPI
    project.edge_endpoint = format!("ws://127.0.0.1:{}/tc", dead_port());
    run(
        &project,
        &[
            "voice_configure",
            "--project",
            &project.id,
            "--api-key",
            KEY,
            "--json",
        ],
    );

    // ticket 06: with a fallback below it, a 401 no longer stops the run — the chain
    // slides through edge (blocked) to SAPI (offline, real PowerShell) and every page
    // still lands
    let (out, v) = run(
        &project,
        &[
            "voice_synthesize",
            "--project",
            &project.id,
            "--engine",
            "mimo",
            "--json",
        ],
    );
    assert!(out.status.success(), "the run must slide, not stop: {v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "tts_done");

    let fallbacks = v["fallbacks"].as_array().unwrap();
    assert_eq!(fallbacks.len(), 2, "{v}");
    assert_eq!(fallbacks[0]["from"], "mimo");
    assert_eq!(fallbacks[0]["to"], "edge");
    assert!(
        fallbacks[0]["reason"].as_str().unwrap().contains("401"),
        "the user-readable hint names the auth failure: {fallbacks:?}"
    );
    assert_eq!(fallbacks[1]["from"], "edge");
    assert_eq!(fallbacks[1]["to"], "sapi");
    assert!(
        fallbacks[1]["reason"]
            .as_str()
            .unwrap()
            .contains("connect failed"),
        "the blocked middle level names its failure: {fallbacks:?}"
    );

    // deterministic auth failure: mimo was probed once (page 1) and never again —
    // pages 2+ went straight to the fallback
    assert_eq!(mock.requests.lock().unwrap().len(), 1);

    // every page landed, as real SAPI wavs in the pipeline format
    for page in 1..=2 {
        let bytes =
            std::fs::read(project.dir.join(format!("work/audio/slide_{page:03}.wav"))).unwrap();
        let info = wav::parse_wav(&bytes).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.bits_per_sample),
            (24_000, 1, 16),
            "page {page} must be a pipeline wav"
        );
    }

    // project.json recorded the degradation and the engine that actually produced the audio
    let (loaded, _) = cc_core::load_project(&project.data_dir, &project.id).unwrap();
    assert_eq!(loaded.stage, cc_core::Stage::TtsDone);
    assert_eq!(loaded.fallbacks.len(), 2);
    assert_eq!(loaded.fallbacks[0].from, "mimo");
    assert_eq!(loaded.fallbacks[0].to, "edge");
    assert_eq!(loaded.fallbacks[1].to, "sapi");
    assert!(loaded
        .stages
        .get("tts:1")
        .unwrap()
        .tool_version
        .starts_with("tts-sapi/"));
}

#[test]
fn mimo_without_a_stored_key_degrades_to_sapi_and_still_produces_wav() {
    let tmp = tempfile::tempdir().unwrap();
    let mut project = fixture(tmp.path());
    // no voice_configure ran: the key file does not exist; edge gets a dead local port
    // so the middle level fails deterministically (ticket 07)
    project.edge_endpoint = format!("ws://127.0.0.1:{}/tc", dead_port());

    let (out, v) = run(
        &project,
        &[
            "voice_synthesize",
            "--project",
            &project.id,
            "--engine",
            "mimo",
            "--json",
        ],
    );
    // acceptance (ticket 06): 无 Key 时 engine=mimo 请求自动降级到 SAPI 并产出 wav
    assert!(out.status.success(), "no key must degrade, not fail: {v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "tts_done");
    let generated: Vec<u32> = v["generated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(generated, vec![1, 2]);

    let fallbacks = v["fallbacks"].as_array().unwrap();
    assert_eq!(fallbacks.len(), 2, "{v}");
    assert_eq!(fallbacks[0]["from"], "mimo");
    assert_eq!(fallbacks[0]["to"], "edge");
    assert!(
        fallbacks[0]["reason"].as_str().unwrap().contains("API key"),
        "the user-readable hint says the key is missing: {fallbacks:?}"
    );
    assert_eq!(fallbacks[1]["from"], "edge");
    assert_eq!(fallbacks[1]["to"], "sapi");

    // wav artifacts actually exist (real SAPI synthesis on this machine)
    for page in 1..=2 {
        let bytes =
            std::fs::read(project.dir.join(format!("work/audio/slide_{page:03}.wav"))).unwrap();
        let info = wav::parse_wav(&bytes).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.bits_per_sample),
            (24_000, 1, 16)
        );
        assert!(info.duration_sec > 0.3, "page {page}: {info:?}");
    }

    // project.json records the degradation + which engine produced the audio,
    // and still never contains any key material
    let project_json = std::fs::read_to_string(project.dir.join("project.json")).unwrap();
    assert!(project_json.contains("tts-sapi/0.1.0+model=system-speech"));
    assert!(project_json.contains("\"fallbacks\""));
    assert!(
        project_json.contains("mimo"),
        "the event names the skipped engine"
    );
    assert!(
        !project_json.contains(KEY),
        "the key must never appear in project.json"
    );
}

/// 合规门：没有《声音授权确认》时，真正会走克隆的 mimo 合成必须在联网前被拒绝；
/// CLI/MCP 与 UI 共用同一份确认文件（票 16 复审）。
#[test]
fn clone_synthesis_requires_the_voice_consent_acknowledgement() {
    let tmp = tempfile::tempdir().unwrap();
    let project = fixture(tmp.path());

    let (out, v) = run(
        &project,
        &[
            "voice_consent",
            "--project",
            &project.id,
            "--revoke",
            "--json",
        ],
    );
    assert!(out.status.success(), "{v}");
    assert_eq!(v["granted"], false);

    run(
        &project,
        &[
            "voice_configure",
            "--project",
            &project.id,
            "--api-key",
            KEY,
            "--json",
        ],
    );

    let (out, v) = run(
        &project,
        &[
            "voice_synthesize",
            "--project",
            &project.id,
            "--engine",
            "mimo",
            "--json",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "consent must block before networking"
    );
    assert_eq!(v["error"]["code"], "invalid_argument");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("声音授权确认"),
        "{v}"
    );
    assert!(
        !project.dir.join("work/audio").exists(),
        "no audio may be written"
    );

    let (out, v) = run(
        &project,
        &[
            "voice_consent",
            "--project",
            &project.id,
            "--grant",
            "--json",
        ],
    );
    assert!(out.status.success(), "{v}");
    assert_eq!(v["granted"], true);
}
