//! Stage record protocol (ticket 04): one implementation of the honest-idempotency
//! contract every stage runner needs — fingerprint match, stage/content dependency
//! match, artifact integrity, and atomic record landing. Stage runners keep their own
//! fingerprint formulas and state transitions; they call this module for the protocol.

use crate::error::Result;
use crate::project::{save_project, OutputArtifact, Project, StageRecord};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

/// sha256 of `bytes`, prefixed like every fingerprint in project.json.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    format!("sha256:{hex}")
}

/// sha256 of a file, streamed (videos and final films are too big for one read).
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let mut hex = String::with_capacity(64);
    for b in hasher.finalize() {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok(format!("sha256:{hex}"))
}

/// Hash one landed artifact relative to the project directory.
pub fn hash_artifact(dir: &Path, rel: &str) -> Result<OutputArtifact> {
    let path = dir.join(rel);
    Ok(OutputArtifact {
        path: rel.to_string(),
        sha256: sha256_file(&path)?,
        bytes: fs::metadata(&path)?.len(),
    })
}

/// Every recorded output is still on disk with unchanged content. An empty output list
/// is never intact: a success record without artifacts asserts nothing.
pub fn artifacts_intact(dir: &Path, record: &StageRecord) -> bool {
    !record.outputs.is_empty()
        && record
            .outputs
            .iter()
            .all(|o| sha256_file(&dir.join(&o.path)).is_ok_and(|sha| sha == o.sha256))
}

/// What a caller asserts before reusing a recorded stage.
///
/// `depends_on` has two dialects, both checked here:
/// - **stage fingerprints** (`tts:7`, `timeline`, …): compare against the current
///   record's `input_fingerprint` via `stage_deps`.
/// - **content hashes** (`video`, `voice_full`, `person`): compare against a freshly
///   computed file hash via `content_deps`.
#[derive(Debug, Clone, Copy)]
pub struct VerifySpec<'a> {
    /// Key in `project.stages` (e.g. `timeline`, `tts:7`).
    pub key: &'a str,
    /// The stage's current input fingerprint.
    pub input_fingerprint: &'a str,
    /// Upstream stage keys whose current fingerprints must equal the recorded ones.
    pub stage_deps: &'a [&'a str],
    /// Content dependencies: key → current content hash (e.g. video, voice_full).
    pub content_deps: &'a [(&'a str, &'a str)],
}

/// True when the recorded stage can be reused: success state, fingerprint match, every
/// stage dependency matches, every content dependency matches, and all artifacts are
/// intact. A stage dependency present on neither side (e.g. hand-placed audio without a
/// tts record) asserts nothing and is not stale.
pub fn verify(project: &Project, dir: &Path, spec: &VerifySpec<'_>) -> bool {
    let Some(record) = project.stages.get(spec.key) else {
        return false;
    };
    if record.state != "success" || record.input_fingerprint != spec.input_fingerprint {
        return false;
    }
    if !spec
        .stage_deps
        .iter()
        .all(|key| stage_dep_matches(project, record, key))
    {
        return false;
    }
    if !spec
        .content_deps
        .iter()
        .all(|(key, current)| content_dep_matches(record, key, current))
    {
        return false;
    }
    artifacts_intact(dir, record)
}

/// Compare one recorded stage dependency against the current project record.
pub fn stage_dep_matches(project: &Project, record: &StageRecord, key: &str) -> bool {
    match (
        project
            .stages
            .get(key)
            .map(|r| r.input_fingerprint.as_str()),
        record.depends_on.get(key).map(String::as_str),
    ) {
        (None, None) => true, // hand-placed input: nothing asserted
        (Some(current), Some(recorded)) => current == recorded,
        _ => false, // record appeared or disappeared since this stage ran
    }
}

/// Compare one recorded content dependency (a hash, not a stage fingerprint).
pub fn content_dep_matches(record: &StageRecord, key: &str, current: &str) -> bool {
    record.depends_on.get(key).is_some_and(|h| h == current)
}

/// Legacy-friendly content dependency: records written before the key existed are
/// trusted (the render stage's hand-placed/legacy alpha rule); once the key is present
/// it must match.
pub fn optional_content_dep_matches(record: &StageRecord, key: &str, current: &str) -> bool {
    match record.depends_on.get(key) {
        None => true,
        Some(recorded) => recorded == current,
    }
}

/// Build a success record with the standard shape.
pub fn success(
    stage: &str,
    input_fingerprint: String,
    tool_version: String,
    outputs: Vec<OutputArtifact>,
    depends_on: BTreeMap<String, String>,
) -> StageRecord {
    StageRecord {
        stage: stage.to_string(),
        state: "success".to_string(),
        input_fingerprint,
        tool_version,
        outputs,
        depends_on,
    }
}

/// Insert the record under `key` and save project.json atomically.
pub fn land(project: &mut Project, dir: &Path, key: &str, record: StageRecord) -> Result<()> {
    project.stages.insert(key.to_string(), record);
    save_project(project, dir)
}

