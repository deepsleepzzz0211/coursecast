//! Neutral progress observer for long-running commands. The Tauri shell forwards
//! events to its webview; CLI/MCP pass `None`. No UI types cross into cc-core.

use crate::error::Result;
use crate::matte::MatteEngine;
use serde::Serialize;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// One progress tick of a generate/matte run.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub project_id: String,
    /// "tts" | "timeline" | "matte" | "render"
    pub segment: &'static str,
    /// "start" | "progress" | "done" | "skipped" | "failed"
    pub state: &'static str,
    /// Pages (tts) / frames (matte) completed so far.
    pub done: Option<u32>,
    /// Total pages, when known.
    pub total: Option<u32>,
    pub message: Option<String>,
}

pub type ProgressSink = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// Send one progress event when a sink is present.
pub fn emit(
    sink: Option<&ProgressSink>,
    project_id: &str,
    segment: &'static str,
    state: &'static str,
    done: Option<u32>,
    total: Option<u32>,
    message: Option<String>,
) {
    if let Some(sink) = sink {
        sink(ProgressEvent {
            project_id: project_id.to_string(),
            segment,
            state,
            done,
            total,
            message,
        });
    }
}

/// Matte adapter that emits one progress tick per successfully inferred frame.
pub struct CountingMatte<'a> {
    inner: &'a mut dyn MatteEngine,
    on_frame: Box<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for CountingMatte<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingMatte")
            .field("engine", &self.inner.name())
            .field("model", &self.inner.model())
            .finish_non_exhaustive()
    }
}

impl MatteEngine for CountingMatte<'_> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn model(&self) -> &'static str {
        self.inner.model()
    }

    fn infer_size(&self, src_w: u32, src_h: u32) -> (u32, u32) {
        self.inner.infer_size(src_w, src_h)
    }

    fn infer(&mut self, rgb_chw: &[f32], w: u32, h: u32) -> Result<Vec<f32>> {
        let alpha = self.inner.infer(rgb_chw, w, h)?;
        (self.on_frame)();
        Ok(alpha)
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}

/// Wrap a matte engine so every inferred frame emits progress.
pub fn count_matte_frames<'a>(
    engine: &'a mut dyn MatteEngine,
    project_id: &str,
    sink: Option<&ProgressSink>,
) -> CountingMatte<'a> {
    let counter = Arc::new(AtomicU32::new(0));
    let sink = sink.cloned();
    let project_id = project_id.to_string();
    let on_frame: Box<dyn Fn() + Send + Sync> = Box::new(move || {
        let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
        emit(
            sink.as_ref(),
            &project_id,
            "matte",
            "progress",
            Some(n),
            None,
            Some(format!("{n} 帧")),
        );
    });
    CountingMatte {
        inner: engine,
        on_frame,
    }
}
