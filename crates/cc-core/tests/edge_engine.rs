//! Seam: the real edge-tts engine (ticket 07, 第二级) — construction/voice config here,
//! the WebSocket protocol against a local mock server in the tests below (the real
//! speech.platform.bing.com never enters tests; PRD §8.3 marks the interface as
//! 非官方、随时可能被阻断, so blocked/timeout paths are first-class cases).

#![expect(
    clippy::result_large_err,
    reason = "mock-server closures return the engine's large Error variant"
)]

use cc_core::wav;
use cc_core::{EdgeEngine, EdgeVoice, TtsEngine, TtsRequest};
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message;

fn request(text: &str) -> TtsRequest {
    TtsRequest {
        text: text.to_string(),
        style: None,
        rate: 1.0,
        reference_audio: None,
    }
}

#[test]
fn default_voice_is_xiaoxiao_and_the_voice_lands_in_the_engine_model() {
    // model() feeds page fingerprints (tts.rs) — a voice switch must re-synthesize
    let engine = EdgeEngine::new();
    assert_eq!(engine.name(), "edge");
    assert_eq!(engine.model(), "zh-CN-XiaoxiaoNeural");

    let yunxi = EdgeEngine::with_voice(EdgeVoice::Yunxi);
    assert_eq!(yunxi.model(), "zh-CN-YunxiNeural");
    assert_ne!(engine.model(), yunxi.model());
}

#[test]
fn cc_edge_voice_env_selects_the_registry_voice() {
    // registry path (`build_fallback_chain` constructs via from_env): the env var is the
    // user-facing configuration point, mirroring CC_MIMO_BASE_URL's role for mimo
    std::env::remove_var("CC_EDGE_VOICE");
    assert_eq!(
        EdgeEngine::from_env().unwrap().model(),
        "zh-CN-XiaoxiaoNeural"
    );

    std::env::set_var("CC_EDGE_VOICE", "Yunxi");
    assert_eq!(EdgeEngine::from_env().unwrap().model(), "zh-CN-YunxiNeural");

    std::env::set_var("CC_EDGE_VOICE", "garbage");
    let err = EdgeEngine::from_env().unwrap_err();
    assert_eq!(err.code(), "invalid_argument");
    assert!(err.to_string().contains("garbage"), "{err}");

    std::env::remove_var("CC_EDGE_VOICE");
}

// ---------------------------------------------------------------------------
// Mock of the Edge read-aloud WebSocket service (127.0.0.1, one connection).
// ---------------------------------------------------------------------------

/// What one connection captured: the upgrade URI (auth params) and the text messages
/// (speech.config, then the SSML request) — the wire facts the contract pins.
#[derive(Debug, Default, Clone)]
struct Captured {
    uri: String,
    texts: Vec<String>,
}

type Capture = Arc<Mutex<Captured>>;

struct MockServer {
    port: u16,
    capture: Capture,
}

/// Bind a mock on 127.0.0.1, complete the WS handshake, and once `after_texts` text
/// messages arrived, send `responses` in order, then close. Runs its own runtime on a
/// plain thread (mirrors how the engine itself is driven off any async runtime).
fn spawn_mock(after_texts: usize, responses: Vec<Message>) -> MockServer {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let capture: Capture = Arc::default();
    let thread_capture = Arc::clone(&capture);

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let thread_capture2 = Arc::clone(&thread_capture);
            let mut ws = tokio_tungstenite::accept_hdr_async(
                stream,
                move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, resp| {
                    thread_capture2.lock().unwrap().uri = req.uri().to_string();
                    Ok(resp)
                },
            )
            .await
            .unwrap();

            let mut seen_texts = 0;
            while seen_texts < after_texts {
                let msg = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                if msg.is_text() || msg.is_binary() {
                    if msg.is_text() {
                        thread_capture
                            .lock()
                            .unwrap()
                            .texts
                            .push(msg.to_text().unwrap().into());
                    }
                    seen_texts += 1;
                }
            }
            for response in responses {
                ws.send(response).await.unwrap();
            }
            let _ = ws.close(None).await;
        });
    });

    MockServer { port, capture }
}

/// One binary audio frame: 2-byte big-endian header length, then headers, then the mp3
/// payload (the reference implementation's wire format).
fn audio_frame(payload: &[u8]) -> Message {
    let headers = "X-RequestId:0102030405060708090a0b0c0d0e0f10\r\nContent-Type:audio/mpeg\r\nPath:audio\r\n\r\n";
    let mut frame = Vec::with_capacity(2 + headers.len() + payload.len());
    frame.extend_from_slice(&(headers.len() as u16).to_be_bytes());
    frame.extend_from_slice(headers.as_bytes());
    frame.extend_from_slice(payload);
    Message::binary(frame)
}

