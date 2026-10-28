//! Four-segment generate orchestration (合成语音 → 拼接音轨 → 处理人像 → 渲染成片)
//! with a neutral progress observer. Segment failures stay isolated and a retry reuses
//! every finished segment through the fingerprint records.

use crate::command::progress::{emit, ProgressSink};
use crate::command::{person, render, voice};
use crate::consent::voice_consent_granted;
use crate::error::{Error, Result};
use crate::matte::MatteEngine;
use crate::render::OUTPUT_MP4;
use crate::tts::{FallbackChain, TtsEngine, TtsRequest};
use crate::{load_project, script, ProjectLock};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

struct CountingTts {
    inner: Arc<dyn TtsEngine>,
    on_page: Box<dyn Fn() + Send + Sync>,
}

impl TtsEngine for CountingTts {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn model(&self) -> &'static str {
        self.inner.model()
    }

    fn synthesize(&self, request: &TtsRequest) -> Result<Vec<u8>> {
        let bytes = self.inner.synthesize(request)?;
        (self.on_page)();
        Ok(bytes)
    }
}

/// Wrap every chain engine so a successful page emits a tts progress tick. Identity
/// (name/model) is forwarded untouched, so fingerprints and records stay identical.
fn chain_with_page_events(
    chain: &FallbackChain,
    project_id: &str,
    total: Option<u32>,
    sink: Option<&ProgressSink>,
) -> FallbackChain {
    let counter = Arc::new(AtomicU32::new(0));
    let sink = sink.cloned();
    let engines = chain
        .engines
        .iter()
        .map(|inner| {
            let counter = Arc::clone(&counter);
            let sink = sink.clone();
            let project_id = project_id.to_string();
            let on_page: Box<dyn Fn() + Send + Sync> = Box::new(move || {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                emit(
                    sink.as_ref(),
                    &project_id,
                    "tts",
                    "progress",
                    Some(n),
                    total,
                    None,
                );
            });
            Arc::new(CountingTts {
                inner: Arc::clone(inner),
                on_page,
            }) as Arc<dyn TtsEngine>
        })
        .collect();
    FallbackChain {
        engines,
        events: chain.events.clone(),
    }
}

fn segment_failure(
    project_id: &str,
    segments: Map<String, Value>,
    code: &str,
    message: String,
    segment: &'static str,
) -> Value {
    json!({
        "ok": false,
        "error": { "code": code, "message": message, "segment": segment },
        "project_id": project_id,
        "segments": segments,
    })
}

/// Whether generate should matte the person clip or use the card fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatteChoice {
    Matte,
    CardFallback,
}

impl From<bool> for MatteChoice {
    fn from(use_matte: bool) -> Self {
        if use_matte {
            MatteChoice::Matte
        } else {
            MatteChoice::CardFallback
        }
    }
}

