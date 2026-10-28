//! Shared fixtures for the cc-mcp integration tests: real ffmpeg-built media so
//! every stage runs its real path (spec Testing Decisions, 缝 1 applied to MCP).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn ffmpeg_ok(args: &[&str]) {
    let out = Command::new("ffmpeg").args(args).output().unwrap();
    assert!(
        out.status.success(),
        "ffmpeg {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

pub(crate) fn write_gray_png(path: &Path, gray: u8) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    ffmpeg_ok(&[
        "-y",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        &format!("color=c=0x{gray:02x}{gray:02x}{gray:02x}:s=320x180"),
        "-frames:v",
        "1",
        &path.to_string_lossy(),
    ]);
}

pub(crate) fn write_sine_wav(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    ffmpeg_ok(&[
        "-y",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=0.5",
        "-ar",
        "24000",
        "-ac",
        "1",
        &path.to_string_lossy(),
    ]);
}

pub(crate) fn write_person_mp4(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    ffmpeg_ok(&[
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "color=c=0x303030:s=200x200:d=1",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-pix_fmt",
        "yuv420p",
        &path.to_string_lossy(),
    ]);
}

/// Minimal pptx (zip of slide + notesSlide parts), the layout PowerPoint writes.
fn write_pptx(path: &Path, notes: [Option<&[&str]>; 2]) {
    fn notes_xml(paragraphs: &[&str]) -> String {
        paragraphs
            .iter()
            .map(|p| format!(r#"<a:p><a:r><a:t>{p}</a:t></a:r></a:p>"#))
            .collect::<String>()
    }
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (i, note) in notes.iter().enumerate() {
        let n = i + 1;
        zip.start_file(
            format!("ppt/slides/slide{n}.xml"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, br#"<p:sld xmlns:a="a"><p:cSld/></p:sld>"#).unwrap();
        let body = match note {
            Some(paragraphs) => notes_xml(paragraphs),
            None => String::new(),
        };
        zip.start_file(
            format!("ppt/notesSlides/notesSlide{n}.xml"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(
            &mut zip,
            format!(r#"<p:notes xmlns:a="a"><p:txBody>{body}</p:txBody></p:notes>"#).as_bytes(),
        )
        .unwrap();
    }
    zip.finish().unwrap();
}

pub(crate) struct Fixtures {
    pub(crate) assets: PathBuf,
    pub(crate) model: PathBuf,
    pub(crate) credentials_file: PathBuf,
}

pub(crate) fn make_fixtures(tmp: &Path) -> Fixtures {
    let assets = tmp.join("assets");
    fs::create_dir_all(&assets).unwrap();
    write_gray_png(&assets.join("img1.png"), 40);
    write_gray_png(&assets.join("img2.png"), 90);
    write_sine_wav(&assets.join("voice.wav"));
    write_person_mp4(&assets.join("person.mp4"));
    write_pptx(
        &assets.join("deck-notes.pptx"),
        [Some(&["第一页备注"]), Some(&["第二页备注"])],
    );
    // the fake matte engine never reads the model bytes; the fingerprint only hashes it
    let model = tmp.join("fake-model.onnx");
    fs::write(&model, b"not-a-real-onnx").unwrap();
    let credentials_file = tmp.join("creds.txt");
    Fixtures {
        assets,
        model,
        credentials_file,
    }
}

/// deck folder inside a project's source/: N gray pages, numeric filename order.
pub(crate) fn seed_deck(project_root: &Path, assets: &Path, pages: usize) {
    let deck = project_root.join("source/deck-img");
    fs::create_dir_all(&deck).unwrap();
    for i in 1..=pages {
        fs::copy(
            assets.join(format!("img{}.png", if i % 2 == 1 { 1 } else { 2 })),
            deck.join(format!("{i}.png")),
        )
        .unwrap();
    }
}
