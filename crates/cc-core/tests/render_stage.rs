//! Seams: the `layout.toml` file format (`cc_core::render::{parse_layout, load_layout,
//! Layout::canonical}`) and the render input fingerprint (`cc_core::render::
//! render_fingerprint`) — PRD §6.5 "layout.toml # 版式参数", PRD §6.6.1
//! "render = timeline 指纹 + 版式参数 + ffmpeg 版本 + 编码参数".

use cc_core::render::{self, Placement};
use cc_core::Stage;

const VALID: &str = "\
position = \"bottom_left\"
width_pct = 25.0
shape = \"circle\"
transition = \"fade\"
";

const FREE: &str = "\
x_pct = 25.0
y_pct = 10.0
width_pct = 20.0
shape = \"rounded_rect\"
transition = \"hard\"
";

#[test]
fn missing_layout_file_falls_back_to_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = render::load_layout(&tmp.path().join("layout.toml")).unwrap();
    assert_eq!(layout.placement, Placement::Preset("bottom_right".into()));
    assert!((layout.width_pct - 20.0).abs() < 1e-9);
    assert_eq!(layout.shape, "rounded_rect");
    assert_eq!(layout.transition, "hard");
}

#[test]
fn layout_parses_all_four_fields() {
    let layout = render::parse_layout(VALID).unwrap();
    assert_eq!(layout.placement, Placement::Preset("bottom_left".into()));
    assert!((layout.width_pct - 25.0).abs() < 1e-9);
    assert_eq!(layout.shape, "circle");
    assert_eq!(layout.transition, "fade");
}

#[test]
fn free_layout_parses_normalized_position() {
    let layout = render::parse_layout(FREE).unwrap();
    assert_eq!(
        layout.placement,
        Placement::Free {
            x_pct: 25.0,
            y_pct: 10.0
        }
    );
    assert!(layout.canonical().contains("x_pct=25.000"));
    assert!(layout.canonical().contains("y_pct=10.000"));
}

#[test]
fn free_layout_requires_a_complete_unambiguous_placement() {
    for (toml, what) in [
        (
            "x_pct = 25.0\nwidth_pct = 20.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n",
            "missing y",
        ),
        (
            "position = \"bottom_right\"\nx_pct = 25.0\ny_pct = 10.0\nwidth_pct = 20.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n",
            "mixed placement",
        ),
        (
            "x_pct = -1.0\ny_pct = 10.0\nwidth_pct = 20.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n",
            "negative x",
        ),
        (
            "x_pct = 25.0\ny_pct = 10.0\nwidth_pct = 4.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n",
            "too narrow",
        ),
        (
            "x_pct = 25.0\ny_pct = 10.0\nwidth_pct = 41.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n",
            "too wide",
        ),
    ] {
        let err = render::parse_layout(toml).expect_err(what);
        assert_eq!(err.code(), "layout_invalid", "{what}: {err}");
    }
}

#[test]
fn layout_rejects_unknown_values() {
    for (bad, what) in [
        ("position = \"top\"", "position"),
        ("shape = \"hexagon\"", "shape"),
        ("transition = \"slide\"", "transition"),
        ("width_pct = 0", "width_pct"),
        ("width_pct = 150", "width_pct"),
        ("width_pct = \"wide\"", "width_pct"),
    ] {
        let toml = format!(
            "{bad}\nshape = \"circle\"\nposition = \"bottom_right\"\ntransition = \"hard\"\n"
        );
        let err = render::parse_layout(&toml).expect_err(&format!("{what} must be rejected"));
        assert!(
            matches!(err, cc_core::Error::InvalidLayout { .. }),
            "{what}: expected layout_invalid, got {err:?}"
        );
        assert_eq!(err.code(), "layout_invalid");
    }
}

#[test]
fn layout_missing_file_field_is_rejected_not_defaulted() {
    // layout.toml is AI-editable truth: a missing key must error, not silently default
    let err = render::parse_layout("position = \"bottom_right\"\n").unwrap_err();
    assert_eq!(err.code(), "layout_invalid");
    let err = render::parse_layout("garbage not toml [[").unwrap_err();
    assert_eq!(err.code(), "layout_invalid");
}

