//! Cross-stage invariant matrix (ticket 19): every stage key obeys the same stage-record
//! protocol — fingerprint, stage dependency, content dependency, artifact integrity,
//! atomic landing and explicit reuse. Runs the protocol directly for each stage shape.

use cc_core::create_project;
use cc_core::stage::{self, VerifySpec};
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

#[test]
fn every_stage_obeys_the_same_record_invariants() {
    for key in ["slide_render", "tts:1", "timeline", "matte", "render"] {
        let data = TempDir::new().unwrap();
        let created = create_project(data.path(), key, None).unwrap();
        let mut project = created.project;
        let dir = created.dir;
        let rel = format!("work/artifacts/{}.bin", key.replace(':', "_"));
        let abs = dir.join(&rel);
        fs::create_dir_all(abs.parent().unwrap()).unwrap();
        fs::write(&abs, b"artifact").unwrap();

        let output = stage::hash_artifact(&dir, &rel).unwrap();
        let upstream = stage::success(
            "tts",
            "upstream-fp".into(),
            "test".into(),
            vec![],
            BTreeMap::new(),
        );
        project.stages.insert("upstream".into(), upstream);
        let record = stage::success(
            key,
            format!("fp-{key}"),
            "test".into(),
            vec![output],
            BTreeMap::from([
                ("upstream".to_string(), "upstream-fp".to_string()),
                ("video".to_string(), "v1".to_string()),
            ]),
        );
        stage::land(&mut project, &dir, key, record).unwrap();

        let fp = format!("fp-{key}");
        let spec = VerifySpec {
            key,
            input_fingerprint: &fp,
            stage_deps: &["upstream"],
            content_deps: &[("video", "v1")],
        };
        assert!(stage::verify(&project, &dir, &spec), "{key}: happy path");

        // Wrong fingerprint.
        assert!(
            !stage::verify(
                &project,
                &dir,
                &VerifySpec {
                    input_fingerprint: "other",
                    ..spec
                }
            ),
            "{key}: fingerprint mismatch"
        );

        // Stale / vanished stage dependency.
        let mut stale = project.clone();
        stale.stages.get_mut("upstream").unwrap().input_fingerprint = "upstream-fp-v2".into();
        assert!(
            !stage::verify(&stale, &dir, &spec),
            "{key}: stale stage dependency"
        );
        let mut missing_dep = project.clone();
        missing_dep.stages.remove("upstream");
        assert!(
            !stage::verify(&missing_dep, &dir, &spec),
            "{key}: vanished stage dependency"
        );

        // Hand-placed input (neither side asserts) is not stale.
        let mut hand_placed = missing_dep.clone();
        hand_placed
            .stages
            .get_mut(key)
            .unwrap()
            .depends_on
            .remove("upstream");
        assert!(
            stage::verify(&hand_placed, &dir, &spec),
            "{key}: hand-placed dependency is not stale"
        );

        // Content dependency mismatch.
        assert!(
            !stage::verify(
                &project,
                &dir,
                &VerifySpec {
                    content_deps: &[("video", "v2")],
                    ..spec
                }
            ),
            "{key}: content dependency mismatch"
        );

        // Missing / tampered artifact.
        fs::write(&abs, b"tampered").unwrap();
        assert!(
            !stage::verify(&project, &dir, &spec),
            "{key}: tampered artifact"
        );
        fs::write(&abs, b"artifact").unwrap();
        fs::remove_file(&abs).unwrap();
        assert!(
            !stage::verify(&project, &dir, &spec),
            "{key}: missing artifact"
        );

        // A non-success record is never reusable.
        let mut failed = project.clone();
        failed.stages.get_mut(key).unwrap().state = "failed".into();
        assert!(!stage::verify(&failed, &dir, &spec), "{key}: failed record");
    }
}

#[test]
fn landing_a_record_is_atomic_and_reloadable() {
    let data = TempDir::new().unwrap();
    let created = create_project(data.path(), "landing", None).unwrap();
    let mut project = created.project;
    let record = stage::success(
        "timeline",
        "fp".into(),
        "test".into(),
        vec![],
        BTreeMap::new(),
    );
    stage::land(&mut project, &created.dir, "timeline", record).unwrap();
    let (loaded, _) = cc_core::load_project(data.path(), &project.project_id).unwrap();
    assert_eq!(loaded.stages["timeline"].input_fingerprint, "fp");

    let target = created.dir.join("work/atom.txt");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    stage::write_atomic(&target, b"one").unwrap();
    stage::write_atomic(&target, b"two").unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "two");
    assert!(!created.dir.join("work/atom.txt.tmp").exists());
}
