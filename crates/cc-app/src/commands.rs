//! Tauri command implementations (see lib.rs for the layering contract). Every
//! `*_impl(data_dir, ...)` returns the same JSON shape as the matching
//! `cc-cli --json` subcommand — one cc-core, three shells; the UI never runs a
//! second implementation of the pipeline.

use cc_core::command::script::{auto_balance, split_script};
use cc_core::Result;
use serde_json::{json, Value};
use std::path::Path;

pub use cc_core::command::error_json;

/// Step 1: create a project (the shared command layer, same as `cc-cli project create`).
pub fn create_project_impl(data_dir: &Path, name: &str, resolution: Option<&str>) -> Result<Value> {
    cc_core::command::project_create(data_dir, name, resolution)
}

/// Project list for the entry screen. Everything is read back from disk — this is
/// the "重启应用进度不丢" command: a fresh app instance lists what previous sessions
/// wrote. `progress` powers the step the wizard reopens at.
pub fn list_projects_impl(data_dir: &Path) -> Result<Value> {
    cc_core::command::project_list(data_dir)
}

/// Delete a project after the UI's explicit confirmation dialog.
pub fn delete_project_impl(
    data_dir: &Path,
    project_id: &str,
    confirmation: cc_core::command::Confirmation,
) -> Result<Value> {
    cc_core::command::project_delete(data_dir, project_id, confirmation)
}

/// Read-only status — the command layer owns the canonical input map and shared lock.
pub fn project_status_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::project_status(data_dir, project_id)
}

/// Step 2: hand a deck to the project through the shared deck command (PDF direct,
/// PPTX via local PowerPoint COM, image folder as-is).
pub fn slide_render_impl(data_dir: &Path, project_id: &str, source_path: &str) -> Result<Value> {
    cc_core::command::deck::slide_render(data_dir, project_id, source_path)
}

/// Absolute slide paths for the thumbnail grid; empty before the first render.
pub fn slides_list_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::deck::slides_list(data_dir, project_id)
}

/// Step 3 read side: the per-page texts through the shared command.
pub fn script_get_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::deck::script_get(data_dir, project_id)
}

/// Step 3 write side: whole pages through the shared script command.
pub fn script_set_impl(data_dir: &Path, project_id: &str, pages: &[String]) -> Result<Value> {
    cc_core::command::script::set_pages(data_dir, project_id, pages)
}

/// Step 3 方式 3: read the speaker notes for review (the app writes them later via
/// script_set; CLI/MCP use the same command with apply=true).
pub fn script_import_notes_impl(data_dir: &Path, project_id: &str) -> Result<Value> {
    cc_core::command::script::import_notes(
        data_dir,
        project_id,
        None,
        cc_core::command::script::ImportMode::Preview,
    )
}

/// Step 3 方式 1 helper: split a whole paste into segments.
pub fn script_split_impl(text: &str) -> Result<Value> {
    Ok(json!({ "ok": true, "segments": split_script(text) }))
}

/// Step 3 兜底 helper: 自动均分 segments onto page_count pages.
pub fn script_balance_impl(segments: &[String], page_count: usize) -> Result<Value> {
    Ok(json!({ "ok": true, "pages": auto_balance(segments.to_vec(), page_count)? }))
}
