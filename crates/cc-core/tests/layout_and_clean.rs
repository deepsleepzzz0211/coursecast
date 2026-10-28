//! Ticket 11 (layout_set / artifact_clean core seam): persisting layout.toml and
//! removing a stage's artifacts + records so the stage re-runs from scratch.

use cc_core::render::{self, default_layout, load_layout, Layout, Placement};
use cc_core::{clean_artifacts, ProjectLock};
use std::fs;
use std::path::Path;

#[test]
fn save_layout_round_trips_through_layout_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = Layout::preset("bottom_left", 25.0, "circle", "fade");
    render::save_layout(&layout, tmp.path()).unwrap();

    let loaded = load_layout(&tmp.path().join(render::LAYOUT_TOML)).unwrap();
    assert_eq!(loaded, layout);

    // the file is human-editable TOML with the documented keys (文件即真相)
    let text = fs::read_to_string(tmp.path().join(render::LAYOUT_TOML)).unwrap();
    assert!(text.contains("position = \"bottom_left\""));
    assert!(text.contains("shape = \"circle\""));
}

#[test]
fn free_layout_round_trips_through_layout_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = Layout::free(25.0, 10.0, 20.0, "rounded_rect", "hard");
    render::save_layout(&layout, tmp.path()).unwrap();

    let text = fs::read_to_string(tmp.path().join(render::LAYOUT_TOML)).unwrap();
    assert!(text.contains("x_pct = 25.0"));
    assert!(text.contains("y_pct = 10.0"));
    assert!(!text.contains("position"));
    assert_eq!(
        load_layout(&tmp.path().join(render::LAYOUT_TOML)).unwrap(),
        layout
    );
}

#[test]
fn save_layout_overwrites_and_keeps_defaults_when_file_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // missing layout.toml loads defaults
    assert_eq!(
        load_layout(&tmp.path().join(render::LAYOUT_TOML)).unwrap(),
        default_layout()
    );
    // writing twice keeps only the last value
    render::save_layout(&default_layout(), tmp.path()).unwrap();
    render::save_layout(
        &Layout::preset("right_bar", 20.0, "rounded_rect", "hard"),
        tmp.path(),
    )
    .unwrap();
    assert_eq!(
        load_layout(&tmp.path().join(render::LAYOUT_TOML))
            .unwrap()
            .placement,
        Placement::Preset("right_bar".into())
    );
}

fn dirty_project(tmp: &Path) -> (String, std::path::PathBuf) {
    let created = cc_core::create_project(tmp, "清理", Some("640x360")).unwrap();
    let mut project = created.project;
    let dir = created.dir;
    let id = project.project_id.clone();

    let artifacts = [
        (
            "slide_render",
            vec!["work/slides/slide_001.png", "work/slides/slide_002.png"],
        ),
        ("tts:1", vec!["work/audio/slide_001.wav"]),
        ("tts:2", vec!["work/audio/slide_002.wav"]),
        (
            "timeline",
            vec!["work/concat.txt", "work/audio/voice_full.wav"],
        ),
        (
            "matte",
            vec!["work/person/alpha.mov", "work/person/preview.png"],
        ),
        ("render", vec!["output/final.mp4"]),
    ];
    for (key, files) in artifacts {
        for f in &files {
            let p = dir.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, format!("bytes of {f}")).unwrap();
        }
        project.stages.insert(
            key.to_string(),
            cc_core::StageRecord {
                stage: key.split(':').next().unwrap().to_string(),
                state: "success".into(),
                input_fingerprint: format!("sha256:{key}"),
                tool_version: "test".into(),
                outputs: files
                    .iter()
                    .map(|p| cc_core::OutputArtifact {
                        path: p.to_string(),
                        sha256: format!("sha256:{p}"),
                        bytes: 8,
                    })
                    .collect(),
                depends_on: Default::default(),
            },
        );
    }
    cc_core::save_project(&project, &dir).unwrap();
    (id, dir)
}

fn pj(dir: &Path) -> cc_core::Project {
    serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap()
}

#[test]
fn clean_matte_removes_only_matte_artifacts_and_record() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = dirty_project(tmp.path());
    let _lock = ProjectLock::acquire(&dir).unwrap();
    let (mut project, _) = cc_core::load_project(tmp.path(), &id).unwrap();

    let report = clean_artifacts(&mut project, &dir, "matte").unwrap();

    assert!(dir.join("work/person/alpha.mov").no_longer_exists());
    assert!(dir.join("work/person/preview.png").no_longer_exists());
    assert!(dir.join("work/slides/slide_001.png").is_file());
    assert!(dir.join("output/final.mp4").is_file());
    let after = pj(&dir);
    assert!(after.stages.contains_key("slide_render"));
    assert!(after.stages.contains_key("tts:1"));
    assert!(!after.stages.contains_key("matte"));
    assert_eq!(report.records_removed, vec!["matte".to_string()]);
    assert!(report
        .removed
        .contains(&"work/person/alpha.mov".to_string()));
}

/// Small assertion helper: negated `is_file` with a readable failure message.
trait NoLongerExists {
    fn no_longer_exists(&self) -> bool;
}
impl NoLongerExists for Path {
    fn no_longer_exists(&self) -> bool {
        !self.exists()
    }
}

#[test]
fn clean_tts_removes_every_page_record_and_the_audio_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = dirty_project(tmp.path());
    let _lock = ProjectLock::acquire(&dir).unwrap();
    let (mut project, _) = cc_core::load_project(tmp.path(), &id).unwrap();

    clean_artifacts(&mut project, &dir, "tts").unwrap();

    assert!(!dir.join("work/audio").exists());
    let after = pj(&dir);
    assert!(!after.stages.contains_key("tts:1"));
    assert!(!after.stages.contains_key("tts:2"));
    assert!(after.stages.contains_key("timeline"));
}

#[test]
fn clean_all_keeps_slides_and_the_slide_record() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = dirty_project(tmp.path());
    let _lock = ProjectLock::acquire(&dir).unwrap();
    let (mut project, _) = cc_core::load_project(tmp.path(), &id).unwrap();

    clean_artifacts(&mut project, &dir, "all").unwrap();

    assert!(!dir.join("work/audio").exists());
    assert!(!dir.join("work/person").exists());
    assert!(!dir.join("work/concat.txt").exists());
    assert!(!dir.join("output/final.mp4").exists());
    assert!(
        dir.join("work/slides/slide_001.png").is_file(),
        "slides are the deck, not intermediates"
    );
    let after = pj(&dir);
    assert!(after.stages.contains_key("slide_render"));
    assert!(after.stages.get("slide_render").unwrap().outputs.len() == 2);
    assert_eq!(after.stages.len(), 1);
}

#[test]
fn cleaning_an_already_clean_stage_is_a_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = dirty_project(tmp.path());
    let _lock = ProjectLock::acquire(&dir).unwrap();
    let (mut project, _) = cc_core::load_project(tmp.path(), &id).unwrap();

    let report = clean_artifacts(&mut project, &dir, "render").unwrap();
    assert_eq!(report.removed, vec!["output/final.mp4".to_string()]);
    let report = clean_artifacts(&mut project, &dir, "render").unwrap();
    assert!(report.removed.is_empty());
    assert!(report.records_removed.is_empty());
}

#[test]
fn unknown_clean_stage_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let (id, dir) = dirty_project(tmp.path());
    let _lock = ProjectLock::acquire(&dir).unwrap();
    let (mut project, _) = cc_core::load_project(tmp.path(), &id).unwrap();

    let err = clean_artifacts(&mut project, &dir, "rendering").unwrap_err();
    assert_eq!(err.code(), "invalid_argument");
}
