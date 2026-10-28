//! edge-tts engine (ticket 07): WebSocket client for Microsoft Edge's read-aloud
//! service — the second level of the fallback chain (PRD §8.3).
//!
//! ⚠️ 此接口为逆向、非官方，随时可能被阻断：连接失败/超时/被拒都是**正常路径** —
//! they surface as structured `EngineFailure`/`EngineTimeout` errors within bounded
//! windows, and the fallback chain (ticket 06) slides down to SAPI. Nothing here may
//! panic or wait forever, and no error here is an auth failure (the service has no
//! per-user credentials; a rejection is a blocking, not a 401 — the chain must keep
//! sliding, never stop the run).
//!
//! Wire protocol pinned to the current reference implementation (rany2/edge-tts
//! master / 7.3.0): `wss://speech.platform.bing.com/…/edge/v1` with the public
//! `TrustedClientToken`, a time-derived `Sec-MS-GEC` v2 token and
//! `Sec-MS-GEC-Version: 2-…`; a `speech.config` text message, then the SSML request,
//! then binary frames of `2-byte big-endian header length + headers + audio/mpeg
//! payload` until `turn.end`. Microsoft rotated the token to v2 in Oct 2026
//! (edge-tts issue #490); the v1 token now gets HTTP 403 on the upgrade. The v2
//! digest (CS64 + SHA-256) is ported from edge-tts-rust 0.1.7 (MIT), which tracks
//! the reference algorithm; it carries Microsoft patent claims (recorded in the ADR).
//! A 403 is deterministic inside the 5-minute token window: the engine adjusts the
//! clock from the response `Date` and retries once with jitter only when that moves
//! the window; otherwise it surfaces `engine_blocked` immediately (no 0 ms retries).
//! Speech arrives as mp3 chunks and is normalized into the pipeline wav format
//! through the shared ffmpeg ingest path (tickets 05/06).
//!
//! Like MiMo, the engine is *driven* on plain threads (tts::call_with_timeout): each
//! synthesize call builds its own single-thread tokio runtime and blocks on it.

use crate::error::{Error, Result};
use crate::tts::{TtsEngine, TtsRequest};
use crate::{ffmpeg, wav};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{
    ACCEPT_ENCODING, ACCEPT_LANGUAGE, CACHE_CONTROL, COOKIE, ORIGIN, PRAGMA, USER_AGENT,
};
use tokio_tungstenite::tungstenite::http::{HeaderValue, Request};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

/// Public constant of the Edge read-aloud service (edge-tts constants.py).
const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
/// Edge/Chromium version the v2 Sec-MS-GEC scheme is pinned to (edge-tts 7.3.0).
const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";
/// Salt prefixed to the token digest (edge-tts-rust 0.1.7 / reference client).
const COPYRIGHT_PREFIX: &[u8] = b"Copyright (C) Microsoft Corporation. All rights reserved.";
const DEFAULT_WSS_URL: &str =
    "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";
/// The service's CBR mono mp3 output (24 kHz, 48 kbps) — normalization input.
const OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";
/// Connect bound (matches the reference implementation's 10 s connect timeout).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Whole-session bound: well under the request layer's 120 s page timeout, so a hung
/// service is reported as this engine's timeout before the request layer gives up.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(60);

fn failure(message: impl Into<String>) -> Error {
    Error::EngineFailure {
        engine: "edge".into(),
        message: message.into(),
    }
}

fn blocked(message: impl Into<String>) -> Error {
    Error::EngineBlocked {
        engine: "edge".into(),
        message: message.into(),
    }
}

type EdgeWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Handshake failure that still needs the HTTP status/headers (a 403 may be fixable
/// by correcting the clock; everything else is a plain engine failure).
enum ConnectRejected {
    Http { status: u16, date: Option<String> },
    Other(Error),
}

impl ConnectRejected {
    fn into_error(self) -> Error {
        match self {
            ConnectRejected::Http { status, .. } => failure(format!(
                "the service rejected the connection: HTTP {status}"
            )),
            ConnectRejected::Other(err) => err,
        }
    }
}

/// The preset Chinese voices (ticket 07 验收: 预置音色，默认 Xiaoxiao). The choice is
/// part of the engine model, so it feeds the per-page fingerprints — switching voices
/// re-synthesizes instead of reusing stale artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EdgeVoice {
    #[default]
    Xiaoxiao,
    Yunxi,
}

