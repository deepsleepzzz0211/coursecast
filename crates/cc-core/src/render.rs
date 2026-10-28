//! Render stage (ticket 08): one ffmpeg call produces `output/final.mp4` — concat image
//! track (`work/concat.txt`, durations per the timeline formula, `-t {T}` hard lock),
//! `work/audio/voice_full.wav`, and the looped person track overlaid as picture-in-
//! picture (`-stream_loop -1`). Hard cuts are the default transition; `transition =
//! "fade"` fades every page to white independently (0.3 s, timeline-enabled fade
//! windows — no xfade chain). When matte was skipped (no `work/person/alpha.mov`),
//! the original video is wrapped in a solid white rounded card instead.
//!
//! Layout comes from `layout.toml` (PRD §6.5); the render fingerprint binds the
//! timeline fingerprint + layout + ffmpeg version + encode params (PRD §6.6.1).

use crate::error::{Error, Result};
use crate::project::{save_project, Project};
use crate::stage;
use crate::state::Stage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Key of this stage's record in `project.json` `stages`.
pub const STAGE_KEY: &str = "render";
/// Final video, relative to the project directory.
pub const OUTPUT_MP4: &str = "output/final.mp4";
/// Layout parameters (PRD §6.5), relative to the project directory.
pub const LAYOUT_TOML: &str = "layout.toml";
/// Matte output (ticket 09): person video with alpha, looped for the overlay.
pub const PERSON_ALPHA: &str = "work/person/alpha.mov";
/// Original person video, used when matte was skipped (card branch).
pub const PERSON_SOURCE: &str = "source/person.mp4";

/// Per-page fade-to-white length (PRD §8.7).
pub const FADE_TRANSITION_SEC: f64 = 0.3;
/// PiP margins from the PRD §8.6 command (bottom positions).
pub const PIP_MARGIN_RIGHT: i64 = 80;
pub const PIP_MARGIN_BOTTOM: i64 = 60;
/// Card branch: white border around the original video, in px.
pub const CARD_MARGIN_PX: i64 = 16;

pub const DEFAULT_POSITION: &str = "bottom_right";
pub const DEFAULT_WIDTH_PCT: f64 = 20.0;
pub const DEFAULT_SHAPE: &str = "rounded_rect";
pub const DEFAULT_TRANSITION: &str = "hard";

/// Minimum/maximum width for a free placement (percent of canvas width).
pub const MIN_FREE_WIDTH_PCT: f64 = 5.0;
pub const MAX_FREE_WIDTH_PCT: f64 = 40.0;

/// Where the person sits: either a legacy three-preset position or a free normalized
/// anchor. Free coordinates are 0–100 of the *remaining* canvas space, so 100 means
/// flush with the right/bottom edge for any person aspect ratio.
#[derive(Debug, Clone, PartialEq)]
pub enum Placement {
    Free { x_pct: f64, y_pct: f64 },
    Preset(String),
}

/// Layout parameters (PRD story 18: 人像位置 / 宽度 / 形状 / 翻页动效).
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub placement: Placement,
    /// Person width as percent of canvas width.
    pub width_pct: f64,
    /// `rounded_rect` | `circle`
    pub shape: String,
    /// `hard` (default) | `fade` (per-page 0.3 s fade to white)
    pub transition: String,
}

pub fn default_layout() -> Layout {
    Layout {
        placement: Placement::Preset(DEFAULT_POSITION.into()),
        width_pct: DEFAULT_WIDTH_PCT,
        shape: DEFAULT_SHAPE.into(),
        transition: DEFAULT_TRANSITION.into(),
    }
}

impl Serialize for Layout {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let (position, x_pct, y_pct) = match &self.placement {
            Placement::Preset(position) => (Some(position.as_str()), None, None),
            Placement::Free { x_pct, y_pct } => (None, Some(*x_pct), Some(*y_pct)),
        };
        RawLayoutOut {
            position,
            x_pct,
            y_pct,
            width_pct: self.width_pct,
            shape: &self.shape,
            transition: &self.transition,
        }
        .serialize(serializer)
    }
}

