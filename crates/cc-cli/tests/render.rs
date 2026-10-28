//! Seam: `cc-cli render --json` stdout, the produced `output/final.mp4` and the
//! `stages.render` record in `project.json` (spec "Testing Decisions": 好测试只测外部
//! 行为). Expected values are the timeline spike's worked example: 5 pages
//! 3.0/2.0/4.0/2.5/3.5 s + g=0.6 → T=17.400 s, turn points 3.6/6.2/10.8/13.9.
//!
//! All media is synthetic (color-block PNGs, sine wavs, an ffmpeg-generated alpha
//! mov / opaque mp4) — no real deck, TTS or matte output is needed.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;

const RATE: f64 = 24000.0;
const DURATIONS: [f64; 5] = [3.0, 2.0, 4.0, 2.5, 3.5];
const TOTAL: f64 = 17.4;
const TURN_POINTS: [f64; 4] = [3.6, 6.2, 10.8, 13.9];
/// Test canvas: small so libx264 preset medium stays fast, but large enough that the
/// 20%-wide PiP (128 px) has measurable interior.
const W: usize = 640;
const H: usize = 360;
/// bottom_right PiP geometry at 20% width: pip_w = 128, x = W-128-80, y = H-128-60.
const PIP_W: i64 = 128;
const PIP_X: i64 = (W as i64) - PIP_W - 80;
const PIP_Y: i64 = (H as i64) - PIP_W - 60;
/// gray ladder (timeline spike's page-turn detection trick)
const SLIDE_COLORS: [&str; 5] = ["0x282828", "0x505050", "0x787878", "0xA0A0A0", "0xC8C8C8"];

fn cli() -> Command {
    Command::cargo_bin("cc-cli").unwrap()
}

fn sys(bin: &str) -> SysCommand {
    SysCommand::new(bin)
}

fn run_ok(command: &mut SysCommand) -> Vec<u8> {
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "command failed: {:?}\n{}",
        command,
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn create_project(data_dir: &Path) -> (String, PathBuf) {
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "project",
            "create",
            "--json",
            "--name",
            "渲染测试",
            "--resolution",
            &format!("{W}x{H}"),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    (
        v["project_id"].as_str().unwrap().to_string(),
        PathBuf::from(v["dir"].as_str().unwrap()),
    )
}

fn write_sine_wav(path: &Path, samples: usize, freq: f64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut pcm = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        let v = (2.0 * std::f64::consts::PI * freq * i as f64 / RATE).sin() * 12000.0;
        pcm.extend_from_slice(&(v as i16).to_le_bytes());
    }
    let mut b = Vec::with_capacity(44 + pcm.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&(RATE as u32).to_le_bytes());
    b.extend_from_slice(&((RATE as u32) * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    b.extend_from_slice(&pcm);
    fs::write(path, b).unwrap();
}

fn write_gray_png(path: &Path, color: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    run_ok(
        sys("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c={color}:s={W}x{H}"),
                "-frames:v",
                "1",
            ])
            .arg(path),
    );
}

/// 200x200 person with alpha: opaque red box (20..180) on a transparent background,
/// encoded as ProRes 4444 (the mov shape ticket 09 produces).
fn write_alpha_mov(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    run_ok(
        sys("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=200x200:d=1,format=rgba,geq=r='p(X,Y)':g='p(X,Y)':b='p(X,Y)':a='if(between(X,20,180)*between(Y,20,180),255,0)'",
                "-c:v",
                "prores_ks",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "yuva444p10le",
            ])
            .arg(path),
    );
}

/// 100x300 portrait alpha: a free 40% width would be taller than the 360 px canvas,
/// exercising the render-side height cap.
fn write_tall_alpha_mov(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    run_ok(
        sys("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=100x300:d=1,format=rgba,geq=r='p(X,Y)':g='p(X,Y)':b='p(X,Y)':a='if(between(X,10,90)*between(Y,10,290),255,0)'",
                "-c:v",
                "prores_ks",
                "-profile:v",
                "4444",
                "-pix_fmt",
                "yuva444p10le",
            ])
            .arg(path),
    );
}