impl EdgeVoice {
    /// The preset key stored in project.json and accepted by the shells.
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeVoice::Xiaoxiao => "xiaoxiao",
            EdgeVoice::Yunxi => "yunxi",
        }
    }

    /// The service-side voice name.
    pub fn neural_name(self) -> &'static str {
        match self {
            EdgeVoice::Xiaoxiao => "zh-CN-XiaoxiaoNeural",
            EdgeVoice::Yunxi => "zh-CN-YunxiNeural",
        }
    }

    /// Parse an `EdgeVoice` key (case-insensitive; the `CC_EDGE_VOICE` value).
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "xiaoxiao" => Ok(EdgeVoice::Xiaoxiao),
            "yunxi" => Ok(EdgeVoice::Yunxi),
            other => Err(Error::InvalidArgument(format!(
                "invalid edge voice `{other}`: expected `xiaoxiao` or `yunxi`"
            ))),
        }
    }
}

impl std::fmt::Display for EdgeVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct EdgeEngine {
    voice: EdgeVoice,
    endpoint: String,
    connect_timeout: Duration,
    total_timeout: Duration,
    /// Seconds added to the local clock after a 403 with a server `Date` (edge-tts
    /// DRM clock-skew correction). Shared across pages of one engine instance.
    skew_seconds: Arc<AtomicI64>,
}

impl EdgeEngine {
    /// The default engine: zh-CN-XiaoxiaoNeural against the real service.
    pub fn new() -> Self {
        EdgeEngine {
            voice: EdgeVoice::Xiaoxiao,
            endpoint: DEFAULT_WSS_URL.to_string(),
            connect_timeout: CONNECT_TIMEOUT,
            total_timeout: TOTAL_TIMEOUT,
            skew_seconds: Arc::new(AtomicI64::new(0)),
        }
    }

    pub fn with_voice(voice: EdgeVoice) -> Self {
        EdgeEngine {
            voice,
            ..EdgeEngine::new()
        }
    }

    /// Test/override seam: replaces the whole WSS URL (query string included); the
    /// engine appends `ConnectionId` / `Sec-MS-GEC` / `Sec-MS-GEC-Version` to whatever
    /// is configured, mirroring `CC_MIMO_BASE_URL`'s role for mimo.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Test/override seam for the bounded-failure windows.
    pub fn with_timeouts(mut self, connect: Duration, total: Duration) -> Self {
        self.connect_timeout = connect;
        self.total_timeout = total;
        self
    }

    /// Registry constructor: the voice comes from `CC_EDGE_VOICE` (`xiaoxiao`|`yunxi`,
    /// default Xiaoxiao), the service URL from `CC_EDGE_ENDPOINT` (default: the real
    /// Microsoft endpoint — `CC_MIMO_BASE_URL`'s counterpart for tests/proxies).
    /// Construction never touches the network, so this level is always available to
    /// the fallback chain.
    pub fn from_env() -> Result<Self> {
        let voice = match std::env::var("CC_EDGE_VOICE") {
            Ok(v) => EdgeVoice::parse(&v)?,
            Err(std::env::VarError::NotPresent) => EdgeVoice::Xiaoxiao,
            Err(std::env::VarError::NotUnicode(v)) => {
                return Err(Error::InvalidArgument(format!(
                    "CC_EDGE_VOICE is not valid unicode: {v:?}"
                )));
            }
        };
        let engine = match std::env::var("CC_EDGE_ENDPOINT") {
            Ok(endpoint) => EdgeEngine::with_voice(voice).with_endpoint(endpoint),
            Err(std::env::VarError::NotPresent) => EdgeEngine::with_voice(voice),
            Err(std::env::VarError::NotUnicode(v)) => {
                return Err(Error::InvalidArgument(format!(
                    "CC_EDGE_ENDPOINT is not valid unicode: {v:?}"
                )));
            }
        };
        Ok(engine)
    }

    /// The upgrade URL: the endpoint plus the auth params of the reference protocol
    /// (edge-tts constants.py). The `TrustedClientToken` rides the query string —
    /// without it the service answers 400 on the upgrade itself.
    fn connect_url(&self) -> String {
        let sep = if self.endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        format!(
            "{endpoint}{sep}TrustedClientToken={TRUSTED_CLIENT_TOKEN}\
             &ConnectionId={conn_id}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}",
            endpoint = self.endpoint,
            sep = sep,
            conn_id = uuid::Uuid::new_v4().simple(),
            gec = sec_ms_gec_v2(self.effective_unix_seconds()),
        )
    }

