//! Non-destructive check that the production credential backend (Windows Credential
//! Manager via keyring's windows-native store) initializes and reads without error.
//! `#[ignore]`d so regular test runs stay hermetic (keyring mock, see credentials.rs);
//! run with `cargo test --test credentials_real_store -- --ignored`. Never writes.

#[test]
fn real_credential_store_initializes_and_reads() {
    // Ok(None) when no key was configured yet, Ok(Some(_)) once voice_configure ran —
    // either way the windows-native backend loaded and answered.
    let key = cc_core::credentials::read_api_key().expect("credential store must be readable");
    let _ = key;
}
