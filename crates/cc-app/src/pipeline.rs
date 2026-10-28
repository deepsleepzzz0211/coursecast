//! cc-app command layer for wizard steps 4–6 (ticket 13): voice configuration +
//! preview + 《声音授权确认》, person video + early matte preview, layout, and the
//! four-segment generate orchestration (合成语音 → 拼接音轨 → 处理人像 → 渲染成片).
//!
//! Same discipline as commands.rs: every `*_impl(data_dir, ...)` returns the CLI-shaped
//! JSON over the one cc-core path, so the UI runs exactly what cc-cli/cc-mcp run.
//! Progress reaches the frontend as Tauri events ("cc-progress") from cc-core's neutral
//! observer; the engine name/model forwarded by the counting adapters stay unchanged
//! so fingerprints stay identical to the unwrapped CLI runs (幂等跨壳一致).

use cc_core::error::Result;
use cc_core::tts::TtsEngine;
use cc_core::{FallbackChain, MatteEngine};
use serde_json::Value;
use std::path::Path;

/// Fixed sample sentence for 试听克隆效果 (PRD §4.2 Step3).
pub use cc_core::command::voice::{SAMPLE_TEXT, VOICE_PREVIEW_WAV};

/// Progress event sink shared by impl functions and the Tauri command layer.
pub use cc_core::command::progress::{ProgressEvent, ProgressSink as EventSink};

// ---------------------------------------------------------------------------
// Step 4 (配置声音): key / reference audio / preview / consent
// ---------------------------------------------------------------------------

/// Same semantics as `cc-cli voice_configure`: store the MiMo key in the OS credential
/// store (never into project.json, logs or this JSON) and/or normalize a reference
/// recording into source/voice_ref.wav (m4a/mp3/wav → 24 kHz mono pcm_s16le).
pub fn voice_configure_impl(
    data_dir: &Path,
    project_id: &str,
    api_key: Option<&str>,
    audio_path: Option<&str>,
    engine: Option<cc_core::TtsEngineKind>,
    edge_voice: Option<cc_core::EdgeVoice>,
) -> Result<Value> {
    cc_core::command::voice::configure(
        data_dir, project_id, api_key, audio_path, None, engine, edge_voice,
    )
}

/// Step 4 read side. `has_key` is presence only — the key itself never crosses this
/// boundary (spec 权限边界).
pub fn voice_status_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::voice::status(data_dir, project_id)
}

/// 试听克隆效果 (PRD §4.2 Step3) through the shared preview command.
pub fn voice_preview_impl(
    data_dir: &Path,
    project_id: &str,
    engine: &dyn TtsEngine,
) -> Result<Value> {
    cc_core::command::voice::preview(data_dir, project_id, engine)
}

/// Preview through the production fallback chain (same degradation policy as synthesis).
pub fn voice_preview_chain_impl(
    data_dir: &Path,
    project_id: &str,
    chain: &FallbackChain,
) -> Result<Value> {
    cc_core::command::voice::preview_chain(data_dir, project_id, chain)
}

/// Record or revoke the 《声音授权确认》 acknowledgement.
pub fn voice_consent_set_impl(
    data_dir: &Path,
    project_id: &str,
    decision: cc_core::ConsentDecision,
) -> Result<Value> {
    cc_core::command::voice::consent_set(data_dir, project_id, decision)
}

pub fn voice_consent_get_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::voice::consent_get(data_dir, project_id)
}

// ---------------------------------------------------------------------------
// Step 5 (传出镜视频): upload + early matte preview
// ---------------------------------------------------------------------------

/// Step 5: ingest the person clip through the shared command layer.
pub fn person_video_set_impl(
    data_dir: &Path,
    project_id: &str,
    source_path: &str,
) -> Result<Value> {
    cc_core::command::person::video_set(data_dir, project_id, source_path)
}

/// Step 5 "skip matte": drop any previous alpha/record so generate/render use the
/// pure-color card fallback. Persisted, so re-entering the project keeps the choice.
pub fn person_matte_skip_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::person::skip(data_dir, project_id)
}

/// Step 5 read side: video presence + matte state for the restore path.
pub fn person_status_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::person::status(data_dir, project_id)
}

/// Step 5 action: run the early matte preview with frame-level progress events (the
/// shared command is legal before tts_done; the pipeline later skips the same record).
pub fn person_matte_impl(
    data_dir: &Path,
    project_id: &str,
    engine: &mut dyn MatteEngine,
    model_path: &Path,
    on_event: EventSink,
) -> Result<Value> {
    let mut engine =
        cc_core::command::progress::count_matte_frames(engine, project_id, Some(&on_event));
    cc_core::command::person::matte_preview(
        data_dir,
        project_id,
        &mut engine,
        model_path,
        Some(&on_event),
    )
}

// ---------------------------------------------------------------------------
// Step 6 (版式 + 生成)
// ---------------------------------------------------------------------------

/// Step 6 read side: the current layout through the shared command.
pub fn layout_get_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::render::layout_get(data_dir, project_id)
}

/// Step 6 write side: partial layout update through the shared command.
pub fn layout_set_impl(
    data_dir: &Path,
    project_id: &str,
    patch: cc_core::command::render::LayoutPatch<'_>,
) -> Result<Value> {
    cc_core::command::render::layout_set(data_dir, project_id, patch)
}

/// The four-segment generate orchestration now lives in the shared command layer;
/// this wrapper only forwards the app's chain/engine/progress sink.
pub fn generate_impl(
    data_dir: &Path,
    project_id: &str,
    chain: &FallbackChain,
    matte: cc_core::command::generate::MatteChoice,
    matte_engine: &mut dyn MatteEngine,
    model_path: &Path,
    on_event: EventSink,
) -> Result<Value> {
    cc_core::command::generate::generate(
        data_dir,
        project_id,
        chain,
        matte,
        matte_engine,
        model_path,
        Some(&on_event),
    )
}

/// Step 6 read side: has the film landed (restore path for the player).
pub fn final_video_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::render::final_video(data_dir, project_id)
}

/// 下载 MP4 through the shared delivery command (existing files are never clobbered).
pub fn download_video_impl(data_dir: &Path, project_id: &str, target_dir: &Path) -> Result<Value> {
    cc_core::command::render::download(data_dir, project_id, target_dir)
}