    /// Local time with the DRM clock-skew correction applied.
    fn effective_unix_seconds(&self) -> u64 {
        let skew = self.skew_seconds.load(Ordering::Relaxed);
        (unix_now() as i64 + skew).max(0) as u64
    }

    /// The upgrade request with the browser-shaped headers the service expects
    /// (edge-tts constants.py WSS_HEADERS + the random `muid` cookie of 7.2.8).
    fn connect_request(&self) -> Result<Request<()>> {
        let url = self.connect_url();
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| failure(format!("invalid endpoint `{}`: {e}", self.endpoint)))?;
        let headers = request.headers_mut();
        let mut set = |name, value: &str| {
            headers.insert(
                name,
                HeaderValue::from_str(value).expect("static header values are valid"),
            );
        };
        set(PRAGMA, "no-cache");
        set(CACHE_CONTROL, "no-cache");
        set(
            ORIGIN,
            "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold",
        );
        set(
            USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0",
        );
        set(ACCEPT_ENCODING, "gzip, deflate, br, zstd");
        set(ACCEPT_LANGUAGE, "en-US,en;q=0.9");
        set(
            COOKIE,
            &format!(
                "muid={};",
                uuid::Uuid::new_v4().simple().to_string().to_uppercase()
            ),
        );
        Ok(request)
    }

    /// Establish the WebSocket. A 403 gets one clock-skew correction attempt (the
    /// edge-tts DRM rule) with a jittered delay, but only if the correction actually
    /// moves the 5-minute token window; otherwise the token would be byte-identical
    /// and retrying at 0 ms is pure hammering — surface `engine_blocked` instead.
    async fn connect(&self) -> Result<EdgeWs> {
        let token_before = sec_ms_gec_v2(self.effective_unix_seconds());
        match self.connect_once().await {
            Ok(ws) => Ok(ws),
            Err(ConnectRejected::Http { status: 403, date }) => {
                let Some(date) = date else {
                    return Err(blocked(
                        "HTTP 403 without a server Date header; the 5-minute token window cannot change",
                    ));
                };
                let Some(server_ts) = parse_http_date(&date) else {
                    return Err(blocked(format!(
                        "HTTP 403 with an unparsable server Date header `{date}`"
                    )));
                };
                let skew = server_ts - unix_now() as i64;
                if skew.abs() > 86_400 {
                    return Err(blocked(format!(
                        "HTTP 403 with a server-clock skew of {skew}s (refusing to jump the clock)"
                    )));
                }
                self.skew_seconds.fetch_add(skew, Ordering::Relaxed);
                let token_after = sec_ms_gec_v2(self.effective_unix_seconds());
                if token_after == token_before {
                    return Err(blocked(
                        "HTTP 403; the clock skew does not move the 5-minute token window (retry would reuse the same token)",
                    ));
                }
                jitter_sleep().await;
                match self.connect_once().await {
                    Ok(ws) => Ok(ws),
                    Err(ConnectRejected::Http { status: 403, .. }) => {
                        Err(blocked("HTTP 403 again after the clock-skew retry"))
                    }
                    Err(other) => Err(other.into_error()),
                }
            }
            Err(other) => Err(other.into_error()),
        }
    }

    async fn connect_once(&self) -> std::result::Result<EdgeWs, ConnectRejected> {
        let request = self.connect_request().map_err(ConnectRejected::Other)?;
        match tokio::time::timeout(self.connect_timeout, connect_async(request)).await {
            Err(_) => Err(ConnectRejected::Other(Error::EngineTimeout {
                engine: "edge".into(),
                timeout: format!("{}s (connect)", self.connect_timeout.as_secs()),
            })),
            Ok(Err(WsError::Http(response))) => {
                let status = response.status().as_u16();
                let date = response
                    .headers()
                    .get("date")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                Err(ConnectRejected::Http { status, date })
            }
            Ok(Err(other)) => Err(ConnectRejected::Other(failure(format!(
                "connect failed: {other}"
            )))),
            Ok(Ok((ws, _))) => Ok(ws),
        }
    }

    /// One WebSocket session: config → SSML → collect mp3 chunks until `turn.end`.
    async fn stream_mp3(&self, text: &str) -> Result<Vec<u8>> {
        let connected = self.connect().await?;
        let (mut writer, mut reader) = connected.split();

        let session = async {
            writer
                .send(Message::text(speech_config_message()))
                .await
                .map_err(|e| failure(format!("send failed: {e}")))?;
            writer
                .send(Message::text(ssml_request(self.voice, text)))
                .await
                .map_err(|e| failure(format!("send failed: {e}")))?;

            let mut audio = Vec::new();
            while let Some(msg) = reader.next().await {
                match msg.map_err(|e| failure(format!("connection error: {e}")))? {
                    Message::Text(t) => {
                        // turn.start / response / audio.metadata carry no audio
                        if message_path(&t) == Some("turn.end") {
                            break;
                        }
                    }
                    Message::Binary(frame) => audio.extend_from_slice(audio_payload(&frame)?),
                    Message::Ping(payload) => writer
                        .send(Message::Pong(payload))
                        .await
                        .map_err(|e| failure(format!("send failed: {e}")))?,
                    Message::Pong(_) | Message::Frame(_) => {}
                    Message::Close(_) => break,
                }
            }
            if audio.is_empty() {
                return Err(failure(
                    "no audio received: the service ended the turn without sending audio",
                ));
            }
            Ok(audio)
        };

        tokio::time::timeout(self.total_timeout, session)
            .await
            .map_err(|_| Error::EngineTimeout {
                engine: "edge".into(),
                timeout: format!("{}s (synthesis)", self.total_timeout.as_secs()),
            })?
    }

    /// mp3 stream → pipeline wav via the shared ffmpeg ingest path (private temp files
    /// only — the sanctioned SAPI-style exception to the no-filesystem engine rule —
    /// removed on every path).
    fn mp3_to_pipeline_wav(&self, mp3: &[u8]) -> Result<Vec<u8>> {
        let id = uuid::Uuid::new_v4().simple();
        let src = std::env::temp_dir().join(format!("cc-edge-{id}.mp3"));
        let dest = std::env::temp_dir().join(format!("cc-edge-{id}.wav"));
        let result = (|| {
            std::fs::write(&src, mp3)
                .map_err(|e| failure(format!("cannot write temp mp3: {e}")))?;
            ffmpeg::normalize_to_tts_wav(&src, &dest)?;
            let bytes = std::fs::read(&dest)
                .map_err(|e| failure(format!("normalized wav missing: {e}")))?;
            let info = wav::parse_wav(&bytes)?;
            if (info.sample_rate, info.channels, info.bits_per_sample)
                != (wav::TTS_SAMPLE_RATE, 1, 16)
            {
                // defensive: ffmpeg was told exactly this; a mismatch means a broken build
                return Err(failure(format!(
                    "ffmpeg produced {} Hz / {} ch / {} bit, expected 24000 Hz / 1 ch / 16 bit",
                    info.sample_rate, info.channels, info.bits_per_sample
                )));
            }
            Ok(bytes)
        })();
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dest);
        result
    }
}

