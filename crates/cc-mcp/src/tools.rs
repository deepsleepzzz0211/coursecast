//! The 16 MCP tools (PRD §7.2). Every tool is a thin wrapper over the same cc-core
//! call the CLI subcommand makes and returns JSON shaped identically to the CLI's
//! `--json` output (spec story 26/29: 能力与 UI 同源). Business failures come back as
//! `ok:false` + `error{code,message}` — the exact shape the CLI prints on exit 1 —
//! so an AI client sees one error contract on both shells.

use crate::paths::resolve_input;
use cc_core::{load_project, validate_project_id};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

pub(crate) struct ToolContext {
    pub(crate) data_dir: PathBuf,
}

/// One tool's static description: name, prose for the model, JSON Schema for args.
pub(crate) struct ToolSpec {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) schema: Value,
}

fn schema(properties: Value, required: &[&str]) -> Value {
    let mut object = Map::new();
    object.insert("type".into(), json!("object"));
    object.insert("properties".into(), properties);
    if !required.is_empty() {
        object.insert(
            "required".into(),
            Value::Array(required.iter().map(|r| json!(r)).collect()),
        );
    }
    Value::Object(object)
}

fn prop(kind: &str, description: &str) -> Value {
    json!({ "type": kind, "description": description })
}

pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "project_create",
            description: "新建课影项目：建立 projects/<id>/ 目录与 project.json，返回 project_id 与目录",
            schema: schema(
                json!({
                    "name": prop("string", "项目名（1-100 字符）"),
                    "resolution": prop("string", "输出分辨率 WIDTHxHEIGHT，默认 1920x1080"),
                }),
                &["name"],
            ),
        },
        ToolSpec {
            name: "project_list",
            description: "列出所有项目（新建在前），返回 project_id、名称、阶段与恢复进度；续跑已有项目时先调它拿到 project_id",
            schema: schema(json!({}), &[]),
        },
        ToolSpec {
            name: "project_status",
            description: "查询项目当前阶段、四个输入（课件/逐字稿/参考音频/出镜视频）就绪情况、人像/抠像产物详情、缺失项与 next 建议",
            schema: schema(
                json!({ "project_id": prop("string", "项目 ID（8-32 位 [a-z0-9]）") }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "project_delete",
            description: "删除项目目录中的全部文件（不可逆，测试/废弃项目清理用）；必须显式 confirm:true",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "confirm": prop("boolean", "必须显式传 true 才执行（不可逆操作）"),
                }),
                &["project_id", "confirm"],
            ),
        },
        ToolSpec {
            name: "slide_render",
            description: "渲染课件（PDF 或图片文件夹）为 work/slides/slide_NNN.png；相对路径相对项目根解析且不得逃逸，绝对路径可用",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "source_path": prop("string", "课件路径：相对项目根（如 source/deck.pdf）或绝对路径；图片文件夹按文件名数字排序"),
                }),
                &["project_id", "source_path"],
            ),
        },
        ToolSpec {
            name: "script_get",
            description: "读回逐字稿 script.md 的结构化内容（按页）",
            schema: schema(
                json!({ "project_id": prop("string", "项目 ID") }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "script_set",
            description: "写入逐字稿（支持只改某几页，页号相对已渲染的课件页数校验）",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "pages": {
                        "type": "array",
                        "description": "要写入的页：[{page: 1-based 页号, text: 该页讲稿}]",
                        "items": {
                            "type": "object",
                            "properties": {
                                "page": prop("integer", "1-based 页号"),
                                "text": prop("string", "该页逐字稿"),
                            },
                            "required": ["page", "text"],
                        },
                    },
                }),
                &["project_id", "pages"],
            ),
        },
        ToolSpec {
            name: "script_import_notes",
            description: "从 PPT 备注批量导入逐字稿（每页备注 → script.md 对应页，无备注的页留空）",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "pptx_path": prop("string", "pptx 路径，缺省 source/deck.pptx"),
                }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "voice_configure",
            description: "配置声音：参考音频转码为 24kHz 单声道 wav 入库，把 MiMo API Key 存入 Windows 凭据管理器（密钥绝不回显），和/或记录《声音授权确认》（consent）。克隆引擎 mimo 在未确认时会拒绝合成",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "engine": prop("string", "TTS 引擎意图：fake | mimo | edge | sapi（决定 next 建议文案）"),
                    "ref_audio": prop("string", "参考录音路径（m4a/mp3/wav）"),
                    "api_key": prop("string", "MiMo API Key（只入凭据存储，不出现在返回值）"),
                    "consent": prop("boolean", "《声音授权确认》= true 记录，false 撤销；克隆前必须为 true"),
                    "edge_voice": prop("string", "Edge 音色：xiaoxiao（默认）| yunxi；engine=edge 时持久化该选择"),
                }),
                &["project_id", "engine"],
            ),
        },
        ToolSpec {
            name: "voice_synthesize",
            description: "按页合成语音到 work/audio/slide_NNN.wav（输入指纹幂等：相同输入已合成页自动跳过；失败页单独可重试）",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "pages": prop("string", "页子集，如 `3`、`1-5`、`1-3,7`；缺省全部页"),
                    "engine": prop("string", "TTS 引擎：fake（离线默认）| mimo（克隆，沿降级链滑落）| edge（Edge 朗读）| sapi（Windows 系统语音）"),
                }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "person_matte",
            description: "抠像出镜视频：work/person/alpha.mov（保 alpha）+ 棋盘格预览帧；输入指纹不变则幂等跳过",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "video_path": prop("string", "出镜视频路径；给出则入库为 source/person.mp4，缺省用已入库视频"),
                }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "person_status",
            description: "查询出镜视频与抠像产物：has_video、matte_done、人像原片/预览帧/alpha 的绝对路径",
            schema: schema(
                json!({ "project_id": prop("string", "项目 ID") }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "person_matte_skip",
            description: "跳过抠像：清掉 alpha 与抠像记录，生成时用纯色圆角卡片包住原视频照样出片",
            schema: schema(
                json!({ "project_id": prop("string", "项目 ID") }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "layout_set",
            description: "设置版式 layout.toml（人像位置/宽度/形状/翻页动效）；只传要改的键，其余保持现值",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "position": prop("string", "旧版预设：bottom_right | bottom_left | right_bar；与 x_pct/y_pct 互斥"),
                    "x_pct": prop("number", "自由位置：水平 0-100（剩余画布空间的百分比）"),
                    "y_pct": prop("number", "自由位置：垂直 0-100（剩余画布空间的百分比）"),
                    "width_pct": prop("number", "人像宽度占画布百分比；自由位置 5-40"),
                    "shape": prop("string", "rounded_rect | circle"),
                    "transition": prop("string", "hard | fade（每页 0.3s 淡入淡出到白底）"),
                }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "render",
            description: "合成成片 output/final.mp4：内部先按时间轴公式重建整轨（幂等），再一次 ffmpeg 调用合成；版式变化只重渲成片",
            schema: schema(
                json!({ "project_id": prop("string", "项目 ID") }),
                &["project_id"],
            ),
        },
        ToolSpec {
            name: "artifact_clean",
            description: "清理中间文件与对应阶段记录（改稿重跑前用）：tts | matte | timeline | render | all；不可逆，必须显式 confirm:true",
            schema: schema(
                json!({
                    "project_id": prop("string", "项目 ID"),
                    "stage": prop("string", "tts | matte | timeline | render | all"),
                    "confirm": prop("boolean", "必须显式传 true 才执行（不可逆操作）"),
                }),
                &["project_id", "stage", "confirm"],
            ),
        },
    ]
}

