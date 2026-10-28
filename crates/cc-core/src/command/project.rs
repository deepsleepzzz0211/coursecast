//! Project commands: create, list, status and delete. These are the canonical shapes
//! shared by the CLI, MCP and the Tauri shell.

use super::Confirmation;
use crate::error::{Error, Result};
use crate::status::compute_status;
use crate::{create_project, load_project, ProjectLock};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

/// Create a project and return the canonical command envelope.
pub fn project_create(data_dir: &Path, name: &str, resolution: Option<&str>) -> Result<Value> {
    let created = create_project(data_dir, name, resolution)?;
    Ok(json!({
        "ok": true,
        "project_id": created.project.project_id,
        "name": created.project.name,
        "resolution": created.project.resolution,
        "stage": created.project.stage.to_string(),
        "dir": created.dir.display().to_string(),
        "next": "将课件放入 source/deck.pdf（或 .pptx），然后运行 slide_render",
    }))
}

/// List every project on disk, newest first. Read-only: no project lock is needed
/// because `project.json` is written atomically.
pub fn project_list(data_dir: &Path) -> Result<Value> {
    let root = data_dir.join("projects");
    let mut entries: Vec<(String, Value)> = Vec::new();
    if root.is_dir() {
        for dir in fs::read_dir(&root)? {
            let dir = dir?.path();
            if !dir.join("project.json").is_file() {
                continue;
            }
            let Some(id) = dir.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Ok((project, pdir)) = load_project(data_dir, id) else {
                continue;
            };
            entries.push((
                project.created_at.clone(),
                json!({
                    "project_id": project.project_id,
                    "name": project.name,
                    "resolution": project.resolution,
                    "stage": project.stage.to_string(),
                    "created_at": project.created_at,
                    "failed": project.failed,
                    "progress": {
                        "slides": project.stages.get("slide_render")
                            .is_some_and(|r| r.state == "success"),
                        "script": pdir.join(crate::script::SCRIPT_FILE).is_file(),
                        // resume checkpoints: the app lands on Step5/Step6 when these exist.
                        // A failed render can leave a partial final.mp4; "film" must follow
                        // the same predicate as final_video (stage done + file).
                        "person": pdir.join(crate::matte::PERSON_VIDEO).is_file(),
                        "film": project.stage == crate::Stage::Done
                            && pdir.join(crate::render::OUTPUT_MP4).is_file(),
                    },
                }),
            ));
        }
    }
    // newest first (RFC3339 created_at sorts lexicographically)
    entries.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(json!({
        "ok": true,
        "projects": entries.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
    }))
}

/// Delete a project directory after explicit confirmation. The project lock is held
/// while the contents are removed (Windows cannot delete or rename an open `.lock`
/// file); `project.json` goes first so an interrupted delete never lists as a project.
pub fn project_delete(
    data_dir: &Path,
    project_id: &str,
    confirmation: Confirmation,
) -> Result<Value> {
    if confirmation == Confirmation::Unconfirmed {
        return Err(Error::InvalidArgument(
            "project_delete 是不可逆操作：需要显式 confirm:true 才会执行".into(),
        ));
    }
    let (project, dir) = load_project(data_dir, project_id)?;
    let lock = ProjectLock::acquire(&dir)?;
    fs::remove_file(dir.join("project.json"))?;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if entry.file_name().to_str() == Some(ProjectLock::LOCK_FILE) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    drop(lock);
    fs::remove_dir_all(&dir)?;
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "name": project.name,
        "deleted": true,
        "dir": dir.display().to_string(),
        "next": "项目已删除，不可恢复",
    }))
}

