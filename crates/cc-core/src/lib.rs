//! cc-core: CourseCast business core. No UI, no protocol — the CLI / MCP / Tauri shells
//! are thin wrappers over this crate (spec "Cargo workspace 四 crate").

pub mod cleanup;
pub mod command;
pub mod consent;
pub mod credentials;
pub mod edge;
pub mod error;
pub mod ffmpeg;
pub mod lock;
pub mod matte;
pub mod mimo;
pub mod project;
pub mod render;
pub mod runtime;
pub mod sapi;
pub mod script;
pub mod slide;
pub mod stage;
pub mod state;
pub mod status;
pub mod timeline;
pub mod tts;
pub mod voice;
pub mod wav;

pub use cleanup::{clean_artifacts, CleanReport, CLEAN_STAGES};
pub use consent::{
    require_voice_clone_consent, set_voice_consent, voice_consent_granted, ConsentDecision,
    VOICE_CONSENT_FILE,
};
pub use edge::{EdgeEngine, EdgeVoice};
pub use error::{Error, Result};
pub use lock::ProjectLock;
pub use matte::{
    fingerprint as matte_fingerprint, run_matte_locked, FakeMatteEngine, MatteEngine, RvmEngine,
    DEFAULT_MODEL_REL,
};
pub use mimo::MimoEngine;
pub use project::{
    create_project, load_project, project_dir, save_project, validate_project_id, CreatedProject,
    FallbackEvent, OutputArtifact, Project, StageRecord, DEFAULT_RESOLUTION, SCHEMA_VERSION,
};
pub use project::{TtsPreference, VoiceEngine};
pub use sapi::SapiEngine;
pub use slide::{
    run_slide_render, run_slide_render_locked, SlideRenderOutcome, SLIDES_DIR, TARGET_WIDTH,
};
pub use state::{is_legal_edge, FailedInfo, Stage, EDGES};
pub use status::{compute_status, InputStatus, StatusReport};
pub use tts::{
    build_fallback_chain, build_fallback_chain_with_voice, synthesize_pages,
    synthesize_pages_locked, FakeTtsEngine, FallbackChain, PageArtifact, PageFailure,
    SynthesisOptions, SynthesisReport, TtsEngine, TtsEngineKind, TtsRequest,
};
