//! Live MiMo throughput probe: does the real endpoint process concurrent requests in
//! parallel, or does it serialize/rate-limit them?
//!
//! This is a capacity measurement, not an acceptance test. It uses the key stored in the
//! OS credential store (or the `CC_CREDENTIALS_FILE` test seam) and a real reference wav.
//! The key is never printed. Run explicitly:
//!
//! ```text
//! CC_MIMO_REF_AUDIO=data/tts-test/voice_ref.wav \
//! cargo test -p cc-core --test mimo_concurrency -- --ignored --nocapture
//! ```
//!
//! `CC_MIMO_CONCURRENCY` (default 4) controls the parallel wave size.
//!
//! Live result 2026-10-27: concurrency=4 and concurrency=2 both returned HTTP 429 on
//! one of the parallel requests (the other succeeded), while the warm-up and serial
//! requests succeeded. This account/key therefore has an effective in-flight limit of
//! one voiceclone request. The concurrency assertion is intentional: this probe is
//! expected to fail while the service rejects parallel requests.

use cc_core::mimo::MimoEngine;
use cc_core::wav;
use cc_core::{TtsEngine, TtsRequest};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::Instant;

fn reference_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CC_MIMO_REF_AUDIO") {
        return PathBuf::from(path);
    }
    // cargo runs integration tests from crates/cc-core; find the workspace sample
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            let candidate = ancestor.join("data/tts-test/voice_ref.wav");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("data/tts-test/voice_ref.wav")
}

fn concurrency() -> usize {
    std::env::var("CC_MIMO_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 2 && *v <= 8)
        .unwrap_or(4)
}

fn request(text: impl Into<String>, reference: &Path) -> TtsRequest {
    TtsRequest {
        text: text.into(),
        style: None,
        rate: 1.0,
        reference_audio: Some(reference.to_path_buf()),
    }
}

fn check_audio(bytes: &[u8]) -> f64 {
    let info = wav::parse_wav(bytes).expect("MiMo returns a valid wav");
    assert_eq!(
        (info.sample_rate, info.channels, info.bits_per_sample),
        (24_000, 1, 16)
    );
    assert!(info.duration_sec > 0.2, "unexpected silence: {info:?}");
    info.duration_sec
}

#[test]
#[ignore = "live MiMo API: reads the stored key and calls the real service"]
fn concurrent_requests_do_not_serialize_on_the_real_service() {
    let reference = reference_path();
    assert!(
        reference.is_file(),
        "reference audio not found at {} (set CC_MIMO_REF_AUDIO)",
        reference.display()
    );
    let engine = match MimoEngine::from_credential_store() {
        Ok(engine) => Arc::new(engine),
        Err(err) => panic!("cannot read the MiMo key from the credential store: {err}"),
    };
    let n = concurrency();
    let texts: Vec<String> = (1..=n)
        .map(|i| format!("课影并发吞吐测试第 {i} 段，用于确认服务端是否并行处理。"))
        .collect();

    // Warm the TLS connection / server-side cold path once so the comparison is about
    // concurrency, not first-request cold start.
    let warm_start = Instant::now();
    let warm = engine
        .synthesize(&request("课影并发测试预热。", &reference))
        .unwrap_or_else(|e| panic!("warm-up request failed: {e}"));
    eprintln!(
        "warm-up: audio={:.2}s wall={:?}",
        check_audio(&warm),
        warm_start.elapsed()
    );

    // Concurrent wave first: if it 429s, that is the answer and we do not let earlier
    // serial requests pollute the rate-limit window. Every thread waits on the same
    // barrier so the requests are actually in flight together.
    let barrier = Arc::new(Barrier::new(n));
    let concurrent_start = Instant::now();
    let handles: Vec<_> = texts
        .iter()
        .map(|text| {
            let text = text.clone();
            let engine = Arc::clone(&engine);
            let reference = reference.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let started = Instant::now();
                let result = engine.synthesize(&request(text, &reference));
                (started.elapsed(), result)
            })
        })
        .collect();
    let mut concurrent_latencies = Vec::with_capacity(n);
    let mut concurrent_errors = Vec::new();
    for handle in handles {
        let (latency, result) = handle.join().expect("worker did not panic");
        match result {
            Ok(bytes) => {
                let audio = check_audio(&bytes);
                concurrent_latencies.push(latency);
                eprintln!("concurrent: audio={audio:.2}s wall={latency:?}");
            }
            Err(err) => {
                eprintln!("concurrent: ERROR after {latency:?}: {err}");
                concurrent_errors.push(err.to_string());
            }
        }
    }
    let concurrent_wall = concurrent_start.elapsed();
    assert!(
        concurrent_errors.is_empty(),
        "concurrent wave returned {} error(s) at concurrency {n}: {concurrent_errors:?}",
        concurrent_errors.len()
    );

    // Serial baseline, for the speedup comparison only after concurrency succeeded.
    let serial_start = Instant::now();
    let mut serial_latencies = Vec::with_capacity(n);
    for text in &texts {
        let started = Instant::now();
        let bytes = engine
            .synthesize(&request(text, &reference))
            .unwrap_or_else(|e| panic!("serial request failed: {e}"));
        let audio = check_audio(&bytes);
        serial_latencies.push(started.elapsed());
        eprintln!(
            "serial: audio={audio:.2}s wall={:?}",
            serial_latencies.last().unwrap()
        );
    }
    let serial_wall = serial_start.elapsed();

    let speedup = serial_wall.as_secs_f64() / concurrent_wall.as_secs_f64().max(0.001);
    eprintln!(
        "mimo concurrency={n}: serial={serial_wall:?} concurrent={concurrent_wall:?} \
         speedup={speedup:.2}x serial_latencies={serial_latencies:?} \
         concurrent_latencies={concurrent_latencies:?}"
    );

    // A real parallel service should be materially faster than the serial sum. This is a
    // capacity probe: a failure means the endpoint serializes or rate-limits, and the
    // TTS optimizer must not assume page-level concurrency.
    assert!(
        speedup > 1.5,
        "concurrent requests did not overlap (speedup {speedup:.2}x); \
         server likely serializes or rate-limits"
    );
}
