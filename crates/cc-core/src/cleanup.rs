//! artifact_clean core seam (ticket 11, PRD §7.2): remove one stage's intermediate
//! artifacts and its project.json records so the stage re-runs from scratch (改稿后
//! 重跑前用). The project's state-machine position is left untouched — the stage
//! runners' invalidation edges (ticket 10) re-enter every runnable stage from where
//! the project sits. Deck slides (`work/slides/` + the slide_render record) are user
//! input, not intermediates, and are never removed here.

use crate::error::{Error, Result};
use crate::project::{save_project, Project};
use std::fs;
use std::path::Path;

/// Stage selectors accepted by `clean_artifacts`.
pub const CLEAN_STAGES: &[&str] = &["tts", "matte", "timeline", "render", "all"];

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CleanReport {
    /// Files/dirs actually removed, relative to the project directory.
    pub removed: Vec<String>,
    /// project.json stage-record keys removed (e.g. `tts:3`, `matte`).
    pub records_removed: Vec<String>,
}

/// Remove the artifacts + stage records of `stage`. Caller must hold the project
/// lock (exclusive) and pass the loaded project; project.json is re-saved here.
pub fn clean_artifacts(project: &mut Project, dir: &Path, stage: &str) -> Result<CleanReport> {
    if !CLEAN_STAGES.contains(&stage) {
        return Err(Error::InvalidArgument(format!(
            "unknown clean stage `{stage}`: expected one of {}",
            CLEAN_STAGES.join(" | ")
        )));
    }
    let stages: &[&str] = if stage == "all" {
        &["tts", "matte", "timeline", "render"]
    } else {
        std::slice::from_ref(&stage)
    };
    let mut report = CleanReport::default();
    for s in stages {
        clean_one(project, dir, s, &mut report)?;
    }
    if !report.records_removed.is_empty() {
        save_project(project, dir)?;
    }
    Ok(report)
}

fn clean_one(
    project: &mut Project,
    dir: &Path,
    stage: &str,
    report: &mut CleanReport,
) -> Result<()> {
    match stage {
        "tts" => {
            remove_dir_collecting(dir, "work/audio", &mut report.removed);
            remove_stage_records(project, "tts", &mut report.records_removed);
        }
        "matte" => {
            remove_dir_collecting(dir, "work/person", &mut report.removed);
            remove_stage_records(project, "matte", &mut report.records_removed);
        }
        "timeline" => {
            remove_file_collecting(dir, crate::timeline::CONCAT_TXT, &mut report.removed);
            remove_file_collecting(dir, crate::timeline::VOICE_FULL, &mut report.removed);
            remove_stage_records(project, "timeline", &mut report.records_removed);
        }
        "render" => {
            remove_file_collecting(dir, crate::render::OUTPUT_MP4, &mut report.removed);
            remove_stage_records(project, "render", &mut report.records_removed);
        }
        _ => unreachable!("validated above"),
    }
    Ok(())
}

/// Remove every `tts:{page}` / exact-stage record key, collecting what went away.
fn remove_stage_records(project: &mut Project, stage: &str, removed: &mut Vec<String>) {
    let keys: Vec<String> = project
        .stages
        .keys()
        .filter(|k| k.as_str() == stage || k.starts_with(&format!("{stage}:")))
        .cloned()
        .collect();
    for k in keys {
        project.stages.remove(&k);
        removed.push(k);
    }
}

fn remove_file_collecting(dir: &Path, rel: &str, removed: &mut Vec<String>) {
    let path = dir.join(rel);
    if fs::remove_file(&path).is_ok() {
        removed.push(rel.to_string());
    }
}

/// Delete a directory tree, collecting the file paths that went away (relative to
/// the project dir) so the report is a useful audit trail.
fn remove_dir_collecting(dir: &Path, rel: &str, removed: &mut Vec<String>) {
    let root = dir.join(rel);
    if !root.is_dir() {
        return;
    }
    collect_files(dir, &root, removed);
    let _ = fs::remove_dir_all(&root);
}

fn collect_files(dir: &Path, current: &Path, removed: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(dir, &path, removed);
        } else if let Ok(rel) = path.strip_prefix(dir) {
            removed.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}
