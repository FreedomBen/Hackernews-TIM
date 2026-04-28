//! Smoke test for the Phase 3 PTY harness (Linux-only).
//!
//! Verifies that [`helpers::spawn_app`] can launch the debug binary,
//! capture its PTY output via `vt100::Parser`, and observe a clean
//! exit. Real user-flow scenarios live in sibling `tests/e2e_*.rs`
//! files starting with TEST_PLAN.md §3.2.
//!
//! On macOS / Windows this file compiles to an empty binary because of
//! the `#![cfg(target_os = "linux")]` gate.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use helpers::{spawn_app, SpawnOptions};

#[test]
fn harness_can_capture_version_output() {
    let opts = SpawnOptions::new().arg("--version");
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");
    let status = handle
        .wait_for_exit(Duration::from_secs(10))
        .expect("binary should exit within 10s");

    assert!(
        status.success(),
        "expected zero exit status from --version, got {status:?}"
    );

    let screen = handle.screen();
    assert!(
        !screen.trim().is_empty(),
        "expected non-empty captured output"
    );
    assert!(
        screen.chars().any(|c| c.is_ascii_digit()),
        "expected version number in --version output, got:\n{screen}"
    );
}
