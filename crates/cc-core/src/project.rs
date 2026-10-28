//! Project model + storage. `project.json` is the source of truth (spec "文件即真相"):
//! written atomically (tmp → fsync → rename), read back by the shells and by AI directly.

use crate::edge::EdgeVoice;
use crate::error::{Error, Result};
use crate::state::{is_legal_edge, FailedInfo, Stage};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_RESOLUTION: &str = "1920x1080";

/// The persisted project state (project.json).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub schema_version: u32,
    pub project_id: String,
    pub name: String,
    pub resolution: String,
    pub created_at: String,
    pub stage: Stage,
    /// Set iff `stage == Failed` (serde default keeps older files loadable).
    #[serde(default)]
    pub failed: Option<FailedInfo>,
    /// Per-stage fingerprint records (PRD §6.6.1). Keyed by stage name; per-page stages
    /// use suffixed keys (e.g. `tts:7`).
    #[serde(default)]
    pub stages: BTreeMap<String, StageRecord>,
    /// Fallback-chain degradation events (ticket 06) of the synthesis run that produced
    /// the current audio: which engine was skipped, which took over, why. Replaced on
    /// every run that does work; an all-skipped rerun leaves it untouched. This is the
    /// UI 提示条 data source alongside the per-page `tool_version`s.
    #[serde(default)]
    pub fallbacks: Vec<FallbackEvent>,
    /// App-facing voice choice: MiMo clone or Edge preset. Older project files without
    /// this field read as the MiMo default.
    #[serde(default)]
    pub tts: TtsPreference,
}

/// The app-facing voice choice (project.json `tts.engine`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VoiceEngine {
    /// MiMo voice clone; needs an API key, reference audio and consent.
    #[default]
    Mimo,
    /// Edge read-aloud with a preset voice; no key or consent required.
    Edge,
}

/// Persisted voice preference (`tts` object in project.json).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TtsPreference {
    #[serde(default)]
    pub engine: VoiceEngine,
    #[serde(default)]
    pub edge_voice: EdgeVoice,
}

/// One fallback-chain degradation (ticket 06 三级降级链): engine `from` was unavailable
/// and `to` took over, with a user-readable `reason` (CLI JSON 提示字段 / UI banner).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FallbackEvent {
    pub from: String,
    pub to: String,
    pub reason: String,
}

/// A stage's success record bound to its input fingerprint, tool version, produced
/// artifacts and dependencies (PRD §6.6.1). Stage runners (later tickets) write these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageRecord {
    /// Pipeline stage this record belongs to: slide_render | tts | matte | timeline | render.
    pub stage: String,
    /// Record state, e.g. "success".
    pub state: String,
    /// Input fingerprint of everything this stage consumed, `sha256:...`.
    pub input_fingerprint: String,
    /// Tool that produced the outputs, e.g. "tts-mimo/0.1.0+model=mimo-v2.5-tts-voiceclone".
    pub tool_version: String,
    /// Artifacts produced, with content hashes.
    pub outputs: Vec<OutputArtifact>,
    /// Fingerprints of upstream stages this record depends on: {stage: fingerprint}.
    pub depends_on: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputArtifact {
    /// Path relative to the project directory.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct CreatedProject {
    pub project: Project,
    pub dir: PathBuf,
}

impl Project {
    /// Move the project along a state machine edge. Illegal edges return
    /// `Error::InvalidTransition` and leave the state untouched.
    pub fn transition(&mut self, to: Stage) -> Result<()> {
        let from = self.stage;
        if !is_legal_edge(from, to) {
            return Err(Error::InvalidTransition { from, to });
        }
        if to == Stage::Failed {
            self.failed = Some(FailedInfo {
                stage: from,
                error: String::new(),
            });
        }
        self.stage = to;
        Ok(())
    }

    /// Enter `failed` from a pipeline stage, recording where it failed and why
    /// (PRD §6.6: 任一阶段可进 failed，记录 stage + 错误信息).
    pub fn fail(&mut self, error: impl Into<String>) -> Result<()> {
        let from = self.stage;
        self.transition(Stage::Failed)?;
        self.failed = Some(FailedInfo {
            stage: from,
            error: error.into(),
        });
        Ok(())
    }
}

/// Directory of a project: `<data_dir>/projects/<project_id>/` (PRD §6.5).
pub fn project_dir(data_dir: &Path, project_id: &str) -> PathBuf {
    data_dir.join("projects").join(project_id)
}

pub fn validate_project_id(id: &str) -> Result<()> {
    let ok = (8..=32).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidProjectId(id.to_string()))
    }
}

pub fn validate_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 100 {
        return Err(Error::InvalidName(name.to_string()));
    }
    Ok(trimmed.to_string())
}

pub fn normalize_resolution(resolution: Option<&str>) -> Result<String> {
    let Some(r) = resolution else {
        return Ok(DEFAULT_RESOLUTION.to_string());
    };
    let parts: Vec<&str> = r.split('x').collect();
    let dims: Option<Vec<u32>> = if parts.len() == 2 {
        parts.iter().map(|p| p.parse::<u32>().ok()).collect()
    } else {
        None
    };
    match dims {
        Some(d) if d.iter().all(|v| *v > 0) => Ok(r.to_string()),
        _ => Err(Error::InvalidResolution(r.to_string())),
    }
}

/// Create a project directory + project.json (atomic write).
pub fn create_project(
    data_dir: &Path,
    name: &str,
    resolution: Option<&str>,
) -> Result<CreatedProject> {
    let name = validate_name(name)?;
    let resolution = normalize_resolution(resolution)?;
    let project_id = uuid::Uuid::new_v4().simple().to_string();
    let dir = project_dir(data_dir, &project_id);
    fs::create_dir_all(dir.join("source"))?;
    fs::create_dir_all(dir.join("work"))?;
    fs::create_dir_all(dir.join("output"))?;
    let project = Project {
        schema_version: SCHEMA_VERSION,
        project_id,
        name,
        resolution,
        created_at: now_rfc3339(),
        stage: Stage::Created,
        failed: None,
        stages: BTreeMap::new(),
        fallbacks: Vec::new(),
        tts: TtsPreference::default(),
    };
    save_project(&project, &dir)?;
    Ok(CreatedProject { project, dir })
}

/// Load project.json for `project_id`. ID is validated first (blocks path traversal).
pub fn load_project(data_dir: &Path, project_id: &str) -> Result<(Project, PathBuf)> {
    validate_project_id(project_id)?;
    let dir = project_dir(data_dir, project_id);
    let path = dir.join("project.json");
    if !path.is_file() {
        return Err(Error::ProjectNotFound {
            id: project_id.to_string(),
        });
    }
    let text = fs::read_to_string(&path)?;
    let project: Project = serde_json::from_str(&text).map_err(|e| Error::InvalidProjectJson {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    Ok((project, dir))
}

/// Atomic write: tmp file → fsync → rename over project.json (spec "并发" section).
/// With the project lock held there is at most one writer, so a fixed tmp name is safe.
pub fn save_project(project: &Project, project_dir: &Path) -> Result<()> {
    let path = project_dir.join("project.json");
    let tmp = project_dir.join("project.json.tmp");
    let file = fs::File::create(&tmp)?;
    serde_json::to_writer_pretty(&file, project)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
