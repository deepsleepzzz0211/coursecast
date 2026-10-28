//! Seam: `cc_core::build_fallback_chain` — the registry defining the three-level
//! degradation chain (ticket 06): MiMo（有 Key）→ edge-tts（ticket 07）→ Windows SAPI.
//! The chain is the registry suffix starting at the preferred engine, so nothing here
//! special-cases engine names. The credential file seam keeps the real OS credential
//! store out of the tests.

use std::path::PathBuf;

/// Point the credential seam at `content` (None = no key stored).
fn set_key_seam(tmp: &std::path::Path, content: Option<&str>) -> PathBuf {
    let path = tmp.join("mimo.key");
    match content {
        Some(key) => std::fs::write(&path, key).unwrap(),
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
    path
}

#[test]
fn chain_resolution_follows_the_stored_key() {
    let tmp = tempfile::tempdir().unwrap();

    // no key anywhere: mimo cannot even be constructed, the chain starts at the next
    // available level, edge-tts (ticket 07) — which needs no credentials — then SAPI
    // (acceptance: 无 Key 时 engine=mimo 请求自动降级)
    std::env::set_var("CC_CREDENTIALS_FILE", set_key_seam(tmp.path(), None));
    let chain = cc_core::build_fallback_chain(cc_core::TtsEngineKind::Mimo).unwrap();
    let names: Vec<&str> = chain.engines.iter().map(|e| e.name()).collect();
    assert_eq!(
        names,
        vec!["edge", "sapi"],
        "no key → the chain starts at the next level"
    );
    assert_eq!(chain.events.len(), 1, "{:?}", chain.events);
    assert_eq!(chain.events[0].from, "mimo");
    assert_eq!(chain.events[0].to, "edge");
    assert!(
        chain.events[0].reason.contains("API key"),
        "{:?}",
        chain.events[0]
    );

    // a stored key: the full three-level chain mimo → edge → sapi, no degradation
    std::env::set_var(
        "CC_CREDENTIALS_FILE",
        set_key_seam(tmp.path(), Some("sk-test-key")),
    );
    let chain = cc_core::build_fallback_chain(cc_core::TtsEngineKind::Mimo).unwrap();
    let names: Vec<&str> = chain.engines.iter().map(|e| e.name()).collect();
    assert_eq!(names, vec!["mimo", "edge", "sapi"]);
    assert!(chain.events.is_empty(), "{:?}", chain.events);

    std::env::remove_var("CC_CREDENTIALS_FILE");
}

#[test]
fn preferred_edge_starts_at_the_middle_level() {
    // edge-tts needs no credentials: requesting it directly yields edge → SAPI
    let chain = cc_core::build_fallback_chain(cc_core::TtsEngineKind::Edge).unwrap();
    let names: Vec<&str> = chain.engines.iter().map(|e| e.name()).collect();
    assert_eq!(names, vec!["edge", "sapi"]);
    assert!(chain.events.is_empty(), "{:?}", chain.events);
}

#[test]
fn preferred_sapi_is_the_chain_end_without_touching_the_key_store() {
    // starts at the registry's last level: mimo's constructor never runs
    let chain = cc_core::build_fallback_chain(cc_core::TtsEngineKind::Sapi).unwrap();
    let names: Vec<&str> = chain.engines.iter().map(|e| e.name()).collect();
    assert_eq!(names, vec!["sapi"]);
    assert!(chain.events.is_empty());
}

#[test]
fn unregistered_engine_preference_is_a_structured_error() {
    // anything outside the four shell names is a structured invalid_argument, never a panic
    let err = "bogus".parse::<cc_core::TtsEngineKind>().unwrap_err();
    assert_eq!(err.code(), "invalid_argument");
    assert!(err.to_string().contains("bogus"), "{err}");
}

#[test]
fn engine_kind_round_trips_its_shell_name() {
    for kind in cc_core::TtsEngineKind::ALL {
        assert_eq!(
            kind.as_str().parse::<cc_core::TtsEngineKind>().unwrap(),
            kind
        );
        assert_eq!(kind.to_string(), kind.as_str());
    }
}