impl Layout {
    /// Legacy three-preset layout.
    pub fn preset(
        position: impl Into<String>,
        width_pct: f64,
        shape: impl Into<String>,
        transition: impl Into<String>,
    ) -> Self {
        Layout {
            placement: Placement::Preset(position.into()),
            width_pct,
            shape: shape.into(),
            transition: transition.into(),
        }
    }

    /// Free normalized placement (`x_pct` / `y_pct` in 0–100 of remaining space).
    pub fn free(
        x_pct: f64,
        y_pct: f64,
        width_pct: f64,
        shape: impl Into<String>,
        transition: impl Into<String>,
    ) -> Self {
        Layout {
            placement: Placement::Free { x_pct, y_pct },
            width_pct,
            shape: shape.into(),
            transition: transition.into(),
        }
    }

    /// Canonical layout string fed into the render fingerprint. Preset output stays
    /// byte-identical to the pre-free-layout formate so old films keep their record.
    pub fn canonical(&self) -> String {
        let placement = match &self.placement {
            Placement::Preset(position) => format!("position={position}"),
            Placement::Free { x_pct, y_pct } => {
                format!("x_pct={x_pct:.3},y_pct={y_pct:.3}")
            }
        };
        format!(
            "{placement},width_pct={:.1},shape={},transition={}",
            self.width_pct, self.shape, self.transition
        )
    }
}

fn is_one_of(value: &str, allowed: &[&str]) -> bool {
    allowed.contains(&value)
}

/// The persisted TOML shape. `position` and `x_pct`/`y_pct` are two alternative
/// placement forms; mixing them is rejected by `parse_layout`.
#[derive(Deserialize)]
struct RawLayout {
    position: Option<String>,
    x_pct: Option<f64>,
    y_pct: Option<f64>,
    width_pct: f64,
    shape: String,
    transition: String,
}

#[derive(Serialize)]
struct RawLayoutOut<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    x_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y_pct: Option<f64>,
    width_pct: f64,
    shape: &'a str,
    transition: &'a str,
}

/// Parse layout.toml content. Strict: a complete placement and all shape fields are
/// required; unknown values are rejected (layout.toml is file-as-truth written by
/// layout_set or the AI).
pub fn parse_layout(text: &str) -> Result<Layout> {
    let bad = |message: String| Error::InvalidLayout { message };
    let raw: RawLayout =
        toml::from_str(text).map_err(|e| bad(format!("not a valid layout table: {e}")))?;
    let placement = match (raw.position, raw.x_pct, raw.y_pct) {
        (Some(position), None, None) => {
            if !is_one_of(&position, &["bottom_right", "bottom_left", "right_bar"]) {
                return Err(bad(format!(
                    "unknown position `{position}`: expected bottom_right | bottom_left | right_bar"
                )));
            }
            Placement::Preset(position)
        }
        (None, Some(x_pct), Some(y_pct)) => {
            if !(0.0..=100.0).contains(&x_pct) || !(0.0..=100.0).contains(&y_pct) {
                return Err(bad(format!(
                    "x_pct and y_pct must be in [0, 100], got {x_pct} / {y_pct}"
                )));
            }
            if !(MIN_FREE_WIDTH_PCT..=MAX_FREE_WIDTH_PCT).contains(&raw.width_pct) {
                return Err(bad(format!(
                    "free width_pct must be in [5, 40], got {}",
                    raw.width_pct
                )));
            }
            Placement::Free { x_pct, y_pct }
        }
        (Some(_), _, _) => {
            return Err(bad(
                "layout cannot mix `position` with `x_pct`/`y_pct`; choose one placement form"
                    .into(),
            ))
        }
        (None, _, _) => {
            return Err(bad(
                "layout needs `position` or both `x_pct` and `y_pct`".into()
            ))
        }
    };
    if !(raw.width_pct > 0.0 && raw.width_pct <= 100.0) {
        return Err(bad(format!(
            "width_pct must be in (0, 100], got {}",
            raw.width_pct
        )));
    }
    if !is_one_of(&raw.shape, &["rounded_rect", "circle"]) {
        return Err(bad(format!(
            "unknown shape `{}`: expected rounded_rect | circle",
            raw.shape
        )));
    }
    if !is_one_of(&raw.transition, &["hard", "fade"]) {
        return Err(bad(format!(
            "unknown transition `{}`: expected hard | fade",
            raw.transition
        )));
    }
    Ok(Layout {
        placement,
        width_pct: raw.width_pct,
        shape: raw.shape,
        transition: raw.transition,
    })
}