impl Default for EdgeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TtsEngine for EdgeEngine {
    fn name(&self) -> &'static str {
        "edge"
    }

    fn model(&self) -> &'static str {
        self.voice.neural_name()
    }

    fn synthesize(&self, request: &TtsRequest) -> Result<Vec<u8>> {
        let mp3 = self.stream_mp3_sync(&request.text)?;
        self.mp3_to_pipeline_wav(&mp3)
    }
}

impl EdgeEngine {
    /// Bridge from the plain-thread driver into the async session: a single-thread
    /// runtime built and consumed per call (never call this inside a tokio worker).
    fn stream_mp3_sync(&self, text: &str) -> Result<Vec<u8>> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| failure(format!("cannot build the async runtime: {e}")))?;
        runtime.block_on(self.stream_mp3(text))
    }
}

/// `Sec-MS-GEC` v2 token (edge-tts 7.3.0 / edge-tts-rust 0.1.7, MIT): the Windows
/// file time of the current instant floored to the 5-minute boundary is converted to
/// 100 ns ticks, formatted ONCE as a decimal string (the same bytes feed both the
/// CS64 and SHA-256 digests — re-formatting it differently is the classic trap), and
/// rendered as `<16 hex cs64>-<64 hex sha256>`.
fn sec_ms_gec_v2(unix_seconds: u64) -> String {
    const WIN_EPOCH: u64 = 11_644_473_600;
    const WINDOW_SECONDS: u64 = 300;
    const TICKS_PER_SECOND: u64 = 10_000_000;
    let rounded = (unix_seconds + WIN_EPOCH) / WINDOW_SECONDS * WINDOW_SECONDS;
    generate_sec_ms_gec_v2_from_ticks(rounded * TICKS_PER_SECOND, TRUSTED_CLIENT_TOKEN)
}

