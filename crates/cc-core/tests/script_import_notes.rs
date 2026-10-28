//! Ticket 12 additions on top of ticket 11's script seam: the two import behaviors a
//! real PowerPoint deck forced out — page order follows presentation.xml's sldIdLst
//! (file-name numbering is NOT page order after a move), and slide-number `<a:fld>`
//! placeholders stay out of the notes. The baseline import behaviors (paragraph
/// text, entities, bare pages, error codes) are pinned in script_edit.rs.
use cc_core::script::import_pptx_notes;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::tempdir;

/// Build a pptx whose sldIdLst order differs from file-name numbering: `order`
/// lists slide numbers in presentation order, `notes` maps each slide to its note.
fn build_pptx(path: &Path, order: &[u32], notes: &[(u32, &str)]) {
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opt: zip::write::SimpleFileOptions = Default::default();

    // rId numbering is deliberately unrelated to slide numbering
    let mut pres_rels = String::from("<?xml version=\"1.0\"?>\n<Relationships>\n");
    let mut sld_ids = String::new();
    for (i, slide) in order.iter().enumerate() {
        let rid = format!("rId{}", i + 2);
        pres_rels.push_str(&format!(
            "  <Relationship Id=\"{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{slide}.xml\"/>\n"
        ));
        sld_ids.push_str(&format!("  <p:sldId id=\"{}\" r:id=\"{rid}\"/>\n", 256 + i));
    }
    pres_rels.push_str("</Relationships>\n");

    zip.start_file("ppt/presentation.xml", opt).unwrap();
    zip.write_all(
        format!(
            "<?xml version=\"1.0\"?>\n<p:presentation xmlns:p=\"urn:p\" xmlns:r=\"urn:r\"><p:sldIdLst>\n{sld_ids}</p:sldIdLst></p:presentation>\n"
        )
        .as_bytes(),
    )
    .unwrap();
    zip.start_file("ppt/_rels/presentation.xml.rels", opt)
        .unwrap();
    zip.write_all(pres_rels.as_bytes()).unwrap();

    for slide in order {
        zip.start_file(format!("ppt/slides/slide{slide}.xml"), opt)
            .unwrap();
        zip.write_all(b"<p:sld/>").unwrap();
        zip.start_file(format!("ppt/slides/_rels/slide{slide}.xml.rels"), opt)
            .unwrap();
        zip.write_all("<?xml version=\"1.0\"?>\n<Relationships/>\n".as_bytes())
            .unwrap();
        if let Some((_, note)) = notes.iter().find(|(s, _)| s == slide) {
            zip.start_file(format!("ppt/notesSlides/notesSlide{slide}.xml"), opt)
                .unwrap();
            zip.write_all(
                format!(
                    "<?xml version=\"1.0\"?>\n<p:notes xmlns:a=\"urn:a\"><p:txBody><a:fld type=\"slidenum\"><a:t>77</a:t></a:fld><a:p><a:r><a:t>{note}</a:t></a:r></a:p></p:txBody></p:notes>\n"
                )
                .as_bytes(),
            )
            .unwrap();
        }
    }
    zip.finish().unwrap();
}

#[test]
fn notes_follow_presentation_order_not_file_names() {
    let dir = tempdir().unwrap();
    let pptx = dir.path().join("deck.pptx");
    // sldIdLst puts slide2 first: its notes must come back as page 1
    build_pptx(
        &pptx,
        &[2, 1],
        &[(1, "第一张幻灯片的备注"), (2, "第二张幻灯片的备注")],
    );

    let pages = import_pptx_notes(&pptx).unwrap();

    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0], "第二张幻灯片的备注");
    assert_eq!(pages[1], "第一张幻灯片的备注");
}

#[test]
fn slide_number_field_text_is_skipped() {
    let dir = tempdir().unwrap();
    let pptx = dir.path().join("deck.pptx");
    // build_pptx injects an <a:fld> with text 77: it must not leak into the note
    build_pptx(&pptx, &[1], &[(1, "讲稿 A &amp; B")]);

    let pages = import_pptx_notes(&pptx).unwrap();

    assert_eq!(pages, vec!["讲稿 A & B".to_string()]);
}

#[test]
fn pages_without_a_notes_part_import_as_empty() {
    let dir = tempdir().unwrap();
    let pptx = dir.path().join("deck.pptx");
    build_pptx(&pptx, &[1, 2], &[(2, "只有第二页有备注")]);

    let pages = import_pptx_notes(&pptx).unwrap();

    assert_eq!(pages, vec![String::new(), "只有第二页有备注".to_string()]);
}

/// Real-machine check against a genuine PowerPoint file (its XML layout is what
/// the minimal zips approximate). Run with:
/// `CC_TEST_PPTX=<path> cargo test -p cc-core --test script_import_notes -- --ignored`
#[test]
#[ignore = "set CC_TEST_PPTX to a real pptx exported by PowerPoint"]
fn real_powerpoint_notes_roundtrip() {
    let path = std::env::var("CC_TEST_PPTX").expect("CC_TEST_PPTX must point at a real pptx");
    let pages = import_pptx_notes(Path::new(&path)).unwrap();
    assert!(!pages.is_empty(), "a real deck has at least one slide");
    for (i, note) in pages.iter().enumerate() {
        println!("page {}: {:?}", i + 1, note);
    }
}