/// Load layout.toml from the project directory; missing file → defaults.
pub fn load_layout(path: &Path) -> Result<Layout> {
    if !path.is_file() {
        return Ok(default_layout());
    }
    let text = fs::read_to_string(path)?;
    parse_layout(&text)
}

/// Write `layout.toml` atomically (tmp → rename), validating the serialized form
/// through `parse_layout` first so a saved file always reads back (文件即真相: the
/// AI may also hand-edit it; both writers go through the same strict parser).
pub fn save_layout(layout: &Layout, project_dir: &Path) -> Result<()> {
    let text = toml::to_string_pretty(layout).map_err(|e| Error::InvalidLayout {
        message: format!("cannot serialize layout: {e}"),
    })?;
    // the value is already validated by the caller; this catches serialization drift
    parse_layout(&text)?;
    let tmp = project_dir.join("layout.toml.tmp");
    fs::write(&tmp, &text)?;
    fs::rename(&tmp, project_dir.join(LAYOUT_TOML))?;
    Ok(())
}

/// Render input fingerprint: timeline fingerprint + layout + ffmpeg version + encode
/// params + resolution (PRD §6.6.1). Changing the layout invalidates only the render.
pub fn render_fingerprint(
    timeline_fp: &str,
    layout: &Layout,
    ffmpeg_version: &str,
    resolution: &str,
) -> String {
    let canonical = format!(
        "cc-render/v1\ntimeline={timeline_fp}\nlayout={}\nffmpeg={ffmpeg_version}\n\
         encode=v=libx264,preset=medium,crf=20,pix_fmt=yuv420p,c=a=aac,b=192k,fps=30\nres={resolution}",
        layout.canonical(),
    );
    stage::sha256_hex(canonical.as_bytes())
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderSummary {
    /// Path relative to the project directory.
    pub output_path: String,
    pub total_sec: f64,
    pub turn_points_sec: Vec<f64>,
    /// `alpha` (matted overlay) or `card` (matte skipped, original wrapped in a card).
    pub pip_source: String,
    /// Person input used, relative to the project directory.
    pub person_input: String,
    pub layout: Layout,
    pub fingerprint: String,
    /// True when the fingerprint + depends_on matched and final.mp4 was intact: nothing
    /// re-rendered (ticket 10 失效传播 / 断点续跑).
    pub skipped: bool,
}

/// Run the render stage. Caller must hold the project lock. State machine: enters
/// `rendering` (from `matte_done`, `tts_done` when matte was skipped, or `failed` for
/// a retry), then `done` on success / `failed` on ffmpeg failure.
pub fn run_locked(project: &mut Project, dir: &Path) -> Result<RenderSummary> {
    // 0. a failure BEFORE rendering means the audio track is not trustworthy (e.g. a
    // partially failed TTS re-run). Re-encoding that track and marking the project
    // done would silently ship the old narration; render may only retry its own
    // failure or fall back to the card after a matte failure.
    if project.stage == Stage::Failed {
        if let Some(failed) = &project.failed {
            if !matches!(failed.stage, Stage::Rendering | Stage::MatteRunning) {
                return Err(Error::UpstreamFailed {
                    stage: failed.stage,
                });
            }
        }
    }

    // 1. timeline record must exist: render depends on its fingerprint
    let timeline_fp = project
        .stages
        .get(crate::timeline::STAGE_KEY)
        .map(|r| r.input_fingerprint.clone())
        .ok_or_else(|| Error::StageMissing {
            stage: crate::timeline::STAGE_KEY.into(),
        })?;

    // 2. its artifacts must be on disk (concat.txt is consumed as-is)
    for rel in [crate::timeline::CONCAT_TXT, crate::timeline::VOICE_FULL] {
        if !dir.join(rel).is_file() {
            return Err(Error::StageMissing {
                stage: crate::timeline::STAGE_KEY.into(),
            });
        }
    }

    // 3. page timings from concat.txt (file-as-truth; timeline wrote durations per formula)
    let text = fs::read_to_string(dir.join(crate::timeline::CONCAT_TXT))?;
    let pages = parse_concat(&text)?;
    if pages.is_empty() {
        return Err(Error::StageMissing {
            stage: crate::timeline::STAGE_KEY.into(),
        });
    }
    let total_sec: f64 = pages.iter().map(|(_, d)| d).sum();
    let mut turn_points_sec = Vec::with_capacity(pages.len().saturating_sub(1));
    let mut acc = 0.0;
    for (i, (_, d)) in pages.iter().enumerate() {
        acc += d;
        if i + 1 < pages.len() {
            turn_points_sec.push(acc);
        }
    }

    // 4. layout
    let layout = load_layout(&dir.join(LAYOUT_TOML))?;

    // 5. person branch: matted alpha overlay, or card-wrapped original when matte
    //    was skipped (alpha.mov absent). An alpha produced for a different clip must
    //    not be composited: newer matte records bind the source video's hash, so a
    //    mismatch falls back to the card. (Records without a video binding are
    //    hand-placed/legacy and stay trusted.)
    let alpha_matches_video = match project.stages.get(crate::matte::STAGE_KEY) {
        Some(record) if record.state == "success" && dir.join(PERSON_SOURCE).is_file() => {
            let current = stage::sha256_file(&dir.join(PERSON_SOURCE))?;
            stage::optional_content_dep_matches(record, "video", &current)
        }
        _ => true,
    };
    let (pip_source, person_rel) = if dir.join(PERSON_ALPHA).is_file() && alpha_matches_video {
        ("alpha", PERSON_ALPHA)
    } else if dir.join(PERSON_SOURCE).is_file() {
        ("card", PERSON_SOURCE)
    } else if dir.join(PERSON_ALPHA).is_file() {
        ("alpha", PERSON_ALPHA)
    } else {
        return Err(Error::PersonMissing {
            alpha: PERSON_ALPHA.into(),
            fallback: PERSON_SOURCE.into(),
        });
    };

    // 6. canvas from the project resolution
    let (canvas_w, canvas_h) = parse_resolution(&project.resolution)?;

    // 7. fingerprint + ffmpeg version (both needed before the skip decision)
    let version = crate::ffmpeg::version()?;
    let fingerprint = render_fingerprint(&timeline_fp, &layout, &version, &project.resolution);

    // 8. idempotent skip (ticket 10): the fingerprint covers timeline/layout/ffmpeg/
    //    encode/res, but NOT the content of voice_full (a re-synthesized page can keep
    //    every duration) or the person track — those ride in depends_on as content
    //    hashes. final.mp4 must still match its record. An explicit retry from `failed`
    //    never skips: the user asked this stage to run again.
    let voice_full_hash = stage::sha256_file(&dir.join(crate::timeline::VOICE_FULL))?;
    let person_hash = stage::sha256_file(&dir.join(person_rel))?;
    if project.stage != Stage::Failed
        && stage::verify(
            project,
            dir,
            &stage::VerifySpec {
                key: STAGE_KEY,
                input_fingerprint: &fingerprint,
                stage_deps: &[],
                content_deps: &[
                    ("voice_full", voice_full_hash.as_str()),
                    ("person", person_hash.as_str()),
                ],
            },
        )
    {
        return Ok(RenderSummary {
            output_path: OUTPUT_MP4.into(),
            total_sec,
            turn_points_sec,
            pip_source: pip_source.into(),
            person_input: person_rel.into(),
            layout,
            fingerprint,
            skipped: true,
        });
    }

    // 9. state machine: legal from matte_done / tts_done (skip) / done (invalidation) /
    //    failed (retry). A kill -9 residue at `rendering` re-enters as resume.
    if project.stage != Stage::Rendering {
        project.transition(Stage::Rendering)?;
    }
    // A retry from `failed` must clear the previous failure record: leaving it behind
    // makes status/next lie after a successful render (票 16 acceptance caught this).
    project.failed = None;

    // 10. ffmpeg argv
    let args = build_args(
        dir, canvas_w, canvas_h, &pages, total_sec, &layout, pip_source, person_rel,
    )?;
    fs::create_dir_all(dir.join("output"))?;

    if let Err(err) = crate::ffmpeg::run(&args) {
        project.fail(format!("render: {err}"))?;
        save_project(project, dir)?;
        return Err(err);
    }

    // 11. success: done state + stage record (PRD §6.6.1). depends_on binds the film to
    //     the exact inputs it consumed: the timeline fingerprint it was built from plus
    //     the content hashes of the two inputs the fingerprint cannot express.
    project.transition(Stage::Done)?;
    let outputs = vec![stage::hash_artifact(dir, OUTPUT_MP4)?];
    let record = stage::success(
        STAGE_KEY,
        fingerprint.clone(),
        format!("ffmpeg/{version}+cc-core/{}", env!("CARGO_PKG_VERSION")),
        outputs,
        BTreeMap::from([
            ("timeline".to_string(), timeline_fp),
            ("voice_full".to_string(), voice_full_hash),
            ("person".to_string(), person_hash),
        ]),
    );
    stage::land(project, dir, STAGE_KEY, record)?;

    Ok(RenderSummary {
        output_path: OUTPUT_MP4.into(),
        total_sec,
        turn_points_sec,
        pip_source: pip_source.into(),
        person_input: person_rel.into(),
        layout,
        fingerprint,
        skipped: false,
    })
}

/// Parse `work/concat.txt`: `(page, display_sec)` per `file`+`duration` pair. The
/// repeated last image has no duration line and is not a page.
fn parse_concat(text: &str) -> Result<Vec<(u32, f64)>> {
    let mut pages = Vec::new();
    let mut current: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("file ") {
            let name = rest.trim().trim_matches('\'');
            let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
            // slide_NNN.png -> NNN
            let digits: String = stem
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            current = digits.parse::<u32>().ok();
        } else if let Some(rest) = line.strip_prefix("duration ") {
            if let (Some(page), Ok(d)) = (current, rest.trim().parse::<f64>()) {
                pages.push((page, d));
            }
        }
    }
    Ok(pages)
}