fn generate_sec_ms_gec_v2_from_ticks(ticks: u64, token: &str) -> String {
    let str_to_hash = format!("{ticks}{token}");
    let mut hasher = Sha256::new();
    hasher.update(COPYRIGHT_PREFIX);
    hasher.update(str_to_hash.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let (w0, w1) = cs64_digest(str_to_hash.as_bytes(), &digest);
    let mut cs64_bytes = [0u8; 8];
    cs64_bytes[0..4].copy_from_slice(&w0.to_le_bytes());
    cs64_bytes[4..8].copy_from_slice(&w1.to_le_bytes());
    format!(
        "{}-{}",
        bytes_to_hex_upper(&cs64_bytes),
        bytes_to_hex_upper(&digest)
    )
}

// CS64 digest tables, ported verbatim from edge-tts-rust 0.1.7 (MIT) / the reference
// client. This is an obfuscated checksum the service validates; see the module note
// about Microsoft's patent claims.
const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u32 = 0x82893933;
const CS64_ADJUSTMENT0: u32 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A0000;

fn rol16(v: u32) -> u32 {
    v.rotate_left(16)
}

fn words_le4_even(data: &[u8]) -> Vec<u32> {
    if data.is_empty() {
        return Vec::new();
    }
    let pad_len = (4 - (data.len() % 4)) % 4;
    let full_len = data.len() - (data.len() % 4);
    let mut words = Vec::with_capacity((data.len() + pad_len) / 4 + 1);
    for chunk in data[..full_len].chunks_exact(4) {
        words.push(u32::from_le_bytes(
            chunk.try_into().expect("chunks_exact(4) yields four bytes"),
        ));
    }
    if pad_len > 0 {
        let mut tail = [0u8; 4];
        tail[..data.len() - full_len].copy_from_slice(&data[full_len..]);
        words.push(u32::from_le_bytes(tail));
    }
    if words.len() % 2 != 0 {
        words.push(0);
    }
    words
}

fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes(digest[0..4].try_into().expect("sha256 digest is 32 bytes")) | 1;
    let key_b = u32::from_le_bytes(digest[4..8].try_into().expect("sha256 digest is 32 bytes")) | 1;
    let factors = [
        key_a.wrapping_mul(CS64_FACTOR),
        key_b.wrapping_mul(CS64_FACTOR),
    ];
    let mut terms = Vec::with_capacity(14 + (data.len() + 8) / 4);
    let mut offsets = Vec::with_capacity(14 + (data.len() + 8) / 4);
    for (index, &(coefficient, offset)) in CS64_COEFFICIENTS.iter().enumerate() {
        terms.push(coefficient.wrapping_mul(factors[index % 2]));
        offsets.push(offset);
    }
    let mut prepended = Vec::with_capacity(1 + data.len());
    prepended.push(0u8);
    prepended.extend_from_slice(data);
    for (index, word) in words_le4_even(&prepended).into_iter().enumerate() {
        let key = if index % 2 == 0 { key_a } else { key_b };
        let mut term = word.wrapping_mul(key);
        let mul = if index % 2 == 0 {
            0xEF0569FB
        } else {
            0xC31713DB
        };
        let mut offset = rol16(word).wrapping_mul(mul);
        if index == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term);
        offsets.push(offset);
    }
    let (mut swap_acc, mut swap_last, mut state, mut acc) = (0u32, 0u32, 0u32, 0u32);
    for i in (0..terms.len()).step_by(2) {
        let (x, y) = (terms[i], terms[i + 1]);
        let t = rol16(offsets[i].wrapping_add(x));
        let q = t.wrapping_mul(0x76F50F34);
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q));

        let t = rol16(offsets[i + 1].wrapping_add(y));
        let q = t.wrapping_mul(0x1881CE3C);
        swap_last = rol16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = rol16(state).wrapping_mul(0xCF98B111);
        state = rol16(state).wrapping_mul(0x87085B9F);
        state = rol16(state).wrapping_mul(0x12CEB96D);
        state = rol16(state).wrapping_mul(0x257E1D83);
        acc = acc.wrapping_add(state);

        let t = state.wrapping_mul(key_b).wrapping_add(y);
        let t = rol16(t).wrapping_mul(0xA27416F5);
        let t = rol16(t).wrapping_mul(0xD38396FF);
        state = rol16(rol16(t).wrapping_mul(0x7C932B89)).wrapping_mul(0xBFA49F69);
        acc = acc.wrapping_add(state);
    }
    (swap_last ^ state, swap_acc ^ acc)
}

