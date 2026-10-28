//! The Tauri shell: every `#[tauri::command]` here is a one-liner over the
//! `*_impl` functions in commands.rs / pipeline.rs (same cc-core path as cc-cli).
//! Commands always resolve with the structured JSON — `{ok:false,error:..}` on
//! failure, exactly the shape cc-cli prints — so the frontend has one error path.
//! Long-running steps (voice / matte / generate) are async + spawn_blocking and
//! stream progress to the webview on the `cc-progress` event (票 13).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cc_app::commands::{
    create_project_impl, delete_project_impl, error_json, list_projects_impl, project_status_impl,
    script_balance_impl, script_get_impl, script_import_notes_impl, script_set_impl,
    script_split_impl, slide_render_impl, slides_list_impl,
};
use cc_app::pipeline::{
    download_video_impl, final_video_impl, generate_impl, layout_get_impl, layout_set_impl,
    person_matte_impl, person_matte_skip_impl, person_status_impl, person_video_set_impl,
    voice_configure_impl, voice_consent_get_impl, voice_consent_set_impl, voice_preview_chain_impl,
    voice_status_impl, EventSink,
};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{Emitter, Manager};

#[derive(Clone)]
struct AppState {
    data_dir: PathBuf,
}

type R = Result<Value, cc_core::Error>;

fn to_json(result: R) -> Value {
    result.unwrap_or_else(|e| error_json(&e))
}

fn progress_sink(app: &tauri::AppHandle) -> EventSink {
    let app = app.clone();
    Arc::new(move |ev| {
        let _ = app.emit("cc-progress", &ev);
    })
}

/// Run a blocking pipeline step on the async runtime; a panic maps to the same
/// structured error shape rather than killing the IPC handler.
async fn blocking<F>(f: F) -> Value
where
    F: FnOnce() -> R + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(f).await {
        Ok(result) => to_json(result),
        Err(e) => error_json(&cc_core::Error::EngineFailure {
            engine: "pipeline".into(),
            message: format!("后台任务异常退出：{e}"),
        }),
    }
}

/// Tauri 2: async commands taking borrowed inputs (State<'_, ..>) must return
/// Result; business errors already ride inside the Value, so the Err side is dead.
fn ok(value: Value) -> Result<Value, String> {
    Ok(value)
}

#[tauri::command]
fn create_project(
    state: tauri::State<'_, AppState>,
    name: String,
    resolution: Option<String>,
) -> Value {
    to_json(create_project_impl(
        &state.data_dir,
        &name,
        resolution.as_deref(),
    ))
}

#[tauri::command]
fn list_projects(state: tauri::State<'_, AppState>) -> Value {
    to_json(list_projects_impl(&state.data_dir))
}

#[tauri::command]
fn project_status(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(project_status_impl(&state.data_dir, &project_id))
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "Tauri command signature is the frontend contract"
)]
#[tauri::command]
fn delete_project(state: tauri::State<'_, AppState>, project_id: String, confirm: bool) -> Value {
    to_json(delete_project_impl(
        &state.data_dir,
        &project_id,
        confirm.into(),
    ))
}

#[tauri::command]
fn slide_render(
    state: tauri::State<'_, AppState>,
    project_id: String,
    source_path: String,
) -> Value {
    to_json(slide_render_impl(
        &state.data_dir,
        &project_id,
        &source_path,
    ))
}

#[tauri::command]
fn slides_list(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(slides_list_impl(&state.data_dir, &project_id))
}

#[tauri::command]
fn script_get(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(script_get_impl(&state.data_dir, &project_id))
}

#[tauri::command]
fn script_set(state: tauri::State<'_, AppState>, project_id: String, pages: Vec<String>) -> Value {
    to_json(script_set_impl(&state.data_dir, &project_id, &pages))
}

#[tauri::command]
fn script_import_notes(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(script_import_notes_impl(&state.data_dir, &project_id))
}

#[tauri::command]
fn script_split(text: String) -> Value {
    to_json(script_split_impl(&text))
}

#[tauri::command]
fn script_balance(segments: Vec<String>, page_count: usize) -> Value {
    to_json(script_balance_impl(&segments, page_count))
}

// ---------------------------------------------------------------------------
// Step 4（配置声音）— ticket 13
// ---------------------------------------------------------------------------

#[tauri::command]
fn voice_status(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(voice_status_impl(&state.data_dir, &project_id))
}

#[tauri::command]
async fn voice_configure(
    state: tauri::State<'_, AppState>,
    project_id: String,
    api_key: Option<String>,
    audio_path: Option<String>,
    engine: Option<String>,
    edge_voice: Option<String>,
) -> Result<Value, String> {
    let data_dir = state.data_dir.clone();
    let engine = match engine
        .as_deref()
        .map(str::parse::<cc_core::TtsEngineKind>)
        .transpose()
    {
        Ok(engine) => engine,
        Err(e) => return ok(error_json(&e)),
    };
    let edge_voice = match edge_voice
        .as_deref()
        .map(cc_core::EdgeVoice::parse)
        .transpose()
    {
        Ok(voice) => voice,
        Err(e) => return ok(error_json(&e)),
    };
    ok(blocking(move || {
        voice_configure_impl(
            &data_dir,
            &project_id,
            api_key.as_deref(),
            audio_path.as_deref(),
            engine,
            edge_voice,
        )
    })
    .await)
}

#[tauri::command]
async fn voice_preview(
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Result<Value, String> {
    let data_dir = state.data_dir.clone();
    ok(blocking(move || {
        let chain = cc_core::command::voice::chain_for_project(&data_dir, &project_id)?;
        voice_preview_chain_impl(&data_dir, &project_id, &chain)
    })
    .await)
}

#[tauri::command]
fn voice_consent_get(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(voice_consent_get_impl(&state.data_dir, &project_id))
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "Tauri command signature is the frontend contract"
)]
#[tauri::command]
fn voice_consent_set(
    state: tauri::State<'_, AppState>,
    project_id: String,
    granted: bool,
) -> Value {
    to_json(voice_consent_set_impl(
        &state.data_dir,
        &project_id,
        granted.into(),
    ))
}