fn service_text(body: &str) -> Message {
    Message::text(body)
}

/// A real 0.3 s mp3 (48 kbps mono, the service's CBR output format), generated locally
/// with ffmpeg — garbage payloads would fail the normalization step, not the protocol.
fn tiny_mp3(dir: &std::path::Path) -> Vec<u8> {
    let path = dir.join("tiny.mp3");
    let out = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.3",
            "-ar",
            "24000",
            "-ac",
            "1",
            "-codec:a",
            "libmp3lame",
            "-b:a",
            "48k",
        ])
        .arg("-f")
        .arg("mp3")
        .arg(path.to_str().unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ffmpeg mp3 generation failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read(&path).unwrap()
}

#[test]
fn happy_path_sends_config_and_ssml_and_normalizes_mp3_chunks_into_pipeline_wav() {
    let tmp = tempfile::tempdir().unwrap();
    let mp3 = tiny_mp3(tmp.path());
    let (a, b, c) = (
        &mp3[..mp3.len() / 3],
        &mp3[mp3.len() / 3..2 * mp3.len() / 3],
        &mp3[2 * mp3.len() / 3..],
    );
    let mock = spawn_mock(
        2,
        vec![
            audio_frame(a),
            audio_frame(b),
            audio_frame(c),
            service_text("X-RequestId:0102030405060708090a0b0c0d0e0f10\r\nPath:turn.end\r\n\r\n"),
        ],
    );

    let engine =
        EdgeEngine::new().with_endpoint(format!("ws://127.0.0.1:{}/edge/v1?Mock=1", mock.port));
    let bytes = engine
        .synthesize(&request("第一页讲稿：A & B <标记>"))
        .unwrap();

    // playable per the pipeline contract: the mp3 chunks made it through ffmpeg
    let info = wav::parse_wav(&bytes).unwrap();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (24_000, 1, 16)
    );
    assert!(
        info.duration_sec > 0.2,
        "a real 0.3s mp3, not silence: {info:?}"
    );

    let cap = mock.capture.lock().unwrap();
    // the auth params ride the upgrade URI's query (the mock endpoint keeps its own)
    assert!(
        cap.uri.starts_with("/edge/v1?Mock=1&"),
        "captured uri: {}",
        cap.uri
    );
    assert!(cap.uri.contains("ConnectionId="), "{}", cap.uri);
    assert!(cap.uri.contains("Sec-MS-GEC-Version=2-"), "{}", cap.uri);
    assert!(
        cap.uri.split("Sec-MS-GEC=").nth(1).is_some_and(|rest| {
            let token = rest.split('&').next().unwrap_or("");
            let (cs64, sha) = token.split_once('-').unwrap_or(("", ""));
            cs64.len() == 16
                && sha.len() == 64
                && token
                    .chars()
                    .all(|c| c == '-' || (c.is_ascii_hexdigit() && !c.is_ascii_lowercase()))
        }),
        "Sec-MS-GEC v2 must be <16 hex cs64>-<64 hex sha256>: {}",
        cap.uri
    );

    // first text message: the speech.config with the CBR mp3 output format
    assert!(
        cap.texts[0].contains("Path:speech.config"),
        "{}",
        cap.texts[0]
    );
    assert!(
        cap.texts[0].contains("audio-24khz-48kbitrate-mono-mp3"),
        "{}",
        cap.texts[0]
    );
    // second text message: the SSML request with the voice and XML-escaped text
    assert!(cap.texts[1].contains("Path:ssml"), "{}", cap.texts[1]);
    assert!(
        cap.texts[1].contains("zh-CN-XiaoxiaoNeural"),
        "{}",
        cap.texts[1]
    );
    assert!(
        cap.texts[1].contains("第一页讲稿：A &amp; B &lt;标记&gt;"),
        "text must be XML-escaped: {}",
        cap.texts[1]
    );
    assert!(
        !cap.texts[1].contains("<标记>"),
        "raw angle brackets must not reach the wire"
    );
}

// ---------------------------------------------------------------------------
// 故障注入（ticket 07 验收②）: 接口不稳定是常态 —— 阻断/拒绝/挂死都必须快速失败
// 为结构化错误，绝不 panic、绝不挂死；链路（tts_fallback.rs 已证）据此滑向 SAPI。
// ---------------------------------------------------------------------------

