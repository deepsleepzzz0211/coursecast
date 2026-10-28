//! Layout, timeline, render, cleanup and film-delivery commands. The render command
//! owns one ordering policy: ensure the timeline is current, then render.

use super::Confirmation;
use crate::cleanup::{clean_artifacts, CLEAN_STAGES};
use crate::error::{Error, Result};
use crate::render::{self, Layout, Placement};
use crate::state::Stage;
use crate::timeline;
use crate::{load_project, ProjectLock};
use serde_json::{json, Value};
use std::path::Path;

/// A partial layout update: `None` keeps the current value. `position` is the legacy
/// preset form and is mutually exclusive with the free `x_pct`/`y_pct` pair.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutPatch<'a> {
    pub position: Option<&'a str>,
    pub x_pct: Option<f64>,
    pub y_pct: Option<f64>,
    pub width_pct: Option<f64>,
    pub shape: Option<&'a str>,
    pub transition: Option<&'a str>,
}

fn layout_json(layout: &Layout) -> Value {
    let mut value = match &layout.placement {
        Placement::Free { x_pct, y_pct } => json!({
            "mode": "free",
            "x_pct": x_pct,
            "y_pct": y_pct,
        }),
        Placement::Preset(position) => json!({
            "mode": "preset",
            "position": position,
        }),
    };
    value["width_pct"] = json!(layout.width_pct);
    value["shape"] = json!(layout.shape);
    value["transition"] = json!(layout.transition);
    value
}

/// Read layout.toml as the canonical command envelope.
pub fn layout_get(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let layout = render::load_layout(&dir.join(render::LAYOUT_TOML))?;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "layout_file": render::LAYOUT_TOML,
        "layout": layout_json(&layout),
    }))
}

/// Partially update layout.toml and return the canonical envelope.
pub fn layout_set(data_dir: &Path, project_id: &str, patch: LayoutPatch<'_>) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    let current = render::load_layout(&dir.join(render::LAYOUT_TOML))?;
    if patch.position.is_some() && (patch.x_pct.is_some() || patch.y_pct.is_some()) {
        return Err(Error::InvalidLayout {
            message: "position 与 x_pct/y_pct 不能同时设置；自由位置请只传 x_pct+y_pct".into(),
        });
    }
    let placement = if let Some(position) = patch.position {
        Placement::Preset(position.to_string())
    } else if patch.x_pct.is_some() || patch.y_pct.is_some() {
        let (current_x, current_y) = match &current.placement {
            Placement::Free { x_pct, y_pct } => (Some(*x_pct), Some(*y_pct)),
            Placement::Preset(_) => (None, None),
        };
        match (patch.x_pct.or(current_x), patch.y_pct.or(current_y)) {
            (Some(x), Some(y)) => Placement::Free { x_pct: x, y_pct: y },
            _ => {
                return Err(Error::InvalidLayout {
                    message: "自由位置需要同时提供 x_pct 和 y_pct（当前是预设版式）".into(),
                })
            }
        }
    } else {
        current.placement.clone()
    };
    let layout = Layout {
        placement,
        width_pct: patch.width_pct.unwrap_or(current.width_pct),
        shape: patch.shape.unwrap_or(&current.shape).to_string(),
        transition: patch.transition.unwrap_or(&current.transition).to_string(),
    };
    // save_layout round-trips through the strict parser: a bad value is rejected
    // before layout.toml is ever touched.
    render::save_layout(&layout, &dir)?;
    let next = if project.stage == Stage::Done {
        "版式已更新：运行 render 只重渲成片（课件/音频/人像不动）"
    } else {
        "版式已保存：render 时生效"
    };
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage,
        "layout_file": render::LAYOUT_TOML,
        "layout": layout_json(&layout),
        "next": next,
    }))
}

