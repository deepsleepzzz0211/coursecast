//! person_matte stage (ticket 09): RVM MobileNetV3 matting over the person video.
//!
//! Pipeline per frame: ffmpeg decodes rgb24 → downscale to 512-wide inference size
//! (PRD §8.5: never feed 1080p straight to the model, 18x slower) → engine inference
//! with temporal rec state → alpha upscaled back to source resolution → despill +
//! 1px erosion → ProRes 4444 mov keeps alpha. A checkerboard preview of the middle
//! frame lands next to it for eyeballing edges. The inference backend is a trait
//! (spec 抠像: 推理后端封装为 trait 以便切换; license swap is ticket 15's business).
//!
//! matte fingerprint = video hash + model hash + downsample_ratio (spec 输入指纹模型).
//! State machine: tts_done → matte_running → matte_done; failed retries re-enter from
//! failed; invalidation re-enters from matte_done when the video/model changed and from
//! matte_running after a kill -9 (ticket 10). Re-running at matte_done with a matching
//! fingerprint and intact artifacts skips all work (重跑前先查已落盘产物).

use crate::error::{Error, Result};
use crate::project::{save_project, OutputArtifact, Project, StageRecord};
use crate::stage;
use crate::state::Stage;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The person video (PRD §6.5; same convention as status.rs).
pub const PERSON_VIDEO: &str = "source/person.mp4";
/// Matte artifacts live here.
pub const PERSON_DIR: &str = "work/person";
/// Alpha output (ProRes 4444, PRD §6.5/§8.5).
pub const ALPHA_MOV: &str = "work/person/alpha.mov";
/// Checkerboard preview frame.
pub const PREVIEW_PNG: &str = "work/person/preview.png";
/// Key of this stage's record in `project.json` `stages`.
pub const STAGE_KEY: &str = "matte";
/// Inference width (PRD §8.5: downsample_ratio=0.375 缩到 512 宽推理).
pub const INF_WIDTH: u32 = 512;
/// Fixed downsample_ratio fed to RVM (PRD §8.5).
pub const DOWNSAMPLE_RATIO: f32 = 0.375;
/// Default ONNX model location, relative to the working directory.
pub const DEFAULT_MODEL_REL: &str = "vendor/models/rvm_mobilenetv3_fp32.onnx";

const NEXT_MATTE_DONE: &str = "运行 render 渲染成片";

/// Inference backend seam (spec 抠像: 推理后端封装为 trait 以便切换). Implementations
/// hold their own temporal state; `reset` starts a new clip.
pub trait MatteEngine {
    /// Backend identifier used in tool_version, e.g. "rvm".
    fn name(&self) -> &'static str;
    /// Model identifier used in tool_version, e.g. "rvm_mobilenetv3_fp32".
    fn model(&self) -> &'static str;
    /// Inference resolution for a source frame of `src_w x src_h` (512-wide for RVM).
    fn infer_size(&self, src_w: u32, src_h: u32) -> (u32, u32);
    /// Infer one frame: RGB planar (CHW, 0..1) at `w x h` in → alpha planar (0..1) at
    /// the same size out, advancing temporal state. Must stay at the engine's
    /// `infer_size` — the caller owns all scaling.
    fn infer(&mut self, rgb_chw: &[f32], w: u32, h: u32) -> Result<Vec<f32>>;
    /// Clear temporal state (new clip).
    fn reset(&mut self);
}

/// Deterministic offline double for pipeline tests (spec Testing Decisions, 缝 2):
/// alpha is a centered circle with a 2px soft edge, independent of the pixels. Counts
/// infer() calls so tests can assert idempotent reruns never re-process.
#[derive(Debug)]
pub struct FakeMatteEngine {
    calls: std::sync::atomic::AtomicU32,
}

impl FakeMatteEngine {
    pub fn new() -> Self {
        FakeMatteEngine {
            calls: std::sync::atomic::AtomicU32::new(0),
        }
    }

