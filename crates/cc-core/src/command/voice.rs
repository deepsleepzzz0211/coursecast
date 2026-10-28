//! Voice synthesis command: one engine-policy/consent/lock/envelope path for CLI, MCP
//! and the app's generate TTS segment.

use crate::command::paths;
use crate::consent::{
    require_voice_clone_consent, set_voice_consent, voice_consent_granted, ConsentDecision,
};
use crate::edge::EdgeVoice;
use crate::error::{Error, Result};
use crate::project::{save_project, TtsPreference, VoiceEngine};
use crate::runtime;
use crate::stage;
use crate::tts::{
    synthesize_pages_locked, FallbackChain, SynthesisOptions, SynthesisReport, TtsEngine,
    TtsEngineKind, TtsRequest,
};
use crate::voice::configure_reference_audio;
use crate::{load_project, ProjectLock};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Fixed sample sentence for 试听克隆效果 (PRD §4.2 Step3).
pub const SAMPLE_TEXT: &str = "大家好，欢迎来到我的课程，这就是我克隆出来的声音。";

/// The landed preview artifact, relative to the project directory.
pub const VOICE_PREVIEW_WAV: &str = "work/voice_preview.wav";

/// Build the chain for `preference` and synthesize through the shared command path.
pub fn synthesize(
    data_dir: &std::path::Path,
    project_id: &str,
    preference: TtsEngineKind,
    pages: Option<&str>,
    options: &SynthesisOptions,
) -> Result<SynthesisReport> {
    let chain = runtime::tts_chain(preference)?;
    synthesize_with_chain(data_dir, project_id, &chain, pages, options)
}

/// Synthesize with an already-built chain (the app wraps engines for page progress).
/// The command owns the clone-engine consent gate and the project lock.
pub fn synthesize_with_chain(
    data_dir: &std::path::Path,
    project_id: &str,
    chain: &FallbackChain,
    pages: Option<&str>,
    options: &SynthesisOptions,
) -> Result<SynthesisReport> {
    let (_, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    synthesize_with_chain_locked(data_dir, project_id, chain, pages, options)
}

/// Lock-held form used by the generate orchestration, which holds one lock across all
/// four segments. Caller must hold the project lock.
pub fn synthesize_with_chain_locked(
    data_dir: &std::path::Path,
    project_id: &str,
    chain: &FallbackChain,
    pages: Option<&str>,
    options: &SynthesisOptions,
) -> Result<SynthesisReport> {
    let (_, dir) = load_project(data_dir, project_id)?;
    // 合规门：真正会走克隆引擎（有 Key 时才在链里）之前必须有《声音授权确认》
    if chain.engines.iter().any(|e| e.name() == "mimo") {
        require_voice_clone_consent(&dir)?;
    }
    synthesize_pages_locked(data_dir, project_id, chain, pages, options)
}

/// Canonical `voice_synthesize` result envelope, shared by CLI and MCP (the app's
/// generate uses the typed report and its own segment envelope).
pub fn report_json(project_id: &str, report: &SynthesisReport) -> Value {
    let mut value = json!({
        "project_id": project_id,
        "stage": report.stage,
        "generated": report.generated,
        "skipped": report.skipped,
        "failed_pages": report.failed_pages,
        "artifacts": report.artifacts,
        "total_duration_sec": report.total_duration_sec,
        "warnings": report.warnings,
        "fallbacks": report.fallbacks,
        "next": report.next,
    });
    if report.failed_pages.is_empty() {
        value["ok"] = json!(true);
    } else {
        value["ok"] = json!(false);
        value["error"] = json!({
            "code": "tts_page_failed",
            "message": format!("{} 页合成失败（详见 failed_pages）", report.failed_pages.len()),
        });
    }
    value
}

fn resolve_audio_path(dir: &Path, path: &Path) -> PathBuf {
    paths::resolve(dir, path)
}

fn next_for_engine(engine: Option<TtsEngineKind>) -> String {
    match engine {
        Some(kind) if kind != TtsEngineKind::Fake => {
            format!("运行 voice_synthesize（engine={kind}）合成各页音频")
        }
        _ => "运行 voice_synthesize 合成各页音频".to_string(),
    }
}

fn tts_json(preference: &TtsPreference) -> Value {
    json!({
        "engine": preference.engine,
        "edge_voice": preference.edge_voice,
    })
}

/// Build the fallback chain from the project's persisted voice preference (MiMo clone
/// or Edge preset). App generate/preview use this; CLI/MCP explicit `engine` overrides
/// still go through `runtime::tts_chain`.
pub fn chain_for_project(data_dir: &Path, project_id: &str) -> Result<FallbackChain> {
    let (project, _) = load_project(data_dir, project_id)?;
    let kind = match project.tts.engine {
        VoiceEngine::Mimo => TtsEngineKind::Mimo,
        VoiceEngine::Edge => TtsEngineKind::Edge,
    };
    runtime::tts_chain_with_voice(kind, Some(project.tts.edge_voice))
}

/// Configure the voice: ingest reference audio, store the API key, record the consent
/// acknowledgement, and/or persist the MiMo-vs-Edge preference. At least one action is
/// required.
pub fn configure(
    data_dir: &Path,
    project_id: &str,
    api_key: Option<&str>,
    audio_path: Option<&str>,
    consent: Option<ConsentDecision>,
    engine: Option<TtsEngineKind>,
    edge_voice: Option<EdgeVoice>,
) -> Result<Value> {
    let preference = match engine {
        Some(TtsEngineKind::Mimo) => Some(VoiceEngine::Mimo),
        Some(TtsEngineKind::Edge) => Some(VoiceEngine::Edge),
        Some(TtsEngineKind::Fake | TtsEngineKind::Sapi) | None => None,
    };
    if api_key.is_none()
        && audio_path.is_none()
        && consent.is_none()
        && preference.is_none()
        && edge_voice.is_none()
    {
        return Err(Error::InvalidArgument(
            "voice_configure 需要参考音频、API Key、consent 或配音方式至少其一".into(),
        ));
    }
    let (mut project, dir) = load_project(data_dir, project_id)?;
    let mut audio_info = None;
    if let Some(a) = audio_path {
        audio_info = Some(configure_reference_audio(
            data_dir,
            project_id,
            &resolve_audio_path(&dir, Path::new(a)),
        )?);
    }
    let mut api_key_stored = false;
    if let Some(k) = api_key {
        crate::credentials::store_api_key(k)?;
        api_key_stored = true;
    }
    if consent.is_some() || preference.is_some() || edge_voice.is_some() {
        let _lock = ProjectLock::acquire(&dir)?;
        if let Some(c) = consent {
            set_voice_consent(&dir, c)?;
        }
        let mut preference_changed = false;
        if let Some(engine) = preference {
            project.tts.engine = engine;
            preference_changed = true;
        }
        if let Some(voice) = edge_voice {
            project.tts.edge_voice = voice;
            preference_changed = true;
        }
        if preference_changed {
            save_project(&project, &dir)?;
        }
    }
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "audio": audio_info,
        "api_key_stored": api_key_stored,
        "consent": voice_consent_granted(&dir),
        "tts": tts_json(&project.tts),
        "next": next_for_engine(engine),
    }))
}

