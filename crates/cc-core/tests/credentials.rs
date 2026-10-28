//! Ticket 05: API key storage (`cc_core::credentials`). The key lives in the OS
//! credential store (Windows Credential Manager via the keyring crate). These tests run
//! against keyring's mock store so the real Credential Manager is never touched; the
//! store/read code path is identical (only the backend builder differs).

use std::sync::Once;

/// Swap keyring's default builder to the mock store, exactly once per test process.
fn use_mock_credential_store() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    });
}

#[test]
fn api_key_round_trips_through_the_credential_store() {
    use_mock_credential_store();

    assert_eq!(
        cc_core::credentials::read_api_key().unwrap(),
        None,
        "nothing stored yet"
    );

    cc_core::credentials::store_api_key("sk-test-key-1").unwrap();
    assert_eq!(
        cc_core::credentials::read_api_key().unwrap().as_deref(),
        Some("sk-test-key-1")
    );

    // reconfiguring overwrites the previous key
    cc_core::credentials::store_api_key("sk-test-key-2").unwrap();
    assert_eq!(
        cc_core::credentials::read_api_key().unwrap().as_deref(),
        Some("sk-test-key-2")
    );
}