    /// Number of infer() invocations so far.
    pub fn call_count(&self) -> u32 {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Default for FakeMatteEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MatteEngine for FakeMatteEngine {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn model(&self) -> &'static str {
        "fake-circle-mat"
    }

    fn infer_size(&self, src_w: u32, src_h: u32) -> (u32, u32) {
        infer_size_512(src_w, src_h)
    }

    fn infer(&mut self, _rgb_chw: &[f32], w: u32, h: u32) -> Result<Vec<f32>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (w, h) = (w as f32, h as f32);
        let r = w.min(h) * 0.35;
        let (cx, cy) = (w / 2.0, h / 2.0);
        Ok((0..h as usize)
            .flat_map(|y| (0..w as usize).map(move |x| (x as f32, y as f32)))
            .map(|(x, y)| {
                let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                ((r - d) / 2.0).clamp(0.0, 1.0)
            })
            .collect())
    }

    fn reset(&mut self) {}
}

/// One recurrent state tensor (RVM r1o..r4o fed back as r1i..r4i).
struct RecTensor {
    shape: Vec<i64>,
    data: Vec<f32>,
}

/// ORT allows exactly one Environment per process; every engine shares it.
static ORT_ENV: std::sync::OnceLock<ort::environment::Environment> = std::sync::OnceLock::new();

fn ort_env(fail: impl Fn(ort::Error) -> Error) -> Result<&'static ort::environment::Environment> {
    if let Some(env) = ORT_ENV.get() {
        return Ok(env);
    }
    match ort::init().build() {
        Ok(env) => Ok(ORT_ENV.get_or_init(|| env)),
        // another thread won the race and its environment is already installed
        Err(e) => match ORT_ENV.get() {
            Some(env) => Ok(env),
            None => Err(fail(e)),
        },
    }
}

/// Real backend: RVM MobileNetV3 fp32 through ONNX Runtime, CPU only (verification
/// report §3.2: 8.7 ms/frame at 512x288; no GPU needed). Temporal state advances
/// frame by frame (PRD §8.5: 逐帧送 rec / r1i..r4i 循环状态，seq_chunk=1 单帧推进).
pub struct RvmEngine {
    session: ort::session::Session,
    /// None until the first frame; frame 1 gets the 1x1x1x1 placeholder zeros the
    /// exported model initializes its recurrent state from.
    rec: Option<[RecTensor; 4]>,
}

impl std::fmt::Debug for RvmEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RvmEngine")
            .field("session", &"<onnx session>")
            .field("rec_initialized", &self.rec.is_some())
            .finish()
    }
}

impl RvmEngine {
    /// Load the ONNX model. Errors carry code `engine_error` like every backend.
    pub fn load(model_path: &Path) -> Result<Self> {
        let fail = |e: ort::Error| Error::EngineFailure {
            engine: "rvm".into(),
            message: e.to_string(),
        };
        let env = ort_env(fail)?;
        let mut builder = ort::session::Session::builder(env).map_err(fail)?;
        let session = builder.commit_from_file(model_path).map_err(fail)?;
        Ok(RvmEngine { session, rec: None })
    }
}

