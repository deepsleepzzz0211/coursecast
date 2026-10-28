//! Project status: current stage + input readiness + missing items + next suggestion
//! (spec story 24, PRD §7.2 project_status). Stage runners (later tickets) extend the
//! readiness view with stage records; the inputs here are what the user/AI must provide.

use crate::state::{FailedInfo, Stage};
use crate::Project;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct InputStatus {
    pub name: &'static str,
    /// Path relative to the project directory.
    pub path: String,
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub stage: Stage,
    pub failed: Option<FailedInfo>,
    pub inputs: Vec<InputStatus>,
    pub missing: Vec<String>,
    pub next: String,
}

/// Compute readiness of the four user-provided inputs (PRD §6.5 layout).
pub fn compute_status(project: &Project, project_dir: &Path) -> StatusReport {
    let deck = if project_dir.join("source").join("deck.pdf").is_file() {
        Some("source/deck.pdf")
    } else if project_dir.join("source").join("deck.pptx").is_file() {
        Some("source/deck.pptx")
    } else {
        None
    };
    let script = project_dir.join("script.md").is_file();
    let voice_ref = project_dir.join("source").join("voice_ref.wav").is_file();
    let person = project_dir.join("source").join("person.mp4").is_file();

    let inputs = vec![
        InputStatus {
            name: "deck",
            path: deck.unwrap_or("source/deck.pdf").to_string(),
            ready: deck.is_some(),
        },
        InputStatus {
            name: "script",
            path: "script.md".into(),
            ready: script,
        },
        InputStatus {
            name: "voice_ref",
            path: "source/voice_ref.wav".into(),
            ready: voice_ref,
        },
        InputStatus {
            name: "person",
            path: "source/person.mp4".into(),
            ready: person,
        },
    ];
    let missing: Vec<String> = inputs
        .iter()
        .filter(|i| !i.ready)
        .map(|i| i.name.to_string())
        .collect();
    let next = next_suggestion(project, &missing);
    StatusReport {
        stage: project.stage,
        failed: project.failed.clone(),
        inputs,
        missing,
        next,
    }
}

fn next_suggestion(project: &Project, missing: &[String]) -> String {
    if let Some(failed) = &project.failed {
        return format!(
            "项目在 {} 阶段失败：{}。修复后重跑该阶段",
            failed.stage, failed.error
        );
    }
    if project.stage == Stage::Done {
        return "项目已完成，成片在 output/final.mp4".into();
    }
    for m in missing {
        match m.as_str() {
            "deck" => {
                return "将课件放入 source/deck.pdf（或 .pptx），然后运行 slide_render".into()
            }
            "script" => {
                return "填写逐字稿：编辑 script.md，或使用 script_set / script_import_notes".into()
            }
            "voice_ref" => {
                return "提供参考音频 source/voice_ref.wav，并运行 voice_configure".into()
            }
            "person" => return "提供出镜视频 source/person.mp4，随后运行 person_matte".into(),
            _ => {}
        }
    }
    "输入已齐备：设置版式（layout_set）后运行 render".into()
}
