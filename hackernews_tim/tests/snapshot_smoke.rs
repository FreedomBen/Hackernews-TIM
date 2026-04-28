//! Smoke tests that verify `insta` is wired into the workspace and
//! cooperates with [`hackernews_tim::test_support::PuppetHarness`].
//!
//! These tests don't exercise any real view module — they're a tiny
//! end-to-end check that:
//!
//! 1. The `insta` dev-dependency builds against the binary crate's
//!    public surface.
//! 2. The puppet harness produces deterministic, snapshot-friendly
//!    output across runs (whitespace trimming, fixed screen size).
//! 3. Snapshot files round-trip from `tests/snapshots/`.
//!
//! Per-view snapshots land in Phase 2.2 — keep these tests deliberately
//! tiny so they don't churn when real view tests start landing.
//!
//! Snapshots are reviewed with `cargo insta review` (preferred) or
//! regenerated in-place with `INSTA_UPDATE=always cargo test
//! --features test-support --test snapshot_smoke`.

use cursive::views::{Dialog, TextView};
use cursive::{Cursive, Vec2};
use hackernews_tim::test_support::PuppetHarness;

#[test]
fn snapshot_text_view() {
    let mut siv = Cursive::new();
    siv.add_layer(TextView::new("hello insta"));
    let mut harness = PuppetHarness::with_size(siv, Vec2::new(40, 6));
    harness.step_until_idle();

    insta::assert_snapshot!("text_view_hello_insta", harness.screen_text());
}

#[test]
fn snapshot_dialog() {
    let mut siv = Cursive::new();
    siv.add_layer(Dialog::info("snapshot me").title("Smoke"));
    let mut harness = PuppetHarness::with_size(siv, Vec2::new(40, 8));
    harness.step_until_idle();

    insta::assert_snapshot!("dialog_smoke_info", harness.screen_text());
}