impl MatteEngine for RvmEngine {
    fn name(&self) -> &'static str {
        "rvm"
    }

    fn model(&self) -> &'static str {
        "rvm_mobilenetv3_fp32"
    }

    fn infer_size(&self, src_w: u32, src_h: u32) -> (u32, u32) {
        infer_size_512(src_w, src_h)
    }

    fn infer(&mut self, rgb_chw: &[f32], w: u32, h: u32) -> Result<Vec<f32>> {
        let fail = |e: ort::Error| Error::EngineFailure {
            engine: "rvm".into(),
            message: e.to_string(),
        };
        let (w, h) = (w as usize, h as usize);
        let src =
            ort::value::Tensor::from_array(([1usize, 3, h, w], rgb_chw.to_vec())).map_err(fail)?;
        let dsr =
            ort::value::Tensor::from_array(([1usize], vec![DOWNSAMPLE_RATIO])).map_err(fail)?;
        let rec: [ort::value::Tensor<f32>; 4] = match &self.rec {
            Some([r1, r2, r3, r4]) => [
                ort::value::Tensor::from_array((r1.shape.clone(), r1.data.clone()))
                    .map_err(fail)?,
                ort::value::Tensor::from_array((r2.shape.clone(), r2.data.clone()))
                    .map_err(fail)?,
                ort::value::Tensor::from_array((r3.shape.clone(), r3.data.clone()))
                    .map_err(fail)?,
                ort::value::Tensor::from_array((r4.shape.clone(), r4.data.clone()))
                    .map_err(fail)?,
            ],
            // first frame after load/reset: the placeholder zeros the exported model
            // initializes its recurrent state from
            None => [
                ort::value::Tensor::from_array(([1usize, 1, 1, 1], vec![0.0f32])).map_err(fail)?,
                ort::value::Tensor::from_array(([1usize, 1, 1, 1], vec![0.0f32])).map_err(fail)?,
                ort::value::Tensor::from_array(([1usize, 1, 1, 1], vec![0.0f32])).map_err(fail)?,
                ort::value::Tensor::from_array(([1usize, 1, 1, 1], vec![0.0f32])).map_err(fail)?,
            ],
        };
        let [r1t, r2t, r3t, r4t] = rec;
        let outputs = self
            .session
            .run(ort::inputs![
                "src" => src,
                "r1i" => r1t, "r2i" => r2t, "r3i" => r3t, "r4i" => r4t,
                "downsample_ratio" => dsr,
            ])
            .map_err(fail)?;
        let (pha_shape, pha) = outputs["pha"].try_extract_tensor::<f32>().map_err(fail)?;
        if pha_shape.len() != 4 || pha_shape[2] != h as i64 || pha_shape[3] != w as i64 {
            return Err(Error::EngineFailure {
                engine: "rvm".into(),
                message: format!("unexpected pha shape {pha_shape:?}, expected [1, 1, {h}, {w}]"),
            });
        }
        let alpha = pha.to_vec();
        let mut new_rec: Vec<RecTensor> = Vec::with_capacity(4);
        for name in ["r1o", "r2o", "r3o", "r4o"] {
            let (shape, data) = outputs[name].try_extract_tensor::<f32>().map_err(fail)?;
            new_rec.push(RecTensor {
                shape: shape.to_vec(),
                data: data.to_vec(),
            });
        }
        drop(outputs);
        let mut rec_out = new_rec.into_iter();
        self.rec = Some([
            rec_out.next().expect("four rec tensors are extracted"),
            rec_out.next().expect("four rec tensors are extracted"),
            rec_out.next().expect("four rec tensors are extracted"),
            rec_out.next().expect("four rec tensors are extracted"),
        ]);
        Ok(alpha)
    }

    fn reset(&mut self) {
        self.rec = None;
    }
}

/// 512-wide inference height, rounded to a multiple of 16 (the model's 4 stride-2
/// stages), minimum one block. 720p → 512x288, 1080p → 512x288 (verification §3.2).
pub fn infer_size_512(src_w: u32, src_h: u32) -> (u32, u32) {
    let raw = INF_WIDTH as f64 * src_h as f64 / src_w as f64;
    let h = (((raw / 16.0).round() as u32).max(1)) * 16;
    (INF_WIDTH, h)
}

/// matte input fingerprint: 视频哈希 + 模型哈希 + downsample_ratio (spec 输入指纹模型).
pub fn fingerprint(video_sha256: &str, model_sha256: &str, downsample_ratio: f32) -> String {
    let canonical = format!(
        "cc-matte/v1\nvideo={video_sha256}\nmodel={model_sha256}\ndownsample_ratio={downsample_ratio:.3}"
    );
    stage::sha256_hex(canonical.as_bytes())
}

/// Drop the alpha/preview artifacts and the matte record. Called when the person video
/// is replaced or the user explicitly skips matting, so render's card fallback kicks in
/// and no alpha from an older clip can be composited. The project stage is left as is:
/// render is legal from tts_done/matte_done/done, and running person_matte again
/// re-records the stage.
pub fn clear_artifacts(project: &mut Project, dir: &Path) -> Result<()> {
    for rel in [
        ALPHA_MOV,
        PREVIEW_PNG,
        "work/person/alpha.mov.tmp",
        "work/person/preview.png.tmp",
    ] {
        let _ = std::fs::remove_file(dir.join(rel));
    }
    project.stages.remove(STAGE_KEY);
    save_project(project, dir)
}

/// Source video geometry parsed from `ffmpeg -i` stderr (no ffprobe in the package,
/// spec 分发). `duration_sec` feeds the preview-frame choice.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_sec: f64,
}