/// 200x200 opaque person video for the matte-skipped card branch.
fn write_person_mp4(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    run_ok(
        sys("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0x303030:s=200x200:d=1,drawbox=x=20:y=20:w=160:h=160:color=red:t=fill",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-preset",
                "veryfast",
            ])
            .arg(path),
    );
}

/// Spike setup: 5 sine wavs + 5 gray-ladder slide PNGs.
fn setup_pages(dir: &Path) {
    for (i, d) in DURATIONS.iter().enumerate() {
        write_sine_wav(
            &dir.join("work/audio")
                .join(format!("slide_{:03}.wav", i + 1)),
            (d * RATE) as usize,
            300.0 + 80.0 * i as f64,
        );
        write_gray_png(
            &dir.join("work/slides")
                .join(format!("slide_{:03}.png", i + 1)),
            SLIDE_COLORS[i],
        );
    }
}

fn write_layout(dir: &Path, position: &str, width_pct: f64, shape: &str, transition: &str) {
    let toml = format!(
        "position = \"{position}\"\nwidth_pct = {width_pct}\nshape = \"{shape}\"\ntransition = \"{transition}\"\n"
    );
    fs::write(dir.join("layout.toml"), toml).unwrap();
}

fn write_free_layout(dir: &Path, x_pct: f64, y_pct: f64, width_pct: f64) {
    let toml = format!(
        "x_pct = {x_pct}\ny_pct = {y_pct}\nwidth_pct = {width_pct}\nshape = \"rounded_rect\"\ntransition = \"hard\"\n"
    );
    fs::write(dir.join("layout.toml"), toml).unwrap();
}

/// File-as-truth: set the project stage directly in project.json (the way the AI or
/// the ticket-10 orchestrator would after tts/matte finished).
fn set_stage(dir: &Path, stage: &str) {
    let path = dir.join("project.json");
    let mut pj: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    pj["stage"] = Value::String(stage.to_string());
    pj["failed"] = Value::Null;
    fs::write(&path, serde_json::to_string_pretty(&pj).unwrap()).unwrap();
}