/// Voice status for the restore path. `has_key` is presence only — the key never
/// crosses this boundary.
pub fn status(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let ref_path = dir.join("source").join("voice_ref.wav");
    let ref_audio = if ref_path.is_file() {
        let info = crate::wav::parse(&ref_path)?;
        Some(json!({
            "path": "source/voice_ref.wav",
            "path_abs": ref_path.display().to_string(),
            "duration_sec": info.duration_sec,
            "sample_rate": info.sample_rate,
            "channels": info.channels,
        }))
    } else {
        None
    };
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "has_key": crate::credentials::read_api_key()?.is_some(),
        "ref_audio": ref_audio,
        "consent": voice_consent_granted(&dir),
        "tts": tts_json(&project.tts),
    }))
}

/// 试听克隆效果: synthesize the fixed sample sentence with `engine` and land it at
/// work/voice_preview.wav for the embedded player.
pub fn preview(data_dir: &Path, project_id: &str, engine: &dyn TtsEngine) -> Result<Value> {
    let (_project, dir) = load_project(data_dir, project_id)?;
    if engine.name() == "mimo" {
        require_voice_clone_consent(&dir)?;
    }
    let ref_path = dir.join("source").join("voice_ref.wav");
    let request = TtsRequest {
        text: SAMPLE_TEXT.to_string(),
        style: None,
        rate: 1.0,
        reference_audio: ref_path.is_file().then(|| ref_path.clone()),
    };
    let bytes = engine.synthesize(&request)?;
    preview_result(project_id, &dir, &bytes)
}