// ---------------------------------------------------------------------------
// argument extraction
// ---------------------------------------------------------------------------

type Args = Map<String, Value>;

fn missing(key: &str) -> cc_core::Error {
    cc_core::Error::InvalidArgument(format!("缺少参数 `{key}`"))
}

fn arg_str(args: &Args, key: &str) -> cc_core::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| missing(key))
}

fn arg_opt_str(args: &Args, key: &str) -> cc_core::Result<Option<String>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(cc_core::Error::InvalidArgument(format!(
            "参数 `{key}` 必须是字符串"
        ))),
    }
}

fn arg_opt_f64(args: &Args, key: &str) -> cc_core::Result<Option<f64>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| cc_core::Error::InvalidArgument(format!("参数 `{key}` 必须是数字"))),
        Some(_) => Err(cc_core::Error::InvalidArgument(format!(
            "参数 `{key}` 必须是数字"
        ))),
    }
}

fn arg_opt_bool(args: &Args, key: &str) -> cc_core::Result<Option<bool>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(cc_core::Error::InvalidArgument(format!(
            "参数 `{key}` 必须是布尔值"
        ))),
    }
}

/// project_id validation doubles as the path-traversal guard for project lookup.
fn arg_project_id(args: &Args) -> cc_core::Result<String> {
    let id = arg_str(args, "project_id")?;
    validate_project_id(&id)?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// tool dispatch
// ---------------------------------------------------------------------------

/// Outcome of one tool call: the structured JSON body plus whether it is an error.
pub(crate) struct ToolOutcome {
    pub(crate) value: Value,
    pub(crate) is_error: bool,
}

impl ToolOutcome {
    fn ok(value: Value) -> Self {
        ToolOutcome {
            value,
            is_error: false,
        }
    }
    fn err(err: cc_core::Error) -> Self {
        ToolOutcome {
            value: cc_core::command::error_json(&err),
            is_error: true,
        }
    }
}

pub(crate) fn call_tool(ctx: &ToolContext, name: &str, args: &Args) -> ToolOutcome {
    match dispatch(ctx, name, args) {
        Ok(value) => ToolOutcome::ok(value),
        // tool-level failure with the CLI's error JSON shape (ok:false + error{code});
        // only protocol-shape problems (unparseable request) surface as MCP errors,
        // which rmcp handles before dispatch ever runs
        Err(err) => ToolOutcome::err(err),
    }
}

fn dispatch(ctx: &ToolContext, name: &str, args: &Args) -> cc_core::Result<Value> {
    let data_dir = &ctx.data_dir;
    match name {
        "project_create" => {
            let name_value = arg_str(args, "name")?;
            let resolution = arg_opt_str(args, "resolution")?;
            cc_core::command::project_create(data_dir, &name_value, resolution.as_deref())
        }
        "project_list" => cc_core::command::project_list(data_dir),
        "project_status" => {
            let id = arg_project_id(args)?;
            cc_core::command::project_status(data_dir, &id)
        }
        "project_delete" => {
            let id = arg_project_id(args)?;
            let confirm = arg_opt_bool(args, "confirm")?.unwrap_or(false);
            cc_core::command::project_delete(data_dir, &id, confirm.into())
        }
        "slide_render" => {
            let id = arg_project_id(args)?;
            let source = arg_str(args, "source_path")?;
            let (_, dir) = load_project(data_dir, &id)?;
            let source_path = resolve_input(&dir, &source)?;
            cc_core::command::deck::slide_render(data_dir, &id, &source_path.to_string_lossy())
        }
        "script_get" => {
            let id = arg_project_id(args)?;
            cc_core::command::deck::script_get(data_dir, &id)
        }
        "script_set" => {
            let id = arg_project_id(args)?;
            let raw = args.get("pages").cloned().ok_or_else(|| missing("pages"))?;
            let updates = cc_core::command::script::parse_updates(&raw)?;
            cc_core::command::script::set(data_dir, &id, &updates)
        }
        "script_import_notes" => {
            let id = arg_project_id(args)?;
            let pptx_arg =
                arg_opt_str(args, "pptx_path")?.unwrap_or_else(|| "source/deck.pptx".into());
            let (_, dir) = load_project(data_dir, &id)?;
            let pptx = resolve_input(&dir, &pptx_arg)?;
            cc_core::command::script::import_notes(
                data_dir,
                &id,
                Some(&pptx.to_string_lossy()),
                cc_core::command::script::ImportMode::Apply,
            )
        }
        "voice_configure" => {
            let id = arg_project_id(args)?;
            let engine: cc_core::TtsEngineKind = arg_str(args, "engine")?.parse()?;
            let edge_voice = arg_opt_str(args, "edge_voice")?
                .map(|v| cc_core::EdgeVoice::parse(&v))
                .transpose()?;
            let audio = arg_opt_str(args, "ref_audio")?;
            let api_key = arg_opt_str(args, "api_key")?;
            let consent = arg_opt_bool(args, "consent")?;
            let audio_path = if let Some(a) = audio {
                let (_, dir) = load_project(data_dir, &id)?;
                let path = resolve_input(&dir, &a)?;
                Some(path.to_string_lossy().to_string())
            } else {
                None
            };
            cc_core::command::voice::configure(
                data_dir,
                &id,
                api_key.as_deref(),
                audio_path.as_deref(),
                consent.map(Into::into),
                Some(engine),
                edge_voice,
            )
        }
        "voice_synthesize" => {
            let id = arg_project_id(args)?;
            let pages = arg_opt_str(args, "pages")?;
            let engine = match arg_opt_str(args, "engine")? {
                Some(name) => name.parse()?,
                None => cc_core::TtsEngineKind::Fake,
            };
            let report = cc_core::command::voice::synthesize(
                data_dir,
                &id,
                engine,
                pages.as_deref(),
                &Default::default(),
            )?;
            Ok(cc_core::command::voice::report_json(&id, &report))
        }
        "person_matte" => {
            let id = arg_project_id(args)?;
            let video = arg_opt_str(args, "video_path")?;
            if let Some(v) = video {
                let (_, dir) = load_project(data_dir, &id)?;
                let src = resolve_input(&dir, &v)?;
                cc_core::command::person::video_set(data_dir, &id, &src.to_string_lossy())?;
            }
            let model_path = cc_core::runtime::model_path();
            let mut engine = cc_core::runtime::matte_engine()?;
            cc_core::command::person::matte(data_dir, &id, engine.as_mut(), &model_path, None)
        }
        "person_status" => {
            let id = arg_project_id(args)?;
            cc_core::command::person::status(data_dir, &id)
        }
        "person_matte_skip" => {
            let id = arg_project_id(args)?;
            cc_core::command::person::skip(data_dir, &id)
        }
        "layout_set" => {
            let id = arg_project_id(args)?;
            let position = arg_opt_str(args, "position")?;
            let shape = arg_opt_str(args, "shape")?;
            let transition = arg_opt_str(args, "transition")?;
            cc_core::command::render::layout_set(
                data_dir,
                &id,
                cc_core::command::render::LayoutPatch {
                    position: position.as_deref(),
                    x_pct: arg_opt_f64(args, "x_pct")?,
                    y_pct: arg_opt_f64(args, "y_pct")?,
                    width_pct: arg_opt_f64(args, "width_pct")?,
                    shape: shape.as_deref(),
                    transition: transition.as_deref(),
                },
            )
        }
        "render" => {
            let id = arg_project_id(args)?;
            cc_core::command::render::render(data_dir, &id)
        }
        "artifact_clean" => {
            let id = arg_project_id(args)?;
            let stage = arg_str(args, "stage")?;
            let confirm = arg_opt_bool(args, "confirm")?.unwrap_or(false);
            cc_core::command::render::artifact_clean(data_dir, &id, &stage, confirm.into())
        }
        other => Err(cc_core::Error::InvalidArgument(format!(
            "unknown tool `{other}`"
        ))),
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------
