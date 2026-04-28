//! Keyring mock backend for the Phase 3 e2e suite. Linux-only.
//!
//! Auth keyring tests should never reach a real OS credential manager
//! (no D-Bus session on CI, no Keychain unlock prompts, no leftover
//! state between runs). [`init_mock_keyring`] swaps in
//! `keyring::mock::default_credential_builder` so `keyring::Entry`
//! operations succeed against an in-memory builder.
//!
//! See TEST_PLAN.md §3.1.3.
//!
//! ## Scope: in-test-process only
//!
//! `keyring::set_default_credential_builder` mutates a process-global
//! `Mutex` inside the `keyring` crate. That covers the **test process**
//! (where helpers and any in-process auth code run) but does **not**
//! propagate to child processes spawned via [`super::spawn_app`] —
//! each binary the harness launches resets the builder back to
//! `keyring`'s compiled-in default. Scenarios that need to observe
//! keyring writes performed by the spawned binary (e.g.
//! `--migrate-auth`, TEST_PLAN.md §3.2.13) must drive the relevant
//! code paths from the test process directly, or arrange another
//! mechanism — that decision belongs to the §3.2.h scenario, not this
//! infrastructure shim.
//!
//! ## Concurrency
//!
//! The `keyring` crate's default-builder slot is process-global, so
//! tests that depend on the mock must run with `--test-threads=1`.
//! `make e2e` enforces this for the whole PTY suite.

#![cfg(target_os = "linux")]
#![allow(dead_code)]

use std::sync::Once;

static INIT: Once = Once::new();

/// Install `keyring::mock::default_credential_builder` as the
/// process-wide default. Idempotent — repeat calls are a no-op, so
/// every test that touches keyring code can call this in its setup
/// without coordinating with siblings.
pub fn init_mock_keyring() {
    INIT.call_once(|| {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    });
}