/// Preview through the same fallback chain as synthesis: try engines in order and
/// land the first result. A chain that includes MiMo requires the consent gate.
pub fn preview_chain(data_dir: &Path, project_id: &str, chain: &FallbackChain) -> Result<Value> {
    let (_project, dir) = load_project(data_dir, project_id)?;
    if chain.engines.iter().any(|e| e.name() == "mimo") {
        require_voice_clone_consent(&dir)?;
    }
    let ref_path = dir.join("source").join("voice_ref.wav");
    let request = TtsRequest {
        text: SAMPLE_TEXT.to_string(),
        style: None,
        rate: 1.0,
        reference_audio: ref_path.is_file().then(|| ref_path.clone()),
    };
    let mut last_error = None;
    for engine in &chain.engines {
        match engine.synthesize(&request) {
            Ok(bytes) => return preview_result(project_id, &dir, &bytes),
            Err(err) => last_error = Some(err),
        }
    }
    Err(last_error.unwrap_or_else(|| Error::InvalidArgument("没有可用的语音引擎，无法试听".into())))
}

fn preview_result(project_id: &str, dir: &Path, bytes: &[u8]) -> Result<Value> {
    let info = crate::wav::parse_wav(bytes)?;
    let abs = dir.join(VOICE_PREVIEW_WAV);
    std::fs::create_dir_all(abs.parent().expect("work dir has a parent"))?;
    stage::write_atomic(&abs, bytes)?;
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "text": SAMPLE_TEXT,
        "path": VOICE_PREVIEW_WAV,
        "path_abs": abs.display().to_string(),
        "duration_sec": info.duration_sec,
        "sample_rate": info.sample_rate,
        "channels": info.channels,
    }))
}

/// Record or revoke the voice-clone acknowledgement.
pub fn consent_set(data_dir: &Path, project_id: &str, decision: ConsentDecision) -> Result<Value> {
    let (_project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    set_voice_consent(&dir, decision)?;
    let granted = decision == ConsentDecision::Grant;
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "granted": granted,
        "next": if granted {
            "运行 voice_synthesize --engine mimo 合成各页音频"
        } else {
            "请先勾选《声音授权确认》再合成"
        },
    }))
}

/// Read the voice-clone acknowledgement.
pub fn consent_get(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (_project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let granted = voice_consent_granted(&dir);
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "granted": granted,
        "next": if granted {
            "运行 voice_synthesize --engine mimo 合成各页音频"
        } else {
            "请先勾选《声音授权确认》再合成"
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_project, load_project, EdgeVoice, VoiceEngine};
    use tempfile::TempDir;

    fn project(tmp: &TempDir) -> String {
        create_project(tmp.path(), "声音偏好", None)
            .unwrap()
            .project
            .project_id
    }

    #[test]
    fn configure_persists_the_edge_voice_and_chain_uses_it() {
        let tmp = TempDir::new().unwrap();
        let id = project(&tmp);
        let v = configure(
            tmp.path(),
            &id,
            None,
            None,
            None,
            Some(TtsEngineKind::Edge),
            Some(EdgeVoice::Yunxi),
        )
        .unwrap();
        assert_eq!(v["tts"]["engine"], "edge");
        assert_eq!(v["tts"]["edge_voice"], "yunxi");

        let (saved, _) = load_project(tmp.path(), &id).unwrap();
        assert_eq!(saved.tts.engine, VoiceEngine::Edge);
        assert_eq!(saved.tts.edge_voice, EdgeVoice::Yunxi);

        let chain = chain_for_project(tmp.path(), &id).unwrap();
        assert_eq!(chain.engines[0].name(), "edge");
        assert_eq!(chain.engines[0].model(), "zh-CN-YunxiNeural");
    }

    #[test]
    fn configure_requires_at_least_one_action() {
        let tmp = TempDir::new().unwrap();
        let id = project(&tmp);
        let err = configure(tmp.path(), &id, None, None, None, None, None).unwrap_err();
        assert_eq!(err.code(), "invalid_argument");
    }

    #[test]
    fn voice_only_change_keeps_the_current_engine() {
        let tmp = TempDir::new().unwrap();
        let id = project(&tmp);
        configure(
            tmp.path(),
            &id,
            None,
            None,
            None,
            Some(TtsEngineKind::Edge),
            Some(EdgeVoice::Xiaoxiao),
        )
        .unwrap();
        configure(
            tmp.path(),
            &id,
            None,
            None,
            None,
            None,
            Some(EdgeVoice::Yunxi),
        )
        .unwrap();
        let (saved, _) = load_project(tmp.path(), &id).unwrap();
        assert_eq!(saved.tts.engine, VoiceEngine::Edge);
        assert_eq!(saved.tts.edge_voice, EdgeVoice::Yunxi);
    }
}