/// Run the four segments in order. The TTS/person/render commands own their locks;
/// the orchestration owns the product-level consent gate, the segment envelope and
/// the progress observer.
pub fn generate(
    data_dir: &Path,
    project_id: &str,
    chain: &FallbackChain,
    matte: MatteChoice,
    matte_engine: &mut dyn MatteEngine,
    model_path: &Path,
    sink: Option<&ProgressSink>,
) -> Result<Value> {
    // 0. 合规门：未勾选《声音授权确认》不得生成。 One lock spans all four segments,
    //    matching the original generate transaction; segment commands use their
    //    lock-held forms.
    let (_, lock_dir) = load_project(data_dir, project_id)?;
    let _lock = ProjectLock::acquire(&lock_dir)?;
    let (_, dir) = load_project(data_dir, project_id)?;
    if !voice_consent_granted(&dir) {
        return Err(Error::InvalidArgument(
            "请先在「配置声音」一步勾选《声音授权确认》（本人声音或已获授权）后才能生成".into(),
        ));
    }
    let page_total = script::read_script_pages(&dir).map(|p| p.len() as u32).ok();

    let mut segments = Map::new();
    for seg in ["tts", "timeline", "matte", "render"] {
        segments.insert(seg.into(), json!({ "state": "pending" }));
    }

    // 1. 合成语音
    emit(
        sink,
        project_id,
        "tts",
        "start",
        None,
        Some(page_total.unwrap_or(0)),
        None,
    );
    let wrapped = chain_with_page_events(chain, project_id, page_total, sink);
    let report = match voice::synthesize_with_chain_locked(
        data_dir,
        project_id,
        &wrapped,
        None,
        &Default::default(),
    ) {
        Ok(report) => report,
        Err(err) => {
            emit(
                sink,
                project_id,
                "tts",
                "failed",
                None,
                page_total,
                Some(err.to_string()),
            );
            segments.insert("tts".into(), json!({ "state": "failed" }));
            return Ok(segment_failure(
                project_id,
                segments,
                err.code(),
                err.to_string(),
                "tts",
            ));
        }
    };
    if !report.failed_pages.is_empty() {
        let message = format!("{} 页合成失败", report.failed_pages.len());
        emit(
            sink,
            project_id,
            "tts",
            "failed",
            None,
            page_total,
            Some(message.clone()),
        );
        segments.insert(
            "tts".into(),
            json!({
                "state": "failed",
                "generated": report.generated,
                "skipped": report.skipped,
                "failed_pages": report.failed_pages,
                "fallbacks": report.fallbacks,
            }),
        );
        return Ok(segment_failure(
            project_id,
            segments,
            "tts_page_failed",
            message,
            "tts",
        ));
    }
    emit(
        sink,
        project_id,
        "tts",
        "done",
        Some((report.generated.len() + report.skipped.len()) as u32),
        page_total,
        Some(format!(
            "语音就绪（新配 {} 页，复用 {} 页）",
            report.generated.len(),
            report.skipped.len()
        )),
    );
    let tool_version = {
        let (project, _) = load_project(data_dir, project_id)?;
        report
            .skipped
            .iter()
            .chain(report.generated.iter())
            .next()
            .and_then(|p| project.stages.get(&format!("tts:{p}")))
            .map(|r| r.tool_version.clone())
            .unwrap_or_default()
    };
    let fallbacks = report.fallbacks.clone();
    segments.insert(
        "tts".into(),
        json!({
            "state": "done",
            "generated": report.generated,
            "skipped": report.skipped,
            "total_duration_sec": report.total_duration_sec,
            "tool_version": tool_version,
            "fallbacks": report.fallbacks,
        }),
    );

    // 2. 拼接音轨
    emit(sink, project_id, "timeline", "start", None, None, None);
    let timeline = match render::timeline_locked(data_dir, project_id) {
        Ok(value) => value,
        Err(err) => {
            emit(
                sink,
                project_id,
                "timeline",
                "failed",
                None,
                None,
                Some(err.to_string()),
            );
            segments.insert("timeline".into(), json!({ "state": "failed" }));
            return Ok(segment_failure(
                project_id,
                segments,
                err.code(),
                err.to_string(),
                "timeline",
            ));
        }
    };
    let timeline_state = if timeline["skipped"].as_bool().unwrap_or(false) {
        "skipped"
    } else {
        "done"
    };
    emit(
        sink,
        project_id,
        "timeline",
        timeline_state,
        None,
        None,
        None,
    );
    segments.insert(
        "timeline".into(),
        json!({
            "state": timeline_state,
            "total_sec": timeline["total_sec"],
            "turn_points_sec": timeline["turn_points_sec"],
        }),
    );

    // 3. 处理人像
    if matte == MatteChoice::Matte {
        emit(sink, project_id, "matte", "start", None, None, None);
        let mut engine =
            crate::command::progress::count_matte_frames(matte_engine, project_id, sink);
        let matte = match person::matte_locked(data_dir, project_id, &mut engine, model_path, None)
        {
            Ok(value) => value,
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
                segments.insert("matte".into(), json!({ "state": "failed" }));
                return Ok(segment_failure(
                    project_id,
                    segments,
                    err.code(),
                    err.to_string(),
                    "matte",
                ));
            }
        };
        let state = if matte["skipped"].as_bool().unwrap_or(false) {
            "skipped"
        } else {
            "done"
        };
        emit(
            sink,
            project_id,
            "matte",
            state,
            Some(matte["frames"].as_u64().unwrap_or_default() as u32),
            None,
            Some(format!(
                "{state}：{} 帧",
                matte["frames"].as_u64().unwrap_or_default()
            )),
        );
        segments.insert(
            "matte".into(),
            json!({
                "state": state,
                "frames": matte["frames"],
                "elapsed_sec": matte["elapsed_sec"],
            }),
        );
    } else {
        // the user chose the card fallback: alpha from an earlier matte must not be
        // picked up by render's existence check
        person::skip_locked(data_dir, project_id)?;
        segments.insert(
            "matte".into(),
            json!({ "state": "skipped", "reason": "card_fallback" }),
        );
    }

    // 4. 渲染成片
    emit(sink, project_id, "render", "start", None, None, None);
    let rendered = match render::render_locked(data_dir, project_id) {
        Ok(value) => value,
        Err(err) => {
            emit(
                sink,
                project_id,
                "render",
                "failed",
                None,
                None,
                Some(err.to_string()),
            );
            segments.insert("render".into(), json!({ "state": "failed" }));
            return Ok(segment_failure(
                project_id,
                segments,
                err.code(),
                err.to_string(),
                "render",
            ));
        }
    };
    emit(
        sink,
        project_id,
        "render",
        "done",
        None,
        None,
        Some(format!(
            "{:.1} 秒成片",
            rendered["total_sec"].as_f64().unwrap_or_default()
        )),
    );
    segments.insert(
        "render".into(),
        json!({
            "state": "done",
            "pip": rendered["pip"]["source"],
            "total_sec": rendered["total_sec"],
            "skipped": rendered["skipped"],
        }),
    );

    let (project, project_dir) = load_project(data_dir, project_id)?;
    Ok(json!({
        "ok": true,
        "project_id": project_id,
        "stage": project.stage.to_string(),
        "segments": segments,
        "fallbacks": fallbacks,
        "output_path": OUTPUT_MP4,
        "output_abs": project_dir.join(OUTPUT_MP4).display().to_string(),
        "total_sec": timeline["total_sec"],
        "next": "成片已生成：预览确认后下载 MP4",
    }))
}
