//! Seam: the Tauri command layer — same JSON shape as the matching `cc-cli --json`
//! subcommands, same cc-core path (ticket 12: UI 走的路和 CLI/AI 是同一条). Broken
//! pptx → "请先另存为 PDF" prompt data; script pages keep empty entries (空页标黄,
//! 5s 静默) so the flow never blocks; everything re-reads from disk, which is what
//! "重启应用进度不丢" needs.

use cc_app::commands::{
    create_project_impl, error_json, list_projects_impl, project_status_impl, script_balance_impl,
    script_get_impl, script_import_notes_impl, script_set_impl, script_split_impl,
    slide_render_impl, slides_list_impl,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn fixture_pdf() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deck2.pdf")
}

fn new_project(data_dir: &Path) -> (String, PathBuf) {
    let v = create_project_impl(data_dir, "测试课程", Some("1920x1080")).unwrap();
    let id = v["project_id"].as_str().unwrap().to_string();
    let dir = data_dir.join("projects").join(&id);
    (id, dir)
}

#[test]
fn create_project_returns_cli_shaped_json() {
    let tmp = tempdir().unwrap();
    let v = create_project_impl(tmp.path(), "药品包装机械", None).unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["name"], "药品包装机械");
    assert_eq!(v["stage"], "created");
    assert_eq!(v["resolution"], "1920x1080");
    assert!(v["project_id"].as_str().unwrap().len() >= 8);
    assert!(v["dir"].as_str().unwrap().contains("projects"));
}

#[test]
fn invalid_create_input_maps_to_structured_error_json() {
    let tmp = tempdir().unwrap();
    let err = create_project_impl(tmp.path(), "   ", None).unwrap_err();
    let v = error_json(&err);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "invalid_name");
}

#[test]
fn list_projects_survives_a_restart_and_reports_progress() {
    let tmp = tempdir().unwrap();
    let (id, dir) = new_project(tmp.path());
    // fresh "session": everything must come back from disk, nothing in memory
    let listed = list_projects_impl(tmp.path()).unwrap();
    assert_eq!(listed["projects"].as_array().unwrap().len(), 1);
    let entry = &listed["projects"][0];
    assert_eq!(entry["project_id"], id.as_str());
    assert_eq!(entry["stage"], "created");
    assert_eq!(entry["progress"]["slides"], false);
    assert_eq!(entry["progress"]["script"], false);
    assert_eq!(entry["progress"]["person"], false);
    assert_eq!(entry["progress"]["film"], false);

    // progress flags move once slides/script exist (still all from disk)
    let v = slide_render_impl(tmp.path(), &id, fixture_pdf().to_str().unwrap()).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["page_count"], 2);
    script_set_impl(tmp.path(), &id, &["a".into(), "b".into()]).unwrap();

    let listed = list_projects_impl(tmp.path()).unwrap();
    let entry = &listed["projects"][0];
    assert_eq!(entry["stage"], "parsed");
    assert_eq!(entry["progress"]["slides"], true);
    assert_eq!(entry["progress"]["script"], true);
    assert!(dir.join("work/slides/slide_001.png").is_file());

    // person/film are the resume checkpoints for Steps 5/6 (files are the truth)
    fs::create_dir_all(dir.join("source")).unwrap();
    fs::write(dir.join("source/person.mp4"), b"person").unwrap();
    fs::create_dir_all(dir.join("output")).unwrap();
    fs::write(dir.join("output/final.mp4"), b"film").unwrap();
    let listed = list_projects_impl(tmp.path()).unwrap();
    let entry = &listed["projects"][0];
    assert_eq!(entry["progress"]["person"], true);
    assert_eq!(
        entry["progress"]["film"], false,
        "a partial final.mp4 must not advertise a finished film"
    );

    // only done + the landed file is a finished film (same predicate as final_video_impl)
    let (mut project, pdir) = cc_core::load_project(tmp.path(), &id).unwrap();
    project.stage = cc_core::Stage::Done;
    cc_core::save_project(&project, &pdir).unwrap();
    let listed = list_projects_impl(tmp.path()).unwrap();
    assert_eq!(listed["projects"][0]["progress"]["film"], true);
}

