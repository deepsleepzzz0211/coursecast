//! slide_render stage (PRD §8.1: 只渲染不解析). One responsibility: any deck source
//! → a set of ordered, 1920-wide PNGs at `work/slides/slide_NNN.png` (from 001) —
//! the naming contract the timeline stage (ticket 02) consumes. No shape/text/layout
//! parsing: PDFs go through pdfium, image folders are sorted numerically by file name.

use crate::error::{Error, Result};
use crate::lock::ProjectLock;
use crate::project::{save_project, OutputArtifact};
use crate::stage;
use crate::state::Stage;
use crate::Project;
use pdfium_render::prelude::*;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Slide images are rendered at this width (PRD §8.1: 按宽度 1920 等比出图).
pub const TARGET_WIDTH: u32 = 1920;
/// Output directory relative to the project directory.
pub const SLIDES_DIR: &str = "work/slides";
/// Stage name used as the project.json `stages` key.
const SLIDE_STAGE: &str = "slide_render";

/// Page count of a rendered deck, from the slide_render stage record (script tools
/// validate page numbers against this; MCP script_set / script_import_notes callers).
pub fn slide_page_count(project: &crate::Project) -> Result<u32> {
    project
        .stages
        .get(SLIDE_STAGE)
        .filter(|r| r.state == "success")
        .map(|r| r.outputs.len() as u32)
        .ok_or_else(|| Error::StageMissing {
            stage: SLIDE_STAGE.into(),
        })
}

/// Image formats accepted from a folder source (checked case-insensitively; PowerPoint
/// exports `幻灯片N.PNG`).
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg"];