fn run_timeline(data_dir: &Path, id: &str) -> Value {
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "timeline",
            "--project",
            id,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn run_render(data_dir: &Path, id: &str) -> (std::process::Output, Value) {
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "render",
            "--project",
            id,
            "--json",
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    (out, v)
}

/// Container / audio-track / video-track duration via ffprobe (test-side only; the
/// render itself must not need ffprobe — spec: 时长从 wav 头解析).
fn probe_durations(mp4: &Path) -> (f64, f64, f64) {
    let mut durations = Vec::new();
    for sel in [
        vec!["-show_entries", "format=duration"],
        vec!["-select_streams", "a:0", "-show_entries", "stream=duration"],
        vec!["-select_streams", "v:0", "-show_entries", "stream=duration"],
    ] {
        let out = sys("ffprobe")
            .args(["-v", "error"])
            .args(&sel)
            .args(["-of", "csv=p=0"])
            .arg(mp4)
            .output()
            .unwrap();
        let value: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
        durations.push(value);
    }
    (durations[0], durations[1], durations[2])
}

/// Page-turn timestamps via the timeline spike's gray-ladder scene-change detection.
fn detect_turn_points(mp4: &Path) -> Vec<f64> {
    let out = sys("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(mp4)
        .args([
            "-filter:v",
            "select='gt(scene,0.02)',showinfo",
            "-f",
            "null",
            "-",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut detected = Vec::new();
    for line in stderr.lines() {
        if let Some(idx) = line.find("pts_time:") {
            let rest = &line[idx + "pts_time:".len()..];
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(t) = num.parse::<f64>() {
                detected.push(t);
            }
        }
    }
    detected
}

/// Frame at `sec` as raw RGB24 (W*H*3 bytes).
fn grab_frame(mp4: &Path, sec: f64) -> Vec<u8> {
    let out = sys("ffmpeg")
        .args(["-y", "-v", "error"])
        .arg("-ss")
        .arg(format!("{sec}"))
        .arg("-i")
        .arg(mp4)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data = out.stdout;
    assert_eq!(data.len(), W * H * 3, "frame payload size");
    data
}

fn px(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8) {
    let i = (y * W + x) * 3;
    (frame[i], frame[i + 1], frame[i + 2])
}

fn is_red(c: (u8, u8, u8)) -> bool {
    c.0 > 150 && c.1 < 100 && c.2 < 100
}

fn is_grayish(c: (u8, u8, u8)) -> bool {
    // neutral color (slide grays are R=G=B), clearly not white
    let (r, g, b) = (c.0 as i32, c.1 as i32, c.2 as i32);
    (r - g).abs() < 12 && (g - b).abs() < 12 && r < 220
}

fn is_white(c: (u8, u8, u8)) -> bool {
    c.0 > 240 && c.1 > 240 && c.2 > 240
}

/// Horizontal extent of red pixels in a row, restricted to [from, to).
fn red_extent(frame: &[u8], y: usize, from: usize, to: usize) -> (usize, usize) {
    let xs: Vec<usize> = (from..to).filter(|&x| is_red(px(frame, x, y))).collect();
    (*xs.first().unwrap(), *xs.last().unwrap())
}

fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// Shared setup: project + pages + timeline + alpha.mov + stage=matte_done.
fn ready_project(tmp: &Path) -> (String, PathBuf) {
    let (id, dir) = create_project(tmp);
    setup_pages(&dir);
    run_timeline(tmp, &id);
    write_alpha_mov(&dir.join("work/person/alpha.mov"));
    set_stage(&dir, "matte_done");
    (id, dir)
}

#[test]
fn render_hard_cut_produces_final_mp4_with_spike_timing_and_pip() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());

    let (out, v) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["ok"], true);
    assert_eq!(v["stage"], "done");
    assert_eq!(v["output_path"], "output/final.mp4");
    assert_eq!(v["pip"]["source"], "alpha");
    assert_eq!(v["layout"]["position"], "bottom_right");
    assert_eq!(v["layout"]["transition"], "hard");
    assert!(approx(v["total_sec"].as_f64().unwrap(), TOTAL, 1e-9));

    let mp4 = dir.join("output/final.mp4");
    assert!(mp4.is_file());

    // 容器/视频/音轨三口径时长一致 — and the -t hard lock is what keeps the repeated
    // last concat image and the infinite looped person track from adding duration
    let (container, audio, video) = probe_durations(&mp4);
    for (label, d) in [("container", container), ("audio", audio), ("video", video)] {
        assert!(approx(d, TOTAL, 0.05), "{label} duration {d} != {TOTAL}");
    }
    assert!(
        video < TOTAL + 0.05,
        "-t hard lock violated: video {video} would be ~{TOTAL}+3.5 without it"
    );

    // 翻页点误差 ≤ 33 ms (1 frame @30fps, the timeline spike's measured floor)
    let turns = detect_turn_points(&mp4);
    assert_eq!(turns.len(), 4, "expected exactly 4 page turns: {turns:?}");
    for (got, want) in turns.iter().zip(TURN_POINTS) {
        assert!(
            approx(*got, want, 0.034),
            "turn point {got} vs expected {want}: error {} ms exceeds 1 frame @30fps",
            ((got - want).abs() * 1000.0).round()
        );
    }

    // 画中画位置/宽度/形状按 layout.toml 生效: bottom_right, 20% width, rounded_rect.
    // Person alpha: red box occupies 20..180 of the 200px source → 12.8..115.2 of the
    // 128px PiP; rounded-rect corner radius = min(W,H)/8 = 16 keeps rel(16,16) inside.
    let frame = grab_frame(&mp4, 1.0);
    assert!(
        is_red(px(&frame, (PIP_X + 64) as usize, (PIP_Y + 64) as usize)),
        "PiP center must show the person: {:?}",
        px(&frame, (PIP_X + 64) as usize, (PIP_Y + 64) as usize)
    );
    // outside the PiP box: plain slide
    assert!(
        is_grayish(px(&frame, (PIP_X - 12) as usize, (PIP_Y + 64) as usize)),
        "left of the PiP must be slide gray: {:?}",
        px(&frame, (PIP_X - 12) as usize, (PIP_Y + 64) as usize)
    );
    // shape mask cuts the corner: rel(6,6) is outside the r=16 corner disk
    assert!(
        is_grayish(px(&frame, (PIP_X + 6) as usize, (PIP_Y + 6) as usize)),
        "PiP corner must be transparent (shape mask): {:?}",
        px(&frame, (PIP_X + 6) as usize, (PIP_Y + 6) as usize)
    );
    // rel(16,16): inside the rounded-rect disk but outside a circle of r=64 —
    // distinguishes rounded_rect from circle
    assert!(
        is_red(px(&frame, (PIP_X + 16) as usize, (PIP_Y + 16) as usize)),
        "rounded-rect corner disk must keep the person visible: {:?}",
        px(&frame, (PIP_X + 16) as usize, (PIP_Y + 16) as usize)
    );
    // width: red extent in the PiP's center row ≈ 0.8 * pip_w, right edge at x = W-80
    let (x0, x1) = red_extent(&frame, (PIP_Y + 64) as usize, (PIP_X - 40) as usize, W - 40);
    let want0 = PIP_X as f64 + 0.1 * PIP_W as f64;
    let want1 = PIP_X as f64 + 0.9 * PIP_W as f64;
    assert!(
        approx(x0 as f64, want0, 3.0) && approx(x1 as f64, want1, 3.0),
        "red extent {x0}..{x1} != expected {want0}..{want1} (width_pct knob)"
    );

    // project.json: stage done + render record with the render fingerprint
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert_eq!(pj["stage"], "done");
    assert!(pj["failed"].is_null());
    let record = &pj["stages"]["render"];
    assert_eq!(record["stage"], "render");
    assert_eq!(record["state"], "success");
    let fp = record["input_fingerprint"].as_str().unwrap();
    assert_eq!(fp, v["fingerprint"].as_str().unwrap());
    assert_eq!(fp.strip_prefix("sha256:").unwrap().len(), 64);
    // render depends on the timeline fingerprint
    assert_eq!(
        record["depends_on"]["timeline"],
        pj["stages"]["timeline"]["input_fingerprint"]
    );
    let outputs = record["outputs"].as_array().unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0]["path"], "output/final.mp4");
    assert!(outputs[0]["bytes"].as_u64().unwrap() > 0);
    assert!(outputs[0]["sha256"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(record["tool_version"]
        .as_str()
        .unwrap()
        .starts_with("ffmpeg/"));
}

#[test]
fn render_fade_transition_fades_each_page_to_white() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    write_layout(&dir, "bottom_right", 20.0, "rounded_rect", "fade");

    let (out, v) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["layout"]["transition"], "fade");

    let mp4 = dir.join("output/final.mp4");
    // slide grays are 40/80/... : the top-left corner (far from the PiP) shows the bg
    let bg = |sec: f64| px(&grab_frame(&mp4, sec), 10, 10);
    // mid-page: slide content visible (page 1 gray 40, page 2 gray 80)
    assert!(
        is_grayish(bg(1.0)) && bg(1.0).0 < 90,
        "page 1 mid-frame must show the slide: {:?}",
        bg(1.0)
    );
    assert!(
        is_grayish(bg(5.0)) && bg(5.0).0 > 60 && bg(5.0).0 < 130,
        "page 2 mid-frame must show the slide: {:?}",
        bg(5.0)
    );
    // fading out shortly before the turn (3.3..3.6 window)
    assert!(
        bg(3.45).0 > 150,
        "frame at 3.45 must be mostly faded to white: {:?}",
        bg(3.45)
    );
    // 淡到白底: exactly white at the turn point and during the fade-in start
    assert!(is_white(bg(3.6)), "turn point frame: {:?}", bg(3.6));
    assert!(bg(3.75).0 > 150, "fade-in window: {:?}", bg(3.75));
    // far from any fade window on page 1: unfaded
    assert!(bg(2.0).0 < 90, "page 1 late frame: {:?}", bg(2.0));

    // duration must be unaffected by the transition mode
    let (container, audio, video) = probe_durations(&mp4);
    for (label, d) in [("container", container), ("audio", audio), ("video", video)] {
        assert!(approx(d, TOTAL, 0.05), "{label} duration {d} != {TOTAL}");
    }
}

