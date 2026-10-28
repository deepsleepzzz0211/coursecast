//! Seam: `project.json` on disk — spec's blessed internal target ("project.json 里的状态
//! 与指纹"). Fingerprint record JSON must match PRD §6.6.1 field names exactly.

use cc_core::{
    create_project, load_project, project_dir, save_project, EdgeVoice, OutputArtifact, Project,
    Stage, StageRecord, TtsPreference, VoiceEngine,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[test]
fn created_project_loads_back_equal() {
    let tmp = tempfile::tempdir().unwrap();
    let created = create_project(tmp.path(), "药包机械", None).unwrap();
    let (loaded, dir) = load_project(tmp.path(), &created.project.project_id).unwrap();
    assert_eq!(loaded, created.project);
    assert_eq!(dir, project_dir(tmp.path(), &created.project.project_id));
}

#[test]
fn concurrent_saves_never_expose_intermediate_state() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = project_dir(tmp.path(), "abcd1234");
    fs::create_dir_all(&dir).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let stop_w = Arc::clone(&stop);
    let writer_dir = dir.clone();
    let writer = std::thread::spawn(move || {
        let mut flip = false;
        while !stop_w.load(Ordering::Relaxed) {
            let mut p = sample_project();
            p.stage = if flip { Stage::Parsed } else { Stage::Parsing };
            flip = !flip;
            save_project(&p, &writer_dir).unwrap();
        }
    });

    // While the writer saves, every read must see complete, coherent JSON — never a
    // half-written file (spec 必测场景: "原子写无中间态").
    //
    // Platform note: Windows rename-over-existing (MoveFileEx REPLACE_EXISTING) has a
    // transient window where the target name is momentarily absent (no POSIX rename
    // atomicity). Real readers hold the project lock, so they never race the writer;
    // the invariant under test is: content is never TORN. Transient NotFound is retried.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < deadline {
        let text = match fs::read_to_string(dir.join("project.json")) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::thread::sleep(std::time::Duration::from_micros(200));
                continue;
            }
            Err(e) => panic!("project.json unreadable during concurrent save: {e}"),
        };
        let v: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("intermediate state observed: {e}\n{text}"));
        let stage = v["stage"].as_str().unwrap();
        assert!(
            matches!(stage, "created" | "parsing" | "parsed"),
            "unexpected stage `{stage}` in {text}"
        );
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();

    assert!(
        !dir.join("project.json.tmp").exists(),
        "atomic write must not leave tmp files"
    );
}

#[test]
fn older_project_json_without_tts_defaults_to_mimo() {
    let tmp = tempfile::tempdir().unwrap();
    let p = sample_project();
    let dir = project_dir(tmp.path(), &p.project_id);
    fs::create_dir_all(&dir).unwrap();
    save_project(&p, &dir).unwrap();

    let path = dir.join("project.json");
    let mut raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    raw.as_object_mut().unwrap().remove("tts");
    fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    let (loaded, _) = load_project(tmp.path(), &p.project_id).unwrap();
    assert_eq!(
        loaded.tts,
        TtsPreference {
            engine: VoiceEngine::Mimo,
            edge_voice: EdgeVoice::Xiaoxiao,
        }
    );
}

#[test]
fn stage_record_round_trips_with_prd_field_names() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = sample_project();

    p.stage = Stage::TtsDone;
    p.stages.insert(
        "tts:7".into(),
        StageRecord {
            stage: "tts".into(),
            state: "success".into(),
            input_fingerprint: "sha256:aaa".into(),
            tool_version: "tts-mimo/0.1.0+model=mimo-v2.5-tts-voiceclone".into(),
            outputs: vec![OutputArtifact {
                path: "work/audio/slide_007.wav".into(),
                sha256: "sha256:bbb".into(),
                bytes: 412345,
            }],
            depends_on: BTreeMap::from([("slide_render".to_string(), "sha256:ccc".to_string())]),
        },
    );
    let dir = project_dir(tmp.path(), &p.project_id);
    fs::create_dir_all(&dir).unwrap();
    save_project(&p, &dir).unwrap();

    let raw: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("project.json")).unwrap()).unwrap();
    let rec = &raw["stages"]["tts:7"];
    for key in [
        "stage",
        "state",
        "input_fingerprint",
        "tool_version",
        "outputs",
        "depends_on",
    ] {
        assert!(rec.get(key).is_some(), "missing key `{key}` in {rec}");
    }
    assert_eq!(rec["input_fingerprint"], "sha256:aaa");
    assert_eq!(
        rec["tool_version"],
        "tts-mimo/0.1.0+model=mimo-v2.5-tts-voiceclone"
    );
    assert_eq!(rec["outputs"][0]["path"], "work/audio/slide_007.wav");
    assert_eq!(rec["outputs"][0]["sha256"], "sha256:bbb");
    assert_eq!(rec["outputs"][0]["bytes"], 412345);
    assert_eq!(rec["depends_on"]["slide_render"], "sha256:ccc");

    let (loaded, _) = load_project(tmp.path(), &p.project_id).unwrap();
    assert_eq!(loaded, p);
}

fn sample_project() -> Project {
    Project {
        schema_version: cc_core::SCHEMA_VERSION,
        project_id: "abcd1234".into(),
        name: "t".into(),
        resolution: "1920x1080".into(),
        created_at: "2026-10-24T00:00:00Z".into(),
        stage: Stage::Created,
        failed: None,
        stages: BTreeMap::new(),
        fallbacks: Vec::new(),
        tts: Default::default(),
    }
}
