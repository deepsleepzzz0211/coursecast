//! script.md reading and writing: the human/AI-editable per-page script (spec story
//! 8/30, 文件即真相). Pages are separated by `---` divider lines; without one, the
//! whole file is page 1.

use crate::error::{Error, Result};
use std::fs;
use std::io::Read;
use std::path::Path;

pub const SCRIPT_FILE: &str = "script.md";

/// Notes XML is tiny in real decks; these caps stop a malicious/corrupt zip from
/// exhausting memory (the MCP tool accepts a caller-supplied pptx path).
const MAX_NOTES_XML_BYTES: u64 = 2 * 1024 * 1024;
const MAX_NOTES_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_NOTES_PARTS: u32 = 5000;

/// Read script.md and split it into per-page texts (1:1 with slide page numbers).
pub fn read_script_pages(project_dir: &Path) -> Result<Vec<String>> {
    let path = project_dir.join(SCRIPT_FILE);
    if !path.is_file() {
        return Err(Error::ScriptNotFound {
            path: path.display().to_string(),
        });
    }
    let content = fs::read_to_string(&path)?;
    if content.is_empty() {
        return Ok(Vec::new());
    }
    let mut raw: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in content.lines() {
        if line.trim() == "---" {
            raw.push(std::mem::take(&mut current));
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    raw.push(current);
    // A blank page is legitimate anywhere, including the last one (spec story 12:
    // 没写讲稿的页按 5 秒静默). Dropping trailing blanks made a last-page-without-
    // script silently shorten the deck; only a literally empty file means "no script".
    Ok(raw.into_iter().map(|p| p.trim().to_string()).collect())
}

/// Merge `updates` (1-based page number → text) into script.md and write it back
/// atomically (spec story 8: 只改某几页). Pages not mentioned keep their current text;
/// a missing script.md starts from `page_count` empty pages. Later updates to the same
/// page win. Returns the resulting page texts.
pub fn update_script_pages(
    project_dir: &Path,
    updates: &[(u32, String)],
    page_count: u32,
) -> Result<Vec<String>> {
    for (page, _) in updates {
        if *page == 0 || *page > page_count {
            return Err(Error::PageOutOfRange {
                page: *page,
                pages: page_count,
            });
        }
    }
    // current state: existing script, padded (never truncated) to page_count
    let mut pages = match read_script_pages(project_dir) {
        Ok(pages) => pages,
        Err(Error::ScriptNotFound { .. }) => Vec::new(),
        Err(err) => return Err(err),
    };
    while (pages.len() as u32) < page_count {
        pages.push(String::new());
    }
    for (page, text) in updates {
        pages[(*page - 1) as usize] = text.trim().to_string();
    }
    write_script_pages(project_dir, &pages)?;
    Ok(pages)
}

/// Write script.md atomically: pages joined by the `---` divider `read_script_pages`
/// splits on (tmp → rename, same convention as project.json).
pub fn write_script_pages(project_dir: &Path, pages: &[String]) -> Result<()> {
    let path = project_dir.join(SCRIPT_FILE);
    let tmp = project_dir.join("script.md.tmp");
    let text = pages.join("\n\n---\n\n");
    // one blank page would serialize to "" = indistinguishable from "no script";
    // a bare newline keeps the single-page structure without changing the text
    fs::write(&tmp, if text.is_empty() { "\n" } else { &text })?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

/// Extract per-page speaker notes from a pptx (spec story 10: 从 PPT 备注一键导入).
/// A pptx is a zip of slide parts; notes live in `ppt/notesSlides/notesSlideN.xml`,
/// text inside `<a:t>` runs grouped into `<a:p>` paragraphs. Pages without a notes
/// part come back as empty strings, 1:1 with the slides.
///
/// Page order follows `ppt/presentation.xml`'s `sldIdLst` when present (ticket 12,
/// verified against a real PowerPoint deck with reordered slides: file-name numbering
/// is NOT page order after a move/delete); archives without presentation.xml fall
/// back to file-name numbering.
pub fn import_pptx_notes(pptx: &Path) -> Result<Vec<String>> {
    if !pptx.is_file() {
        return Err(Error::SourceNotFound(pptx.display().to_string()));
    }
    let file = fs::File::open(pptx)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| Error::UnsupportedSource {
        path: pptx.display().to_string(),
        message: format!("not a readable pptx (zip) archive: {e}"),
    })?;

    // page count = highest slideN.xml
    let mut slide_numbers: Vec<u32> = Vec::new();
    for i in 0..archive.len() {
        if let Some(Ok(name)) = archive.name_for_index(i) {
            if let Some(n) = slide_index_of(&name, "ppt/slides/slide") {
                slide_numbers.push(n);
            }
        }
    }
    if slide_numbers.is_empty() {
        return Err(Error::UnsupportedSource {
            path: pptx.display().to_string(),
            message: "no ppt/slides/slideN.xml parts: not a presentation".to_string(),
        });
    }

    // presentation order wins when the archive carries it
    let ordered = match presentation_slide_order(&mut archive) {
        Some(order) if !order.is_empty() => order,
        _ => {
            let mut sorted = slide_numbers;
            sorted.sort_unstable();
            sorted
        }
    };
    let index_of_slide: std::collections::HashMap<u32, usize> =
        ordered.iter().enumerate().map(|(i, n)| (*n, i)).collect();

    let mut pages = vec![String::new(); ordered.len()];
    let mut total_bytes = 0u64;
    let mut parts = 0u32;
    for i in 0..archive.len() {
        let name = match archive.name_for_index(i) {
            Some(Ok(name)) => name,
            _ => continue,
        };
        let Some(n) = slide_index_of(&name, "ppt/notesSlides/notesSlide") else {
            continue;
        };
        let Some(&page) = index_of_slide.get(&n) else {
            continue;
        };
        let name = name.into_owned();
        parts += 1;
        if parts > MAX_NOTES_PARTS {
            return Err(Error::UnsupportedSource {
                path: pptx.display().to_string(),
                message: format!("too many notes parts (> {MAX_NOTES_PARTS})"),
            });
        }
        let mut xml = archive.by_index(i).map_err(|e| Error::UnsupportedSource {
            path: pptx.display().to_string(),
            message: format!("cannot read {name}: {e}"),
        })?;
        // read through a hard cap so a zip bomb cannot allocate unbounded memory
        let mut text = String::new();
        std::io::Read::take(&mut xml, MAX_NOTES_XML_BYTES + 1).read_to_string(&mut text)?;
        if text.len() as u64 > MAX_NOTES_XML_BYTES {
            return Err(Error::UnsupportedSource {
                path: pptx.display().to_string(),
                message: format!("{name} exceeds {MAX_NOTES_XML_BYTES} bytes"),
            });
        }
        total_bytes += text.len() as u64;
        if total_bytes > MAX_NOTES_TOTAL_BYTES {
            return Err(Error::UnsupportedSource {
                path: pptx.display().to_string(),
                message: format!("notes payload exceeds {MAX_NOTES_TOTAL_BYTES} bytes"),
            });
        }
        pages[page] = extract_notes_text(&text);
    }
    Ok(pages)
}

/// Slide numbers in presentation order: `presentation.xml`'s sldIdLst (its `r:id`
/// sequence) resolved through `ppt/_rels/presentation.xml.rels` to `slides/slideN.xml`.
/// None when the archive has no such parts (minimal/hand-built zips).
fn presentation_slide_order<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Option<Vec<u32>> {
    let mut xml = String::new();
    if let Ok(mut pres) = archive.by_name("ppt/presentation.xml") {
        if std::io::Read::read_to_string(&mut pres, &mut xml).is_err() {
            return None;
        }
    } else {
        return None;
    }
    let rids = sldid_rids(&xml)?;

    let mut rels_xml = String::new();
    if let Ok(mut rels_file) = archive.by_name("ppt/_rels/presentation.xml.rels") {
        if std::io::Read::read_to_string(&mut rels_file, &mut rels_xml).is_err() {
            return None;
        }
    } else {
        return None;
    }
    let map = relationships(&rels_xml);

    let mut out = Vec::with_capacity(rids.len());
    for rid in rids {
        let target = map.get(&rid)?;
        let n = target
            .strip_prefix("slides/slide")?
            .strip_suffix(".xml")?
            .parse()
            .ok()?;
        out.push(n);
    }
    Some(out)
}

/// The `r:id` attribute values of `p:sldId` entries, in sldIdLst order.
fn sldid_rids(xml: &str) -> Option<Vec<String>> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut rids = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == "sldId" {
                    for attr in e.attributes().with_checks(false) {
                        let attr = attr.ok()?;
                        if attr.key.as_ref() == "r:id" {
                            let value = attr
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .ok()?;
                            rids.push(value.into_owned());
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    if rids.is_empty() {
        None
    } else {
        Some(rids)
    }
}

/// Relationship Id → Target map of a `.rels` part.
fn relationships(xml: &str) -> std::collections::HashMap<String, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut map = std::collections::HashMap::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == "Relationship" {
                    let mut id = None;
                    let mut target = None;
                    for attr in e.attributes().with_checks(false) {
                        let Ok(attr) = attr else { continue };
                        match attr.key.as_ref() {
                            "Id" => {
                                id = attr
                                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                    .ok()
                                    .map(|v| v.into_owned())
                            }
                            "Target" => {
                                target = attr
                                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                    .ok()
                                    .map(|v| v.into_owned())
                            }
                            _ => {}
                        }
                    }
                    if let (Some(id), Some(target)) = (id, target) {
                        map.insert(id, target);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    map
}

/// `prefix` + digits + `.xml` → the digits (e.g. `ppt/slides/slide12.xml` → 12).
fn slide_index_of(name: &str, prefix: &str) -> Option<u32> {
    name.strip_prefix(prefix)?
        .strip_suffix(".xml")?
        .parse()
        .ok()
}

/// Pull the visible text out of a notesSlide XML: `<a:p>` paragraphs joined by
/// newlines, `<a:t>` runs concatenated. quick-xml events keep entity escaping
/// (`&amp;`) correct without pulling in a DOM. Slide-number placeholders
/// (`<a:fld type="slidenum">…</a:fld>`) are skipped — they are chrome, not notes.
fn extract_notes_text(xml: &str) -> String {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    // no trim: whitespace between runs is meaningful text; paragraph/page text is
    // trimmed at the end
    reader.config_mut().trim_text(false);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_run_text = false; // inside <a:t>…</a:t>
    let mut fld_depth = 0usize; // inside <a:fld>…</a:fld>
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => match tag.local_name().as_ref() {
                "fld" => fld_depth += 1,
                "t" if fld_depth == 0 => in_run_text = true,
                // a:p (paragraph) flushes the runs accumulated so far
                "p" => {
                    if !current.is_empty() {
                        paragraphs.push(std::mem::take(&mut current));
                    }
                }
                _ => {}
            },
            Ok(Event::End(tag)) => match tag.local_name().as_ref() {
                "t" => in_run_text = false,
                "fld" => fld_depth = fld_depth.saturating_sub(1),
                _ => {}
            },
            Ok(Event::Text(text)) if in_run_text => {
                if let Ok(unescaped) = quick_xml::escape::unescape(&text) {
                    current.push_str(&unescaped);
                }
            }
            // quick-xml 0.42 emits entity references as their own events (`amp`, `#60`…)
            Ok(Event::GeneralRef(reference)) if in_run_text => {
                let name: &str = &reference;
                if let Ok(unescaped) = quick_xml::escape::unescape(&format!("&{name};")) {
                    current.push_str(&unescaped);
                }
            }
            Ok(Event::CData(text)) if in_run_text => current.push_str(&text),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    let mut out: Vec<&str> = Vec::new();
    for paragraph in &paragraphs {
        let trimmed = paragraph.trim();
        if !trimmed.is_empty() {
            out.push(trimmed);
        }
    }
    out.join("\n")
}
