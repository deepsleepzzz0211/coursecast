//! Seam: the state machine as exposed by `cc_core::Project` (transition / fail) and
//! `cc_core::EDGES`. Ticket 01 requires unit tests covering ALL edges plus illegal
//! transitions erroring with state unchanged (PRD §6.6).

use cc_core::{FailedInfo, Project, Stage, EDGES};
use std::collections::BTreeMap;

const ALL: [Stage; 10] = [
    Stage::Created,
    Stage::Parsing,
    Stage::Parsed,
    Stage::TtsRunning,
    Stage::TtsDone,
    Stage::MatteRunning,
    Stage::MatteDone,
    Stage::Rendering,
    Stage::Done,
    Stage::Failed,
];

fn project_at(stage: Stage) -> Project {
    let failed = if stage == Stage::Failed {
        Some(FailedInfo {
            stage: Stage::Rendering,
            error: "boom".into(),
        })
    } else {
        None
    };
    Project {
        schema_version: cc_core::SCHEMA_VERSION,
        project_id: "abcd1234".into(),
        name: "t".into(),
        resolution: "1920x1080".into(),
        created_at: "2026-10-24T00:00:00Z".into(),
        stage,
        failed,
        stages: BTreeMap::new(),
        fallbacks: Vec::new(),
        tts: Default::default(),
    }
}

#[test]
fn every_defined_edge_is_allowed() {
    // sanity: the linear chain must be present in the table
    for from in ALL {
        for to in ALL {
            if EDGES.contains(&(from, to)) {
                let mut p = project_at(from);
                p.transition(to)
                    .unwrap_or_else(|e| panic!("edge {from} -> {to} rejected: {e}"));
                assert_eq!(p.stage, to);
            }
        }
    }
    let mut p = project_at(Stage::Created);
    for s in [
        Stage::Parsing,
        Stage::Parsed,
        Stage::TtsRunning,
        Stage::TtsDone,
        Stage::MatteRunning,
        Stage::MatteDone,
        Stage::Rendering,
        Stage::Done,
    ] {
        p.transition(s).unwrap();
        assert_eq!(p.stage, s);
    }
}

#[test]
fn every_undefined_edge_is_rejected_and_state_unchanged() {
    for from in ALL {
        for to in ALL {
            if EDGES.contains(&(from, to)) {
                continue;
            }
            let mut p = project_at(from);
            let err = p
                .transition(to)
                .expect_err(&format!("{from} -> {to} not in EDGES; should be rejected"));
            assert!(
                matches!(err, cc_core::Error::InvalidTransition { .. }),
                "{from} -> {to}: expected InvalidTransition, got {err:?}"
            );
            assert_eq!(
                p.stage, from,
                "state must be unchanged after illegal {from} -> {to}"
            );
        }
    }
}

#[test]
fn fail_records_stage_and_error_then_allows_retry() {
    let mut p = project_at(Stage::TtsRunning);
    p.fail("tts api 401").unwrap();
    assert_eq!(p.stage, Stage::Failed);
    let failed = p.failed.as_ref().unwrap();
    assert_eq!(failed.stage, Stage::TtsRunning);
    assert_eq!(failed.error, "tts api 401");

    // retry edge: failed -> the running stage that failed
    p.transition(Stage::TtsRunning).unwrap();
    assert_eq!(p.stage, Stage::TtsRunning);
}

#[test]
fn fail_is_rejected_from_created_done_and_failed() {
    for start in [Stage::Created, Stage::Done, Stage::Failed] {
        let mut p = project_at(start);
        assert!(p.fail("x").is_err(), "fail from {start} must be illegal");
        assert_eq!(p.stage, start);
    }
}

#[test]
fn invalidation_lets_completed_stages_reenter_for_changed_inputs() {
    // ticket 10 失效传播: changed inputs re-enter a completed stage from every
    // checkpoint the pipeline can legitimately sit at
    assert!(
        cc_core::is_legal_edge(Stage::TtsDone, Stage::TtsRunning),
        "page edit (ticket 04/09)"
    );
    assert!(
        cc_core::is_legal_edge(Stage::MatteDone, Stage::TtsRunning),
        "page edit after matte"
    );
    assert!(
        cc_core::is_legal_edge(Stage::Done, Stage::TtsRunning),
        "page edit after the final render"
    );
    assert!(
        cc_core::is_legal_edge(Stage::MatteDone, Stage::MatteRunning),
        "person video/model changed"
    );
    assert!(
        cc_core::is_legal_edge(Stage::Done, Stage::MatteRunning),
        "person video changed after the final render"
    );
    assert!(
        cc_core::is_legal_edge(Stage::Done, Stage::Rendering),
        "layout.toml changed"
    );
    // states that only exist transiently inside a command stay locked
    assert!(!cc_core::is_legal_edge(Stage::Rendering, Stage::TtsRunning));
    assert!(!cc_core::is_legal_edge(
        Stage::MatteRunning,
        Stage::MatteRunning
    ));
}

#[test]
fn stage_serializes_as_snake_case_string() {
    assert_eq!(
        serde_json::to_value(Stage::TtsRunning).unwrap(),
        "tts_running"
    );
    assert_eq!(
        serde_json::to_value(Stage::MatteDone).unwrap(),
        "matte_done"
    );
    let s: Stage = serde_json::from_str("\"tts_running\"").unwrap();
    assert_eq!(s, Stage::TtsRunning);
}