#[test]
fn render_circle_shape_masks_the_pip_corner() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    write_layout(&dir, "bottom_right", 20.0, "circle", "hard");

    let (out, _) = run_render(tmp.path(), &id);
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["layout"]["shape"],
        "circle"
    );

    let frame = grab_frame(&dir.join("output/final.mp4"), 1.0);
    // circle of radius 64: rel(16,16) sits outside the circle (dist ≈ 67) — the same
    // point is INSIDE the rounded-rect corner disk, so this pixel proves the shape knob
    assert!(
        is_grayish(px(&frame, (PIP_X + 16) as usize, (PIP_Y + 16) as usize)),
        "circle must cut rel(16,16): {:?}",
        px(&frame, (PIP_X + 16) as usize, (PIP_Y + 16) as usize)
    );
    assert!(
        is_red(px(&frame, (PIP_X + 64) as usize, (PIP_Y + 64) as usize)),
        "circle center must show the person"
    );
    // and the person's own transparent background stays transparent inside the circle
    assert!(
        is_grayish(px(&frame, (PIP_X + 6) as usize, (PIP_Y + 6) as usize)),
        "outside the circle: {:?}",
        px(&frame, (PIP_X + 6) as usize, (PIP_Y + 6) as usize)
    );
}

#[test]
fn render_free_layout_places_the_person_at_the_normalized_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    write_free_layout(&dir, 25.0, 25.0, 20.0);

    let (out, v) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["layout"]["mode"], "free");
    assert_eq!(v["layout"]["x_pct"], 25.0);
    assert_eq!(v["layout"]["y_pct"], 25.0);

    // Canvas 640x360, PiP 128x128: free anchor 25/25 puts the top-left at
    // ((640-128)*.25, (360-128)*.25) = (128, 58).
    let x = ((W as f64 - PIP_W as f64) * 0.25).round() as i64;
    let y = ((H as f64 - PIP_W as f64) * 0.25).round() as i64;
    let frame = grab_frame(&dir.join("output/final.mp4"), 1.0);
    assert!(
        is_red(px(&frame, (x + 64) as usize, (y + 64) as usize)),
        "free PiP center must show the person: {:?}",
        px(&frame, (x + 64) as usize, (y + 64) as usize)
    );
    assert!(
        is_grayish(px(&frame, (x - 12) as usize, (y + 64) as usize)),
        "left of the free PiP must be slide gray: {:?}",
        px(&frame, (x - 12) as usize, (y + 64) as usize)
    );
    assert!(
        is_grayish(px(&frame, (x + 64) as usize, (y - 12) as usize)),
        "above the free PiP must be slide gray: {:?}",
        px(&frame, (x + 64) as usize, (y - 12) as usize)
    );
}

