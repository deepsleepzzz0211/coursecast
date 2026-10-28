//! Deck and script read commands: deck ingest (PDF/PPTX/image folder) → slide render,
//! the slide file list, and the per-page script read. All shells call these.

use crate::command::paths;
use crate::command::pptx;
use crate::error::{Error, Result};
use crate::script;
use crate::{load_project, run_slide_render_locked, ProjectLock};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn copy_if_different(src: &Path, dst: &Path) -> Result<()> {
    let same = src.canonicalize().ok() == dst.canonicalize().ok();
    if same && dst.is_file() {
        return Ok(());
    }
    fs::copy(src, dst)?;
    Ok(())
}

/// Ingest a deck (PDF direct, PPTX via local PowerPoint COM, image folder as-is) and
/// run the slide_render stage.
pub fn slide_render(data_dir: &Path, project_id: &str, source_path: &str) -> Result<Value> {
    let (mut project, dir) = load_project(data_dir, project_id)?;
    let source = paths::resolve(&dir, Path::new(source_path));
    if !source.is_file() && !source.is_dir() {
        return Err(Error::SourceNotFound(source_path.to_string()));
    }
    let _lock = ProjectLock::acquire(&dir)?;

    let deck_pdf_rel = "source/deck.pdf";
    let deck_pdf = dir.join(deck_pdf_rel);
    let render_source: PathBuf = if source.is_dir() {
        source
    } else {
        let ext = source
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "pdf" => copy_if_different(&source, &deck_pdf)?,
            "pptx" => {
                let deck_pptx = dir.join("source/deck.pptx");
                copy_if_different(&source, &deck_pptx)?;
                if let Err(message) = pptx::convert_pptx_to_pdf(&deck_pptx, &deck_pdf) {
                    // keep the pptx (notes import works without rendering); the user
                    // resolves this by exporting a PDF from PowerPoint
                    return Err(Error::UnsupportedSource {
                        path: source.display().to_string(),
                        message: format!(
                            "pptx 本机转换失败（{message}）。请先在 PowerPoint 中另存为 PDF，再拖入该 PDF 文件"
                        ),
                    });
                }
            }
            other => {
                return Err(Error::UnsupportedSource {
                    path: source.display().to_string(),
                    message: format!("不支持的课件类型 `{other}`：请拖入 .pdf 或 .pptx 文件"),
                });
            }
        }
        PathBuf::from(deck_pdf_rel)
    };

    let outcome = run_slide_render_locked(&mut project, &dir, &render_source)?;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage.to_string(),
        "page_count": outcome.page_count,
        "slides": outcome.slides,
        "skipped": outcome.skipped,
        "fingerprint": outcome.fingerprint,
        "slides_dir": dir.join(crate::SLIDES_DIR).display().to_string(),
        "next": "填写逐字稿：整段粘贴、逐页填写或从 PPT 备注导入",
    }))
}

/// Absolute slide paths for the thumbnail grid; empty before the first render.
pub fn slides_list(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let files: Vec<String> = project
        .stages
        .get("slide_render")
        .map(|record| {
            record
                .outputs
                .iter()
                .map(|o| dir.join(&o.path).display().to_string())
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "count": files.len(),
        "files": files,
        "slides_dir": dir.join(crate::SLIDES_DIR).display().to_string(),
    }))
}

/// Per-page script read. `exists` is false until script_set (or an external editor)
/// first writes script.md; pages are plain strings so the UI and AI see the same list.
pub fn script_get(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let exists = dir.join(script::SCRIPT_FILE).is_file();
    let pages = if exists {
        script::read_script_pages(&dir)?
    } else {
        Vec::new()
    };
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage,
        "script": script::SCRIPT_FILE,
        "exists": exists,
        "pages": pages,
        "next": "运行 voice_synthesize 合成各页音频（改动只影响对应页）",
    }))
}
