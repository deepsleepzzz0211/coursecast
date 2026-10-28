//! person_matte stage behavior with the offline engine double (spec Testing
//! Decisions, 缝 2): state transitions, fingerprint, idempotent rerun, artifact
//! landing, alpha channel surviving ProRes. The real RVM model is exercised in
//! matte_inference.rs.

use cc_core::matte::{
    composite_checkerboard, despill, erode_1px, fingerprint, infer_size_512, resize_bilinear,
    run_matte_locked, ALPHA_MOV, PERSON_VIDEO, PREVIEW_PNG, STAGE_KEY,
};
use cc_core::{
    create_project, load_project, save_project, FakeMatteEngine, FakeTtsEngine, MatteEngine, Stage,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Project fixture driven to tts_done through the real stage runner (stage forced to
/// Parsed the same way the tts tests do — slide_render, ticket 03, is not wired yet).
fn fixture_tts_done(data_dir: &Path) -> (String, PathBuf) {
    let created = create_project(data_dir, "抠像测试", None).unwrap();
    let id = created.project.project_id.clone();
    let dir = created.dir;
    std::fs::write(dir.join("script.md"), "第一页讲稿\n").unwrap();
    let (mut project, dir) = load_project(data_dir, &id).unwrap();
    project.stage = Stage::Parsed;
    save_project(&project, &dir).unwrap();
    cc_core::synthesize_pages(
        data_dir,
        &id,
        &cc_core::FallbackChain::single(std::sync::Arc::new(FakeTtsEngine::new())),
        None,
        &cc_core::SynthesisOptions::default(),
    )
    .unwrap();
    (id, dir)
}

/// A placeholder model file: the stage hashes whatever sits at the model path, the
/// fake engine never loads it. Real-model tests live in matte_inference.rs.
fn fake_model(dir: &Path) -> PathBuf {
    let path = dir.join("fake-rvm.onnx");
    std::fs::write(&path, b"not-a-real-model-but-hashable").unwrap();
    path
}

/// Tiny synthetic clip via ffmpeg (ffmpeg is a hard dependency of the pipeline).
fn make_video(path: &Path, w: u32, h: u32, seconds: f64) {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={w}x{h}:rate=10:duration={seconds}"))
        .args(["-pix_fmt", "yuv420p"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn matte_runs_end_to_end_and_lands_alpha_with_transparency() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = fixture_tts_done(tmp.path());
    make_video(&dir.join(PERSON_VIDEO), 320, 240, 1.0);
    let model = fake_model(tmp.path());

    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    let report = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();

    assert_eq!(report.stage, Stage::MatteDone);
    assert!(!report.skipped);
    assert_eq!((report.width, report.height), (320, 240));
    assert_eq!(report.fps, 10.0);
    assert!(
        (8..=12).contains(&report.frames),
        "1s@10fps: {}",
        report.frames
    );
    assert!(report.fingerprint.starts_with("sha256:"));
    assert!(report.next.contains("render"));

    // artifacts on disk, paths relative to the project, preview is a png
    let paths: Vec<&str> = report.artifacts.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(paths, vec![ALPHA_MOV, PREVIEW_PNG]);
    for a in &report.artifacts {
        let abs = dir.join(&a.path);
        assert!(abs.is_file(), "{} missing", a.path);
        let bytes = std::fs::read(&abs).unwrap();
        assert_eq!(bytes.len() as u64, a.bytes);
    }
    let preview = std::fs::read(dir.join(PREVIEW_PNG)).unwrap();
    assert_eq!(&preview[..4], b"\x89PNG", "preview must be a png");

    // alpha survived ProRes 4444: the fake engine's circle mask leaves corners
    // transparent and the center opaque in frame 0
    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(dir.join(ALPHA_MOV))
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-frames:v", "1", "-"])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let frame = decoded.stdout;
    assert_eq!(
        frame.len(),
        320 * 240 * 4,
        "decoded size must match the source"
    );
    let alphas: Vec<u8> = frame.iter().skip(3).step_by(4).copied().collect();
    assert_eq!(*alphas.iter().max().unwrap(), 255, "center must be opaque");
    assert!(
        *alphas.iter().min().unwrap() <= 64,
        "corners must be transparent"
    );

    // stage record: fingerprint + tool version recorded for the invalidation model
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    let record = project.stages.get(STAGE_KEY).unwrap();
    assert_eq!(record.state, "success");
    assert_eq!(record.input_fingerprint, report.fingerprint);
    assert!(
        record.tool_version.contains("matte-fake"),
        "{}",
        record.tool_version
    );
    assert_eq!(project.stage, Stage::MatteDone);
}

#[test]
fn matte_rerun_with_same_inputs_skips_without_engine_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = fixture_tts_done(tmp.path());
    make_video(&dir.join(PERSON_VIDEO), 320, 240, 1.0);
    let model = fake_model(tmp.path());

    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    let first = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();
    let calls_after_first = engine.call_count();
    assert!(calls_after_first > 0);

    let second = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();
    assert!(second.skipped);
    assert_eq!(second.frames, 0, "skipped run processes nothing");
    assert_eq!(second.fingerprint, first.fingerprint);
    assert_eq!(
        second.artifacts, first.artifacts,
        "skipped run reports the landed artifacts"
    );
    assert_eq!(second.stage, Stage::MatteDone);
    assert_eq!(
        engine.call_count(),
        calls_after_first,
        "idempotent rerun must not call the engine (spec story 27)"
    );
}

#[test]
fn matte_missing_video_errors_before_any_state_change() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, _) = fixture_tts_done(tmp.path());
    let model = fake_model(tmp.path());

    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    let err = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap_err();
    assert_eq!(err.code(), "person_video_missing");
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(
        project.stage,
        Stage::TtsDone,
        "failed input check must not move the state"
    );
    assert!(!project.stages.contains_key(STAGE_KEY));
    assert!(project.failed.is_none());
}

#[test]
fn matte_rejects_projects_that_cannot_legally_enter_matte() {
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "刚创建", None).unwrap();
    let id = created.project.project_id.clone();
    let dir = created.dir;
    std::fs::write(dir.join(PERSON_VIDEO), b"fake").unwrap();
    let model = fake_model(tmp.path());

    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    let err = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap_err();
    assert!(
        matches!(err, cc_core::Error::InvalidTransition { .. }),
        "created → matte_running is not an edge: {err}"
    );
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::Created);
}

