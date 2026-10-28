//! Seam: the real SAPI engine (ticket 06, SAPI 兜底). Windows-only by design (spec:
//! 仅 Windows) — PowerShell System.Speech synthesizes offline, and the engine normalizes
//! its output into the 24kHz mono pcm_s16le pipeline format. This is the one engine that
//! is verified against the real local machine (the ticket's 本机 Windows 即可验证).

use cc_core::{SapiEngine, TtsEngine, TtsRequest};

fn request(text: &str) -> TtsRequest {
    TtsRequest {
        text: text.to_string(),
        style: None,
        rate: 1.0,
        reference_audio: None,
    }
}

#[test]
fn sapi_synthesizes_chinese_text_into_a_playable_24k_mono_wav() {
    let engine = SapiEngine::new();
    let bytes = engine
        .synthesize(&request("你好，这是 CourseCast 的 SAPI 兜底语音测试。"))
        .unwrap();

    // playable per the pipeline contract: parseable container, real speech duration
    let info = cc_core::wav::parse_wav(&bytes).unwrap();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (24_000, 1, 16)
    );
    assert!(
        info.duration_sec > 0.5,
        "a spoken sentence, not silence: {info:?}"
    );
}

#[test]
fn sapi_text_with_quotes_and_dollar_signs_is_read_literally_not_executed() {
    // argument-array discipline: the page text travels inside the encoded PowerShell
    // command as a single-quoted literal — quotes/dollars must never break out
    let engine = SapiEngine::new();
    let bytes = engine
        .synthesize(&request("单引号'与分号;与$env:PATH按字面处理，不执行。"))
        .unwrap();
    let info = cc_core::wav::parse_wav(&bytes).unwrap();
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (24_000, 1, 16)
    );
    assert!(info.duration_sec > 0.3, "{info:?}");
}