/// Atomic artifact write: tmp file in the same directory, then rename over the target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => path.with_extension(format!("{ext}.tmp")),
        None => path.with_extension("tmp"),
    };
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::create_project;
    use tempfile::TempDir;

    fn artifact(dir: &Path, rel: &str, bytes: &[u8]) -> OutputArtifact {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        hash_artifact(dir, rel).unwrap()
    }

    fn record(fp: &str, outputs: Vec<OutputArtifact>, deps: &[(&str, &str)]) -> StageRecord {
        StageRecord {
            stage: "timeline".into(),
            state: "success".into(),
            input_fingerprint: fp.into(),
            tool_version: "test/1".into(),
            outputs,
            depends_on: deps
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        }
    }

    #[test]
    fn sha256_hex_matches_the_known_abc_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_file_and_hash_artifact_use_the_same_known_vector() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("hello.txt"), b"hello").unwrap();
        let expected = "sha256:2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert_eq!(
            sha256_file(&tmp.path().join("hello.txt")).unwrap(),
            expected
        );
        let out = hash_artifact(tmp.path(), "hello.txt").unwrap();
        assert_eq!(out.path, "hello.txt");
        assert_eq!(out.sha256, expected);
        assert_eq!(out.bytes, 5);
    }

    #[test]
    fn artifacts_intact_rejects_missing_tampered_and_empty_records() {
        let tmp = TempDir::new().unwrap();
        let out = artifact(tmp.path(), "work/track.wav", b"one");
        let rec = record("fp", vec![out], &[]);
        assert!(artifacts_intact(tmp.path(), &rec));

        fs::write(tmp.path().join("work/track.wav"), b"two").unwrap();
        assert!(
            !artifacts_intact(tmp.path(), &rec),
            "tampered artifact must not count"
        );

        fs::remove_file(tmp.path().join("work/track.wav")).unwrap();
        assert!(
            !artifacts_intact(tmp.path(), &rec),
            "missing artifact must not count"
        );
        assert!(!artifacts_intact(tmp.path(), &record("fp", vec![], &[])));
    }

    #[test]
    fn verify_requires_fingerprint_dependencies_and_intact_artifacts() {
        let data = TempDir::new().unwrap();
        let created = create_project(data.path(), "stage verify", None).unwrap();
        let dir = &created.dir;
        let mut project = created.project;
        let out = artifact(dir, "work/track.wav", b"one");
        let tts = record("tts-fp", vec![], &[]);
        project.stages.insert("tts:1".into(), tts);
        let rec = record("timeline-fp", vec![out], &[("tts:1", "tts-fp")]);
        project.stages.insert("timeline".into(), rec.clone());

        let ok = VerifySpec {
            key: "timeline",
            input_fingerprint: "timeline-fp",
            stage_deps: &["tts:1"],
            content_deps: &[],
        };
        assert!(verify(&project, dir, &ok));
        assert!(!verify(
            &project,
            dir,
            &VerifySpec {
                input_fingerprint: "other-fp",
                ..ok
            }
        ));

        // A dependency that vanished is stale; a hand-placed dependency on neither side
        // asserts nothing and stays usable.
        let mut without_tts = project.clone();
        without_tts.stages.remove("tts:1");
        assert!(!verify(&without_tts, dir, &ok));
        let mut hand_placed = without_tts.clone();
        hand_placed.stages.insert(
            "timeline".into(),
            record(
                "timeline-fp",
                vec![artifact(dir, "work/track.wav", b"one")],
                &[],
            ),
        );
        assert!(verify(&hand_placed, dir, &ok));

        // Content dependencies are hashes, not stage fingerprints.
        let content = VerifySpec {
            key: "timeline",
            input_fingerprint: "timeline-fp",
            stage_deps: &[],
            content_deps: &[("voice_full", "v1")],
        };
        let mut with_content = project.clone();
        with_content.stages.insert(
            "timeline".into(),
            record(
                "timeline-fp",
                vec![artifact(dir, "work/track.wav", b"one")],
                &[("voice_full", "v1")],
            ),
        );
        assert!(verify(&with_content, dir, &content));
        assert!(!verify(
            &with_content,
            dir,
            &VerifySpec {
                content_deps: &[("voice_full", "v2")],
                ..content
            }
        ));

        // A non-success record is never reusable, even with a matching fingerprint.
        let mut failed = project.clone();
        failed.stages.get_mut("timeline").unwrap().state = "failed".into();
        assert!(!verify(&failed, dir, &ok));

        // A tampered artifact invalidates the record.
        fs::write(dir.join("work/track.wav"), b"evil").unwrap();
        assert!(!verify(&project, dir, &ok));
    }

    #[test]
    fn optional_content_dependency_trusts_legacy_records_only_when_absent() {
        let rec = record("fp", vec![], &[]);
        assert!(optional_content_dep_matches(&rec, "video", "v1"));
        let rec = record("fp", vec![], &[("video", "v1")]);
        assert!(optional_content_dep_matches(&rec, "video", "v1"));
        assert!(!optional_content_dep_matches(&rec, "video", "v2"));
        assert!(!content_dep_matches(
            &record("fp", vec![], &[]),
            "video",
            "v1"
        ));
    }

    #[test]
    fn land_persists_a_record_and_write_atomic_leaves_no_tmp_file() {
        let data = TempDir::new().unwrap();
        let created = create_project(data.path(), "land", None).unwrap();
        let mut project = created.project;
        let record = success(
            "timeline",
            "fp".into(),
            "test/1".into(),
            vec![],
            BTreeMap::new(),
        );
        assert_eq!(record.state, "success");
        land(&mut project, &created.dir, "timeline", record).unwrap();

        let (loaded, _) = crate::load_project(data.path(), &project.project_id).unwrap();
        assert_eq!(loaded.stages["timeline"].input_fingerprint, "fp");

        let target = created.dir.join("work/note.txt");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        write_atomic(&target, b"first").unwrap();
        write_atomic(&target, b"second").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "second");
        assert!(!created.dir.join("work/note.txt.tmp").exists());
    }
}