#[test]
fn matte_broken_video_records_failure_at_matte_running() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = fixture_tts_done(tmp.path());
    std::fs::write(dir.join(PERSON_VIDEO), b"this is not a video").unwrap();
    let model = fake_model(tmp.path());

    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    let err = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap_err();
    assert!(
        matches!(
            err,
            cc_core::Error::Ffmpeg { .. } | cc_core::Error::VideoInvalid { .. }
        ),
        "{err}"
    );
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(project.stage, Stage::Failed);
    let failed = project.failed.expect("failed must record stage + error");
    assert_eq!(failed.stage, Stage::MatteRunning);
    // retry from failed re-enters matte_running (the state machine allows it)
    make_video(&dir.join(PERSON_VIDEO), 320, 240, 1.0);
    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    let report = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();
    assert_eq!(report.stage, Stage::MatteDone);
}

#[test]
fn matte_done_with_changed_video_re_mattes_and_records_a_new_fingerprint() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = fixture_tts_done(tmp.path());
    make_video(&dir.join(PERSON_VIDEO), 320, 240, 1.0);
    let model = fake_model(tmp.path());

    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    let first = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();
    let calls_after_first = engine.call_count();

    // swap the video: the fingerprint no longer matches, and the ticket-10 invalidation
    // edge lets matte_done re-enter matte_running so the stale alpha is replaced
    make_video(&dir.join(PERSON_VIDEO), 320, 240, 2.0);
    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let second = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();
    assert!(!second.skipped, "changed video must re-matte");
    assert_ne!(second.fingerprint, first.fingerprint);
    assert!(
        engine.call_count() > calls_after_first,
        "the engine must re-process the new clip"
    );
    assert_eq!(second.stage, Stage::MatteDone);
    let (project, _) = load_project(tmp.path(), &id).unwrap();
    assert_eq!(
        project.stages.get(STAGE_KEY).unwrap().input_fingerprint,
        second.fingerprint
    );
}

#[test]
fn fingerprint_tracks_video_model_and_ratio() {
    let v1 = "sha256:video-one";
    let m1 = "sha256:model-one";
    let base = fingerprint(v1, m1, 0.375);
    assert_eq!(
        base,
        fingerprint(v1, m1, 0.375),
        "same inputs → same fingerprint"
    );
    assert_ne!(
        base,
        fingerprint("sha256:video-two", m1, 0.375),
        "video hash feeds in"
    );
    assert_ne!(
        base,
        fingerprint(v1, "sha256:model-two", 0.375),
        "model hash feeds in"
    );
    assert_ne!(base, fingerprint(v1, m1, 0.25), "downsample_ratio feeds in");
}

