//! Seam: `cc_core::run_slide_render` — the slide_render stage behavior (ticket 03).
//! PDF fixtures are built in code as minimal multi-page PDFs (no data/ dependency);
//! page order is verified through rendered pixel colors, the way the pipeline consumes it.

use cc_core::{create_project, run_slide_render, Stage};
use image::GenericImageView as _;
use std::fs;
use std::path::Path;

/// Build a minimal valid PDF where page i is one solid gray rectangle (`gray_levels[i]`,
/// 0.0 = black … 1.0 = white). Objects: 1 catalog, 2 pages tree, then per page a page
/// object + content object. Offsets are computed as the bytes are assembled.
fn minimal_pdf(gray_levels: &[f32]) -> Vec<u8> {
    let count = gray_levels.len();
    let mut objs: Vec<String> = Vec::with_capacity(2 + 2 * count);
    let kids: Vec<String> = (0..count).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    objs.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objs.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        count
    ));
    for g in gray_levels {
        let content_obj = objs.len() as u32 + 2; // object number of the content stream
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {content_obj} 0 R /Resources << >> >>"
        ));
        let content = format!("{g} g 0 0 612 792 re f");
        objs.push(format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ));
    }

    let mut out: Vec<u8> = b"%PDF-1.4\n".to_vec();
    let mut offsets: Vec<usize> = Vec::with_capacity(objs.len() + 1);
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, body).as_bytes());
    }
    let xref_pos = out.len();
    let size = objs.len() + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

/// Sample the pixel at (10, 10) of a rendered slide: solid fills have no anti-aliasing,
/// so it equals the page's gray level exactly (±2 for encoder rounding).
fn center_pixel_luma(path: &Path) -> u8 {
    let img = image::open(path).unwrap();
    let px = img.get_pixel(10, 10);
    let [r, g, b, _] = px.0;
    assert_eq!(r, g, "solid gray fill must decode as gray: {path:?}");
    assert_eq!(g, b, "solid gray fill must decode as gray: {path:?}");
    r
}

/// Write a 64x48 solid-gray PNG (PowerPoint exports this shape of file name).
fn solid_png(dir: &Path, name: &str, luma: u8) {
    image::RgbaImage::from_pixel(64, 48, image::Rgba([luma, luma, luma, 255]))
        .save(dir.join(name))
        .unwrap();
}

/// Read project.json back from disk (file-based truth: assert what a human/AI sees).
fn project_on_disk(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap()
}

#[test]
fn rerun_with_unchanged_fingerprint_skips_clean_outputs_and_rerenders_tampered_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "t", None).unwrap();
    fs::write(
        created.dir.join("source").join("deck.pdf"),
        minimal_pdf(&[0.0, 0.5, 1.0]),
    )
    .unwrap();

    let mut project = created.project;
    let first = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();
    assert!(!first.skipped);
    assert!(
        first.fingerprint.starts_with("sha256:"),
        "fingerprint must be a sha256: value"
    );

    let record = &project_on_disk(&created.dir)["stages"]["slide_render"];
    assert_eq!(record["state"], "success");
    assert_eq!(record["input_fingerprint"], first.fingerprint.as_str());
    assert!(
        record["tool_version"]
            .as_str()
            .unwrap()
            .starts_with("slide-render/"),
        "tool_version must name the renderer"
    );
    assert_eq!(record["outputs"].as_array().unwrap().len(), 3);
    assert!(record["outputs"][0]["sha256"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert!(record["outputs"][0]["bytes"].as_u64().unwrap() > 0);

    // Overwrite one output with junk: the artifact integrity check must invalidate the
    // skip and re-render (a tampered output is not a reusable artifact).
    fs::write(created.dir.join("work/slides/slide_001.png"), b"junk").unwrap();

    let second = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();
    assert!(!second.skipped, "tampered output must re-render, not reuse");
    assert_eq!(second.fingerprint, first.fingerprint);
    assert_eq!(second.page_count, 3);
    assert_eq!(project.stage, Stage::Parsed);
    assert_ne!(
        fs::read(created.dir.join("work/slides/slide_001.png")).unwrap(),
        b"junk",
        "render must restore the slide"
    );

    // A clean rerun now skips and leaves the restored outputs untouched.
    let before = fs::read(created.dir.join("work/slides/slide_001.png")).unwrap();
    let third = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();
    assert!(third.skipped);
    assert_eq!(
        fs::read(created.dir.join("work/slides/slide_001.png")).unwrap(),
        before
    );
}

#[test]
fn missing_output_invalidates_skip_even_when_fingerprint_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "t", None).unwrap();
    fs::write(
        created.dir.join("source").join("deck.pdf"),
        minimal_pdf(&[0.2, 0.8]),
    )
    .unwrap();

    let mut project = created.project;
    run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();
    fs::remove_file(created.dir.join("work/slides/slide_002.png")).unwrap();

    let rerun = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();
    assert!(
        !rerun.skipped,
        "recorded outputs missing on disk must re-render"
    );
    assert!(created.dir.join("work/slides/slide_002.png").is_file());
}

