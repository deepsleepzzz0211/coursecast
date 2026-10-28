//! Script commands: split/balance helpers, partial page writes, and PPT notes import.
//! The app previews notes and writes later; CLI/MCP apply them in one call.

use crate::command::paths;
use crate::error::{Error, Result};
use crate::script::{import_pptx_notes, update_script_pages, SCRIPT_FILE};
use crate::slide::slide_page_count;
use crate::{load_project, ProjectLock};
use serde_json::{json, Value};
use std::path::Path;

/// Split a whole-pasted script into segments. `---` divider lines take priority;
/// otherwise `第N页` markers separate; otherwise the whole paste is one segment.
pub fn split_script(text: &str) -> Vec<String> {
    let has_dash = text.lines().any(|l| l.trim() == "---");
    let mut raw: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        let t = line.trim();
        let is_sep = if has_dash {
            t == "---"
        } else {
            is_page_marker(t)
        };
        if is_sep {
            raw.push(std::mem::take(&mut current));
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    raw.push(current);
    let mut segs: Vec<String> = raw.into_iter().map(|s| s.trim().to_string()).collect();
    while segs.last().is_some_and(|s| s.is_empty()) {
        segs.pop();
    }
    while segs.first().is_some_and(|s| s.is_empty()) {
        segs.remove(0);
    }
    segs
}

fn is_page_marker(line: &str) -> bool {
    let s = line.trim_start_matches('#').trim();
    if !(s.starts_with('第') && s.ends_with('页') && s.len() > 6) {
        return false;
    }
    let mid = &s[3..s.len() - 3];
    mid.chars().any(|c| c.is_ascii_digit())
        && mid.chars().all(|c| c.is_ascii_digit() || c.is_whitespace())
}

/// The "自动均分" fallback: N segments onto M pages.
pub fn auto_balance(segments: Vec<String>, page_count: usize) -> Result<Vec<String>> {
    if page_count == 0 {
        return Err(Error::InvalidArgument("page_count 必须大于 0".into()));
    }
    let n = segments.len();
    let mut pages = Vec::with_capacity(page_count);
    for i in 0..page_count {
        if n <= page_count {
            pages.push(segments.get(i).cloned().unwrap_or_default());
        } else {
            let start = i * n / page_count;
            let end = (i + 1) * n / page_count;
            pages.push(segments[start..end].join("\n"));
        }
    }
    Ok(pages)
}

/// Parse the JSON page-update argument shared by CLI and MCP:
/// `[{page: 1-based u32, text: string}]`. Page bounds are validated later against the
/// rendered slide count.
pub fn parse_updates(value: &Value) -> Result<Vec<(u32, String)>> {
    let raw = value
        .as_array()
        .ok_or_else(|| Error::InvalidArgument("pages 需要数组".into()))?;
    if raw.is_empty() {
        return Err(Error::InvalidArgument("pages 不能为空数组".into()));
    }
    let mut updates = Vec::with_capacity(raw.len());
    for item in raw {
        let page = item
            .get("page")
            .and_then(Value::as_u64)
            .filter(|p| *p >= 1 && *p <= u32::MAX as u64)
            .ok_or_else(|| Error::InvalidArgument("每页需要 1-based 整数 `page`".into()))?;
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidArgument("每页需要字符串 `text`".into()))?;
        updates.push((page as u32, text.to_string()));
    }
    Ok(updates)
}

fn page_result(project: &crate::Project, pages: Vec<String>, next: &str) -> Value {
    json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage,
        "script": SCRIPT_FILE,
        "page_count": pages.len(),
        "pages": pages,
        "next": next,
    })
}

/// Partial page update (CLI/MCP): page numbers are validated against the rendered
/// slide count.
pub fn set(data_dir: &Path, project_id: &str, updates: &[(u32, String)]) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    let page_count = slide_page_count(&project)?;
    let pages = update_script_pages(&dir, updates, page_count)?;
    Ok(page_result(
        &project,
        pages,
        "运行 voice_synthesize 合成各页音频（改动只影响对应页）",
    ))
}

/// Whole-page write (app): slides are optional; empty pages are legal.
pub fn set_pages(data_dir: &Path, project_id: &str, pages: &[String]) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    let page_count = slide_page_count(&project)
        .unwrap_or(pages.len() as u32)
        .max(pages.len() as u32);
    let updates: Vec<(u32, String)> = pages
        .iter()
        .enumerate()
        .map(|(i, text)| ((i + 1) as u32, text.clone()))
        .collect();
    let pages = update_script_pages(&dir, &updates, page_count)?;
    Ok(page_result(
        &project,
        pages,
        "进入下一步配置声音（后续版本开放）",
    ))
}

/// Whether imported notes are only previewed or written to script.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Preview,
    Apply,
}

/// Read PPT speaker notes. `ImportMode::Apply` writes them through the same page-merge
/// path (CLI/MCP); `ImportMode::Preview` returns them for the app's review step.
pub fn import_notes(
    data_dir: &Path,
    project_id: &str,
    pptx_path: Option<&str>,
    mode: ImportMode,
) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    let pptx = match pptx_path {
        Some(p) => paths::resolve(&dir, Path::new(p)),
        None => dir.join("source").join("deck.pptx"),
    };
    if !pptx.is_file() {
        return Err(Error::InvalidArgument(
            "未找到 source/deck.pptx：PPT 备注只能从 pptx 课件导入，PDF 输入无此能力（请改用整段粘贴或逐页填写）".into(),
        ));
    }
    let notes = import_pptx_notes(&pptx)?;
    let imported = notes.iter().filter(|p| !p.is_empty()).count();
    if mode == ImportMode::Preview {
        return Ok(json!({
            "ok": true,
            "project_id": project.project_id,
            "stage": project.stage,
            "script": SCRIPT_FILE,
            "page_count": notes.len(),
            "pages": notes,
            "imported_pages": imported,
            "written": false,
            "next": "核对备注后通过 script_set 写入",
        }));
    }
    let page_count = slide_page_count(&project)?;
    let updates: Vec<(u32, String)> = notes
        .into_iter()
        .enumerate()
        .map(|(i, text)| ((i + 1) as u32, text))
        .collect();
    let pages = update_script_pages(&dir, &updates, page_count)?;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage,
        "script": SCRIPT_FILE,
        "page_count": pages.len(),
        "pages": pages,
        "imported_pages": imported,
        "written": true,
        "next": "检查导入的逐字稿，然后运行 voice_synthesize 合成各页音频",
    }))
}
