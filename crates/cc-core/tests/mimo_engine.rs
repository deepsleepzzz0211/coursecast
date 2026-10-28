//! Ticket 05: `engine=mimo` (MimoEngine) against a local mock HTTP server. The real API
//! never enters tests (spec Testing Decisions); these pin the wire contract of PRD §8.2:
//! OpenAI-compatible chat/completions, `api-key` header auth, the text to read in the
//! assistant message / style instruction in the user message, reference audio inline as
//! a `data:audio/wav;base64,…` URI, and the base64 wav reply decoded back to bytes.
//!
//! reqwest::blocking refuses to build inside a tokio runtime, so every engine call runs
//! on a plain OS thread (exactly how tts::synthesize_pages drives it).

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use cc_core::wav;
use cc_core::{TtsEngine, TtsRequest};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// One captured request: the wire facts the contract pins.
#[derive(Debug, Clone)]
struct Captured {
    path: String,
    api_key: Option<String>,
    body: Value,
}

type Capture = Arc<Mutex<Vec<Captured>>>;

/// Mount a catch-all POST mock: capture every request, answer with `status`/`body`.
async fn mount(server: &MockServer, capture: Capture, status: u16, body: Value) {
    let responder = move |req: &wiremock::Request| {
        let headers = &req.headers;
        capture.lock().unwrap().push(Captured {
            path: req.url.path().to_string(),
            api_key: headers
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
}

fn engine(base_url: &str, api_key: &str) -> Arc<cc_core::mimo::MimoEngine> {
    Arc::new(cc_core::mimo::MimoEngine::with_base_url(api_key, base_url))
}

/// Run one synthesize on a plain thread (off any async runtime).
fn synthesize(
    engine: &Arc<cc_core::mimo::MimoEngine>,
    request: &TtsRequest,
) -> cc_core::Result<Vec<u8>> {
    let engine = Arc::clone(engine);
    let request = request.clone();
    std::thread::spawn(move || engine.synthesize(&request))
        .join()
        .unwrap()
}

/// A reference wav on disk (any 24kHz mono bytes; content is asserted via the data URI).
fn reference_wav(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("voice_ref.wav");
    std::fs::write(
        &path,
        wav::encode_mono_pcm16(wav::TTS_SAMPLE_RATE, &[100i16; 2400]),
    )
    .unwrap();
    path
}

fn request(text: &str, ref_path: PathBuf) -> TtsRequest {
    TtsRequest {
        text: text.into(),
        style: None,
        rate: 1.0,
        reference_audio: Some(ref_path),
    }
}

#[tokio::test]
async fn request_carries_model_api_key_messages_and_inline_reference() {
    let tmp = tempfile::tempdir().unwrap();
    let ref_path = reference_wav(tmp.path());

    let server = MockServer::start().await;
    let capture: Capture = Arc::new(Mutex::new(Vec::new()));
    // independent source of truth for the reply: a wav we build here, base64-encoded
    let reply_wav = wav::encode_mono_pcm16(wav::TTS_SAMPLE_RATE, &[7i16; 4800]);
    let reply = json!({
        "choices": [{ "message": { "audio": { "data": BASE64.encode(&reply_wav) } } }]
    });
    mount(&server, Arc::clone(&capture), 200, reply).await;

    let engine = engine(&format!("{}/v1", server.uri()), "test-key-123");
    let out = synthesize(&engine, &request("你好，世界。", ref_path.clone())).unwrap();
    assert_eq!(
        out, reply_wav,
        "base64 wav reply must decode to the exact audio bytes"
    );

    let reqs = capture.lock().unwrap();
    assert_eq!(reqs.len(), 1, "one page = one HTTP request");
    let r = &reqs[0];
    assert_eq!(r.path, "/v1/chat/completions", "OpenAI-compatible endpoint");
    assert_eq!(
        r.api_key.as_deref(),
        Some("test-key-123"),
        "`api-key` header auth (PRD §8.2, not Authorization: Bearer)"
    );
    assert_eq!(r.body["model"], "mimo-v2.5-tts-voiceclone");
    let messages = r.body["messages"]
        .as_array()
        .expect("messages array")
        .clone();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(
        messages[0]["content"], "",
        "no style configured -> empty user message (PRD §8.2: 留空也可)"
    );
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(
        messages[1]["content"], "你好，世界。",
        "the text to read goes in the assistant message (PRD §8.2)"
    );
    assert_eq!(r.body["audio"]["format"], "wav");
    let voice = r.body["audio"]["voice"]
        .as_str()
        .expect("audio.voice inline data URI");
    let encoded = voice
        .strip_prefix("data:audio/wav;base64,")
        .expect("reference audio must be a data:audio/wav;base64,… URI");
    assert_eq!(
        BASE64.decode(encoded).unwrap(),
        std::fs::read(&ref_path).unwrap(),
        "the data URI carries the reference wav bytes"
    );
}

#[tokio::test]
async fn http_401_maps_to_the_structured_auth_signal() {
    let tmp = tempfile::tempdir().unwrap();
    let ref_path = reference_wav(tmp.path());

    let server = MockServer::start().await;
    let capture: Capture = Arc::new(Mutex::new(Vec::new()));
    mount(
        &server,
        Arc::clone(&capture),
        401,
        json!({ "error": { "message": "invalid api key" } }),
    )
    .await;

    let engine = engine(&format!("{}/v1", server.uri()), "wrong-key");
    let err = synthesize(&engine, &request("第一页讲稿", ref_path)).unwrap_err();
    assert_eq!(
        err.code(),
        "tts_unauthorized",
        "fallback chain (ticket 06) matches on this code"
    );
    assert!(
        err.to_string().contains("401"),
        "message names the HTTP status: {err}"
    );
    assert!(
        err.to_string().contains("invalid api key"),
        "server message is surfaced: {err}"
    );
}

#[tokio::test]
async fn reference_file_is_read_and_encoded_once_but_uri_is_sent_on_every_request() {
    let tmp = tempfile::tempdir().unwrap();
    let ref_path = reference_wav(tmp.path());

    let server = MockServer::start().await;
    let capture: Capture = Arc::new(Mutex::new(Vec::new()));
    let reply_wav = wav::encode_mono_pcm16(wav::TTS_SAMPLE_RATE, &[7i16; 480]);
    mount(
        &server,
        Arc::clone(&capture),
        200,
        json!({ "choices": [{ "message": { "audio": { "data": BASE64.encode(&reply_wav) } } }] }),
    )
    .await;

    let engine = engine(&format!("{}/v1", server.uri()), "test-key-123");
    // three pages, same reference audio — and the file disappears after page 1: the
    // encoded URI must live in memory, not be re-read from disk per page (spec §8.2)
    let req = request("第一页", ref_path.clone());
    synthesize(&engine, &req).unwrap();
    std::fs::remove_file(&ref_path).unwrap();
    synthesize(&engine, &request("第二页", ref_path.clone())).unwrap();
    synthesize(&engine, &request("第三页", ref_path)).unwrap();

    let reqs = capture.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    let uris: Vec<&str> = reqs
        .iter()
        .map(|r| {
            r.body["audio"]["voice"]
                .as_str()
                .expect("audio.voice present")
        })
        .collect();
    assert!(
        uris.iter().all(|u| u.starts_with("data:audio/wav;base64,")),
        "every request carries the inline reference audio (PRD §8.2: 每次请求都要带)"
    );
    assert_eq!(uris[0], uris[1], "cached URI is resent unchanged");
    assert_eq!(uris[1], uris[2]);
}
