//! State machine types (PRD §6.6). Edges and transitions live in this module; storage
//! lives in `project.rs`.

use serde::{Deserialize, Serialize};
use std::fmt;

/// State machine states: `created → parsing → parsed → tts_running → tts_done
/// → matte_running → matte_done → rendering → done`, any pipeline stage may go to
/// `failed` (recording stage + error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Created,
    Parsing,
    Parsed,
    TtsRunning,
    TtsDone,
    MatteRunning,
    MatteDone,
    Rendering,
    Done,
    Failed,
}

impl Stage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stage::Created => "created",
            Stage::Parsing => "parsing",
            Stage::Parsed => "parsed",
            Stage::TtsRunning => "tts_running",
            Stage::TtsDone => "tts_done",
            Stage::MatteRunning => "matte_running",
            Stage::MatteDone => "matte_done",
            Stage::Rendering => "rendering",
            Stage::Done => "done",
            Stage::Failed => "failed",
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failure record: the state the project was in when it failed, plus the error message
/// (PRD §6.6: failed records stage + error info).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedInfo {
    pub stage: Stage,
    pub error: String,
}

/// All legal transitions (PRD §6.6). Only these edges exist; anything else is an error.
///
/// - linear pipeline: created → parsing → parsed → tts_running → tts_done
///   → matte_running → matte_done → rendering → done
/// - any pipeline stage (parsing..rendering) may go to `failed`
/// - retry from `failed` back into the four running stages (the stage runner decides
///   which one based on the recorded failure; e.g. failed@tts_running → tts_running)
/// - invalidation re-entry (ticket 10, spec 失效传播): changed inputs re-enter a
///   completed stage from every checkpoint the pipeline can legitimately sit at.
///   Re-entering the *running* state a crash left behind (kill -9 residue at
///   tts_running / matte_running / rendering) needs no edge — the runners treat it as
///   resume.
pub const EDGES: &[(Stage, Stage)] = &[
    (Stage::Created, Stage::Parsing),
    (Stage::Parsing, Stage::Parsed),
    (Stage::Parsing, Stage::Failed),
    (Stage::Parsed, Stage::TtsRunning),
    (Stage::Parsed, Stage::Failed),
    (Stage::TtsRunning, Stage::TtsDone),
    (Stage::TtsRunning, Stage::Failed),
    // per-page invalidation: edited page text re-enters tts from tts_done
    // (spec stories 22/23; ticket 10 wires the full invalidation propagation)
    (Stage::TtsDone, Stage::TtsRunning),
    // ...and from the later checkpoints (matte finished / film rendered)
    (Stage::MatteDone, Stage::TtsRunning),
    (Stage::Done, Stage::TtsRunning),
    (Stage::TtsDone, Stage::MatteRunning),
    // matte skipped: render straight from tts_done (pure-color card wraps the original
    // video; ticket 08). Minimal edge for the skip branch.
    (Stage::TtsDone, Stage::Rendering),
    (Stage::TtsDone, Stage::Failed),
    (Stage::MatteRunning, Stage::MatteDone),
    (Stage::MatteRunning, Stage::Failed),
    // invalidation: person video or model changed → re-matte the new clip (ticket 10;
    // also from a finished film: replacing the person must not need a full reset)
    (Stage::MatteDone, Stage::MatteRunning),
    (Stage::Done, Stage::MatteRunning),
    (Stage::MatteDone, Stage::Rendering),
    (Stage::MatteDone, Stage::Failed),
    (Stage::Rendering, Stage::Done),
    (Stage::Rendering, Stage::Failed),
    // invalidation: layout.toml changed → only the film re-renders (ticket 10)
    (Stage::Done, Stage::Rendering),
    (Stage::Failed, Stage::Parsing),
    (Stage::Failed, Stage::TtsRunning),
    (Stage::Failed, Stage::MatteRunning),
    (Stage::Failed, Stage::Rendering),
];

pub fn is_legal_edge(from: Stage, to: Stage) -> bool {
    EDGES.contains(&(from, to))
}