#[test]
fn changed_source_rerenders_and_updates_fingerprint() {
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "t", None).unwrap();
    let deck = created.dir.join("source").join("deck.pdf");
    fs::write(&deck, minimal_pdf(&[0.0, 0.5, 1.0])).unwrap();

    let mut project = created.project;
    let first = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();

    fs::write(&deck, minimal_pdf(&[0.3, 0.6])).unwrap();
    let second = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();

    assert!(!second.skipped);
    assert_eq!(second.page_count, 2);
    assert_ne!(
        second.fingerprint, first.fingerprint,
        "changed source must change fingerprint"
    );
    let slides_dir = created.dir.join("work/slides");
    let files: Vec<_> = fs::read_dir(&slides_dir).unwrap().collect();
    assert_eq!(
        files.len(),
        2,
        "stale slide_003.png from the previous render must be gone"
    );
    assert_eq!(project.stage, Stage::Parsed);
}

#[test]
fn broken_pdf_fails_with_recorded_failure_then_retry_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "t", None).unwrap();
    let deck = created.dir.join("source").join("deck.pdf");
    fs::write(&deck, b"this is not a pdf").unwrap();

    let mut project = created.project;
    let err = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf"))
        .expect_err("garbage pdf must fail");
    assert_eq!(err.code(), "slide_render_failed");

    let pj = project_on_disk(&created.dir);
    assert_eq!(pj["stage"], "failed");
    assert_eq!(pj["failed"]["stage"], "parsing");
    assert!(!pj["failed"]["error"].as_str().unwrap().is_empty());

    fs::write(&deck, minimal_pdf(&[0.1, 0.9])).unwrap();
    let retry = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();
    assert_eq!(retry.page_count, 2, "failed -> parsing retry must work");
    assert_eq!(project.stage, Stage::Parsed);
    assert_eq!(project_on_disk(&created.dir)["stage"], "parsed");
}

#[test]
fn image_folder_is_sorted_numerically_not_lexically() {
    // Lexical order would be 幻灯片1, 幻灯片10, 幻灯片12, 幻灯片2 — the PRD §8.1 pitfall.
    // Distinct gray levels make the mapping observable through the output pixels.
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "t", None).unwrap();
    let folder = created.dir.join("source").join("slides-in");
    fs::create_dir_all(&folder).unwrap();
    solid_png(&folder, "幻灯片1.PNG", 20);
    solid_png(&folder, "幻灯片2.png", 40);
    solid_png(&folder, "幻灯片10.png", 200);
    solid_png(&folder, "幻灯片12.png", 240);
    fs::write(folder.join("Thumbs.db"), b"not an image").unwrap(); // noise must be ignored

    let mut project = created.project;
    let outcome =
        run_slide_render(&mut project, &created.dir, Path::new("source/slides-in")).unwrap();

    assert_eq!(outcome.page_count, 4);
    let expected = [20u8, 40, 200, 240];
    for (i, rel) in outcome.slides.iter().enumerate() {
        assert_eq!(rel.as_str(), format!("work/slides/slide_{:03}.png", i + 1));
        let luma = center_pixel_luma(&created.dir.join(rel));
        assert!(
            (i16::from(luma) - i16::from(expected[i])).abs() <= 2,
            "slide_00{} must be 幻灯片{}: luma {luma}, expected ~{}",
            i + 1,
            [1, 2, 10, 12][i],
            expected[i]
        );
    }
}

#[test]
fn pdf_renders_pages_in_order_as_numbered_pngs() {
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "渲染", None).unwrap();
    fs::write(
        created.dir.join("source").join("deck.pdf"),
        minimal_pdf(&[0.0, 0.5, 1.0]),
    )
    .unwrap();

    let mut project = created.project;
    let outcome = run_slide_render(&mut project, &created.dir, Path::new("deck.pdf")).unwrap();

    assert_eq!(outcome.page_count, 3);
    assert_eq!(
        outcome.slides,
        vec![
            "work/slides/slide_001.png",
            "work/slides/slide_002.png",
            "work/slides/slide_003.png"
        ],
        "slides must be named slide_NNN.png from 001, in page order"
    );
    assert_eq!(
        project.stage,
        Stage::Parsed,
        "slide_render moves created -> parsing -> parsed"
    );

    let expected = [0u8, 127, 255];
    for (i, rel) in outcome.slides.iter().enumerate() {
        let luma = center_pixel_luma(&created.dir.join(rel));
        assert!(
            (i16::from(luma) - i16::from(expected[i])).abs() <= 2,
            "slide_00{} must be page {}: luma {luma}, expected ~{}",
            i + 1,
            i + 1,
            expected[i]
        );
    }
}