#[test]
fn infer_size_is_512_wide_and_multiple_of_16() {
    assert_eq!(infer_size_512(1280, 720), (512, 288));
    assert_eq!(infer_size_512(1920, 1080), (512, 288));
    let (w, h) = infer_size_512(640, 481);
    assert_eq!(w, 512);
    assert_eq!(h % 16, 0, "height must stay on the model's stride grid");
    let (w2, h2) = infer_size_512(720, 1280);
    assert_eq!(w2, 512);
    assert!(h2 > 512, "portrait sources keep their aspect");
    // engines agree with the shared helper
    assert_eq!(FakeMatteEngine::new().infer_size(1280, 720), (512, 288));
}

#[test]
fn resize_bilinear_is_identity_at_same_size() {
    let src: Vec<f32> = (0..3 * 4 * 4).map(|i| (i % 7) as f32 / 7.0).collect();
    let out = resize_bilinear(&src, 4, 4, 4, 4, 3);
    assert_eq!(out, src);
}

#[test]
fn erode_1px_shrinks_solid_regions_by_one_pixel() {
    // 6x6 solid block centered in an 8x8 transparent frame → the boundary ring
    // erodes to 0, the 4x4 core stays opaque
    let mut alpha = vec![0.0f32; 64];
    for y in 1..7 {
        for x in 1..7 {
            alpha[y * 8 + x] = 1.0;
        }
    }
    let out = erode_1px(&alpha, 8, 8);
    assert_eq!(out[8 + 1], 0.0, "solid corner erodes");
    assert_eq!(out[8 + 4], 0.0, "top edge erodes");
    assert_eq!(out[6 * 8 + 6], 0.0, "far corner erodes");
    assert_eq!(out[3 * 8 + 3], 1.0, "core stays opaque");
    assert_eq!(out[2 * 8 + 4], 1.0, "inner ring stays opaque");
    // replicate border: a solid block touching the frame edge does not erode there
    let full = vec![1.0f32; 16];
    assert!(erode_1px(&full, 4, 4).iter().all(|v| *v == 1.0));
}

#[test]
fn despill_clamps_green_only_on_semi_transparent_pixels() {
    let mut rgba = vec![
        // opaque pixel keeps its green
        10, 200, 10, 255, // semi pixel: green clamped to max(r, b)
        10, 200, 10, 128, // transparent pixel untouched
        0, 255, 0, 0,
    ];
    despill(&mut rgba);
    assert_eq!(rgba[1], 200, "opaque pixels keep their color");
    assert_eq!(rgba[5], 10, "semi pixel green clamped to max(r,b)");
    assert_eq!(rgba[9], 255, "transparent pixels untouched");
}

#[test]
fn checkerboard_composite_blends_by_alpha() {
    // one fully transparent pixel (pure checker), one fully opaque (pure color)
    let mut rgba = vec![255u8; 4 * 4 * 4];
    for px in rgba.chunks_exact_mut(4) {
        px[3] = 0;
    }
    rgba[11] = 255; // pixel (2,0): opaque white
    let mut out = vec![0u8; 4 * 4 * 4];
    composite_checkerboard(&rgba, 4, &mut out);
    let transparent = &out[0..4];
    // 4x4 image: pixel (0,0) is checker cell 0, so it must be the light square.
    let expected = 255u8;
    assert_eq!(
        transparent[0], expected,
        "transparent pixel shows the checker"
    );
    assert_eq!(
        &out[8..11],
        &[255, 255, 255],
        "opaque pixel keeps its color"
    );
}

#[test]
fn tampered_matte_artifact_reruns_instead_of_skipping() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = fixture_tts_done(tmp.path());
    make_video(&dir.join(PERSON_VIDEO), 320, 240, 1.0);
    let model = fake_model(tmp.path());

    let (mut project, dir) = load_project(tmp.path(), &id).unwrap();
    let mut engine = FakeMatteEngine::new();
    run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();
    let calls = engine.call_count();

    std::fs::write(dir.join(ALPHA_MOV), b"tampered").unwrap();
    let again = run_matte_locked(&mut project, &dir, &mut engine, &model).unwrap();
    assert!(!again.skipped, "tampered alpha must not certify a skip");
    assert!(engine.call_count() > calls, "the engine must run again");
    assert_ne!(std::fs::read(dir.join(ALPHA_MOV)).unwrap(), b"tampered");
    assert_eq!(again.stage, Stage::MatteDone);
}