#[test]
fn slide_render_copies_the_deck_into_the_project() {
    let tmp = tempdir().unwrap();
    let (id, dir) = new_project(tmp.path());

    let v = slide_render_impl(tmp.path(), &id, fixture_pdf().to_str().unwrap()).unwrap();

    assert_eq!(v["page_count"], 2);
    assert_eq!(v["stage"], "parsed");
    let slides = v["slides"].as_array().unwrap();
    assert_eq!(slides[0], "work/slides/slide_001.png");
    // the deck lives inside the project (文件即真相): re-renders survive app restarts
    assert!(dir.join("source/deck.pdf").is_file());
    // absolute slide dir for the frontend asset urls
    assert!(PathBuf::from(v["slides_dir"].as_str().unwrap()).is_dir());
}

#[test]
fn broken_pptx_returns_the_save_as_pdf_prompt() {
    let tmp = tempdir().unwrap();
    let (id, _dir) = new_project(tmp.path());
    let broken = tmp.path().join("deck.pptx");
    fs::write(&broken, b"not a real pptx, conversion must fail").unwrap();

    let err = slide_render_impl(tmp.path(), &id, broken.to_str().unwrap()).unwrap_err();
    let v = error_json(&err);

    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "source_unsupported");
    let message = v["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("另存为 PDF"),
        "message should tell the user to save as PDF: {message}"
    );
}

#[test]
fn script_set_keeps_empty_pages_and_get_reads_them_back() {
    let tmp = tempdir().unwrap();
    let (id, _dir) = new_project(tmp.path());
    slide_render_impl(tmp.path(), &id, fixture_pdf().to_str().unwrap()).unwrap();

    let set = script_set_impl(tmp.path(), &id, &["第一页讲稿".into(), String::new()]).unwrap();
    assert_eq!(set["page_count"], 2);

    let got = script_get_impl(tmp.path(), &id).unwrap();
    assert_eq!(got["exists"], true);
    // an empty page is a real page (5 s silence) wherever it sits, including last
    assert_eq!(got["pages"].as_array().unwrap().len(), 2);
    assert_eq!(got["pages"][1], "");

    let missing = script_get_impl(tmp.path(), &new_project(tmp.path()).0).unwrap();
    assert_eq!(missing["exists"], false);
    assert_eq!(missing["pages"].as_array().unwrap().len(), 0);
}

#[test]
fn script_import_notes_reads_the_projects_deck_pptx() {
    let tmp = tempdir().unwrap();
    let (id, dir) = new_project(tmp.path());
    // minimal pptx with one noted slide (zip layout as in cc-core's tests)
    let pptx = dir.join("source/deck.pptx");
    write_noted_pptx(&pptx, &[Some("备注第一页"), None]);

    let v = script_import_notes_impl(tmp.path(), &id).unwrap();
    let pages = v["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0], "备注第一页");
    assert_eq!(pages[1], "");
}

#[test]
fn script_import_without_pptx_is_a_structured_error() {
    let tmp = tempdir().unwrap();
    let (id, _dir) = new_project(tmp.path());
    let err = script_import_notes_impl(tmp.path(), &id).unwrap_err();
    let v = error_json(&err);
    assert_eq!(v["error"]["code"], "invalid_argument");
    assert!(v["error"]["message"].as_str().unwrap().contains("pptx"));
}

#[test]
fn slides_list_returns_absolute_paths_after_render() {
    let tmp = tempdir().unwrap();
    let (id, _dir) = new_project(tmp.path());

    let empty = slides_list_impl(tmp.path(), &id).unwrap();
    assert_eq!(empty["count"], 0);

    slide_render_impl(tmp.path(), &id, fixture_pdf().to_str().unwrap()).unwrap();
    let v = slides_list_impl(tmp.path(), &id).unwrap();
    assert_eq!(v["count"], 2);
    for f in v["files"].as_array().unwrap() {
        assert!(
            PathBuf::from(f.as_str().unwrap()).is_file(),
            "missing slide file {f}"
        );
    }
}

#[test]
fn split_and_balance_commands_return_json_shapes() {
    let split = script_split_impl("a\n---\nb").unwrap();
    assert_eq!(split["segments"], serde_json::json!(["a", "b"]));

    let balanced = script_balance_impl(&["s1".into(), "s2".into(), "s3".into()], 2).unwrap();
    // proportional floor buckets: 3 segments onto 2 pages -> [0..1), [1..3)
    assert_eq!(balanced["pages"], serde_json::json!(["s1", "s2\ns3"]));
}

/// Real-machine check of the PowerPoint COM conversion path (needs Office + a real
/// pptx). Run with:
/// `CC_TEST_PPTX=<path> cargo test -p cc-app --test commands -- --ignored`
#[test]
#[ignore = "set CC_TEST_PPTX to a real pptx on a machine with PowerPoint"]
fn real_pptx_converts_and_renders() {
    let pptx = std::env::var("CC_TEST_PPTX").expect("CC_TEST_PPTX must point at a real pptx");
    let tmp = tempdir().unwrap();
    let (id, dir) = new_project(tmp.path());

    let v = slide_render_impl(tmp.path(), &id, &pptx).unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["page_count"], 3);
    assert!(
        dir.join("source/deck.pptx").is_file(),
        "original pptx kept for notes import"
    );
    assert!(
        dir.join("source/deck.pdf").is_file(),
        "converted pdf kept as the render source"
    );
    assert!(dir.join("work/slides/slide_003.png").is_file());
}

