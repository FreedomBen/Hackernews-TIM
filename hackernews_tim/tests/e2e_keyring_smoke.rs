//! Smoke test for the Phase 3 keyring mock backend (Linux-only).
//!
//! Verifies that [`helpers::keyring::init_mock_keyring`] swaps in
//! `keyring::mock` so `keyring::Entry` round-trips inside the test
//! process without a real OS credential manager. Real auth-flow
//! scenarios that exercise the spawned binary live in sibling
//! `tests/e2e_*.rs` files (docs/planning/TEST_PLAN.md §3.2).
//!
//! See docs/planning/TEST_PLAN.md §3.1.3.
//!
//! On macOS / Windows this file compiles to an empty binary because of
//! the `#![cfg(target_os = "linux")]` gate.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use helpers::keyring::init_mock_keyring;

#[test]
fn mock_keyring_round_trips_within_entry() {
    init_mock_keyring();

    let entry = ::keyring::Entry::new("hackernews-tim-test", "alice")
        .expect("Entry::new should succeed under the mock backend");
    entry
        .set_password("hunter2")
        .expect("set_password should succeed under the mock backend");
    assert_eq!(
        entry
            .get_password()
            .expect("get_password should succeed under the mock backend"),
        "hunter2",
    );
    let _ = entry.delete_credential();
}

#[test]
fn init_mock_keyring_is_idempotent() {
    init_mock_keyring();
    init_mock_keyring();
    init_mock_keyring();
}