/// A resolved source: one PDF, or the numerically ordered image files of a folder.
#[derive(Debug, Clone, PartialEq)]
struct Inputs {
    kind: SourceKind,
    files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SourceKind {
    Pdf,
    Images,
}

/// What a slide_render run did. `slides` are paths relative to the project directory,
/// in page order; `skipped` is true when existing outputs were reused (idempotent run).
#[derive(Debug, Clone, PartialEq)]
pub struct SlideRenderOutcome {
    pub page_count: usize,
    pub slides: Vec<String>,
    /// Input fingerprint bound into the stage record: source hash + renderer version
    /// + target width. Set on render; reused verbatim on skip.
    pub fingerprint: String,
    pub skipped: bool,
    pub stage: Stage,
}

/// Convenience wrapper for tests and standalone callers: acquires the project lock and
/// delegates to `run_slide_render_locked`. Commands should acquire the lock themselves
/// and call the runner.
pub fn run_slide_render(
    project: &mut Project,
    dir: &Path,
    source: &Path,
) -> Result<SlideRenderOutcome> {
    let _lock = ProjectLock::acquire(dir)?;
    run_slide_render_locked(project, dir, source)
}

/// Run the slide_render stage for `project` living in `dir`. Caller must hold the
/// project lock. Drives the state machine (created/parsing/failed → parsing → parsed)
/// and persists project.json.
pub fn run_slide_render_locked(
    project: &mut Project,
    dir: &Path,
    source: &Path,
) -> Result<SlideRenderOutcome> {
    let resolved = resolve_source(dir, source)?;
    let inputs = collect_inputs(&resolved)?;

    let source_hash = source_hash(&inputs.files)?;
    let renderer = renderer_version(&inputs.kind)?;
    let fingerprint = fingerprint(&source_hash, &renderer);
    let tool_version = format!("slide-render/{}+{}", env!("CARGO_PKG_VERSION"), renderer);

    // Idempotent rerun (story 27: 重跑前先查已落盘产物): recorded success + same
    // fingerprint + every output still intact → reuse them untouched.
    if stage::verify(
        project,
        dir,
        &stage::VerifySpec {
            key: SLIDE_STAGE,
            input_fingerprint: &fingerprint,
            stage_deps: &[],
            content_deps: &[],
        },
    ) {
        let record = project
            .stages
            .get(SLIDE_STAGE)
            .expect("verified record exists");
        return Ok(SlideRenderOutcome {
            page_count: record.outputs.len(),
            slides: record.outputs.iter().map(|o| o.path.clone()).collect(),
            fingerprint,
            skipped: true,
            stage: project.stage,
        });
    }

    // Entry states: fresh run (created), crash residue mid-stage (parsing), re-run with
    // a changed deck (parsed), or retry after a failure of this stage (failed). Anything
    // else means the pipeline has moved past slide rendering.
    let retrying = matches!(project.stage, Stage::Failed)
        && project
            .failed
            .as_ref()
            .is_some_and(|f| matches!(f.stage, Stage::Parsing | Stage::Parsed));
    let runnable = matches!(
        project.stage,
        Stage::Created | Stage::Parsing | Stage::Parsed
    ) || retrying;
    if !runnable {
        return Err(Error::StageNotRunnable {
            runner: SLIDE_STAGE,
            stage: project.stage,
        });
    }
    if project.stage == Stage::Created || retrying {
        project.transition(Stage::Parsing)?;
        save_project(project, dir)?;
    }

    let rendered = render(dir, &inputs).and_then(|outcome| {
        let outputs = artifacts(dir, &outcome.slides)?;
        Ok((outcome, outputs))
    });
    let (outcome, outputs) = match rendered {
        Ok(pair) => pair,
        Err(err) => {
            let message = err.to_string();
            project.fail(message.clone())?;
            save_project(project, dir)?;
            return Err(err);
        }
    };

    if project.stage == Stage::Parsing {
        project.transition(Stage::Parsed)?;
    }
    // This stage has recovered from any earlier failure; don't keep showing it.
    project.failed = None;
    let record = stage::success(
        SLIDE_STAGE,
        fingerprint.clone(),
        tool_version,
        outputs,
        BTreeMap::new(),
    );
    stage::land(project, dir, SLIDE_STAGE, record)?;
    Ok(SlideRenderOutcome {
        stage: project.stage,
        fingerprint,
        ..outcome
    })
}

/// Resolve `source` to an existing path: as given (absolute or cwd-relative), then
/// relative to the project's `source/` (what project_status suggests), then the project
/// directory itself.
fn resolve_source(dir: &Path, source: &Path) -> Result<PathBuf> {
    if source.is_file() || source.is_dir() {
        return Ok(source.to_path_buf());
    }
    let in_source = dir.join("source").join(source);
    if in_source.is_file() || in_source.is_dir() {
        return Ok(in_source);
    }
    let in_root = dir.join(source);
    if in_root.is_file() || in_root.is_dir() {
        return Ok(in_root);
    }
    Err(Error::SourceNotFound(source.display().to_string()))
}

/// Classify the resolved source: one PDF, or a folder's numerically ordered images.
fn collect_inputs(resolved: &Path) -> Result<Inputs> {
    if resolved.is_dir() {
        return Ok(Inputs {
            kind: SourceKind::Images,
            files: list_image_files(resolved)?,
        });
    }
    let is_pdf = resolved
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Err(Error::UnsupportedSource {
            path: resolved.display().to_string(),
            message:
                "expected a .pdf file or a folder of images (pptx: please export to PDF first)"
                    .into(),
        });
    }
    Ok(Inputs {
        kind: SourceKind::Pdf,
        files: vec![resolved.to_path_buf()],
    })
}

/// Render the inputs into `<project_dir>/work/slides/`, returning the slide list.
fn render(dir: &Path, inputs: &Inputs) -> Result<SlideRenderOutcome> {
    let out_dir = dir.join(SLIDES_DIR);
    // Rebuild from scratch: a re-render with fewer pages must not leave stale slide_NNN
    // files behind for the timeline stage to pick up.
    if out_dir.exists() {
        fs::remove_dir_all(&out_dir)?;
    }
    fs::create_dir_all(&out_dir)?;
    let slides = match inputs.kind {
        SourceKind::Pdf => render_pdf(&inputs.files[0], &out_dir)?,
        SourceKind::Images => transcode_images(&inputs.files, &out_dir)?,
    };
    Ok(SlideRenderOutcome {
        page_count: slides.len(),
        slides,
        fingerprint: String::new(),
        skipped: false,
        stage: Stage::Parsing,
    })
}

