//! Seam: the `TtsEngine` trait with the deterministic fake engine (spec Testing
//! Decisions, 缝 2: 内部缝 = TTS 引擎 trait 的测试替身). The fake engine's wav output is
//! the contract later MiMo/SAPI/edge engines must also satisfy: 24kHz mono pcm_s16le.

use cc_core::tts::{FakeTtsEngine, TtsEngine, TtsRequest};

fn request(text: &str) -> TtsRequest {
    TtsRequest {
        text: text.into(),
        style: None,
        rate: 1.0,
        reference_audio: None,
    }
}

#[test]
fn fake_engine_output_is_deterministic_and_24k_mono_pcm16() {
    let engine = FakeTtsEngine::new();
    let first = engine.synthesize(&request("第一页的讲稿内容")).unwrap();
    let second = engine.synthesize(&request("第一页的讲稿内容")).unwrap();
    assert_eq!(first, second, "same request must produce identical bytes");

    let info = cc_core::wav::parse_wav(&first).unwrap();
    assert_eq!(info.sample_rate, 24_000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.bits_per_sample, 16);
    assert!(info.duration_sec > 0.0);
}

#[test]
fn fake_engine_duration_scales_with_text_and_rate() {
    let engine = FakeTtsEngine::new();
    let short = cc_core::wav::parse_wav(&engine.synthesize(&request("短")).unwrap()).unwrap();
    let long = cc_core::wav::parse_wav(
        &engine
            .synthesize(&request("这是一段长得多的讲稿内容"))
            .unwrap(),
    )
    .unwrap();
    assert!(
        long.duration_sec > short.duration_sec,
        "longer text must produce longer audio: {} vs {}",
        long.duration_sec,
        short.duration_sec
    );

    let mut fast_request = request("这是一段长得多的讲稿内容");
    fast_request.rate = 2.0;
    let fast = cc_core::wav::parse_wav(&engine.synthesize(&fast_request).unwrap()).unwrap();
    assert!(
        (fast.duration_sec * 2.0 - long.duration_sec).abs() < 0.05,
        "rate 2.0 must halve duration: {} vs {}",
        fast.duration_sec,
        long.duration_sec
    );
}