/// Build the timeline under the command boundary's lock.
pub fn timeline(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (_, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    timeline_locked(data_dir, project_id)
}

/// Lock-held form for the generate orchestration. Caller must hold the project lock.
pub fn timeline_locked(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (mut project, dir) = load_project(data_dir, project_id)?;
    let summary = timeline::run_locked(&mut project, &dir)?;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage,
        "gap_sec": summary.gap_sec,
        "fade_out_sec": summary.fade_out_sec,
        "pages": summary.pages,
        "turn_points_sec": summary.turn_points_sec,
        "total_sec": summary.total_sec,
        "fingerprint": summary.fingerprint,
        "skipped": summary.skipped,
        "artifacts": {
            "concat_txt": summary.concat_txt,
            "voice_full": summary.voice_full,
        },
        "next": "运行 person_matte 处理人像，然后 render 渲染成片",
    }))
}

/// Ensure the timeline is current, then render. This is the one ordering policy for
/// CLI, MCP and the app's generate orchestration.
pub fn render(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (_, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    render_locked(data_dir, project_id)
}

/// Ensure the timeline is current, then render. This is the one ordering policy for
/// CLI, MCP and the app's generate orchestration. Caller must hold the project lock.
pub fn render_locked(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (mut project, dir) = load_project(data_dir, project_id)?;
    timeline::run_locked(&mut project, &dir)?;
    let summary = render::run_locked(&mut project, &dir)?;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage,
        "output_path": summary.output_path,
        "total_sec": summary.total_sec,
        "turn_points_sec": summary.turn_points_sec,
        "pip": {
            "source": summary.pip_source,
            "input": summary.person_input,
        },
        "layout": layout_json(&summary.layout),
        "fingerprint": summary.fingerprint,
        "skipped": summary.skipped,
        "artifacts": { "final_mp4": summary.output_path },
        "next": "成片已生成：output/final.mp4",
    }))
}

/// Remove one stage's intermediate artifacts after explicit confirmation.
pub fn artifact_clean(
    data_dir: &Path,
    project_id: &str,
    stage: &str,
    confirmation: Confirmation,
) -> Result<Value> {
    if confirmation == Confirmation::Unconfirmed {
        return Err(Error::InvalidArgument(
            "artifact_clean 是不可逆操作：需要显式 confirm:true 才会执行".into(),
        ));
    }
    if !CLEAN_STAGES.contains(&stage) {
        return Err(Error::InvalidArgument(format!(
            "unknown clean stage `{stage}`: expected one of {}",
            CLEAN_STAGES.join(" | ")
        )));
    }
    let (mut project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    let report = clean_artifacts(&mut project, &dir, stage)?;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "stage": project.stage,
        "cleaned": stage,
        "removed": report.removed,
        "records_removed": report.records_removed,
        "next": "中间产物已清理：重跑对应阶段即可（重跑前先 project_status 查看现状）",
    }))
}

/// Report whether the final film has landed.
pub fn final_video(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let output = dir.join(render::OUTPUT_MP4);
    let done = output.is_file() && project.stage == Stage::Done;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "done": done,
        "path": render::OUTPUT_MP4,
        "path_abs": output.display().to_string(),
    }))
}

/// Byte-identical copy into `target_dir`; existing files are never clobbered.
pub fn download(data_dir: &Path, project_id: &str, target_dir: &Path) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let output = dir.join(render::OUTPUT_MP4);
    if !output.is_file() {
        return Err(Error::InvalidArgument(
            "成片尚未生成：请先在第 6 步完成生成，再下载 MP4".into(),
        ));
    }
    std::fs::create_dir_all(target_dir)?;
    let safe_name: String = project
        .name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    let mut candidate = target_dir.join(format!("课影-{safe_name}.mp4"));
    let mut n = 2;
    while candidate.is_file() {
        candidate = target_dir.join(format!("课影-{safe_name}({n}).mp4"));
        n += 1;
    }
    std::fs::copy(&output, &candidate)?;
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "saved": candidate.display().to_string(),
    }))
}