/// Content hashes for the outputs just written, in page order.
fn artifacts(dir: &Path, slides: &[String]) -> Result<Vec<OutputArtifact>> {
    slides
        .iter()
        .map(|rel| stage::hash_artifact(dir, rel))
        .collect()
}

/// Combined input fingerprint (ticket 03): sha256 over source file hash + renderer
/// version + target width.
fn fingerprint(source_hash: &str, renderer: &str) -> String {
    let mut h = Sha256::new();
    h.update(source_hash.as_bytes());
    h.update(b"\n");
    h.update(renderer.as_bytes());
    h.update(b"\n");
    h.update(TARGET_WIDTH.to_string().as_bytes());
    format!("sha256:{}", hex(h.finalize()))
}

/// Hash of the ordered source files (name + content each) — the "源文件哈希" component.
fn source_hash(files: &[PathBuf]) -> Result<String> {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file_name_of(file).as_bytes());
        hasher.update([0u8]);
        io::copy(&mut fs::File::open(file)?, &mut hasher)?;
    }
    Ok(hex(hasher.finalize()))
}

/// Identity of whatever produces the pixels: the pdfium.dll binary (it has no runtime
/// version export, so its hash is the honest version), or the image transcoder.
fn renderer_version(kind: &SourceKind) -> Result<String> {
    Ok(match kind {
        SourceKind::Images => "image-transcode/1".into(),
        SourceKind::Pdf => {
            let dll = find_pdfium_dll()?;
            if !dll.is_file() {
                return Err(Error::PdfiumLoad {
                    path: dll.display().to_string(),
                    message: "pdfium.dll not found (set CC_PDFIUM_DLL or place it in \
                              vendor/pdfium/bin/)"
                        .into(),
                });
            }
            format!("pdfium/{}", &file_hash(&dll)?[..16])
        }
    })
}

fn file_hash(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    io::copy(&mut fs::File::open(path)?, &mut hasher)?;
    Ok(hex(hasher.finalize()))
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// List a folder's image files in numeric file-name order.
fn list_image_files(folder: &Path) -> Result<Vec<PathBuf>> {
    let images: Vec<PathBuf> = fs::read_dir(folder)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        })
        .collect();
    if images.is_empty() {
        return Err(Error::UnsupportedSource {
            path: folder.display().to_string(),
            message: format!(
                "folder contains no images (expected any of: {})",
                IMAGE_EXTENSIONS.join(", ")
            ),
        });
    }
    let mut images = images;
    images.sort_by(|a, b| natural_cmp(&file_name_of(a), &file_name_of(b)));
    Ok(images)
}