/// A port that is guaranteed (for the next moment) to refuse connections: bind, read
/// the port, drop the listener.
fn dead_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[test]
fn a_403_without_a_moving_token_window_fails_fast_and_never_retries() {
    // The service answers the upgrade with a 403 and no Date header: the 5-minute
    // token window cannot change, so a retry would send the same token. The engine
    // must surface engine_blocked immediately (the request layer then slides to SAPI)
    // and must not open a second connection.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(Mutex::new(0usize));
    let accepted_thread = Arc::clone(&accepted);
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        // serve at most two connections so a forbidden retry would be observable
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            *accepted_thread.lock().unwrap() += 1;
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        }
    });

    let engine = EdgeEngine::new().with_endpoint(format!("ws://127.0.0.1:{}/tc", port));
    let started = std::time::Instant::now();
    let err = engine.synthesize(&request("阻断场景")).unwrap_err();

    assert_eq!(
        err.code(),
        "engine_blocked",
        "structured, not a panic: {err}"
    );
    assert!(
        err.to_string().contains("403"),
        "the reason names the HTTP status: {err}"
    );
    assert!(
        started.elapsed() < CONNECT_BOUND,
        "fast fail, not a hang: {:?}",
        started.elapsed()
    );
    assert_eq!(
        *accepted.lock().unwrap(),
        1,
        "no 0 ms retry within the same token window"
    );
}

#[test]
fn connection_refused_fails_fast_with_a_structured_engine_error() {
    // nothing listens: TCP refuses, the engine must surface it as a failure quickly
    let engine = EdgeEngine::new().with_endpoint(format!("ws://127.0.0.1:{}/tc", dead_port()));
    let started = std::time::Instant::now();
    let err = engine.synthesize(&request("拒绝场景")).unwrap_err();

    assert_eq!(err.code(), "engine_error", "{err}");
    assert!(err.to_string().contains("connect failed"), "{err}");
    assert!(
        started.elapsed() < CONNECT_BOUND,
        "fast fail: {:?}",
        started.elapsed()
    );
}

#[test]
fn a_hanging_service_times_out_within_the_bounded_window() {
    // handshake + both text messages arrive, then the service never sends turn.end:
    // the session must end as a structured engine_timeout, never hang the page slot
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            // swallow both text messages, then stay silent forever
            for _ in 0..2 {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
                    .await
                    .unwrap()
                    .unwrap();
            }
            std::future::pending::<()>().await;
        });
    });

    let engine = EdgeEngine::new()
        .with_endpoint(format!("ws://127.0.0.1:{}/tc", port))
        .with_timeouts(
            std::time::Duration::from_secs(10),
            std::time::Duration::from_millis(400),
        );
    let started = std::time::Instant::now();
    let err = engine.synthesize(&request("挂死场景")).unwrap_err();

    assert_eq!(err.code(), "engine_timeout", "{err}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the bounded window closed the session: {:?}",
        started.elapsed()
    );
}

#[test]
fn turn_end_without_audio_is_a_structured_failure_not_empty_success() {
    // the service ends the turn without a single audio frame: an engine failure, not
    // an empty wav silently handed downstream
    let mock = spawn_mock(
        2,
        vec![service_text(
            "X-RequestId:0102030405060708090a0b0c0d0e0f10\r\nPath:turn.end\r\n\r\n",
        )],
    );
    let engine = EdgeEngine::new().with_endpoint(format!("ws://127.0.0.1:{}/tc", mock.port));
    let err = engine.synthesize(&request("无音频场景")).unwrap_err();
    assert_eq!(err.code(), "engine_error", "{err}");
    assert!(err.to_string().contains("no audio"), "{err}");
}

#[test]
fn cc_edge_endpoint_env_overrides_the_service_url() {
    // CC_MIMO_BASE_URL's counterpart: tests (and users behind a proxy) redirect the
    // whole service URL; the auth params are appended to whatever is configured
    let tmp = tempfile::tempdir().unwrap();
    let mp3 = tiny_mp3(tmp.path());
    let mock = spawn_mock(
        2,
        vec![
            audio_frame(&mp3),
            service_text("X-RequestId:0102030405060708090a0b0c0d0e0f10\r\nPath:turn.end\r\n\r\n"),
        ],
    );
    std::env::set_var(
        "CC_EDGE_ENDPOINT",
        format!("ws://127.0.0.1:{}/edge/v1", mock.port),
    );

    let engine = EdgeEngine::from_env().unwrap();
    let bytes = engine.synthesize(&request("环境端点覆盖")).unwrap();
    let info = wav::parse_wav(&bytes).unwrap();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (24_000, 1, 16)
    );

    let cap = mock.capture.lock().unwrap();
    assert!(
        cap.uri.starts_with(
            "/edge/v1?TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4&ConnectionId="
        ),
        "the env URL was used as-is with the auth params appended: {}",
        cap.uri
    );
    std::env::remove_var("CC_EDGE_ENDPOINT");
}

