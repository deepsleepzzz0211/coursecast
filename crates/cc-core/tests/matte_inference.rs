//! Real RVM inference (ticket 09 acceptance): alpha is not constant, dimensions match
//! the inference plan, temporal state advances, and a 30 s @ 720p clip stays inside the
//! 2-minute budget (verification report §3.2: 8.7 ms/frame at 512x288 on CPU). Skipped
//! when the vendored model is absent so the suite stays green without it.

use cc_core::matte::{run_matte_locked, ALPHA_MOV, PERSON_VIDEO};
use cc_core::{create_project, FakeTtsEngine, MatteEngine, RvmEngine};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

/// ort is built with load-dynamic: the runtime dll comes from ORT_DYLIB_PATH (or the
/// default search path, which packaged builds satisfy). Dev checkouts find it in the
/// main repo's python venv.
fn ensure_ort_dylib() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var_os("ORT_DYLIB_PATH").is_some_and(|p| !p.is_empty()) {
            return;
        }
        for anc in PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors() {
            let cand = anc.join(".venv/Lib/site-packages/onnxruntime/capi/onnxruntime.dll");
            if cand.is_file() {
                // integration tests share one process; same value every caller would set
                std::env::set_var("ORT_DYLIB_PATH", &cand);
                return;
            }
        }
    });
}

fn model_path() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/models/rvm_mobilenetv3_fp32.onnx");
    path.is_file().then_some(path)
}

fn skip_without_model() -> bool {
    if model_path().is_none() {
        eprintln!("skipping: vendor/models/rvm_mobilenetv3_fp32.onnx not present");
        return true;
    }
    false
}

/// Crude human silhouette as a P6 ppm (RVM reacts to head+torso geometry; calibrated
/// against the model: frame-0 alpha std ≈ 0.15, max ≈ 0.98 at 512x288). `x_shift`
/// moves the figure horizontally so two frames can differ.
fn write_silhouette_ppm(path: &Path, w: u32, h: u32, x_shift: f32) {
    use std::io::Write;
    let (w, h) = (w as f32, h as f32);
    let cx = w * 0.5 + x_shift;
    let mut px = Vec::with_capacity((w * h * 3.0) as usize);
    for y in 0..h as u32 {
        for x in 0..w as u32 {
            let (fx, fy) = (x as f32, y as f32);
            let head = ((fx - cx).powi(2) + (fy - h * 0.25).powi(2)) < (h * 0.09).powi(2);
            let torso = (fx - cx).abs() < w * 0.13 && fy > h * 0.35 && fy < h * 0.75;
            let (r, g, b) = if head || torso {
                (217, 166, 140)
            } else {
                (89, 89, 89)
            };
            px.extend_from_slice(&[r, g, b]);
        }
    }
    let mut f = std::fs::File::create(path).unwrap();
    write!(f, "P6\n{} {}\n255\n", w as u32, h as u32).unwrap();
    f.write_all(&px).unwrap();
}

/// Decode a ppm into CHW f32 0..1 (the engine's input contract).
fn ppm_to_chw(path: &Path, w: u32, h: u32) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    let header_end = bytes.windows(4).position(|w| w == b"255\n").unwrap() + 4;
    let px = &bytes[header_end..];
    let n = (w * h) as usize;
    let mut out = vec![0.0f32; 3 * n];
    for i in 0..n {
        out[i] = px[3 * i] as f32 / 255.0;
        out[n + i] = px[3 * i + 1] as f32 / 255.0;
        out[2 * n + i] = px[3 * i + 2] as f32 / 255.0;
    }
    out
}

fn silhouette_frame(path: &Path, w: u32, h: u32) -> (Vec<f32>, u32, u32) {
    write_silhouette_ppm(path, w, h, 0.0);
    (ppm_to_chw(path, w, h), w, h)
}

fn mean_std(v: &[f32]) -> (f32, f32) {
    let mean = v.iter().sum::<f32>() / v.len() as f32;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32;
    (mean, var.sqrt())
}