/// Image folder input: the renderer does not interpret the images, only orders them
/// (PRD §8.1) — decode + re-encode as slide_NNN.png in numeric file-name order.
fn transcode_images(images: &[PathBuf], out_dir: &Path) -> Result<Vec<String>> {
    let mut slides = Vec::with_capacity(images.len());
    for (i, path) in images.iter().enumerate() {
        let name = format!("slide_{:03}.png", i + 1);
        image::open(path)
            .map_err(|e| Error::SlideRender(format!("cannot decode {}: {e}", path.display())))?
            .save(out_dir.join(&name))
            .map_err(|e| Error::SlideRender(format!("cannot write slide image {name}: {e}")))?;
        slides.push(format!("{SLIDES_DIR}/{name}"));
    }
    Ok(slides)
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Numeric-aware ordering (PRD §8.1 ⚠️: 数字排序禁止字符串排序): runs of ASCII digits
/// compare by numeric value, everything else by characters — so `幻灯片10` sorts after
/// `幻灯片2`. Numerically equal names (`slide2` vs `slide02`) fall back to text order.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    #[derive(PartialEq)]
    enum Token<'s> {
        Num(u64),
        Text(&'s str),
    }
    fn tokens(s: &str) -> Vec<Token<'_>> {
        let mut out = Vec::new();
        let mut rest = s;
        while !rest.is_empty() {
            // Byte-wise scanning is UTF-8 safe: ASCII digits only occur as single-byte
            // chars, multi-byte sequences use continuation bytes >= 0x80.
            let is_digit = rest.as_bytes()[0].is_ascii_digit();
            let len = rest
                .as_bytes()
                .iter()
                .take_while(|b| b.is_ascii_digit() == is_digit)
                .count();
            let tok = if is_digit {
                // Overflowing numbers degrade to u64::MAX ordering, text tiebreak below.
                Token::Num(rest[..len].parse::<u64>().unwrap_or(u64::MAX))
            } else {
                Token::Text(&rest[..len])
            };
            out.push(tok);
            rest = &rest[len..];
        }
        out
    }
    let (ta, tb) = (tokens(a), tokens(b));
    for (x, y) in ta.iter().zip(tb.iter()) {
        let ord = match (x, y) {
            (Token::Num(x), Token::Num(y)) => x.cmp(y),
            (Token::Num(_), Token::Text(_)) => Ordering::Less,
            (Token::Text(_), Token::Num(_)) => Ordering::Greater,
            (Token::Text(x), Token::Text(y)) => x.cmp(y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    ta.len().cmp(&tb.len()).then_with(|| a.cmp(b))
}

fn render_pdf(pdf: &Path, out_dir: &Path) -> Result<Vec<String>> {
    let dll = find_pdfium_dll()?;
    let bindings = Pdfium::bind_to_library(&dll).map_err(|e| Error::PdfiumLoad {
        path: dll.display().to_string(),
        message: e.to_string(),
    })?;
    let pdfium = Pdfium::new(bindings);
    let document = pdfium
        .load_pdf_from_file(pdf, None)
        .map_err(|e| Error::SlideRender(format!("cannot load pdf `{}`: {e}", pdf.display())))?;
    let pages = document.pages();
    let count = pages.len();
    let config = PdfRenderConfig::new().set_target_width(TARGET_WIDTH as i32);
    let mut slides = Vec::with_capacity(count as usize);
    for i in 0..count {
        let page = pages
            .get(i)
            .map_err(|e| Error::SlideRender(format!("cannot open page {}: {e}", i + 1)))?;
        let image = page
            .render_with_config(&config)
            .map_err(|e| Error::SlideRender(format!("cannot render page {}: {e}", i + 1)))?
            .as_image();
        let name = format!("slide_{:03}.png", i + 1);
        image
            .save(out_dir.join(&name))
            .map_err(|e| Error::SlideRender(format!("cannot write slide image {name}: {e}")))?;
        slides.push(format!("{SLIDES_DIR}/{name}"));
    }
    Ok(slides)
}

/// Locate pdfium.dll: explicit `CC_PDFIUM_DLL` override, then next to the running exe
/// (how the shipped product bundles it), then `vendor/pdfium/bin/pdfium.dll` walking up
/// from the cwd (repo checkouts), then the OS loader search path.
fn find_pdfium_dll() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("CC_PDFIUM_DLL") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            let candidate = exe_dir.join("pdfium.dll");
            if candidate.is_file() {
                return Ok(candidate);
            }
            let candidate = exe_dir
                .join("vendor")
                .join("pdfium")
                .join("bin")
                .join("pdfium.dll");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    let mut dir = std::env::current_dir()?;
    loop {
        let candidate = dir
            .join("vendor")
            .join("pdfium")
            .join("bin")
            .join("pdfium.dll");
        if candidate.is_file() {
            return Ok(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    Ok(PathBuf::from("pdfium.dll"))
}
