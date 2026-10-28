//! Runtime discovery and engine construction (ticket 01): one place for ORT/RVM/
//! packaged-file resolution and for building TTS/matte engines, including the fake
//! seams. Shells pass user intent (engine preference); they do not probe the environment.

use crate::edge::EdgeVoice;
use crate::error::Result;
use crate::matte::{MatteEngine, RvmEngine, DEFAULT_MODEL_REL};
use crate::tts::{build_fallback_chain_with_voice, FakeTtsEngine, FallbackChain, TtsEngineKind};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Resolve the packaged runtime (exe-adjacent files) and the dev ORT fallback. Safe to
/// call repeatedly; explicit environment variables always win.
pub fn prepare() {
    pin_packaged_runtime();
    ensure_ort_dylib();
}

/// Packaged layout (ticket 14): ffmpeg.exe, onnxruntime.dll, the RVM model and
/// pdfium.dll sit next to the exe. Dev checkouts keep the PATH/vendor/venv rules.
fn pin_packaged_runtime() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = exe.parent() else {
        return;
    };
    for (var, file) in [
        ("CC_FFMPEG", "ffmpeg.exe"),
        ("ORT_DYLIB_PATH", "onnxruntime.dll"),
        ("CC_RVM_MODEL", "rvm_mobilenetv3_fp32.onnx"),
        ("CC_PDFIUM_DLL", "pdfium.dll"),
    ] {
        let path = dir.join(file);
        if std::env::var_os(var).is_none() && path.is_file() {
            std::env::set_var(var, &path);
        }
    }
}

/// ort is built load-dynamic: the dll comes from ORT_DYLIB_PATH or the default search
/// path. Dev checkouts find it in the main repo's Python venv by walking up.
fn ensure_ort_dylib() {
    if std::env::var_os("ORT_DYLIB_PATH").is_some_and(|p| !p.is_empty()) {
        return;
    }
    if PathBuf::from("onnxruntime.dll").is_file() {
        return;
    }
    if let Ok(cwd) = std::env::current_dir() {
        for anc in cwd.ancestors() {
            let cand = anc.join(".venv/Lib/site-packages/onnxruntime/capi/onnxruntime.dll");
            if cand.is_file() {
                std::env::set_var("ORT_DYLIB_PATH", cand);
                return;
            }
        }
    }
}

/// RVM model location: CC_RVM_MODEL, else the default relative path, else the same
/// path found by walking up from the cwd (worktrees do not carry gitignored vendor/).
pub fn model_path() -> PathBuf {
    if let Some(p) = std::env::var_os("CC_RVM_MODEL") {
        return PathBuf::from(p);
    }
    let rel = PathBuf::from(DEFAULT_MODEL_REL);
    if rel.is_file() {
        return rel;
    }
    if let Ok(cwd) = std::env::current_dir() {
        for anc in cwd.ancestors() {
            let cand = anc.join(DEFAULT_MODEL_REL);
            if cand.is_file() {
                return cand;
            }
        }
    }
    rel
}

/// Offline fake TTS engine; `CC_FAKE_TTS_DELAY_MS` is the acceptance-test seam that
/// makes kill -9 land mid-synthesis deterministically.
pub fn fake_tts_engine() -> FakeTtsEngine {
    let mut fake = FakeTtsEngine::new();
    if let Some(ms) = std::env::var("CC_FAKE_TTS_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
    {
        fake = fake.slow_first_attempts(u32::MAX, Duration::from_millis(ms));
    }
    fake
}

/// Build the TTS chain for a user preference with the default Edge voice.
/// `CC_TTS_ENGINE=fake` is the test seam and overrides the preference.
pub fn tts_chain(preference: TtsEngineKind) -> Result<FallbackChain> {
    tts_chain_with_voice(preference, None)
}

/// Build the TTS chain with an explicit Edge voice (project preference). The fake test
/// seam overrides every real preference.
pub fn tts_chain_with_voice(
    preference: TtsEngineKind,
    edge_voice: Option<EdgeVoice>,
) -> Result<FallbackChain> {
    if std::env::var("CC_TTS_ENGINE").as_deref() == Ok("fake") || preference == TtsEngineKind::Fake
    {
        return Ok(FallbackChain::single(Arc::new(fake_tts_engine())));
    }
    build_fallback_chain_with_voice(preference, edge_voice)
}

/// The matte backend: the fake seam or the real RVM engine. Calls `prepare()` so ORT is
/// resolvable even when a shell forgot the startup call.
pub fn matte_engine() -> Result<Box<dyn MatteEngine>> {
    prepare();
    if std::env::var("CC_MATTE_ENGINE").as_deref() == Ok("fake") {
        return Ok(Box::new(crate::FakeMatteEngine::new()));
    }
    Ok(Box::new(RvmEngine::load(&model_path())?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn fake_chain_is_a_single_fake_engine() {
        let chain = tts_chain(TtsEngineKind::Fake).unwrap();
        assert_eq!(chain.engines.len(), 1);
        assert_eq!(chain.engines[0].name(), "fake");
        assert_eq!(chain.engines[0].model(), "fake-sine-24k");
    }

    #[test]
    fn unknown_engine_is_a_structured_invalid_argument() {
        let err = "nope".parse::<TtsEngineKind>().unwrap_err();
        assert!(matches!(err, Error::InvalidArgument(_)));
    }

    #[test]
    fn explicit_edge_voice_changes_the_engine_model() {
        let chain = tts_chain_with_voice(TtsEngineKind::Edge, Some(EdgeVoice::Yunxi)).unwrap();
        assert_eq!(chain.engines[0].name(), "edge");
        assert_eq!(chain.engines[0].model(), "zh-CN-YunxiNeural");
    }
}