#[test]
fn render_right_bar_strips_the_person_along_the_right_edge() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    write_layout(&dir, "right_bar", 20.0, "circle", "hard");

    let (out, v) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["pip"]["source"],
        "alpha"
    );
    assert_eq!(v["layout"]["position"], "right_bar");

    let frame = grab_frame(&dir.join("output/final.mp4"), 1.0);
    // strip: full height at the right edge, width = 20% of 640 = 128 → x 512..640.
    // Person red box (20..180 of 200) scaled to canvas height covers 36..324 of the
    // strip's height and the full strip width after the center crop.
    assert!(
        is_red(px(&frame, W - 64, H / 2)),
        "strip interior must show the person: {:?}",
        px(&frame, W - 64, H / 2)
    );
    assert!(
        is_grayish(px(&frame, W - PIP_W as usize - 20, H / 2)),
        "left of the strip must be slide: {:?}",
        px(&frame, W - PIP_W as usize - 20, H / 2)
    );
    // person's own transparent background shows the slide above/below the red box
    assert!(
        is_grayish(px(&frame, W - 64, 10)) && is_grayish(px(&frame, W - 64, H - 10)),
        "strip top/bottom must be transparent: {:?} {:?}",
        px(&frame, W - 64, 10),
        px(&frame, W - 64, H - 10)
    );
}

