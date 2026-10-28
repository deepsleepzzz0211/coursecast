//! Ticket 11 (script_set / script_import_notes core seam): partial script.md updates
//! keyed by page number and PPT notes import. The MCP shell is a thin wrapper over
//! these, exactly like every other stage runner.

use cc_core::script;
use std::fs;
use std::path::Path;

fn project_dir_with_slides(tmp: &Path, pages: u32) -> std::path::PathBuf {
    let dir = tmp.join("proj");
    fs::create_dir_all(dir.join("work/slides")).unwrap();
    for p in 1..=pages {
        fs::write(dir.join(format!("work/slides/slide_{p:03}.png")), b"png").unwrap();
    }
    // minimal project.json so slide_page_count has a record to read
    let created = cc_core::create_project(tmp, "脚本编辑", None).unwrap();
    let mut project = created.project;
    let record_dir = created.dir;
    project.stages.insert(
        "slide_render".to_string(),
        cc_core::StageRecord {
            stage: "slide_render".into(),
            state: "success".into(),
            input_fingerprint: "sha256:x".into(),
            tool_version: "test".into(),
            outputs: (1..=pages)
                .map(|p| cc_core::OutputArtifact {
                    path: format!("work/slides/slide_{p:03}.png"),
                    sha256: format!("sha256:{p}"),
                    bytes: 3,
                })
                .collect(),
            depends_on: Default::default(),
        },
    );
    cc_core::save_project(&project, &record_dir).unwrap();
    record_dir
}

#[test]
fn slide_page_count_reads_the_slide_record() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir_with_slides(tmp.path(), 3);
    let (project, _) =
        cc_core::load_project(tmp.path(), &dir.file_name().unwrap().to_string_lossy()).unwrap();
    assert_eq!(cc_core::slide::slide_page_count(&project).unwrap(), 3);
}

#[test]
fn updating_pages_writes_them_and_keeps_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir_with_slides(tmp.path(), 3);

    // first write: pages 1 and 3 given, page 2 stays empty
    let pages = script::update_script_pages(&dir, &[(1, "第一页".into()), (3, "第三页".into())], 3)
        .unwrap();
    assert_eq!(
        pages,
        vec!["第一页".to_string(), String::new(), "第三页".to_string()]
    );
    assert_eq!(
        script::read_script_pages(&dir).unwrap(),
        vec!["第一页".to_string(), String::new(), "第三页".to_string()]
    );

    // partial update: only page 2 changes, 1 and 3 untouched
    let pages = script::update_script_pages(&dir, &[(2, "新增的第二页".into())], 3).unwrap();
    assert_eq!(pages[0], "第一页");
    assert_eq!(pages[1], "新增的第二页");
    assert_eq!(pages[2], "第三页");
}

#[test]
fn later_updates_to_the_same_page_win() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir_with_slides(tmp.path(), 1);
    let pages = script::update_script_pages(&dir, &[(1, "第一次".into()), (1, "第二次".into())], 1)
        .unwrap();
    assert_eq!(pages, vec!["第二次".to_string()]);
}

#[test]
fn page_zero_and_beyond_page_count_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir_with_slides(tmp.path(), 2);
    let err = script::update_script_pages(&dir, &[(0, "无".into())], 2).unwrap_err();
    assert_eq!(err.code(), "page_out_of_range");
    let err = script::update_script_pages(&dir, &[(3, "越界".into())], 2).unwrap_err();
    assert_eq!(err.code(), "page_out_of_range");
}

/// Build a minimal pptx (a zip with slide + notesSlide parts) the way PowerPoint
/// lays them out.
fn build_pptx(path: &Path, notes: &[Option<&str>]) {
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (i, note) in notes.iter().enumerate() {
        let n = i + 1;
        let slide =
            r#"<?xml version="1.0"?><p:sld xmlns:a="a"><p:cSld><p:spTree/></p:cSld></p:sld>"#;
        zip.start_file(
            format!("ppt/slides/slide{n}.xml"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, slide.as_bytes()).unwrap();
        let notes_xml = match note {
            Some(text) => format!(
                r#"<?xml version="1.0"?><p:notes xmlns:a="a"><p:txBody>{}</p:txBody></p:notes>"#,
                text
            ),
            None => r#"<?xml version="1.0"?><p:notes xmlns:a="a"><p:txBody></p:txBody></p:notes>"#
                .to_string(),
        };
        zip.start_file(
            format!("ppt/notesSlides/notesSlide{n}.xml"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, notes_xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn notes_body(paragraphs: &[&str]) -> String {
    paragraphs
        .iter()
        .map(|p| format!(r#"<a:p><a:r><a:t>{p}</a:t></a:r></a:p>"#))
        .collect()
}

#[test]
fn import_pptx_notes_extracts_per_page_paragraph_text() {
    let tmp = tempfile::tempdir().unwrap();
    let pptx = tmp.path().join("deck.pptx");
    let page2 = notes_body(&["第二页第一段", "第二页第二段"]);
    build_pptx(&pptx, &[Some(&notes_body(&["第一页备注"])), Some(&page2)]);

    let pages = script::import_pptx_notes(&pptx).unwrap();
    assert_eq!(
        pages,
        vec![
            "第一页备注".to_string(),
            "第二页第一段\n第二页第二段".to_string()
        ]
    );
}

#[test]
fn import_pptx_notes_escapes_entities_and_leaves_bare_pages_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let pptx = tmp.path().join("deck.pptx");
    build_pptx(
        &pptx,
        &[Some(&notes_body(&["A &amp; B &lt;课件&gt;"])), None],
    );

    let pages = script::import_pptx_notes(&pptx).unwrap();
    assert_eq!(pages, vec!["A & B <课件>".to_string(), String::new()]);
}

#[test]
fn import_pptx_notes_rejects_missing_files_and_non_pptx_archives() {
    let tmp = tempfile::tempdir().unwrap();
    let err = script::import_pptx_notes(&tmp.path().join("nope.pptx")).unwrap_err();
    assert_eq!(err.code(), "source_not_found");

    let not_pptx = tmp.path().join("plain.txt");
    fs::write(&not_pptx, "hello").unwrap();
    let err = script::import_pptx_notes(&not_pptx).unwrap_err();
    assert_eq!(err.code(), "source_unsupported");
}

#[test]
fn a_trailing_blank_page_is_preserved_and_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir_with_slides(tmp.path(), 3);
    let pages = vec!["第一页".to_string(), "第二页".to_string(), String::new()];
    script::write_script_pages(&dir, &pages).unwrap();
    assert_eq!(
        script::read_script_pages(&dir).unwrap(),
        pages,
        "last page without a script must survive as a 5 s silent page"
    );
    // an empty update list must not collapse the trailing blank page
    assert_eq!(script::update_script_pages(&dir, &[], 3).unwrap(), pages);
}

#[test]
fn a_single_blank_page_is_not_an_empty_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir_with_slides(tmp.path(), 1);
    script::write_script_pages(&dir, &[String::new()]).unwrap();
    assert_eq!(
        script::read_script_pages(&dir).unwrap(),
        vec![String::new()]
    );

    // a literally empty file still means "no script written yet"
    fs::write(dir.join("script.md"), "").unwrap();
    assert!(script::read_script_pages(&dir).unwrap().is_empty());
}