fn parse_resolution(resolution: &str) -> Result<(i64, i64)> {
    let parts: Vec<&str> = resolution.split('x').collect();
    if parts.len() == 2 {
        if let (Ok(w), Ok(h)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
            if w > 0 && h > 0 {
                return Ok((w, h));
            }
        }
    }
    Err(Error::InvalidResolution(resolution.to_string()))
}

/// Person width in px: `width_pct` of the canvas, rounded down to an even value
/// (yuv420p chroma alignment).
fn pip_width_px(canvas_w: i64, width_pct: f64) -> i64 {
    let w = ((canvas_w as f64) * width_pct / 100.0).floor() as i64;
    let w = w.max(2);
    w - (w % 2)
}

/// Rounded-rectangle alpha expression for geq, radius = min(W,H)/8. W/H are the
/// frame's own dimensions, so no probing of the person video is needed.
fn rounded_rect_expr() -> String {
    let r = "(min(W,H)/8)";
    format!(
        "if(between(X,{r},W-1-{r})+between(Y,{r},H-1-{r})\
+lt(hypot(X-{r},Y-{r}),{r})+lt(hypot(X-(W-1-{r}),Y-{r}),{r})\
+lt(hypot(X-{r},Y-(H-1-{r})),{r})+lt(hypot(X-(W-1-{r}),Y-(H-1-{r})),{r}),255,0)"
    )
}