#[test]
fn render_free_layout_caps_a_tall_person_to_the_canvas_height() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    run_timeline(tmp.path(), &id);
    write_tall_alpha_mov(&dir.join("work/person/alpha.mov"));
    set_stage(&dir, "matte_done");
    write_free_layout(&dir, 50.0, 50.0, 40.0);

    let (out, _) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // 40% of 640 = 256 px wide; 100:300 aspect would make it 768 px tall. Render caps
    // the width to 120 px (360 px tall), so the PiP fits from y=0 to y=359.
    let frame = grab_frame(&dir.join("output/final.mp4"), 1.0);
    assert!(is_red(px(&frame, W / 2, H / 2)), "tall PiP center");
    assert!(
        is_red(px(&frame, W / 2, 20)),
        "tall PiP top area is visible"
    );
    assert!(
        is_red(px(&frame, W / 2, H - 20)),
        "tall PiP bottom edge is visible, not clipped"
    );
}

#[test]
fn render_free_card_layout_keeps_the_card_inside_the_normalized_anchor() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    run_timeline(tmp.path(), &id);
    write_person_mp4(&dir.join("source/person.mp4"));
    set_stage(&dir, "tts_done");
    write_free_layout(&dir, 10.0, 10.0, 20.0);

    let (out, v) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["pip"]["source"], "card");
    let frame = grab_frame(&dir.join("output/final.mp4"), 1.0);
    // Free anchor uses the padded card box (128 video + 32 border = 160) in the
    // remaining 640-160 / 360-160 space.
    let card_x = ((W as f64 - 160.0) * 0.10).round() as i64;
    let card_y = ((H as f64 - 160.0) * 0.10).round() as i64;
    assert!(
        is_red(px(&frame, (card_x + 80) as usize, (card_y + 80) as usize)),
        "free card interior must show the person"
    );
    assert!(
        is_white(px(&frame, (card_x + 8) as usize, (card_y + 80) as usize)),
        "free card ring must stay white"
    );
    assert!(
        is_grayish(px(&frame, (card_x - 12) as usize, (card_y + 80) as usize)),
        "outside the free card must be slide"
    );
}

#[test]
fn render_without_matte_wraps_the_original_video_in_a_card() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    run_timeline(tmp.path(), &id);
    // matte skipped: no work/person/alpha.mov, only the original upload
    write_person_mp4(&dir.join("source/person.mp4"));
    set_stage(&dir, "tts_done");

    let (out, v) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(v["pip"]["source"], "card");
    assert_eq!(v["pip"]["input"], "source/person.mp4");
    assert_eq!(v["stage"], "done");

    let frame = grab_frame(&dir.join("output/final.mp4"), 1.0);
    // card = 128px video + 16px white border → 160x160 at (W-160-80, H-160-60)
    let card_x = (W as i64) - PIP_W - 2 * 16 - 80;
    let card_y = (H as i64) - PIP_W - 2 * 16 - 60;
    // video interior shows the person (red box center)
    assert!(
        is_red(px(&frame, (card_x + 80) as usize, (card_y + 80) as usize)),
        "card interior must show the original video: {:?}",
        px(&frame, (card_x + 80) as usize, (card_y + 80) as usize)
    );
    // the 16px ring between card edge and video is the white card
    assert!(
        is_white(px(&frame, (card_x + 8) as usize, (card_y + 80) as usize)),
        "card ring must be white: {:?}",
        px(&frame, (card_x + 8) as usize, (card_y + 80) as usize)
    );
    // outside the card: slide
    assert!(
        is_grayish(px(&frame, (card_x - 12) as usize, (card_y + 80) as usize)),
        "outside the card must be slide: {:?}",
        px(&frame, (card_x - 12) as usize, (card_y + 80) as usize)
    );
    // the card's corner is rounded (r = min(W,H)/8 = 20 → rel(4,4) is outside)
    assert!(
        is_grayish(px(&frame, (card_x + 4) as usize, (card_y + 4) as usize)),
        "card corner must be rounded: {:?}",
        px(&frame, (card_x + 4) as usize, (card_y + 4) as usize)
    );
}