/// Minimal pptx zip: presentation.xml sldIdLst in order, notes on the noted slides.
fn write_noted_pptx(path: &Path, notes: &[Option<&str>]) {
    use std::io::Write;
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let opt: zip::write::SimpleFileOptions = Default::default();
    let mut rels = String::from("<?xml version=\"1.0\"?>\n<Relationships>\n");
    let mut ids = String::new();
    for i in 0..notes.len() {
        rels.push_str(&format!(
            "  <Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{}.xml\"/>\n",
            i + 2,
            i + 1
        ));
        ids.push_str(&format!(
            "  <p:sldId id=\"{}\" r:id=\"rId{}\"/>\n",
            256 + i,
            i + 2
        ));
    }
    rels.push_str("</Relationships>\n");
    zip.start_file("ppt/presentation.xml", opt).unwrap();
    zip.write_all(format!("<?xml version=\"1.0\"?>\n<p:presentation xmlns:p=\"urn:p\" xmlns:r=\"urn:r\"><p:sldIdLst>\n{ids}</p:sldIdLst></p:presentation>\n").as_bytes()).unwrap();
    zip.start_file("ppt/_rels/presentation.xml.rels", opt)
        .unwrap();
    zip.write_all(rels.as_bytes()).unwrap();
    for (i, note) in notes.iter().enumerate() {
        let slide = format!("slide{}", i + 1);
        let some = i + 1;
        zip.start_file(format!("ppt/slides/{slide}.xml"), opt)
            .unwrap();
        zip.write_all(b"<p:sld/>").unwrap();
        if let Some(text) = note {
            zip.start_file(format!("ppt/slides/_rels/{slide}.xml.rels"), opt)
                .unwrap();
            zip.write_all(format!("<?xml version=\"1.0\"?>\n<Relationships>\n  <Relationship Id=\"rId9\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide\" Target=\"../notesSlides/notesSlide{some}.xml\"/>\n</Relationships>\n").as_bytes()).unwrap();
            zip.start_file(format!("ppt/notesSlides/notesSlide{some}.xml"), opt)
                .unwrap();
            zip.write_all(format!("<?xml version=\"1.0\"?>\n<p:notes xmlns:a=\"urn:a\"><p:txBody><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:notes>\n").as_bytes()).unwrap();
        } else {
            zip.start_file(format!("ppt/slides/_rels/{slide}.xml.rels"), opt)
                .unwrap();
            zip.write_all("<?xml version=\"1.0\"?>\n<Relationships/>\n".as_bytes())
                .unwrap();
        }
    }
    zip.finish().unwrap();
}

#[test]
fn project_status_inputs_match_the_cli_shape() {
    let tmp = tempdir().unwrap();
    let created = create_project_impl(tmp.path(), "状态", None).unwrap();
    let id = created["project_id"].as_str().unwrap();
    let v = project_status_impl(tmp.path(), id).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["name"], "状态");
    for key in ["deck", "script", "voice_ref", "person"] {
        assert!(
            v["inputs"].get(key).is_some(),
            "inputs must be a name-keyed object (missing {key}): {v}"
        );
        assert_eq!(v["inputs"][key]["ready"], false);
    }
    assert!(!v["next"].as_str().unwrap().is_empty());
}