/// Read the canonical project status envelope.
pub fn project_status(data_dir: &Path, project_id: &str) -> Result<Value> {
    let (project, dir) = load_project(data_dir, project_id)?;
    // read-only command: shared lock so concurrent status calls coexist
    let _lock = ProjectLock::acquire_shared(&dir)?;
    let report = compute_status(&project, &dir);
    let inputs: Map<String, Value> = report
        .inputs
        .iter()
        .map(|i| {
            (
                i.name.to_string(),
                json!({ "path": i.path, "ready": i.ready }),
            )
        })
        .collect();
    Ok(json!({
        "ok": true,
        "project_id": project.project_id,
        "name": project.name,
        "stage": project.stage.to_string(),
        "failed": project.failed,
        "inputs": inputs,
        "person": super::person::status_json(&project, &dir),
        "missing": report.missing,
        "next": report.next,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_returns_name_resolution_and_dir() {
        let tmp = TempDir::new().unwrap();
        let v = project_create(tmp.path(), "药品包装", None).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["name"], "药品包装");
        assert_eq!(v["resolution"], "1920x1080");
        assert_eq!(v["stage"], "created");
        assert!(v["dir"].as_str().unwrap().contains("projects"));
    }

    #[test]
    fn status_inputs_are_a_name_keyed_object() {
        let tmp = TempDir::new().unwrap();
        let created = project_create(tmp.path(), "状态", None).unwrap();
        let id = created["project_id"].as_str().unwrap();
        let v = project_status(tmp.path(), id).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["stage"], "created");
        let inputs = v["inputs"].as_object().expect("inputs must be an object");
        for key in ["deck", "script", "voice_ref", "person"] {
            assert!(inputs.contains_key(key), "missing input {key}");
            assert_eq!(inputs[key]["ready"], false);
        }
        assert_eq!(v["person"]["has_video"], false);
        assert_eq!(v["person"]["matte_done"], false);
        assert!(!v["next"].as_str().unwrap().is_empty());
    }

    #[test]
    fn list_returns_every_project_with_progress_facts() {
        let tmp = TempDir::new().unwrap();
        let first = project_create(tmp.path(), "第一门", None).unwrap();
        let second = project_create(tmp.path(), "第二门", Some("1280x720")).unwrap();
        let v = project_list(tmp.path()).unwrap();
        assert_eq!(v["ok"], true);
        let projects = v["projects"].as_array().expect("projects array");
        assert_eq!(projects.len(), 2);
        for id in [
            first["project_id"].as_str().unwrap(),
            second["project_id"].as_str().unwrap(),
        ] {
            let entry = projects
                .iter()
                .find(|p| p["project_id"] == id)
                .expect("created project is listed");
            assert_eq!(entry["progress"]["slides"], false);
            assert_eq!(entry["progress"]["script"], false);
            assert_eq!(entry["progress"]["person"], false);
            assert_eq!(entry["progress"]["film"], false);
        }
        assert_eq!(
            project_list(&tmp.path().join("missing")).unwrap()["projects"],
            json!([])
        );
    }

    #[test]
    fn delete_requires_confirmation_and_removes_the_project() {
        let tmp = TempDir::new().unwrap();
        let created = project_create(tmp.path(), "待删除", None).unwrap();
        let id = created["project_id"].as_str().unwrap();
        let dir = Path::new(created["dir"].as_str().unwrap());

        let err = project_delete(tmp.path(), id, Confirmation::Unconfirmed).unwrap_err();
        assert_eq!(err.code(), "invalid_argument");
        assert!(
            dir.is_dir(),
            "unconfirmed delete must not touch the directory"
        );

        let v = project_delete(tmp.path(), id, Confirmation::Confirmed).unwrap();
        assert_eq!(v["deleted"], true);
        assert_eq!(v["name"], "待删除");
        assert!(!dir.exists(), "delete removes the project directory");
        assert_eq!(
            project_status(tmp.path(), id).unwrap_err().code(),
            "project_not_found"
        );
        assert_eq!(project_list(tmp.path()).unwrap()["projects"], json!([]));
    }

    #[test]
    fn delete_refuses_while_another_writer_holds_the_lock() {
        let tmp = TempDir::new().unwrap();
        let created = project_create(tmp.path(), "占用中", None).unwrap();
        let id = created["project_id"].as_str().unwrap();
        let dir = Path::new(created["dir"].as_str().unwrap());
        let held = ProjectLock::acquire(dir).unwrap();
        let err = project_delete(tmp.path(), id, Confirmation::Confirmed).unwrap_err();
        assert_eq!(err.code(), "project_locked");
        assert!(dir.join("project.json").is_file());
        drop(held);
        project_delete(tmp.path(), id, Confirmation::Confirmed).unwrap();
        assert!(!dir.exists());
    }
}