#[test]
fn render_errors_are_structured_for_missing_inputs_and_illegal_state() {
    // no person input at all
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = create_project(tmp.path());
    setup_pages(&dir);
    run_timeline(tmp.path(), &id);
    set_stage(&dir, "tts_done");
    let (out, v) = run_render(tmp.path(), &id);
    assert!(!out.status.success());
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "person_missing");

    // no timeline record: render ensures the track from the audio/slides
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    let path = dir.join("project.json");
    let mut pj: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    pj["stages"].as_object_mut().unwrap().remove("timeline");
    fs::write(&path, serde_json::to_string_pretty(&pj).unwrap()).unwrap();
    let (out, v) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "render must repair a missing timeline: {v}"
    );
    assert_eq!(v["ok"], true);
    let pj: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(pj["stages"]["timeline"].is_object());

    // illegal state: created → rendering is not an edge (state must stay unchanged)
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    set_stage(&dir, "created");
    let (out, v) = run_render(tmp.path(), &id);
    assert!(!out.status.success());
    assert_eq!(v["error"]["code"], "invalid_transition");
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert_eq!(
        pj["stage"], "created",
        "state unchanged after illegal render"
    );
}

#[test]
fn changing_layout_invalidates_only_the_render() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());

    let (out, v1) = run_render(tmp.path(), &id);
    assert!(out.status.success());
    let fp1 = v1["fingerprint"].as_str().unwrap().to_string();

    // 改版式 → 只有成片失效: width 20% -> 25%, re-render after resetting the stage
    write_layout(&dir, "bottom_right", 25.0, "rounded_rect", "hard");
    set_stage(&dir, "matte_done");
    let (out, v2) = run_render(tmp.path(), &id);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fp2 = v2["fingerprint"].as_str().unwrap().to_string();
    assert_ne!(fp1, fp2, "layout change must change the render fingerprint");

    // timeline fingerprint untouched, and the new render still depends on it
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert!(pj["stages"]["timeline"]["input_fingerprint"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(
        pj["stages"]["render"]["depends_on"]["timeline"],
        pj["stages"]["timeline"]["input_fingerprint"]
    );
    assert_eq!(v2["stage"], "done");

    // the width knob actually took effect: 25% of 640 = 160px PiP — the red box now
    // reaches further right than the 128px PiP ever could (old box ends at x≈547)
    let frame = grab_frame(&dir.join("output/final.mp4"), 1.0);
    let (x0, x1) = red_extent(&frame, (PIP_Y + 64) as usize, (PIP_X - 60) as usize, W - 40);
    let want1 = (W as i64) - 80 - 16 - 3; // right edge of a 160px box minus tolerance
    assert!(
        x1 as i64 >= want1,
        "25% width: red extent right edge {x1} should reach ~{want1}"
    );
    assert!(x0 < (PIP_X + 64) as usize, "extent starts left of old box");
}

/// 票 16 验收抓到的静默过期：渲染失败进 failed(rendering) 后，同一阶段重试成功
/// 必须清掉旧失败记录——否则 stage=done 但 status/next 仍说“上次渲染失败”。
#[test]
fn successful_retry_after_a_failed_render_clears_the_failure_record() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let (id, dir) = create_project(&data_dir);
    setup_pages(&dir);
    write_person_mp4(&dir.join("source/person.mp4"));
    write_layout(&dir, "bottom_right", 20.0, "rounded_rect", "hard");
    set_stage(&dir, "tts_done");
    run_timeline(&data_dir, &id);

    // 破坏人物视频让 render 在 rendering 阶段真实失败（timeline 预检不依赖人物内容）
    let person = dir.join("source/person.mp4");
    let valid_person = fs::read(&person).unwrap();
    fs::write(&person, b"not a video").unwrap();
    let failed = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "render",
            "--project",
            &id,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !failed.status.success(),
        "broken person must fail the render"
    );
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert_eq!(pj["stage"], "failed");
    assert_eq!(pj["failed"]["stage"], "rendering");

    // 恢复输入后重试同一阶段：成功且旧失败记录清空
    fs::write(&person, valid_person).unwrap();
    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "render",
            "--project",
            &id,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["stage"], "done");
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert_eq!(pj["stage"], "done");
    assert!(
        pj["failed"].is_null(),
        "successful retry must clear the stale failure record"
    );
}