#[test]
fn an_edge_failure_slides_the_real_chain_to_the_next_level() {
    // bridge test (ticket 07 验收②: 链继续滑向 SAPI): a REAL EdgeEngine pointed at a
    // dead endpoint + the deterministic fake below it — synthesize_pages consumes the
    // engine's structured error exactly like the stub engines' (tts_fallback.rs) and
    // every page still lands. The real SAPI level is exercised by sapi_engine.rs and
    // the voice_mimo CLI e2e.
    let tmp = tempfile::tempdir().unwrap();
    let created = cc_core::create_project(tmp.path(), "edge 降级链", None).unwrap();
    let id = created.project.project_id.clone();
    std::fs::write(
        created.dir.join("script.md"),
        "第一页讲稿\n\n---\n\n第二页讲稿\n",
    )
    .unwrap();
    let (mut project, dir) = cc_core::load_project(tmp.path(), &id).unwrap();
    project.stage = cc_core::Stage::Parsed;
    cc_core::save_project(&project, &dir).unwrap();

    let blocked = EdgeEngine::new().with_endpoint(format!("ws://127.0.0.1:{}/tc", dead_port()));
    let chain = cc_core::FallbackChain {
        engines: vec![
            std::sync::Arc::new(blocked),
            std::sync::Arc::new(cc_core::FakeTtsEngine::new()),
        ],
        events: Vec::new(),
    };
    let report = cc_core::synthesize_pages(
        tmp.path(),
        &id,
        &chain,
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    assert_eq!(
        report.generated,
        vec![1, 2],
        "the run slid, it did not stop: {report:?}"
    );
    assert_eq!(report.fallbacks.len(), 1, "{:?}", report.fallbacks);
    assert_eq!(report.fallbacks[0].from, "edge");
    assert_eq!(report.fallbacks[0].to, "fake");
    assert!(
        report.fallbacks[0].reason.contains("edge"),
        "{:?}",
        report.fallbacks
    );
}

/// The engine's connect bound (edge.rs): every fast-fail test must finish well under it.
const CONNECT_BOUND: std::time::Duration = std::time::Duration::from_secs(10);

// ---------------------------------------------------------------------------
// 真实服务（ticket 07 验收①）: 网络可达时产出可播放 wav。非官方接口，阻断是常态，
// 默认忽略 —— 显式开启: cargo test -p cc-core --test edge_engine -- --ignored --nocapture
// ---------------------------------------------------------------------------

#[test]
#[ignore = "hits the real Microsoft endpoint (unofficial per PRD 8.3, may be blocked anytime); run explicitly with --ignored"]
fn real_service_produces_a_playable_wav_when_reachable() {
    let engine = EdgeEngine::new();
    let started = std::time::Instant::now();
    let bytes = match engine.synthesize(&request("课影边缘语音合成链路实测：你好，世界。"))
    {
        Ok(bytes) => bytes,
        Err(err) => panic!(
            "the real service is unreachable/blocked ({err}); this is NOT a ticket failure \
             (PRD §8.3: 非官方接口，随时可能被阻断) — the mock tests above prove the protocol path"
        ),
    };
    let info = wav::parse_wav(&bytes).unwrap();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (24_000, 1, 16)
    );
    assert!(
        info.duration_sec > 0.5,
        "real speech, not silence: {info:?}"
    );
    eprintln!(
        "real edge-tts synthesis ok: {:.2}s audio in {:?}, voice={}",
        info.duration_sec,
        started.elapsed(),
        engine.model()
    );
}

