//! Error type for cc-core. Every error carries a stable machine-readable code so the
//! CLI/MCP shells can emit structured JSON (spec: "全部返回结构化 JSON").

use crate::state::Stage;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("project not found: {id}")]
    ProjectNotFound { id: String },

    #[error("project is locked by another process: {path}")]
    ProjectLocked { path: String },

    #[error("invalid project id `{0}`: expected 8-32 chars of [a-z0-9]")]
    InvalidProjectId(String),

    #[error("invalid project name `{0}`: must be 1-100 characters after trimming")]
    InvalidName(String),

    #[error("invalid resolution `{0}`: expected WIDTHxHEIGHT, e.g. 1920x1080")]
    InvalidResolution(String),

    #[error("illegal stage transition: {from} -> {to}")]
    InvalidTransition { from: Stage, to: Stage },

    #[error("invalid project.json in `{path}`: {message}")]
    InvalidProjectJson { path: String, message: String },

    #[error("slide source not found: `{0}` (pass a path, or place the deck at source/deck.pdf)")]
    SourceNotFound(String),

    #[error("unsupported slide source `{path}`: {message}")]
    UnsupportedSource { path: String, message: String },

    #[error("slide render failed: {0}")]
    SlideRender(String),

    #[error("cannot load pdfium library from `{path}`: {message}")]
    PdfiumLoad { path: String, message: String },

    #[error("`{runner}` cannot run in stage `{stage}`; expected created, parsed or a failed slide stage")]
    StageNotRunnable { runner: &'static str, stage: Stage },

    #[error("invalid wav file `{path}`: {message}")]
    WavInvalid { path: String, message: String },

    #[error("no per-page audio found under `{dir}`: expected work/audio/slide_001.wav (run voice_synthesize first)")]
    AudioMissing { dir: String },

    #[error("slide image for page {page} not found: `{path}` (run slide_render first)")]
    SlideMissing { page: u32, path: String },

    #[error("page count mismatch: {audio} page(s) of audio vs {slides} slide image(s); every slide needs one script page (an empty page is 5 s of silence)")]
    PageCountMismatch { audio: u32, slides: u32 },

    #[error("upstream stage `{stage}` failed; fix and rerun that stage first")]
    UpstreamFailed { stage: Stage },

    #[error("person video not found: `{path}` (provide source/person.mp4 first)")]
    PersonVideoMissing { path: String },

    #[error("matte model not found: `{path}` (point CC_RVM_MODEL at the RVM onnx or place it under vendor/models/)")]
    MatteModelMissing { path: String },

    #[error("invalid person video `{path}`: {message}")]
    VideoInvalid { path: String, message: String },

    #[error("stage `{stage}` record or its artifacts are missing (run the stage first)")]
    StageMissing { stage: String },

    #[error("person video not found: expected `{alpha}` (run person_matte) or `{fallback}` (matte skipped)")]
    PersonMissing { alpha: String, fallback: String },

    #[error("invalid layout.toml: {message}")]
    InvalidLayout { message: String },

    #[error("ffmpeg failed: {stderr}")]
    Ffmpeg { stderr: String },

    #[error("invalid wav: {0}")]
    InvalidWav(String),

    #[error("script not found: {path}")]
    ScriptNotFound { path: String },

    #[error("script is empty: {path}")]
    ScriptEmpty { path: String },

    #[error("invalid pages spec `{0}`: expected e.g. `3`, `1-5` or `1-3,7`")]
    InvalidPagesSpec(String),

    #[error("page {page} out of range: script has {pages} pages")]
    PageOutOfRange { page: u32, pages: u32 },

    #[error("engine `{engine}` timed out after {timeout}")]
    EngineTimeout { engine: String, timeout: String },

    #[error("engine `{engine}` failed: {message}")]
    EngineFailure { engine: String, message: String },

    #[error("tts engine `{engine}` authentication failed (HTTP 401): {message}")]
    TtsUnauthorized { engine: String, message: String },

    #[error("engine `{engine}` is blocked: {message}")]
    EngineBlocked { engine: String, message: String },

    #[error("credential store failed: {0}")]
    CredentialStore(String),

    #[error("MiMo API key not found: run voice_configure --api-key <key> first (the key is stored in the OS credential store)")]
    ApiKeyMissing,

    #[error("{0}")]
    InvalidArgument(String),

    #[error("path escapes the project root: {0}")]
    PathEscape(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    /// Stable error code used in structured JSON output (`error.code`).
    pub fn code(&self) -> &'static str {
        match self {
            Error::ProjectNotFound { .. } => "project_not_found",
            Error::ProjectLocked { .. } => "project_locked",
            Error::InvalidProjectId(_) => "invalid_project_id",
            Error::InvalidName(_) => "invalid_name",
            Error::InvalidResolution(_) => "invalid_resolution",
            Error::InvalidTransition { .. } => "invalid_transition",
            Error::InvalidProjectJson { .. } => "project_json_invalid",
            Error::SourceNotFound(_) => "source_not_found",
            Error::UnsupportedSource { .. } => "source_unsupported",
            Error::SlideRender(_) => "slide_render_failed",
            Error::PdfiumLoad { .. } => "pdfium_load_failed",
            Error::StageNotRunnable { .. } => "stage_not_runnable",
            Error::WavInvalid { .. } => "wav_invalid",
            Error::AudioMissing { .. } => "audio_missing",
            Error::SlideMissing { .. } => "slide_missing",
            Error::PageCountMismatch { .. } => "page_count_mismatch",
            Error::UpstreamFailed { .. } => "upstream_failed",
            Error::PersonVideoMissing { .. } => "person_video_missing",
            Error::MatteModelMissing { .. } => "matte_model_missing",
            Error::VideoInvalid { .. } => "video_invalid",
            Error::StageMissing { .. } => "stage_missing",
            Error::PersonMissing { .. } => "person_missing",
            Error::InvalidLayout { .. } => "layout_invalid",
            Error::Ffmpeg { .. } => "ffmpeg_failed",
            Error::InvalidWav(_) => "wav_invalid",
            Error::ScriptNotFound { .. } => "script_not_found",
            Error::ScriptEmpty { .. } => "script_empty",
            Error::InvalidPagesSpec(_) => "invalid_pages",
            Error::PageOutOfRange { .. } => "page_out_of_range",
            Error::EngineTimeout { .. } => "engine_timeout",
            Error::EngineFailure { .. } => "engine_error",
            Error::EngineBlocked { .. } => "engine_blocked",
            Error::TtsUnauthorized { .. } => "tts_unauthorized",
            Error::CredentialStore(_) => "credential_store_failed",
            Error::ApiKeyMissing => "api_key_missing",
            Error::InvalidArgument(_) => "invalid_argument",
            Error::PathEscape(_) => "path_escape",
            Error::Io(_) => "io_error",
            Error::Json(_) => "json_error",
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