pub(crate) fn probe_video(path: &Path) -> Result<VideoInfo> {
    let out = Command::new(crate::ffmpeg::ffmpeg_exe())
        .args(["-hide_banner", "-i"])
        .arg(path)
        .output()
        .map_err(|e| Error::Ffmpeg {
            stderr: format!("failed to spawn ffmpeg: {e}"),
        })?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    let display = path.display().to_string();
    let video_line = stderr
        .lines()
        .find(|l| l.contains("Video:"))
        .ok_or_else(|| Error::VideoInvalid {
            path: display.clone(),
            message: "no video stream found".into(),
        })?;
    let (width, height) = parse_dims(video_line).ok_or_else(|| Error::VideoInvalid {
        path: display.clone(),
        message: "could not parse frame size".into(),
    })?;
    let fps = parse_rate(video_line, "fps")
        .or_else(|| parse_rate(video_line, "tbr"))
        .unwrap_or(30.0);
    let duration_sec = stderr.lines().find_map(parse_duration).unwrap_or(0.0);
    Ok(VideoInfo {
        width,
        height,
        fps,
        duration_sec,
    })
}

/// First `WxH` token on the line (both sides at least two digits, so hex ids like
/// `0x31637661` don't match).
fn parse_dims(line: &str) -> Option<(u32, u32)> {
    for token in line.split([',', ' ']) {
        let token = token.trim();
        if let Some((w, h)) = token.split_once('x') {
            if w.len() >= 2 && h.len() >= 2 {
                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    if w > 0 && h > 0 {
                        return Some((w, h));
                    }
                }
            }
        }
    }
    None
}

/// The number before `keyword` (e.g. `30 fps` / `15360 tbr`).
fn parse_rate(line: &str, keyword: &str) -> Option<f64> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let idx = tokens
        .iter()
        .position(|t| t.trim_end_matches(',').eq_ignore_ascii_case(keyword))?;
    if idx == 0 {
        return None;
    }
    tokens[idx - 1]
        .trim_end_matches(',')
        .parse::<f64>()
        .ok()
        .filter(|v| *v > 0.0)
}