#[test]
#[ignore = "hits the real Microsoft endpoint for both preset voices; run explicitly with --ignored"]
fn real_edge_preference_path_synthesizes_both_preset_voices() {
    use cc_core::command::voice::{chain_for_project, synthesize_with_chain};
    use cc_core::tts::SynthesisOptions;
    use cc_core::{create_project, load_project, save_project, Stage, VoiceEngine};

    let tmp = tempfile::tempdir().unwrap();
    for (voice, label) in [
        (EdgeVoice::Xiaoxiao, "xiaoxiao"),
        (EdgeVoice::Yunxi, "yunxi"),
    ] {
        let created = create_project(tmp.path(), label, None).unwrap();
        let id = created.project.project_id.clone();
        std::fs::write(
            created.dir.join("script.md"),
            "课影 Edge 项目偏好实测：第一页。\n\n---\n\n课影 Edge 项目偏好实测：第二页。\n",
        )
        .unwrap();
        let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
        project.stage = Stage::Parsed;
        project.tts.engine = VoiceEngine::Edge;
        project.tts.edge_voice = voice;
        save_project(&project, &dir).unwrap();

        let chain = match chain_for_project(tmp.path(), &id) {
            Ok(chain) => chain,
            Err(err) => panic!("building the persisted Edge chain failed: {err}"),
        };
        let started = std::time::Instant::now();
        let report = match synthesize_with_chain(
            tmp.path(),
            &id,
            &chain,
            None,
            &SynthesisOptions::default(),
        ) {
            Ok(report) => report,
            Err(err) => panic!(
                "real Edge {label} synthesis failed ({err}); this is NOT a ticket failure \
                 (PRD §8.3: 非官方接口，随时可能被阻断)"
            ),
        };
        assert!(
            report.failed_pages.is_empty(),
            "{label}: failed pages: {:?}",
            report.failed_pages
        );
        assert_eq!(report.artifacts.len(), 2, "{label}: {report:?}");
        for artifact in &report.artifacts {
            let bytes = std::fs::read(dir.join(&artifact.path)).unwrap();
            let info = wav::parse_wav(&bytes).unwrap();
            assert!(info.duration_sec > 0.5, "{label}: {info:?}");
        }
        eprintln!(
            "real edge preference e2e ok: voice={label}, pages={}, audio={:.2}s in {:?}",
            report.artifacts.len(),
            report.total_duration_sec,
            started.elapsed()
        );
    }
}

/// 403 + 服务器 Date 指向另一个 5 分钟窗口：引擎校正时钟偏移、抖动退避后重试一次，
/// 第二次用不同的令牌成功建连。这是 v2 移植的“不要 0ms 重试”规则的正向路径。
#[test]
fn a_403_with_a_server_date_in_another_window_retries_once_with_a_new_token() {
    use std::io::{Read, Write};
    use std::sync::mpsc;

    let tmp = tempfile::tempdir().unwrap();
    let mp3 = tiny_mp3(tmp.path());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (uri_tx, uri_rx) = mpsc::channel::<String>();

    // A date 10 minutes ahead is always at least two 5-minute windows away.
    let server_time = time::OffsetDateTime::now_utc() + time::Duration::seconds(600);
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = format!(
        "Sun, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        server_time.day(),
        months[server_time.month() as usize - 1],
        server_time.year(),
        server_time.hour(),
        server_time.minute(),
        server_time.second()
    );

    std::thread::spawn(move || {
        // First connection: a plain 403 carrying a Date in a different token window.
        let (mut first, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = first.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8_lossy(&buf);
        let uri = text.lines().next().unwrap_or("").to_string();
        uri_tx.send(uri).unwrap();
        let response = format!(
            "HTTP/1.1 403 Forbidden\r\nDate: {date}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let _ = first.write_all(response.as_bytes());
        drop(first);

        // Second connection: a normal WebSocket session, proving the retry used a
        // corrected clock/token and actually landed audio.
        let (second, _) = listener.accept().unwrap();
        second.set_nonblocking(true).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let stream = tokio::net::TcpStream::from_std(second).unwrap();
            let mut ws = tokio_tungstenite::accept_hdr_async(
                stream,
                move |req: &tokio_tungstenite::tungstenite::handshake::server::Request, resp| {
                    uri_tx.send(req.uri().to_string()).unwrap();
                    Ok(resp)
                },
            )
            .await
            .unwrap();
            for _ in 0..2 {
                let _ = ws.next().await;
            }
            ws.send(audio_frame(&mp3)).await.unwrap();
            ws.send(service_text(
                "X-RequestId:0102030405060708090a0b0c0d0e0f10\r\nPath:turn.end\r\n\r\n",
            ))
            .await
            .unwrap();
            let _ = ws.close(None).await;
        });
    });

    let engine = EdgeEngine::new()
        .with_endpoint(format!("ws://127.0.0.1:{}/tc", port))
        .with_timeouts(
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(10),
        );
    let bytes = engine.synthesize(&request("时钟偏移重试")).unwrap();
    assert!(wav::parse_wav(&bytes).unwrap().duration_sec > 0.2);

    let first_uri = uri_rx.recv().unwrap();
    let second_uri = uri_rx.recv().unwrap();
    let token = |uri: &str| {
        uri.split("Sec-MS-GEC=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .unwrap_or("")
            .to_string()
    };
    assert_ne!(
        token(&first_uri),
        token(&second_uri),
        "the retry must use a token from the corrected window"
    );
}