/// 上游（TTS）失败后直接 render 会把旧配音渲染成 done，掩盖失败；必须拒绝。
#[test]
fn render_refuses_to_hide_an_upstream_tts_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let (id, dir) = create_project(&data_dir);
    setup_pages(&dir);
    write_person_mp4(&dir.join("source/person.mp4"));
    write_layout(&dir, "bottom_right", 20.0, "rounded_rect", "hard");
    set_stage(&dir, "tts_done");
    run_timeline(&data_dir, &id);

    let path = dir.join("project.json");
    let mut pj: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    pj["stage"] = Value::String("failed".into());
    pj["failed"] = serde_json::json!({ "stage": "tts_running", "error": "page 2 failed" });
    fs::write(&path, serde_json::to_string_pretty(&pj).unwrap()).unwrap();

    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "render",
            "--project",
            &id,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "render must not turn a TTS failure into done"
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "upstream_failed");
    let pj: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    assert_eq!(pj["stage"], "failed", "the failure must stay visible");
}

/// 换了出镜视频但旧 alpha 还在：matte 记录的 video 绑定不匹配时必须走卡片兜底，
/// 不能把上一段视频的人像叠进新成片（票 16 复审）。
#[test]
fn an_alpha_from_a_different_video_is_not_composited() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let (id, dir) = create_project(&data_dir);
    setup_pages(&dir);
    write_person_mp4(&dir.join("source/person.mp4"));
    write_alpha_mov(&dir.join("work/person/alpha.mov"));
    write_layout(&dir, "bottom_right", 20.0, "rounded_rect", "hard");
    set_stage(&dir, "matte_done");
    run_timeline(&data_dir, &id);

    let path = dir.join("project.json");
    let mut pj: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    pj["stages"]["matte"] = serde_json::json!({
        "stage": "matte",
        "state": "success",
        "input_fingerprint": "sha256:old",
        "tool_version": "test",
        "outputs": [],
        "depends_on": { "video": "sha256:a-different-video" }
    });
    fs::write(&path, serde_json::to_string_pretty(&pj).unwrap()).unwrap();

    let out = cli()
        .args([
            "--data-dir",
            data_dir.to_str().unwrap(),
            "render",
            "--project",
            &id,
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["pip"]["source"], "card",
        "stale alpha must fall back to the card"
    );
    assert!(dir.join("output/final.mp4").is_file());
}

#[test]
fn tampered_final_mp4_is_not_certified_by_a_skip() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = ready_project(tmp.path());
    let (out, v) = run_render(tmp.path(), &id);
    assert!(out.status.success(), "first render: {v}");
    assert_eq!(v["skipped"], false);

    std::fs::write(dir.join("output/final.mp4"), b"tampered").unwrap();
    let (out, v) = run_render(tmp.path(), &id);
    assert!(out.status.success(), "second render: {v}");
    assert_eq!(
        v["skipped"], false,
        "a tampered final.mp4 must invalidate the render skip"
    );
    assert_ne!(
        std::fs::read(dir.join("output/final.mp4")).unwrap(),
        b"tampered",
        "render must restore the film"
    );
}