/// `Duration: 00:00:30.00` from the container header.
fn parse_duration(line: &str) -> Option<f64> {
    let rest = line.trim().strip_prefix("Duration:")?.trim();
    let mut parts = rest.split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    let s: f64 = parts.next()?.trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Interleaved rgb24 bytes → planar CHW f32 in 0..1.
fn rgb_to_chw(bytes: &[u8], w: usize, h: usize) -> Vec<f32> {
    let n = w * h;
    let mut out = vec![0.0f32; 3 * n];
    for i in 0..n {
        out[i] = bytes[3 * i] as f32 / 255.0;
        out[n + i] = bytes[3 * i + 1] as f32 / 255.0;
        out[2 * n + i] = bytes[3 * i + 2] as f32 / 255.0;
    }
    out
}

/// Bilinear resize of planar channels (src/dst both `channels * w * h`). Works for
/// downscale (frame → inference size) and upscale (alpha → source size).
pub fn resize_bilinear(
    src: &[f32],
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    channels: usize,
) -> Vec<f32> {
    let mut out = vec![0.0f32; channels * dw * dh];
    let x_ratio = sw as f32 / dw as f32;
    let y_ratio = sh as f32 / dh as f32;
    for dy in 0..dh {
        let sy = ((dy as f32 + 0.5) * y_ratio - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f32;
        for dx in 0..dw {
            let sx = ((dx as f32 + 0.5) * x_ratio - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let fx = sx - x0 as f32;
            for c in 0..channels {
                let tl = src[c * sw * sh + y0 * sw + x0];
                let tr = src[c * sw * sh + y0 * sw + x1];
                let bl = src[c * sw * sh + y1 * sw + x0];
                let br = src[c * sw * sh + y1 * sw + x1];
                let top = tl + (tr - tl) * fx;
                let bottom = bl + (br - bl) * fx;
                out[c * dw * dh + dy * dw + dx] = top + (bottom - top) * fy;
            }
        }
    }
    out
}

/// 3x3 min filter on the alpha plane: the spec's 1px 腐蚀去白边. Border pixels
/// replicate, so a person touching the frame edge doesn't grow a transparent line.
pub fn erode_1px(alpha: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        let y0 = if y == 0 { 0 } else { y - 1 };
        let y1 = if y + 1 == h { h - 1 } else { y + 1 };
        for x in 0..w {
            let x0 = if x == 0 { 0 } else { x - 1 };
            let x1 = if x + 1 == w { w - 1 } else { x + 1 };
            let m = alpha[y0 * w + x0]
                .min(alpha[y0 * w + x])
                .min(alpha[y0 * w + x1])
                .min(alpha[y * w + x0])
                .min(alpha[y * w + x])
                .min(alpha[y * w + x1])
                .min(alpha[y1 * w + x0])
                .min(alpha[y1 * w + x])
                .min(alpha[y1 * w + x1]);
            out[y * w + x] = m;
        }
    }
    out
}

/// Despill (PRD §8.5): on semi-transparent (edge) pixels clamp green to max(r, b) so
/// background color bleeding into the fringe can't tint the composite.
pub fn despill(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        if a > 0 && a < 255 {
            let max_rb = px[0].max(px[2]);
            if px[1] > max_rb {
                px[1] = max_rb;
            }
        }
    }
}

/// Composite one frame over a 16px checkerboard (story 16: 棋盘格上的边缘预览).
/// `out` receives opaque RGBX pixels (X = 255, kept for the rawvideo pipe).
pub fn composite_checkerboard(rgba: &[u8], w: usize, out: &mut [u8]) {
    for (i, px) in rgba.chunks_exact(4).enumerate() {
        let (x, y) = (i % w, i / w);
        let checker = if (x / 16 + y / 16) % 2 == 0 {
            255u8
        } else {
            204u8
        };
        let a = px[3] as f32 / 255.0;
        for c in 0..3 {
            out[4 * i + c] = (px[c] as f32 * a + checker as f32 * (1.0 - a)).round() as u8;
        }
        out[4 * i + 3] = 255;
    }
}

/// Everything `process_video` produced, before records are written.
struct ProcessOutput {
    width: u32,
    height: u32,
    fps: f64,
    frames: u32,
    elapsed_sec: f64,
}

/// Decode → infer → postprocess → encode the whole clip (streaming: one frame in
/// memory at a time besides the rec state and the preview capture).
fn process_video(dir: &Path, video: &Path, engine: &mut dyn MatteEngine) -> Result<ProcessOutput> {
    let info = probe_video(video)?;
    let (iw, ih) = engine.infer_size(info.width, info.height);
    let (w, h) = (info.width as usize, info.height as usize);
    engine.reset();

    let alpha_tmp = dir.join(PERSON_DIR).join("alpha.mov.tmp");
    let fps_arg = format!("{}", info.fps);
    let size_arg = format!("{}x{}", info.width, info.height);
    let encoder = Command::new(crate::ffmpeg::ffmpeg_exe())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &size_arg,
            "-r",
            &fps_arg,
            "-i",
            "-",
            "-c:v",
            "prores_ks",
            "-profile:v",
            "4444",
            "-pix_fmt",
            "yuva444p10le",
            "-f",
            "mov",
        ])
        .arg(&alpha_tmp)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Ffmpeg {
            stderr: format!("failed to spawn ffmpeg: {e}"),
        })?;
    let decoder = Command::new(crate::ffmpeg::ffmpeg_exe())
        .args(["-v", "error", "-i"])
        .arg(video)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Ffmpeg {
            stderr: format!("failed to spawn ffmpeg: {e}"),
        })?;

    // both ffmpegs write progress/errors to stderr; drain it so a chatty failure
    // can't deadlock on a full pipe, and keep the tail for error reporting
    let dec_err = Arc::new(Mutex::new(String::new()));
    let enc_err = Arc::new(Mutex::new(String::new()));
    std::thread::scope(|s| {
        let mut dec = decoder;
        let mut enc = encoder;
        if let Some(stderr) = dec.stderr.take() {
            let sink = Arc::clone(&dec_err);
            s.spawn(move || {
                let mut stderr = stderr;
                let mut buf = String::new();
                let _ = stderr.read_to_string(&mut buf);
                *sink.lock().unwrap() = buf;
            });
        }
        if let Some(stderr) = enc.stderr.take() {
            let sink = Arc::clone(&enc_err);
            s.spawn(move || {
                let mut stderr = stderr;
                let mut buf = String::new();
                let _ = stderr.read_to_string(&mut buf);
                *sink.lock().unwrap() = buf;
            });
        }

        let started = Instant::now();
        let mut dec_out = dec.stdout.take().expect("decoder stdout was piped");
        let mut enc_in = enc.stdin.take().expect("encoder stdin was piped");
        let rgb_bytes = w * h * 3;
        let mut rgb = vec![0u8; rgb_bytes];
        let mut rgba = vec![0u8; w * h * 4];
        let mut preview = vec![0u8; w * h * 4];
        let mut latest = vec![0u8; w * h * 4];
        // middle frame for the preview; probed duration gives the index, the last
        // frame is the fallback for videos whose real frame count differs
        let preview_index = (info.duration_sec * info.fps / 2.0).round() as u64;
        let mut captured = false;
        let mut frames = 0u32;
        let mut io_error: Option<String> = None;

        loop {
            match read_exact_or_eof(&mut dec_out, &mut rgb) {
                Ok(false) => break,
                Ok(true) => {}
                Err(e) => {
                    io_error = Some(format!("decoder stream ended early: {e}"));
                    break;
                }
            }
            let chw = rgb_to_chw(&rgb, w, h);
            let small = resize_bilinear(&chw, w, h, iw as usize, ih as usize, 3);
            let small_alpha = engine.infer(&small, iw, ih)?;
            let big_alpha = resize_bilinear(&small_alpha, iw as usize, ih as usize, w, h, 1);
            let eroded = erode_1px(&big_alpha, w, h);
            for i in 0..w * h {
                rgba[4 * i] = rgb[3 * i];
                rgba[4 * i + 1] = rgb[3 * i + 1];
                rgba[4 * i + 2] = rgb[3 * i + 2];
                rgba[4 * i + 3] = (eroded[i].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            despill(&mut rgba);
            latest.copy_from_slice(&rgba);
            if frames as u64 == preview_index {
                preview.copy_from_slice(&rgba);
                captured = true;
            }
            if let Err(e) = enc_in.write_all(&rgba).and_then(|_| enc_in.flush()) {
                io_error = Some(format!("encoder rejected frame {frames}: {e}"));
                break;
            }
            frames += 1;
        }
        drop(enc_in);
        let dec_status = dec.wait()?;
        let enc_status = enc.wait()?;
        if let Some(message) = io_error {
            let tail = stderr_tail(&dec_err);
            return Err(Error::Ffmpeg {
                stderr: format!("{message}; ffmpeg: {tail}"),
            });
        }
        if !dec_status.success() {
            return Err(Error::Ffmpeg {
                stderr: format!("ffmpeg decode: {}", stderr_tail(&dec_err)),
            });
        }
        if !enc_status.success() {
            return Err(Error::Ffmpeg {
                stderr: format!("ffmpeg prores encode: {}", stderr_tail(&enc_err)),
            });
        }
        if frames == 0 {
            return Err(Error::VideoInvalid {
                path: video.display().to_string(),
                message: "no frames decoded".into(),
            });
        }
        if !captured {
            preview.copy_from_slice(&latest);
        }

        // checkerboard preview → png, atomic via the tmp name (image2 accepts literal
        // filenames, verified against ffmpeg 8)
        composite_checkerboard(&preview, w, &mut rgba);
        let preview_tmp = dir.join(PERSON_DIR).join("preview.png.tmp");
        write_preview_png(&preview_tmp, &size_arg, &rgba)?;

        let elapsed_sec = started.elapsed().as_secs_f64();
        Ok(ProcessOutput {
            width: info.width,
            height: info.height,
            fps: info.fps,
            frames,
            elapsed_sec,
        })
    })
}

/// Pipe one composited frame through ffmpeg into the preview png. Codec forced: the
/// muxer would otherwise pick jpeg for the .tmp name.
fn write_preview_png(path: &Path, size_arg: &str, rgba: &[u8]) -> Result<()> {
    let mut child = Command::new(crate::ffmpeg::ffmpeg_exe())
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            size_arg,
            "-i",
            "-",
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2",
        ])
        .arg(path)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::Ffmpeg {
            stderr: format!("failed to spawn ffmpeg: {e}"),
        })?;
    child
        .stdin
        .take()
        .expect("preview stdin was piped")
        .write_all(rgba)
        .map_err(|e| Error::Ffmpeg {
            stderr: format!("preview pipe failed: {e}"),
        })?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Ffmpeg {
            stderr: format!("preview encode: {stderr}"),
        });
    }
    Ok(())
}

