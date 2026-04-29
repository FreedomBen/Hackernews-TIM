//! Smoke test for the Phase 3 fake HN backend (Linux-only).
//!
//! Verifies:
//!
//! 1. [`helpers::fakehn::FakeHnServer`] can spin up a `wiremock`
//!    listener inside a private tokio runtime.
//! 2. `HNClient::with_timeout` honours `HN_ALGOLIA_BASE` and
//!    `HN_FIREBASE_BASE` so the binary's HTTP traffic can be steered
//!    at the fake server end-to-end.
//! 3. Recorded requests are visible to the test thread for later
//!    assertions in §3.2 scenarios.
//!
//! Single-test binary on purpose: `std::env::set_var` is process-
//! global, so the e2e suite must run under `--test-threads=1` (per
//! docs/planning/TEST_PLAN.md §3.1.4) and this binary must not contain other tests
//! that race on the same env vars. macOS / Windows compile to an
//! empty binary via the `cfg` gate.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use hackernews_tim::client::HNClient;
use helpers::fakehn::FakeHnServer;

#[test]
fn hnclient_routes_through_fake_backend() {
    let server = FakeHnServer::start();
    server.mount_get_json(
        "/api/v1/items/12345",
        200,
        serde_json::json!({
            "id": 12345,
            "type": "story",
            "title": "fake item",
            "author": "tester",
            "url": "https://example.invalid/",
            "points": 0,
            "children": [],
            "created_at_i": 0,
        }),
    );

    std::env::set_var("HN_ALGOLIA_BASE", server.algolia_base());
    std::env::set_var("HN_FIREBASE_BASE", server.firebase_base());

    let client = HNClient::with_timeout(5).expect("HNClient::with_timeout");
    let _: serde_json::Value = client
        .get_item_from_id(12345)
        .expect("get_item_from_id should hit the fake server");

    let requests = server.received_requests();
    let paths: Vec<String> = requests.iter().map(|r| r.url.path().to_string()).collect();
    assert!(
        paths.iter().any(|p| p == "/api/v1/items/12345"),
        "expected request to /api/v1/items/12345; saw paths: {paths:?}"
    );

    std::env::remove_var("HN_ALGOLIA_BASE");
    std::env::remove_var("HN_FIREBASE_BASE");
}
