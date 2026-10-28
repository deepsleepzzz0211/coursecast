//! MiMo voiceclone TTS engine (ticket 05): the real `engine=mimo` client.
//!
//! Wire contract pinned by PRD §8.2 / `spikes/tts-test`: OpenAI-compatible
//! `chat/completions` at `https://api.xiaomimimo.com/v1`, `api-key` header auth, the
//! text to read in the `assistant` message (style instruction in `user`), reference
//! audio inline as a `data:audio/wav;base64,…` URI — sent on **every** request — and
//! reply `choices[0].message.audio.data` = base64 of a 24kHz mono pcm_s16le wav.
//!
//! Per-page timeout + retry live one layer up (`tts::synthesize_pages`, spec "TTS 主引擎").
//! HTTP 401 maps to `Error::TtsUnauthorized` — the structured signal the fallback chain
//! (ticket 06) and frontend guidance depend on. The engine itself is stateless beyond the
//! in-memory reference-audio URI cache (encode once per file, not once per page).

use crate::error::{Error, Result};
use crate::tts::{TtsEngine, TtsRequest};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const MIMO_MODEL: &str = "mimo-v2.5-tts-voiceclone";
pub const DEFAULT_BASE_URL: &str = "https://api.xiaomimimo.com/v1";

/// Per-request HTTP timeout (spec "TTS 主引擎": 每页请求带超时 120 s). The request layer
/// re-applies the same bound around every engine call.
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

fn failure(message: impl Into<String>) -> Error {
    Error::EngineFailure {
        engine: "mimo".into(),
        message: message.into(),
    }
}

/// Tail of a response body for error messages — enough to identify the failure without
/// dumping a huge payload into logs.
fn body_tail(body: &str) -> String {
    let body = body.trim();
    if body.chars().count() <= 300 {
        body.to_string()
    } else {
        let tail: String = body.chars().skip(body.chars().count() - 300).collect();
        format!("…{tail}")
    }
}

pub struct MimoEngine {
    api_key: String,
    base_url: String,
    /// Built lazily: `reqwest::blocking` refuses to construct inside a tokio runtime, and
    /// the engine is always *driven* from plain threads (see tts::call_with_timeout).
    http: OnceLock<reqwest::blocking::Client>,
    /// Encoded `data:audio/wav;base64,…` URI per reference file (spec §8.2: 编码后的 URI
    /// 在内存缓存，不要每页重新读文件编码). Keyed by path: an engine instance lives for
    /// exactly one CLI invocation, within which the reference file does not change.
    uri_cache: Mutex<HashMap<PathBuf, String>>,
}

impl std::fmt::Debug for MimoEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MimoEngine")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl MimoEngine {
    pub fn with_base_url(api_key: impl Into<String>, base_url: &str) -> Self {
        MimoEngine {
            api_key: api_key.into(),
            base_url: base_url.trim_end_matches('/').to_string(),
            http: OnceLock::new(),
            uri_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Build from the stored API key (Windows Credential Manager via `credentials`).
    /// Test/override seam: `CC_MIMO_BASE_URL` replaces the endpoint (the mock-HTTP tests
    /// point it at a local server; production never sets it).
    pub fn from_credential_store() -> Result<Self> {
        let key = crate::credentials::read_api_key()?.ok_or(Error::ApiKeyMissing)?;
        let base_url =
            std::env::var("CC_MIMO_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Ok(Self::with_base_url(key, &base_url))
    }

    /// Encoded reference-audio URI, read + encoded once per file.
    fn reference_data_uri(&self, path: &Path) -> Result<String> {
        let mut cache = self.uri_cache.lock().expect("uri cache mutex poisoned");
        if let Some(uri) = cache.get(path) {
            return Ok(uri.clone());
        }
        let bytes = std::fs::read(path).map_err(|e| {
            failure(format!(
                "cannot read reference audio {}: {e}",
                path.display()
            ))
        })?;
        let uri = format!("data:audio/wav;base64,{}", BASE64.encode(&bytes));
        cache.insert(path.to_path_buf(), uri.clone());
        Ok(uri)
    }

    fn post(&self, body: &Value) -> Result<(u16, String)> {
        let client = self.http.get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .expect("building the reqwest client cannot fail")
        });
        let response = client
            .post(format!("{}/chat/completions", self.base_url))
            .header("api-key", &self.api_key)
            .json(body)
            .send()
            .map_err(|e| failure(format!("request failed: {e}")))?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .map_err(|e| failure(format!("reading the response body failed: {e}")))?;
        Ok((status, text))
    }
}

impl TtsEngine for MimoEngine {
    fn name(&self) -> &'static str {
        "mimo"
    }

    fn model(&self) -> &'static str {
        MIMO_MODEL
    }

    fn synthesize(&self, request: &TtsRequest) -> Result<Vec<u8>> {
        let ref_path = request.reference_audio.as_ref().ok_or_else(|| {
            failure("voiceclone 需要参考音频 source/voice_ref.wav（先运行 voice_configure）")
        })?;
        let data_uri = self.reference_data_uri(ref_path)?;
        let body = json!({
            "model": MIMO_MODEL,
            "messages": [
                { "role": "user", "content": request.style.as_deref().unwrap_or("") },
                { "role": "assistant", "content": request.text },
            ],
            "audio": { "format": "wav", "voice": data_uri },
        });

        let (status, text) = self.post(&body)?;
        if status == 401 {
            // the structured auth signal the fallback chain (ticket 06) and frontend
            // guidance match on — never flattened into a generic engine failure
            return Err(Error::TtsUnauthorized {
                engine: "mimo".into(),
                message: body_tail(&text),
            });
        }
        if status != 200 {
            return Err(failure(format!("HTTP {status}: {}", body_tail(&text))));
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| failure(format!("response is not JSON: {e}: {}", body_tail(&text))))?;
        let data = value
            .pointer("/choices/0/message/audio/data")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                failure(format!(
                    "response missing choices[0].message.audio.data: {}",
                    body_tail(&text)
                ))
            })?;
        BASE64
            .decode(data)
            .map_err(|e| failure(format!("audio.data is not valid base64: {e}")))
    }
}