/// Read exactly `buf.len()` bytes; `Ok(false)` = clean EOF at a frame boundary.
fn read_exact_or_eof(stream: &mut impl Read, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = stream.read(&mut buf[filled..])?;
        if n == 0 {
            if filled == 0 {
                return Ok(false);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("truncated frame: got {filled} of {} bytes", buf.len()),
            ));
        }
        filled += n;
    }
    Ok(true)
}

fn stderr_tail(sink: &Arc<Mutex<String>>) -> String {
    let text = sink.lock().unwrap().clone();
    let lines: Vec<&str> = text.lines().rev().take(6).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

#[derive(Debug, Clone, Serialize)]
pub struct MatteReport {
    /// True when the fingerprint matched and all artifacts were intact: nothing ran.
    pub skipped: bool,
    pub frames: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub elapsed_sec: f64,
    pub fingerprint: String,
    /// alpha.mov + preview.png, paths relative to the project directory.
    pub artifacts: Vec<OutputArtifact>,
    pub preview: String,
    pub stage: Stage,
    pub next: String,
}

/// Inputs shared by the full matte and the early preview.
struct PreparedMatte {
    video: PathBuf,
    video_hash: String,
    fingerprint: String,
}

fn prepare_matte(dir: &Path, model_path: &Path) -> Result<PreparedMatte> {
    let video = dir.join(PERSON_VIDEO);
    if !video.is_file() {
        return Err(Error::PersonVideoMissing {
            path: video.display().to_string(),
        });
    }
    if !model_path.is_file() {
        return Err(Error::MatteModelMissing {
            path: model_path.display().to_string(),
        });
    }
    let video_hash = stage::sha256_file(&video)?;
    let model_hash = stage::sha256_file(model_path)?;
    let fingerprint = fingerprint(&video_hash, &model_hash, DOWNSAMPLE_RATIO);
    Ok(PreparedMatte {
        video,
        video_hash,
        fingerprint,
    })
}

fn skip_if_recorded(
    project: &Project,
    dir: &Path,
    prepared: &PreparedMatte,
) -> Result<Option<MatteReport>> {
    if !stage::verify(
        project,
        dir,
        &stage::VerifySpec {
            key: STAGE_KEY,
            input_fingerprint: &prepared.fingerprint,
            stage_deps: &[],
            content_deps: &[("video", prepared.video_hash.as_str())],
        },
    ) {
        return Ok(None);
    }
    let record = project
        .stages
        .get(STAGE_KEY)
        .expect("verified record exists");
    Ok(Some(MatteReport {
        skipped: true,
        frames: 0,
        width: 0,
        height: 0,
        fps: 0.0,
        elapsed_sec: 0.0,
        fingerprint: prepared.fingerprint.clone(),
        artifacts: record.outputs.clone(),
        preview: PREVIEW_PNG.to_string(),
        stage: project.stage,
        next: NEXT_MATTE_DONE.to_string(),
    }))
}

struct ProcessedMatte {
    outputs: Vec<OutputArtifact>,
    frames: u32,
    width: u32,
    height: u32,
    fps: f64,
    elapsed_sec: f64,
}

fn process_artifacts(
    dir: &Path,
    engine: &mut dyn MatteEngine,
    prepared: &PreparedMatte,
) -> Result<ProcessedMatte> {
    let out = process_video(dir, &prepared.video, engine)?;
    std::fs::rename(
        dir.join(PERSON_DIR).join("alpha.mov.tmp"),
        dir.join(ALPHA_MOV),
    )?;
    std::fs::rename(
        dir.join(PERSON_DIR).join("preview.png.tmp"),
        dir.join(PREVIEW_PNG),
    )?;
    let outputs = vec![
        stage::hash_artifact(dir, ALPHA_MOV)?,
        stage::hash_artifact(dir, PREVIEW_PNG)?,
    ];
    Ok(ProcessedMatte {
        outputs,
        frames: out.frames,
        width: out.width,
        height: out.height,
        fps: out.fps,
        elapsed_sec: out.elapsed_sec,
    })
}

fn success_record(
    engine: &dyn MatteEngine,
    prepared: &PreparedMatte,
    processed: &ProcessedMatte,
) -> StageRecord {
    stage::success(
        STAGE_KEY,
        prepared.fingerprint.clone(),
        format!(
            "matte-{}/{}+model={}",
            engine.name(),
            env!("CARGO_PKG_VERSION"),
            engine.model()
        ),
        processed.outputs.clone(),
        // lets render detect an alpha produced for a different clip
        BTreeMap::from([("video".to_string(), prepared.video_hash.clone())]),
    )
}

fn report(project: &Project, prepared: &PreparedMatte, processed: ProcessedMatte) -> MatteReport {
    MatteReport {
        skipped: false,
        frames: processed.frames,
        width: processed.width,
        height: processed.height,
        fps: processed.fps,
        elapsed_sec: processed.elapsed_sec,
        fingerprint: prepared.fingerprint.clone(),
        artifacts: processed.outputs,
        preview: PREVIEW_PNG.to_string(),
        stage: project.stage,
        next: NEXT_MATTE_DONE.to_string(),
    }
}

/// `person_matte` stage runner: tts_done → matte_running → matte_done (also entered
/// from failed). Caller must hold the project lock. `model_path` points at the RVM
/// onnx; the CLI resolves it via CC_RVM_MODEL or `DEFAULT_MODEL_REL`.
pub fn run_matte_locked(
    project: &mut Project,
    dir: &Path,
    engine: &mut dyn MatteEngine,
    model_path: &Path,
) -> Result<MatteReport> {
    let prepared = prepare_matte(dir, model_path)?;
    if let Some(report) = skip_if_recorded(project, dir, &prepared)? {
        return Ok(report);
    }
    // legal entries: tts_done (fresh), failed (retry), matte_done (invalidation: the
    // person video or model changed → re-matte, ticket 10), matte_running (kill -9
    // residue — re-entry IS the resume, no transition needed)
    if !matches!(
        project.stage,
        Stage::TtsDone | Stage::Failed | Stage::MatteDone | Stage::MatteRunning
    ) {
        return Err(Error::InvalidTransition {
            from: project.stage,
            to: Stage::MatteRunning,
        });
    }
    // enter matte_running (crash leaves a resumable failed state behind)
    if project.stage != Stage::MatteRunning {
        project.transition(Stage::MatteRunning)?;
    }
    project.failed = None;
    save_project(project, dir)?;
    std::fs::create_dir_all(dir.join(PERSON_DIR))?;

    let processed = match process_artifacts(dir, engine, &prepared) {
        Ok(processed) => processed,
        Err(err) => {
            project.fail(format!("matte: {err}"))?;
            save_project(project, dir)?;
            return Err(err);
        }
    };
    project.transition(Stage::MatteDone)?;
    stage::land(
        project,
        dir,
        STAGE_KEY,
        success_record(engine, &prepared, &processed),
    )?;
    Ok(report(project, &prepared, processed))
}

/// Wizard preview path (Step 5): run the FULL matte early — typically at `parsed`,
/// before any TTS — WITHOUT moving the state machine. The artifacts and the
/// fingerprint record land now, so the generate pipeline's matte segment later
/// skips through them instead of re-running inference (幂等复用). Unlike
/// `run_matte_locked` there are no legal-entry checks and no transitions: a failure
/// leaves the project state exactly as it was. Caller must hold the project lock.
pub fn run_matte_preview_locked(
    project: &mut Project,
    dir: &Path,
    engine: &mut dyn MatteEngine,
    model_path: &Path,
) -> Result<MatteReport> {
    let prepared = prepare_matte(dir, model_path)?;
    if let Some(report) = skip_if_recorded(project, dir, &prepared)? {
        return Ok(report);
    }
    std::fs::create_dir_all(dir.join(PERSON_DIR))?;
    let processed = process_artifacts(dir, engine, &prepared)?;
    // the ONLY persistence: the stage record. project.stage / failed untouched.
    stage::land(
        project,
        dir,
        STAGE_KEY,
        success_record(engine, &prepared, &processed),
    )?;
    Ok(report(project, &prepared, processed))
}
