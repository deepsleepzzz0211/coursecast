//! Person commands. Ingest is the one place that validates a clip, copies it into the
//! project and invalidates the previous matte.

use crate::command::progress::{emit, ProgressSink};
use crate::error::{Error, Result};
use crate::matte::{self, MatteEngine, MatteReport};
use crate::{load_project, ProjectLock};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Ingest a person clip after validating its extension and existence.
pub fn video_set(data_dir: &Path, project_id: &str, source_path: &str) -> Result<Value> {
    let source = PathBuf::from(source_path);
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext != "mp4" && ext != "mov" {
        return Err(Error::UnsupportedSource {
            path: source_path.to_string(),
            message: "不支持的出镜视频类型：请拖入 .mp4 或 .mov 文件（建议 30–60 秒）".into(),
        });
    }
    if !source.is_file() {
        return Err(Error::InvalidArgument(format!(
            "出镜视频不存在：{source_path}"
        )));
    }
    let (mut project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    let dest = dir.join(matte::PERSON_VIDEO);
    fs::create_dir_all(dest.parent().expect("source dir has a parent"))?;
    if source.canonicalize().ok() != dest.canonicalize().ok() {
        fs::copy(&source, &dest)?;
    }
    // replacing the clip invalidates the previous matte: otherwise a failed/skipped
    // auto-preview would leave an alpha of the OLD person for render to composite
    matte::clear_artifacts(&mut project, &dir)?;
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "path": matte::PERSON_VIDEO,
        "path_abs": dest.display().to_string(),
        "next": "自动抠像并检查棋盘格预览的边缘",
    }))
}

fn matte_json(project_id: &str, dir: &Path, report: &MatteReport, next: String) -> Value {
    json!({
        "ok": true,
        "project_id": project_id,
        "stage": report.stage,
        "skipped": report.skipped,
        "frames": report.frames,
        "width": report.width,
        "height": report.height,
        "fps": report.fps,
        "elapsed_sec": report.elapsed_sec,
        "fingerprint": report.fingerprint,
        "artifacts": report.artifacts.clone(),
        "preview": report.preview.clone(),
        "preview_abs": dir.join(matte::PREVIEW_PNG).display().to_string(),
        "alpha_abs": dir.join(matte::ALPHA_MOV).display().to_string(),
        "next": next,
    })
}

/// Full matte stage: legal from tts_done / failed / matte_done / matte_running.
pub fn matte(
    data_dir: &Path,
    project_id: &str,
    engine: &mut dyn MatteEngine,
    model_path: &Path,
    sink: Option<&ProgressSink>,
) -> Result<Value> {
    let (_, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    matte_locked(data_dir, project_id, engine, model_path, sink)
}

/// Lock-held form used by the generate orchestration. Caller must hold the lock.
pub fn matte_locked(
    data_dir: &Path,
    project_id: &str,
    engine: &mut dyn MatteEngine,
    model_path: &Path,
    sink: Option<&ProgressSink>,
) -> Result<Value> {
    let (mut project, dir) = load_project(data_dir, project_id)?;
    emit(sink, project_id, "matte", "start", None, None, None);
    match matte::run_matte_locked(&mut project, &dir, engine, model_path) {
        Ok(report) => {
            let state = if report.skipped { "skipped" } else { "done" };
            emit(
                sink,
                project_id,
                "matte",
                state,
                Some(report.frames),
                None,
                Some(format!("{state}：{} 帧", report.frames)),
            );
            Ok(matte_json(project_id, &dir, &report, report.next.clone()))
        }
        Err(err) => {
            emit(
                sink,
                project_id,
                "matte",
                "failed",
                None,
                None,
                Some(err.to_string()),
            );
            Err(err)
        }
    }
}

/// Early matte preview: legal before tts_done; writes the same fingerprint record so
/// generate skips the already-previewed work.
pub fn matte_preview(
    data_dir: &Path,
    project_id: &str,
    engine: &mut dyn MatteEngine,
    model_path: &Path,
    sink: Option<&ProgressSink>,
) -> Result<Value> {
    let (_, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    matte_preview_locked(data_dir, project_id, engine, model_path, sink)
}

/// Lock-held preview form. Caller must hold the project lock.
pub fn matte_preview_locked(
    data_dir: &Path,
    project_id: &str,
    engine: &mut dyn MatteEngine,
    model_path: &Path,
    sink: Option<&ProgressSink>,
) -> Result<Value> {
    let (mut project, dir) = load_project(data_dir, project_id)?;
    emit(sink, project_id, "matte", "start", None, None, None);
    match matte::run_matte_preview_locked(&mut project, &dir, engine, model_path) {
        Ok(report) => {
            let state = if report.skipped { "skipped" } else { "done" };
            emit(
                sink,
                project_id,
                "matte",
                state,
                Some(report.frames),
                None,
                Some(format!("{state}：{} 帧", report.frames)),
            );
            Ok(matte_json(
                project_id,
                &dir,
                &report,
                "检查边缘无误后进入下一步；失败或跳过将在生成时用卡片兜底".to_string(),
            ))
        }
        Err(err) => {
            emit(
                sink,
                project_id,
                "matte",
                "failed",
                None,
                None,
                Some(err.to_string()),
            );
            Err(err)
        }
    }
}

/// Person/matte artifact facts, shared by `person_status` and `project_status`.
pub(crate) fn status_json(project: &crate::Project, dir: &Path) -> Value {
    let video = dir.join(matte::PERSON_VIDEO);
    let matte_record = project
        .stages
        .get(matte::STAGE_KEY)
        .is_some_and(|r| r.state == "success");
    let preview_abs = dir.join(matte::PREVIEW_PNG);
    json!({
        "has_video": video.is_file(),
        "video_abs": video.display().to_string(),
        "matte_done": matte_record && preview_abs.is_file(),
        "preview_abs": preview_abs.display().to_string(),
        "alpha_abs": dir.join(matte::ALPHA_MOV).display().to_string(),
    })
}

/// Report person clip and matte artifact presence.
pub fn status(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let mut value = status_json(&project, &dir);
    value["ok"] = json!(true);
    value["project_id"] = json!(project.project_id);
    value["stage"] = json!(project.stage.to_string());
    Ok(value)
}

/// Drop the alpha/record so render uses the card fallback.
pub fn skip(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (_, dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&dir)?;
    skip_locked(data_dir, project_id)
}

/// Lock-held skip form used by the generate orchestration. Caller must hold the lock.
pub fn skip_locked(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (mut project, dir) = load_project(data_dir, project_id)?;
    matte::clear_artifacts(&mut project, &dir)?;
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "matte_done": false,
        "next": "生成时用纯色圆角卡片包住原视频（照样能出片）",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::create_project;
    use tempfile::TempDir;

    #[test]
    fn ingest_copies_a_supported_clip_and_rejects_other_extensions() {
        let data = TempDir::new().unwrap();
        let created = create_project(data.path(), "person", None).unwrap();
        let clip = data.path().join("clip.mp4");
        fs::write(&clip, b"fake mp4").unwrap();
        let v = video_set(
            data.path(),
            &created.project.project_id,
            clip.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(v["ok"], true);
        assert!(created.dir.join(matte::PERSON_VIDEO).is_file());

        let bad = data.path().join("clip.avi");
        fs::write(&bad, b"fake avi").unwrap();
        let err = video_set(
            data.path(),
            &created.project.project_id,
            bad.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::UnsupportedSource { .. }));
    }
}