// ---------------------------------------------------------------------------
// Step 5（出镜视频）— ticket 13
// ---------------------------------------------------------------------------

#[tauri::command]
fn person_status(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(person_status_impl(&state.data_dir, &project_id))
}

#[tauri::command]
fn person_video_set(
    state: tauri::State<'_, AppState>,
    project_id: String,
    source_path: String,
) -> Value {
    to_json(person_video_set_impl(
        &state.data_dir,
        &project_id,
        &source_path,
    ))
}

#[tauri::command]
fn person_matte_skip(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(person_matte_skip_impl(&state.data_dir, &project_id))
}

#[tauri::command]
async fn person_matte(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    project_id: String,
) -> Result<Value, String> {
    let data_dir = state.data_dir.clone();
    let model = cc_core::runtime::model_path();
    let sink = progress_sink(&app);
    // MatteEngine 未标 Send：引擎在 blocking 线程内构造，不跨线程传递
    ok(blocking(move || {
        let mut engine = cc_core::runtime::matte_engine()?;
        // person_matte_impl 内部已用 matte_with_frame_events 包帧级进度
        person_matte_impl(&data_dir, &project_id, engine.as_mut(), &model, sink)
    })
    .await)
}

// ---------------------------------------------------------------------------
// Step 6（版式与生成）— ticket 13
// ---------------------------------------------------------------------------

#[tauri::command]
fn layout_get(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(layout_get_impl(&state.data_dir, &project_id))
}

#[tauri::command]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command signature is the frontend contract"
)]
fn layout_set(
    state: tauri::State<'_, AppState>,
    project_id: String,
    position: Option<String>,
    x_pct: Option<f64>,
    y_pct: Option<f64>,
    width_pct: Option<f64>,
    shape: Option<String>,
    transition: Option<String>,
) -> Value {
    to_json(layout_set_impl(
        &state.data_dir,
        &project_id,
        cc_core::command::render::LayoutPatch {
            position: position.as_deref(),
            x_pct,
            y_pct,
            width_pct,
            shape: shape.as_deref(),
            transition: transition.as_deref(),
        },
    ))
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "Tauri command signature is the frontend contract"
)]
#[tauri::command]
async fn generate(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    project_id: String,
    use_matte: bool,
) -> Result<Value, String> {
    let data_dir = state.data_dir.clone();
    let model = cc_core::runtime::model_path();
    let sink = progress_sink(&app);
    // MatteEngine 未标 Send：引擎在 blocking 线程内构造
    ok(blocking(move || {
        let chain = cc_core::command::voice::chain_for_project(&data_dir, &project_id)?;
        let mut matte_engine = cc_core::runtime::matte_engine()?;
        generate_impl(
            &data_dir,
            &project_id,
            &chain,
            use_matte.into(),
            matte_engine.as_mut(),
            &model,
            sink,
        )
    })
    .await)
}

#[tauri::command]
fn final_video(state: tauri::State<'_, AppState>, project_id: String) -> Value {
    to_json(final_video_impl(&state.data_dir, &project_id))
}

#[tauri::command]
fn download_video(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    project_id: String,
) -> Value {
    // 下载目标：系统「下载」文件夹（webview 侧无权探它，也不该有权）
    let dir = match app.path().download_dir() {
        Ok(dir) => dir,
        Err(e) => {
            return error_json(&cc_core::Error::InvalidArgument(format!(
                "无法定位下载文件夹：{e}"
            )))
        }
    };
    to_json(download_video_impl(&state.data_dir, &project_id, &dir))
}

/// Same root as `app_data_dir()` (dirs::data_dir + identifier), or the test override.
/// Deliberately not `%APPDATA%` directly: the known-folder API and the env var can differ.
fn fatal_log_path(app_id: &str) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CC_DATA_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir).join("cc-app.log"));
    }
    dirs::data_dir().map(|base| base.join(app_id).join("cc-app.log"))
}

/// Release Windows has no console; keep the fatal error where support can find it.
fn write_fatal_log(app_id: &str, message: &str) {
    let Some(path) = fatal_log_path(app_id) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let stamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown-time".to_string());
    let _ = writeln!(file, "{stamp} v{} {message}", env!("CARGO_PKG_VERSION"));
}

fn main() {
    cc_core::runtime::prepare();
    let context = tauri::generate_context!();
    let app_id = context.config().identifier.clone();
    let result = tauri::Builder::default()
        .setup(|app| {
            // 重启不丢：projects live under the OS app-data dir (文件即真相)
            let data_dir = match std::env::var_os("CC_DATA_DIR").filter(|v| !v.is_empty()) {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?.join("data"),
            };
            std::fs::create_dir_all(&data_dir)?;
            app.manage(AppState { data_dir });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_project,
            list_projects,
            project_status,
            delete_project,
            slide_render,
            slides_list,
            script_get,
            script_set,
            script_import_notes,
            script_split,
            script_balance,
            voice_status,
            voice_configure,
            voice_preview,
            voice_consent_get,
            voice_consent_set,
            person_status,
            person_video_set,
            person_matte_skip,
            person_matte,
            layout_get,
            layout_set,
            generate,
            final_video,
            download_video,
        ])
        .run(context);
    if let Err(err) = result {
        let message = format!("[cc-app] fatal: {err}");
        write_fatal_log(&app_id, &message);
        eprintln!("{message}");
        std::process::exit(1);
    }
}