#[test]
fn rvm_alpha_matches_infer_size_is_bounded_and_not_constant() {
    ensure_ort_dylib();
    if skip_without_model() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let frame_path = tmp.path().join("frame.ppm");
    let (chw, w, h) = silhouette_frame(&frame_path, 1280, 720);
    let mut engine = RvmEngine::load(&model_path().unwrap()).unwrap();
    let (iw, ih) = engine.infer_size(w, h);
    assert_eq!(
        (iw, ih),
        (512, 288),
        "720p infers at 512x288 (verification §3.2)"
    );

    // caller contract: downscale to infer_size before infer (never feed 1080p)
    let small =
        cc_core::matte::resize_bilinear(&chw, w as usize, h as usize, iw as usize, ih as usize, 3);
    let alpha = engine.infer(&small, iw, ih).unwrap();
    assert_eq!(
        alpha.len(),
        (iw * ih) as usize,
        "alpha is returned at inference size"
    );
    assert!(
        alpha.iter().all(|v| (-0.001..=1.001).contains(v)),
        "alpha stays in 0..1"
    );
    let (_, std) = mean_std(&alpha);
    let max = alpha.iter().cloned().fold(0.0f32, f32::max);
    assert!(std > 0.05, "frame-0 alpha must not be constant (std={std})");
    assert!(
        max > 0.5,
        "silhouette must produce confident alpha somewhere (max={max})"
    );
}

#[test]
fn rvm_is_deterministic_after_reset_and_state_advances() {
    ensure_ort_dylib();
    if skip_without_model() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let a_path = tmp.path().join("a.ppm");
    let b_path = tmp.path().join("b.ppm");
    let (frame_a, w, h) = silhouette_frame(&a_path, 640, 360);
    write_silhouette_ppm(&b_path, w, h, 120.0); // same figure, shifted: a real scene cut
    let frame_b = ppm_to_chw(&b_path, w, h);
    let mut engine = RvmEngine::load(&model_path().unwrap()).unwrap();

    let a1 = engine.infer(&frame_a, w, h).unwrap();
    engine.reset();
    let a1b = engine.infer(&frame_a, w, h).unwrap();
    assert_eq!(a1, a1b, "same frame after reset → identical alpha");

    // frame B after frame A differs from frame B alone: the rec state really advances
    engine.reset();
    let _ = engine.infer(&frame_a, w, h).unwrap();
    let b_after_a = engine.infer(&frame_b, w, h).unwrap();
    engine.reset();
    let b_alone = engine.infer(&frame_b, w, h).unwrap();
    let diff = b_after_a
        .iter()
        .zip(&b_alone)
        .map(|(x, y)| (*x - *y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        diff > 0.01,
        "temporal state must influence the result (max diff={diff})"
    );
}

#[test]
fn matte_30s_720p_stays_inside_two_minute_budget() {
    ensure_ort_dylib();
    if skip_without_model() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(data_dir.join("projects")).unwrap();

    // project at tts_done (stage forced to Parsed like the other fixtures —
    // slide_render is ticket 03 and not wired yet)
    let created = create_project(&data_dir, "耗时测试", None).unwrap();
    let id = created.project.project_id.clone();
    std::fs::write(created.dir.join("script.md"), "一页讲稿\n").unwrap();
    {
        let (mut p, d) = cc_core::load_project(&data_dir, &id).unwrap();
        p.stage = cc_core::Stage::Parsed;
        cc_core::save_project(&p, &d).unwrap();
    }
    cc_core::synthesize_pages(
        &data_dir,
        &id,
        &cc_core::FallbackChain::single(std::sync::Arc::new(FakeTtsEngine::new())),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();

    // 30 s @ 720p / 30 fps person video from the silhouette still
    let still = tmp.path().join("sil.ppm");
    write_silhouette_ppm(&still, 1280, 720, 0.0);
    let person = created.dir.join(PERSON_VIDEO);
    let enc = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-loop", "1", "-framerate", "30", "-i"])
        .arg(&still)
        .args(["-t", "30", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&person)
        .output()
        .unwrap();
    assert!(
        enc.status.success(),
        "{}",
        String::from_utf8_lossy(&enc.stderr)
    );

    let (mut project, dir) = cc_core::load_project(&data_dir, &id).unwrap();
    let mut engine = RvmEngine::load(&model_path().unwrap()).unwrap();
    let report = run_matte_locked(&mut project, &dir, &mut engine, &model_path().unwrap()).unwrap();

    assert_eq!(report.stage, cc_core::Stage::MatteDone);
    assert_eq!((report.width, report.height, report.fps), (1280, 720, 30.0));
    assert!(
        (880..=920).contains(&report.frames),
        "30s@30fps: {}",
        report.frames
    );
    assert!(
        report.elapsed_sec < 120.0,
        "matte of 30s@720p must stay under 2 minutes, took {:.1}s (baseline ≈ 8 s)",
        report.elapsed_sec
    );
    let alpha = std::fs::metadata(dir.join(ALPHA_MOV)).unwrap();
    assert!(alpha.len() > 0, "alpha.mov must not be empty");
    eprintln!(
        "matte 30s@720p: {} frames in {:.1}s ({:.1} ms/frame incl. decode/encode)",
        report.frames,
        report.elapsed_sec,
        report.elapsed_sec * 1000.0 / report.frames as f64
    );
}