fn bytes_to_hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// RFC 1123 `Date` header → unix seconds (`Sun, 25 Oct 2026 17:30:00 GMT`). Only the
/// numeric fields matter; the trailing zone token is ignored (HTTP dates are GMT).
fn parse_http_date(value: &str) -> Option<i64> {
    let mut parts = value.split_whitespace();
    let _weekday = parts.next()?;
    let day: u8 = parts.next()?.trim_end_matches(',').parse().ok()?;
    let month = match parts.next()? {
        "Jan" => time::Month::January,
        "Feb" => time::Month::February,
        "Mar" => time::Month::March,
        "Apr" => time::Month::April,
        "May" => time::Month::May,
        "Jun" => time::Month::June,
        "Jul" => time::Month::July,
        "Aug" => time::Month::August,
        "Sep" => time::Month::September,
        "Oct" => time::Month::October,
        "Nov" => time::Month::November,
        "Dec" => time::Month::December,
        _ => return None,
    };
    let year: i32 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':');
    let hour: u8 = hms.next()?.parse().ok()?;
    let minute: u8 = hms.next()?.parse().ok()?;
    let second: u8 = hms.next()?.parse().ok()?;
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let time_of_day = time::Time::from_hms(hour, minute, second.min(59)).ok()?;
    Some(date.with_time(time_of_day).assume_utc().unix_timestamp())
}

/// 150–449 ms jittered delay before the single clock-skew retry: never a 0 ms retry.
async fn jitter_sleep() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    tokio::time::sleep(Duration::from_millis(150 + u64::from(nanos % 300))).await;
}

/// Best-effort clock: a broken clock yields a stale token → the service rejects → the
/// chain slides down. Never a panic.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// JavaScript-style date string for the X-Timestamp headers
/// (`%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)` in UTC).
fn js_timestamp() -> String {
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{} {} {:02} {} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
        WEEKDAYS[now.weekday().number_days_from_sunday() as usize],
        MONTHS[u8::from(now.month()) as usize - 1],
        now.day(),
        now.year(),
        now.hour(),
        now.minute(),
        now.second(),
    )
}

/// Escape element content for the SSML request.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

/// The `speech.config` text message (sentence boundaries on, the CBR mp3 output format).
fn speech_config_message() -> String {
    format!(
        "X-Timestamp:{ts}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\
         \"sentenceBoundaryEnabled\":\"true\",\"wordBoundaryEnabled\":\"false\"}},\
         \"outputFormat\":\"{OUTPUT_FORMAT}\"}}}}}}}}\r\n",
        ts = js_timestamp(),
    )
}

/// The SSML request text message with the voice and the XML-escaped page text.
fn ssml_request(voice: EdgeVoice, text: &str) -> String {
    format!(
        "X-RequestId:{req}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{ts}Z\r\nPath:ssml\r\n\r\n\
         <speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'><prosody pitch='+0Hz' rate='+0%' volume='+0%'>{text}</prosody></voice></speak>",
        req = uuid::Uuid::new_v4().simple(),
        ts = js_timestamp(),
        voice = voice.neural_name(),
        text = xml_escape(text),
    )
}

/// `Path:` header of a service text message (headers end at the first blank line).
fn message_path(text: &str) -> Option<&str> {
    let headers_end = text.find("\r\n\r\n").unwrap_or(text.len());
    text[..headers_end]
        .split("\r\n")
        .find_map(|line| line.strip_prefix("Path:"))
}

