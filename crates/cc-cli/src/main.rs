//! cc-cli: thin CLI shell over cc-core. All subcommands support `--json` (structured
//! output for AI users, spec story 29). Errors are always structured JSON with exit code 1.

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cc-cli", about = "课影 CourseCast CLI")]
struct Cli {
    /// Data directory; projects live under `data-dir/projects/project_id/`
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Project management
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Render the deck (PDF or image folder) into work/slides/slide_NNN.png
    #[command(name = "slide_render")]
    SlideRender {
        #[arg(long)]
        project: String,
        /// Deck file or image folder: a path, or relative to the project's source/
        #[arg(long)]
        source: String,
        #[arg(long)]
        json: bool,
    },
    /// Read the per-page 逐字稿 (script.md) as a page list
    #[command(name = "script_get")]
    ScriptGet {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Write part of the 逐字稿: --pages '[{"page":1,"text":"..."}]'
    #[command(name = "script_set")]
    ScriptSet {
        #[arg(long)]
        project: String,
        /// JSON array of {page, text}
        #[arg(long)]
        pages: String,
        #[arg(long)]
        json: bool,
    },
    /// Import PPT speaker notes into script.md (app previews before writing)
    #[command(name = "script_import_notes")]
    ScriptImportNotes {
        #[arg(long)]
        project: String,
        /// pptx path; defaults to the project's source/deck.pptx
        #[arg(long)]
        pptx: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Build the timeline: per-page durations per the formula, work/concat.txt and
    /// work/audio/voice_full.wav (0.6 s inter-page silence, 1.0 s end fade-out)
    Timeline {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Synthesize per-page audio into work/audio/slide_NNN.wav (TTS engine stage; the
    /// deterministic fake engine is the default. `--engine mimo` runs the real MiMo
    /// voiceclone client and falls back down the degradation chain (ticket 06:
    /// MiMo → edge-tts → Windows SAPI, 无 Key/不可用时自动滑落), `--engine edge`
    /// starts at the Edge read-aloud level (ticket 07, 非官方接口被阻断时自动滑落
    /// 到 SAPI), `--engine sapi` asks for the offline bottom level directly)
    #[command(name = "voice_synthesize")]
    VoiceSynthesize {
        #[arg(long)]
        project: String,
        /// Page subset to synthesize, e.g. `3`, `1-5` or `1-3,7` (default: all pages)
        #[arg(long)]
        pages: Option<String>,
        /// TTS engine: `fake` (offline default), `mimo` (voiceclone with fallback
        /// chain), `edge` (Edge read-aloud, voice via CC_EDGE_VOICE=xiaoxiao|yunxi)
        /// or `sapi` (Windows system speech)
        #[arg(long)]
        engine: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Configure the voice: transcode the reference recording to 24kHz mono wav at
    /// source/voice_ref.wav and/or store the MiMo API key in the OS credential store
    /// (the key never appears in project.json, logs or this command's output)
    #[command(name = "voice_configure")]
    VoiceConfigure {
        #[arg(long)]
        project: String,
        /// Reference recording (m4a/mp3/wav); re-encoded to 24kHz mono pcm_s16le wav
        #[arg(long)]
        audio: Option<String>,
        /// MiMo API key to store in the OS credential store
        #[arg(long)]
        api_key: Option<String>,
        /// Voice preference: `mimo` (clone) or `edge` (preset voice)
        #[arg(long)]
        engine: Option<String>,
        /// Edge voice preset: `xiaoxiao` (default) or `yunxi`
        #[arg(long)]
        edge_voice: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// 《声音授权确认》: read, grant (--grant) or revoke (--revoke) the voice-clone
    /// acknowledgement. Voice cloning (engine `mimo`) refuses to run without it.
    #[command(name = "voice_consent")]
    VoiceConsent {
        #[arg(long)]
        project: String,
        /// Record the acknowledgement (required before the first clone run)
        #[arg(long)]
        grant: bool,
        /// Revoke the acknowledgement
        #[arg(long)]
        revoke: bool,
        #[arg(long)]
        json: bool,
    },
    /// Render output/final.mp4: single ffmpeg call — concat image track + voice_full.wav
    /// + looped person PiP (alpha overlay, or card-wrapped original when matte skipped)
    Render {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Show the current layout.toml (人像位置/宽度/形状/翻页动效)
    #[command(name = "layout_get")]
    LayoutGet {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Update layout.toml; omitted keys keep their current values
    #[command(name = "layout_set")]
    LayoutSet {
        #[arg(long)]
        project: String,
        #[arg(long)]
        position: Option<String>,
        /// Free placement: horizontal anchor 0-100 of the remaining canvas space
        #[arg(long)]
        x_pct: Option<f64>,
        /// Free placement: vertical anchor 0-100 of the remaining canvas space
        #[arg(long)]
        y_pct: Option<f64>,
        #[arg(long)]
        width_pct: Option<f64>,
        #[arg(long)]
        shape: Option<String>,
        #[arg(long)]
        transition: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Remove one stage's intermediate artifacts (tts|matte|timeline|render|all);
    /// irreversible, requires --confirm
    #[command(name = "artifact_clean")]
    ArtifactClean {
        #[arg(long)]
        project: String,
        #[arg(long)]
        stage: String,
        #[arg(long)]
        confirm: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show whether output/final.mp4 has landed
    #[command(name = "final_video")]
    FinalVideo {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Copy output/final.mp4 to a directory without clobbering existing files
    #[command(name = "download")]
    Download {
        #[arg(long)]
        project: String,
        /// Target directory (defaults to the current directory)
        #[arg(long)]
        out_dir: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Mat the person video: RVM (CPU) per-frame inference → work/person/alpha.mov
    /// (ProRes 4444, keeps alpha) + checkerboard preview frame; fingerprint-bound so
    /// unchanged inputs never re-process
    #[command(name = "person_matte")]
    PersonMatte {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Ingest a person clip (mp4/mov) as source/person.mp4 and invalidate the old matte
    #[command(name = "person_video_set")]
    PersonVideoSet {
        #[arg(long)]
        project: String,
        #[arg(long)]
        source: String,
        #[arg(long)]
        json: bool,
    },
    /// Show whether the person clip and matte artifacts exist
    #[command(name = "person_status")]
    PersonStatus {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Skip matte: drop the alpha/record so render uses the card fallback
    #[command(name = "person_matte_skip")]
    PersonMatteSkip {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// Create a new project
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        resolution: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List projects on disk, newest first
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show project status (current stage + missing inputs)
    Status {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    /// Delete a project directory (irreversible; requires --confirm)
    Delete {
        #[arg(long)]
        project: String,
        #[arg(long)]
        confirm: bool,
        #[arg(long)]
        json: bool,
    },
}

fn data_dir(cli: &Cli) -> PathBuf {
    cli.data_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("data"))
}

fn json_text(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).expect("serializing a JSON value cannot fail")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Json,
    Human,
}

impl From<bool> for OutputMode {
    fn from(json: bool) -> Self {
        if json {
            OutputMode::Json
        } else {
            OutputMode::Human
        }
    }
}

/// One output policy for every subcommand: JSON prints the canonical envelope,
/// Human renders the line built from the same value.
fn output(value: &serde_json::Value, mode: OutputMode, human: impl FnOnce() -> String) -> String {
    match mode {
        OutputMode::Json => json_text(value),
        OutputMode::Human => human(),
    }
}

fn main() {
    cc_core::runtime::prepare();
    let cli = Cli::parse();
    match run(&cli) {
        Ok(line) => println!("{line}"),
        Err(err) => {
            println!("{}", json_text(&cc_core::command::error_json(&err)));
            std::process::exit(1);
        }
    }
}

fn run(cli: &Cli) -> cc_core::Result<String> {
    match &cli.cmd {
        Cmd::Project { action } => match action {
            ProjectAction::Create {
                name,
                resolution,
                json,
            } => {
                let value =
                    cc_core::command::project_create(&data_dir(cli), name, resolution.as_deref())?;
                Ok(output(&value, (*json).into(), || {
                    format!(
                        "created project {} at {}",
                        value["project_id"].as_str().unwrap_or_default(),
                        value["dir"].as_str().unwrap_or_default()
                    )
                }))
            }
            ProjectAction::List { json } => {
                let value = cc_core::command::project_list(&data_dir(cli))?;
                Ok(output(&value, (*json).into(), || {
                    let projects = value["projects"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    if projects.is_empty() {
                        return "no projects".to_string();
                    }
                    let mut lines = vec![format!("{} project(s):", projects.len())];
                    for p in projects {
                        lines.push(format!(
                            "{}  {}  stage={}",
                            p["project_id"].as_str().unwrap_or_default(),
                            p["name"].as_str().unwrap_or_default(),
                            p["stage"].as_str().unwrap_or_default()
                        ));
                    }
                    lines.join("\n")
                }))
            }
            ProjectAction::Status { project, json } => {
                let value = cc_core::command::project_status(&data_dir(cli), project)?;
                Ok(output(&value, (*json).into(), || {
                    let missing = value["missing"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default();
                    format!(
                        "{}: stage={}, missing=[{}]",
                        value["project_id"].as_str().unwrap_or_default(),
                        value["stage"].as_str().unwrap_or_default(),
                        missing
                    )
                }))
            }
            ProjectAction::Delete {
                project,
                confirm,
                json,
            } => {
                let value =
                    cc_core::command::project_delete(&data_dir(cli), project, (*confirm).into())?;
                Ok(output(&value, (*json).into(), || {
                    format!(
                        "project deleted: {} ({})",
                        value["project_id"].as_str().unwrap_or_default(),
                        value["name"].as_str().unwrap_or_default()
                    )
                }))
            }
        },
        Cmd::SlideRender {
            project,
            source,
            json,
        } => {
            let value = cc_core::command::deck::slide_render(&data_dir(cli), project, source)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "{}: {} slides ({}), fingerprint {}",
                    value["project_id"].as_str().unwrap_or_default(),
                    value["page_count"].as_u64().unwrap_or_default(),
                    if value["skipped"].as_bool().unwrap_or(false) {
                        "skipped, outputs reused"
                    } else {
                        "rendered"
                    },
                    value["fingerprint"].as_str().unwrap_or_default()
                )
            }))
        }
        Cmd::ScriptGet { project, json } => {
            let value = cc_core::command::deck::script_get(&data_dir(cli), project)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "script_get: exists={}, pages={}",
                    value["exists"].as_bool().unwrap_or(false),
                    value["pages"]
                        .as_array()
                        .map(|a| a.len())
                        .unwrap_or_default()
                )
            }))
        }
        Cmd::ScriptSet {
            project,
            pages,
            json,
        } => {
            let raw: serde_json::Value = serde_json::from_str(pages).map_err(|e| {
                cc_core::Error::InvalidArgument(format!("--pages 需要 JSON 数组：{e}"))
            })?;
            let updates = cc_core::command::script::parse_updates(&raw)?;
            let value = cc_core::command::script::set(&data_dir(cli), project, &updates)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "script_set: {} pages -> {}",
                    value["page_count"].as_u64().unwrap_or_default(),
                    value["script"].as_str().unwrap_or_default()
                )
            }))
        }
        Cmd::ScriptImportNotes {
            project,
            pptx,
            json,
        } => {
            let value = cc_core::command::script::import_notes(
                &data_dir(cli),
                project,
                pptx.as_deref(),
                cc_core::command::script::ImportMode::Apply,
            )?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "script_import_notes: {} pages imported ({})",
                    value["imported_pages"].as_u64().unwrap_or_default(),
                    value["script"].as_str().unwrap_or_default()
                )
            }))
        }
        Cmd::Timeline { project, json } => {
            let value = cc_core::command::render::timeline(&data_dir(cli), project)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "timeline: {} pages, total {:.3}s -> {}, {}",
                    value["pages"]
                        .as_array()
                        .map(|a| a.len())
                        .unwrap_or_default(),
                    value["total_sec"].as_f64().unwrap_or_default(),
                    value["artifacts"]["concat_txt"]
                        .as_str()
                        .unwrap_or_default(),
                    value["artifacts"]["voice_full"]
                        .as_str()
                        .unwrap_or_default()
                )
            }))
        }
        Cmd::Render { project, json } => {
            let value = cc_core::command::render::render(&data_dir(cli), project)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "render: {:.3}s -> {} (pip={}, fingerprint={})",
                    value["total_sec"].as_f64().unwrap_or_default(),
                    value["output_path"].as_str().unwrap_or_default(),
                    value["pip"]["source"].as_str().unwrap_or_default(),
                    value["fingerprint"].as_str().unwrap_or_default()
                )
            }))
        }
        Cmd::LayoutGet { project, json } => {
            let value = cc_core::command::render::layout_get(&data_dir(cli), project)?;
            Ok(output(&value, (*json).into(), || {
                format!("layout: {}", value["layout"])
            }))
        }
        Cmd::LayoutSet {
            project,
            position,
            x_pct,
            y_pct,
            width_pct,
            shape,
            transition,
            json,
        } => {
            let value = cc_core::command::render::layout_set(
                &data_dir(cli),
                project,
                cc_core::command::render::LayoutPatch {
                    position: position.as_deref(),
                    x_pct: *x_pct,
                    y_pct: *y_pct,
                    width_pct: *width_pct,
                    shape: shape.as_deref(),
                    transition: transition.as_deref(),
                },
            )?;
            Ok(output(&value, (*json).into(), || {
                format!("layout_set: {}", value["layout"])
            }))
        }
        Cmd::ArtifactClean {
            project,
            stage,
            confirm,
            json,
        } => {
            let value = cc_core::command::render::artifact_clean(
                &data_dir(cli),
                project,
                stage,
                (*confirm).into(),
            )?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "artifact_clean: {} removed, {} records removed",
                    value["removed"]
                        .as_array()
                        .map(|a| a.len())
                        .unwrap_or_default(),
                    value["records_removed"]
                        .as_array()
                        .map(|a| a.len())
                        .unwrap_or_default()
                )
            }))
        }
        Cmd::FinalVideo { project, json } => {
            let value = cc_core::command::render::final_video(&data_dir(cli), project)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "final_video: done={}, {}",
                    value["done"].as_bool().unwrap_or(false),
                    value["path_abs"].as_str().unwrap_or_default()
                )
            }))
        }
        Cmd::Download {
            project,
            out_dir,
            json,
        } => {
            let target = out_dir
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let value = cc_core::command::render::download(&data_dir(cli), project, &target)?;
            Ok(output(&value, (*json).into(), || {
                format!("download: {}", value["saved"].as_str().unwrap_or_default())
            }))
        }
        Cmd::VoiceSynthesize {
            project,
            pages,
            engine,
            json,
        } => {
            let preference = match engine.as_deref() {
                Some(name) => name.parse::<cc_core::TtsEngineKind>()?,
                None => cc_core::TtsEngineKind::Fake,
            };
            let report = cc_core::command::voice::synthesize(
                &data_dir(cli),
                project,
                preference,
                pages.as_deref(),
                &cc_core::SynthesisOptions::default(),
            )?;
            let value = cc_core::command::voice::report_json(project, &report);
            let summary = || {
                let pages = |pages: &[u32]| {
                    pages
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let mut line = format!(
                    "voice_synthesize: generated=[{}] skipped=[{}]",
                    pages(&report.generated),
                    pages(&report.skipped)
                );
                if !report.failed_pages.is_empty() {
                    let failed = report
                        .failed_pages
                        .iter()
                        .map(|f| f.page.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    line.push_str(&format!(" failed=[{failed}]"));
                }
                line.push_str(&format!(" total={:.3}s", report.total_duration_sec));
                line
            };
            if report.failed_pages.is_empty() {
                return Ok(output(&value, (*json).into(), summary));
            }
            // partial failure: still report everything that landed, but fail the command
            println!("{}", output(&value, (*json).into(), summary));
            std::process::exit(1);
        }
        Cmd::PersonMatte { project, json } => {
            let model = cc_core::runtime::model_path();
            let mut engine = cc_core::runtime::matte_engine()?;
            let value = cc_core::command::person::matte(
                &data_dir(cli),
                project,
                engine.as_mut(),
                &model,
                None,
            )?;
            Ok(output(&value, (*json).into(), || {
                if value["skipped"].as_bool().unwrap_or(false) {
                    return format!(
                        "person_matte: skipped (fingerprint unchanged) -> {}",
                        value["preview"].as_str().unwrap_or_default()
                    );
                }
                format!(
                    "person_matte: {} frames {}x{}@{:.0} -> {} ({:.1}s)",
                    value["frames"].as_u64().unwrap_or_default(),
                    value["width"].as_u64().unwrap_or_default(),
                    value["height"].as_u64().unwrap_or_default(),
                    value["fps"].as_f64().unwrap_or_default(),
                    cc_core::matte::ALPHA_MOV,
                    value["elapsed_sec"].as_f64().unwrap_or_default()
                )
            }))
        }
        Cmd::PersonVideoSet {
            project,
            source,
            json,
        } => {
            let value = cc_core::command::person::video_set(&data_dir(cli), project, source)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "person_video_set: {} -> {}",
                    value["path"].as_str().unwrap_or_default(),
                    value["path_abs"].as_str().unwrap_or_default()
                )
            }))
        }
        Cmd::PersonStatus { project, json } => {
            let value = cc_core::command::person::status(&data_dir(cli), project)?;
            Ok(output(&value, (*json).into(), || {
                format!(
                    "person_status: video={}, matte={}",
                    value["has_video"].as_bool().unwrap_or(false),
                    value["matte_done"].as_bool().unwrap_or(false)
                )
            }))
        }
        Cmd::PersonMatteSkip { project, json } => {
            let value = cc_core::command::person::skip(&data_dir(cli), project)?;
            Ok(output(&value, (*json).into(), || {
                "person_matte_skip: card fallback selected".to_string()
            }))
        }
        Cmd::VoiceConfigure {
            project,
            audio,
            api_key,
            engine,
            edge_voice,
            json,
        } => {
            let engine = engine
                .as_deref()
                .map(str::parse::<cc_core::TtsEngineKind>)
                .transpose()?;
            let edge_voice = edge_voice
                .as_deref()
                .map(cc_core::EdgeVoice::parse)
                .transpose()?;
            let value = cc_core::command::voice::configure(
                &data_dir(cli),
                project,
                api_key.as_deref(),
                audio.as_deref(),
                None,
                engine,
                edge_voice,
            )?;
            Ok(output(&value, (*json).into(), || {
                let mut line = "voice_configure:".to_string();
                if let Some(a) = value["audio"].as_object() {
                    line.push_str(&format!(
                        " {} ({}Hz {}ch {:.1}s)",
                        a["path"].as_str().unwrap_or_default(),
                        a["sample_rate"].as_u64().unwrap_or_default(),
                        a["channels"].as_u64().unwrap_or_default(),
                        a["duration_sec"].as_f64().unwrap_or_default()
                    ));
                }
                if value["api_key_stored"].as_bool().unwrap_or(false) {
                    line.push_str(" api_key=stored");
                }
                line.push_str(&format!(
                    " tts={}/{}",
                    value["tts"]["engine"].as_str().unwrap_or_default(),
                    value["tts"]["edge_voice"].as_str().unwrap_or_default()
                ));
                line
            }))
        }
        Cmd::VoiceConsent {
            project,
            grant,
            revoke,
            json,
        } => {
            if *grant && *revoke {
                return Err(cc_core::Error::InvalidArgument(
                    "--grant 与 --revoke 互斥".into(),
                ));
            }
            let value = if *grant {
                cc_core::command::voice::consent_set(&data_dir(cli), project, true.into())?
            } else if *revoke {
                cc_core::command::voice::consent_set(&data_dir(cli), project, false.into())?
            } else {
                cc_core::command::voice::consent_get(&data_dir(cli), project)?
            };
            Ok(output(&value, (*json).into(), || {
                format!(
                    "voice_consent: granted={}",
                    value["granted"].as_bool().unwrap_or(false)
                )
            }))
        }
    }
}
