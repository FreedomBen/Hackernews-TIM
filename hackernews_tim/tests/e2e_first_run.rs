//! docs/planning/TEST_PLAN.md §3.2.1 — first run with no config (Linux-only).
//!
//! Scenario:
//!
//! 1. Spawn the binary with empty `HOME` / `XDG_CONFIG_HOME` so the
//!    config and auth files are missing.
//! 2. The TTY-gated `prompt_for_flavor` writes its
//!    `[l]ight / [d]ark / [s]kip` line; we send `l\n`.
//! 3. The binary writes the embedded light default to
//!    `${XDG_CONFIG_HOME}/hackernews-tim/config.toml`, prints
//!    `Wrote config to <path>`, then asks `prompt_for_auth` which we
//!    decline by sending an empty newline.
//! 4. Cursive starts and renders the front-page story view, populated
//!    by the fixture story served from [`FakeHnServer`] (`/topstories.json`
//!    + `/search?tags=story,(story_<id>,)`).
//! 5. We send the default quit key and assert a clean exit.
//!
//! Note: the unauthenticated front-page render also fires
//! `get_listing_vote_state` against the news_base `/news` endpoint.
//! This test deliberately leaves `HN_NEWS_BASE` unset, so the harness
//! routes that request to the blackhole port (`http://127.0.0.1:1`)
//! defined in `tests/e2e/mod.rs`; the failure is swallowed by the
//! client (warn!'d and dropped), so the front page still renders.
//! The cross-test no-production-host guarantee is asserted by
//! `tests/e2e_no_real_network.rs` (docs/planning/TEST_PLAN.md §3.3).

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, SpawnOptions, DEFAULT_WAIT};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);

#[test]
fn first_run_writes_default_config_then_renders_front_page() {
    let server = FakeHnServer::start();

    // `get_stories_no_sort("front_page", ...)` first hits the official
    // `/topstories.json` endpoint for ordered IDs.
    server.mount_get_json("/v0/topstories.json", 200, json!([10001]));

    // …then queries Algolia for the corresponding stories. wiremock's
    // `path` matcher ignores the query string, so the exact tag list
    // and `hitsPerPage` don't need to match.
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [
                {
                    "objectID": "10001",
                    "author": "alice",
                    "url": "https://example.com/post",
                    "story_text": null,
                    "points": 250,
                    "num_comments": 12,
                    "created_at_i": 1_700_000_000_u64,
                    "_highlightResult": {
                        "title": { "value": "Phase 3 e2e is alive" }
                    },
                    "dead": false,
                    "flagged": false
                }
            ]
        }),
    );

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    handle
        .wait_for_text("[l]ight", DEFAULT_WAIT)
        .expect("flavor prompt should print");
    handle.send_keys("l\n").expect("send light flavor");

    handle
        .wait_for_text("Wrote config to", Duration::from_secs(10))
        .expect("binary should announce the freshly-written config");

    handle
        .wait_for_text("No auth file found", DEFAULT_WAIT)
        .expect("auth prompt should print after config write");
    handle.send_keys("\n").expect("skip auth (default = N)");

    handle
        .wait_for_text("Phase 3 e2e is alive", FRONT_PAGE_RENDER_TIMEOUT)
        .expect("fixture story should appear in the rendered front page");

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");

    let config_path = handle
        .dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("config.toml");
    assert!(
        config_path.exists(),
        "config file should have been written to {}",
        config_path.display()
    );
    let written = std::fs::read_to_string(&config_path)
        .expect("written config should be readable")
        .into_bytes();
    assert!(
        !written.is_empty(),
        "written config should not be empty (path={})",
        config_path.display()
    );
}