#[test]
fn canonical_layout_is_normalized_for_the_fingerprint() {
    let a = render::parse_layout("position = \"bottom_right\"\nwidth_pct = 20\nshape = \"rounded_rect\"\ntransition = \"hard\"\n").unwrap();
    let b = render::parse_layout("transition = \"hard\"\nshape = \"rounded_rect\"\nwidth_pct = 20.0\nposition = \"bottom_right\"\n").unwrap();
    assert_eq!(
        a.canonical(),
        b.canonical(),
        "key order / int-vs-float must not matter"
    );
    // any real change must show up
    let c = render::parse_layout("position = \"bottom_right\"\nwidth_pct = 25.0\nshape = \"rounded_rect\"\ntransition = \"hard\"\n").unwrap();
    assert_ne!(a.canonical(), c.canonical());
}

#[test]
fn render_fingerprint_binds_timeline_layout_ffmpeg_and_encode() {
    let tl = "sha256:aa";
    let layout = render::default_layout();

    let fp = render::render_fingerprint(tl, &layout, "8.1.1", "1920x1080");
    // stable + hex sha256
    assert_eq!(
        fp,
        render::render_fingerprint(tl, &layout, "8.1.1", "1920x1080")
    );
    let hex = fp.strip_prefix("sha256:").unwrap();
    assert_eq!(hex.len(), 64);

    // timeline fingerprint changed -> render fingerprint changes (改稿 → 整轨 → 成片)
    assert_ne!(
        fp,
        render::render_fingerprint("sha256:bb", &layout, "8.1.1", "1920x1080")
    );
    // layout changed -> render fingerprint changes (改版式 → 只有成片失效)
    let mut wide = layout.clone();
    wide.width_pct = 25.0;
    assert_ne!(
        fp,
        render::render_fingerprint(tl, &wide, "8.1.1", "1920x1080")
    );
    let mut circle = layout.clone();
    circle.shape = "circle".into();
    assert_ne!(
        fp,
        render::render_fingerprint(tl, &circle, "8.1.1", "1920x1080")
    );
    let free = render::Layout::free(25.0, 10.0, 20.0, "rounded_rect", "hard");
    let free_fp = render::render_fingerprint(tl, &free, "8.1.1", "1920x1080");
    assert_ne!(fp, free_fp);
    let moved = render::Layout::free(30.0, 10.0, 20.0, "rounded_rect", "hard");
    assert_ne!(
        free_fp,
        render::render_fingerprint(tl, &moved, "8.1.1", "1920x1080")
    );
    // ffmpeg version changed -> render fingerprint changes
    assert_ne!(
        fp,
        render::render_fingerprint(tl, &layout, "7.0.0", "1920x1080")
    );
    // resolution changed -> render fingerprint changes
    assert_ne!(
        fp,
        render::render_fingerprint(tl, &layout, "8.1.1", "1280x720")
    );
}

#[test]
fn matte_skipped_may_enter_rendering_from_tts_done() {
    // 抠像跳过分支：tts_done 直接进 rendering（最小新增边，既有边不动）
    assert!(cc_core::is_legal_edge(Stage::TtsDone, Stage::Rendering));
    // the linear chain is untouched
    assert!(cc_core::is_legal_edge(Stage::MatteDone, Stage::Rendering));
    // pre-matte states still cannot render
    assert!(!cc_core::is_legal_edge(Stage::Parsed, Stage::Rendering));
    assert!(!cc_core::is_legal_edge(Stage::TtsRunning, Stage::Rendering));
    assert!(!cc_core::is_legal_edge(Stage::Created, Stage::Rendering));
}

#[test]
fn ffmpeg_version_is_parsed_from_first_line() {
    let v = cc_core::ffmpeg::version().unwrap();
    assert!(
        !v.is_empty() && v.split_whitespace().count() == 1,
        "expected a single version token like `8.1.1-full_build...`, got `{v}`"
    );
}