/// Circle alpha expression (frame is square-cropped before this runs).
fn circle_expr() -> String {
    "if(lt(hypot(X-(W-1)/2,Y-(H-1)/2),(min(W,H)/2)),255,0)".into()
}

/// Build the full ffmpeg argv. Argument array only — never a shell string (spec MCP
/// 权限边界). Inputs: 0 = concat image track, 1 = voice_full.wav, 2 = person track
/// (looped). All filter expressions were verified against ffmpeg 8.1.
#[expect(
    clippy::too_many_arguments,
    reason = "ffmpeg argument assembly keeps the positional inputs explicit"
)]
fn build_args(
    dir: &Path,
    canvas_w: i64,
    canvas_h: i64,
    pages: &[(u32, f64)],
    total_sec: f64,
    layout: &Layout,
    pip_source: &str,
    person_rel: &str,
) -> Result<Vec<String>> {
    let mut filter = String::new();

    // 1. background: slides scaled onto the canvas with white pad, 30 fps
    filter.push_str(&format!(
        "[0:v]scale={canvas_w}:{canvas_h}:force_original_aspect_ratio=decrease,\
pad={canvas_w}:{canvas_h}:(ow-iw)/2:(oh-ih)/2:color=white,setsar=1,fps=30"
    ));
    if layout.transition == "fade" {
        // Per-page fade to white: the fade filters are start-anchored, so each window
        // is gated with `enable` — before/after the window the frame passes through
        // untouched. Page k fades out over [e-0.3, e] and page k+1 fades back in over
        // [e, e+0.3]: white exactly at the turn point. Pages shorter than 2 fades
        // (< 0.6 s) pass through un-faded.
        let mut s = 0.0;
        for (_, disp) in pages {
            let e = s + disp;
            if *disp >= 2.0 * FADE_TRANSITION_SEC {
                filter.push_str(&format!(
                    ",fade=t=in:st={s:.3}:d={FADE_TRANSITION_SEC:.3}:color=white\
:enable='between(t,{s:.3},{:.3})'",
                    s + FADE_TRANSITION_SEC
                ));
                filter.push_str(&format!(
                    ",fade=t=out:st={:.3}:d={FADE_TRANSITION_SEC:.3}:color=white\
:enable='between(t,{:.3},{e:.3})'",
                    e - FADE_TRANSITION_SEC,
                    e - FADE_TRANSITION_SEC
                ));
            }
            s = e;
        }
    }
    filter.push_str("[bg];");

    // 2. person track: alpha overlay (with optional shape mask) or card-wrapped original.
    // Free placement additionally caps the width so a tall source cannot overflow the
    // canvas height; legacy presets keep their historical geometry untouched.
    let mut pip_w = pip_width_px(canvas_w, layout.width_pct);
    if matches!(layout.placement, Placement::Free { .. }) {
        let info = crate::matte::probe_video(&dir.join(person_rel))?;
        let aspect = info.width as f64 / info.height.max(1) as f64;
        let max_content_h = canvas_h
            - if pip_source == "card" {
                2 * CARD_MARGIN_PX
            } else {
                0
            };
        let max_w = (max_content_h as f64 * aspect).floor() as i64;
        if max_w >= 2 {
            pip_w = pip_w.min(max_w - max_w % 2).max(2);
        }
    }
    match pip_source {
        // matte skipped: white rounded card wraps the original (opaque) video
        "card" => {
            let m = CARD_MARGIN_PX;
            filter.push_str(&format!(
                "[2:v]scale={pip_w}:-2,pad=iw+{m2}:ih+{m2}:{m}:{m}:color=white,\
format=yuva444p,geq=lum='p(X,Y)':cb='p(X,Y)':cr='p(X,Y)':a='{expr}'[p];",
                m2 = 2 * m,
                expr = rounded_rect_expr(),
            ));
        }
        // matted alpha track; the shape mask must intersect the source alpha (geq
        // cannot read the alpha plane, so: extract it, build the shape mask, blend
        // with min, merge back)
        _ => {
            // right vertical strip: full canvas height, cropped to the configured
            // width. Shape masks don't apply to a strip layout.
            if matches!(&layout.placement, Placement::Preset(position) if position == "right_bar") {
                filter.push_str(&format!(
                    "[2:v]scale=-2:{canvas_h},crop=w='min(iw,{pip_w})':h=ih\
:x='(iw-ow)/2':y=0,format=yuva420p[p];"
                ));
            } else {
                let (scale, expr) = match layout.shape.as_str() {
                    "circle" => (
                        format!(
                            "[2:v]scale={pip_w}:-2,crop=w='min(iw,ih)':h='min(iw,ih)'\
:x='(iw-ow)/2':y='(ih-oh)/2',format=yuva444p,"
                        ),
                        circle_expr(),
                    ),
                    _ => (
                        format!("[2:v]scale={pip_w}:-2,format=yuva444p,"),
                        rounded_rect_expr(),
                    ),
                };
                filter.push_str(&format!(
                    "{scale}split=3[px][pa][pb];[pa]alphaextract[am];\
[pb]format=gray,geq=lum='{expr}'[sm];[am][sm]blend=all_expr='min(A,B)'[fm];\
[px][fm]alphamerge[p];"
                ));
            }
        }
    }

    // 3. overlay at the configured position. Free placement uses the remaining canvas
    // space, so 0/100 means flush left/top and right/bottom for any person aspect.
    let overlay = match &layout.placement {
        Placement::Free { x_pct, y_pct } => {
            format!("overlay=x='(W-w)*{x_pct:.6}/100':y='(H-h)*{y_pct:.6}/100':shortest=0")
        }
        Placement::Preset(position) => match position.as_str() {
            "right_bar" => "overlay=x=W-w:y=0:shortest=0".to_string(),
            "bottom_left" => {
                format!("overlay=x={PIP_MARGIN_RIGHT}:y=H-h-{PIP_MARGIN_BOTTOM}:shortest=0")
            }
            _ => {
                format!("overlay=x=W-w-{PIP_MARGIN_RIGHT}:y=H-h-{PIP_MARGIN_BOTTOM}:shortest=0")
            }
        },
    };
    filter.push_str(&format!("[bg][p]{overlay}[v]"));

    let args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        // concat demuxer logs "Error during demuxing" for a missing/corrupt slide but
        // still exits 0 — without -xerror render would "succeed" with a wrong film.
        "-xerror".into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        dir.join(crate::timeline::CONCAT_TXT).display().to_string(),
        "-i".into(),
        dir.join(crate::timeline::VOICE_FULL).display().to_string(),
        "-stream_loop".into(),
        "-1".into(),
        "-i".into(),
        dir.join(person_rel).display().to_string(),
        "-filter_complex".into(),
        filter,
        "-map".into(),
        "[v]".into(),
        "-map".into(),
        "1:a".into(),
        // H.264 + AAC, +faststart (PRD §8.6); encode params are part of the fingerprint
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "medium".into(),
        "-crf".into(),
        "20".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "192k".into(),
        // hard lock: the repeated last image in concat.txt must not add duration, and
        // the looped person track must not extend the output
        "-t".into(),
        format!("{total_sec:.3}"),
        "-movflags".into(),
        "+faststart".into(),
        dir.join(OUTPUT_MP4).display().to_string(),
    ];
    Ok(args)
}