/// Payload of a binary audio frame: `2-byte BE header length + headers + mp3 data`.
/// The end-of-stream terminator frame (no Content-Type, no data) contributes nothing.
fn audio_payload(frame: &[u8]) -> Result<&[u8]> {
    if frame.len() < 2 {
        return Err(failure(
            "binary message shorter than the header length prefix",
        ));
    }
    let header_len = u16::from_be_bytes([frame[0], frame[1]]) as usize;
    // The u16 is the length of the HEADER BLOCK that follows it (reference client:
    // header_end = 2 + header_len); treating it as an absolute offset ate the last
    // two bytes of the headers and left "\r\n" at the start of the audio payload.
    let header_end = 2usize
        .checked_add(header_len)
        .filter(|end| *end <= frame.len())
        .ok_or_else(|| failure("binary header length is out of range"))?;
    let headers = std::str::from_utf8(&frame[2..header_end])
        .map_err(|e| failure(format!("binary headers are not utf-8: {e}")))?;
    let (mut path, mut content_type) = (None, None);
    for line in headers.split("\r\n").filter(|line| !line.is_empty()) {
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("Path") {
                path = Some(value.trim());
            }
            if key.eq_ignore_ascii_case("Content-Type") {
                content_type = Some(value.trim());
            }
        }
    }
    let payload = frame.get(header_end..).unwrap_or(&[]);
    let payload = payload.strip_prefix(b"\r\n").unwrap_or(payload);
    match (path, content_type, payload.is_empty()) {
        (Some("audio"), Some(ct), false) if ct.starts_with("audio/") => Ok(payload),
        (Some("audio"), None, true) => Ok(&[]),
        _ => Err(failure(format!(
            "unexpected binary message: path={path:?} content-type={content_type:?} bytes={}",
            payload.len()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_payload_reads_the_header_block_after_the_length_prefix() {
        let headers = "Path:audio\r\nContent-Type:audio/mpeg\r\nX-RequestId:abc\r\n\r\n";
        let payload = b"\xFF\xFBfake-mp3-bytes";
        let mut frame = (headers.len() as u16).to_be_bytes().to_vec();
        frame.extend_from_slice(headers.as_bytes());
        frame.extend_from_slice(payload);
        assert_eq!(audio_payload(&frame).unwrap(), payload);

        // end-of-stream terminator: Path:audio, no Content-Type, no payload
        let headers = "Path:audio\r\n\r\n";
        let mut terminator = (headers.len() as u16).to_be_bytes().to_vec();
        terminator.extend_from_slice(headers.as_bytes());
        assert!(audio_payload(&terminator).unwrap().is_empty());
    }

    #[test]
    fn audio_payload_rejects_out_of_range_header_lengths_without_panicking() {
        for frame in [
            &[0u8, 0][..],
            &[0u8, 1][..],
            &[0u8, 2][..],
            &[0u8, 9, 9][..],
        ] {
            let err = audio_payload(frame).unwrap_err();
            assert_eq!(err.code(), "engine_error", "frame {frame:?}");
        }
    }

    #[test]
    fn sec_ms_gec_v2_matches_the_reference_vector() {
        // edge-tts-rust 0.1.7 tests/protocol.rs (MIT) — the same vector the reference
        // implementation pins, so the CS64 table and salt are byte-for-byte right
        assert_eq!(
            generate_sec_ms_gec_v2_from_ticks(133_444_734_000_000_000, TRUSTED_CLIENT_TOKEN),
            "2CD23A2CA0B2595A-F3C846C370E05F9CC83ACC5F995C5F5D87A4D0F14CE8936A0F902E64099E38A2"
        );
    }

    #[test]
    fn sec_ms_gec_v2_floors_to_the_five_minute_window() {
        assert_eq!(sec_ms_gec_v2(0), sec_ms_gec_v2(42), "same window");
        assert_ne!(sec_ms_gec_v2(300), sec_ms_gec_v2(0), "next window");
        let token = sec_ms_gec_v2(0);
        let (cs64, sha) = token.split_once('-').expect("v2 format is <cs64>-<sha256>");
        assert_eq!(cs64.len(), 16);
        assert_eq!(sha.len(), 64);
        assert!(token.chars().all(|c| !c.is_ascii_lowercase() && c != ' '));
    }

    #[test]
    fn http_date_parses_rfc1123_and_rejects_garbage() {
        let expected = time::Date::from_calendar_date(2026, time::Month::October, 25)
            .unwrap()
            .with_hms(17, 30, 0)
            .unwrap()
            .assume_utc()
            .unix_timestamp();
        assert_eq!(
            parse_http_date("Sun, 25 Oct 2026 17:30:00 GMT"),
            Some(expected)
        );
        assert_eq!(parse_http_date("not a date"), None);
        assert_eq!(parse_http_date("Sun, 32 Oct 2026 17:30:00 GMT"), None);
    }
}
