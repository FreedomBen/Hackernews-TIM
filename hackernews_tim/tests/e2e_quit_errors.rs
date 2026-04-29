//! docs/planning/TEST_PLAN.md §3.2.i — Quit + error / network-failure paths
//! (Linux-only). Covers scenarios 3.2.14 and 3.2.15.
//!
//! See [`e2e_first_run.rs`] for the surrounding harness conventions
//! (TTY-gated flavor / auth prompts, `HOME` / `XDG_*` isolation,
//! `HN_ALGOLIA_BASE` / `HN_FIREBASE_BASE` / `HN_NEWS_BASE`
//! overrides). Both base URLs point at the in-process
//! [`FakeHnServer`], so no traffic escapes to the real HN hosts.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, AppHandle, SpawnOptions, DEFAULT_WAIT};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);

const STORY_ID: u32 = 90001;
const STORY_TITLE: &str = "quit fixture story";

/// Mount a minimal one-story front page so the binary reaches a
/// stable, healthy render before the quit key is sent.
fn mount_front_page(server: &FakeHnServer) {
    server.mount_get_json("/v0/topstories.json", 200, json!([STORY_ID]));
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [{
                "objectID": STORY_ID.to_string(),
                "author": "alice",
                "url": format!("https://example.com/{STORY_ID}"),
                "story_text": null,
                "points": 42,
                "num_comments": 0,
                "created_at_i": 1_700_000_000_u64,
                "_highlightResult": { "title": { "value": STORY_TITLE } },
                "dead": false,
                "flagged": false,
            }]
        }),
    );
}

/// Step the binary through the first-run prompts so the subsequent
/// front-page render is the only thing under test. Mirrors the
/// helper in `e2e_navigation.rs`.
fn dismiss_first_run_prompts(handle: &mut AppHandle) {
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
}

#[test]
fn quit_key_q_exits_with_zero_status() {
    let server = FakeHnServer::start();
    mount_front_page(&server);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    dismiss_first_run_prompts(&mut handle);

    handle
        .wait_for_text(STORY_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("front page should render before the quit key is sent");

    // `shutdown()` writes the default quit key (`q`) and waits for the
    // child to exit within the harness's 5 s shutdown timeout.
    let status = handle
        .shutdown()
        .expect("binary should exit within the shutdown timeout");
    assert!(
        status.success(),
        "expected zero exit status, got {status:?}"
    );
}

#[test]
fn front_page_http_500_surfaces_error_view() {
    let server = FakeHnServer::start();

    // The front-page tag uses `StorySortMode::None`, so the binary
    // calls `get_stories_no_sort`, which hits the official Firebase
    // `/topstories.json` endpoint first. A 500 there fails the whole
    // `get_stories_by_tag` call before any Algolia traffic. The
    // parallel `get_listing_vote_state` request to news_base `/news`
    // is unmounted on purpose: its errors are swallowed (warn!'d and
    // dropped), so the wiremock 404 doesn't affect this assertion.
    server.mount_get_json("/v0/topstories.json", 500, json!({}));

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    dismiss_first_run_prompts(&mut handle);

    // The async view replaces the loading bar with `ResultView::Err`,
    // which renders an `ErrorView` whose centered title bar reads
    // "Error View".
    handle
        .wait_for_text("Error View", FRONT_PAGE_RENDER_TIMEOUT)
        .expect("Error View title should appear when the front page fails to load");

    // The wrapped context message identifies the call site — anyhow's
    // debug format prints the full chain, so `failed to get stories`
    // is part of the dialog body.
    let screen = handle.screen();
    assert!(
        screen.contains("failed to get stories"),
        "expected the wrapped error context on screen; saw:\n{screen}"
    );

    // docs/planning/TEST_PLAN.md §3.2.15 acceptance: PTY-rendered error-view snapshot.
    // The wiremock URL embeds a random port, so redact `127.0.0.1:<port>`
    // alongside the usual `\d+ \w+ ago` filter (the wrapping ureq error
    // also includes a status-code line that varies in trivial ways).
    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
        (r"127\.0\.0\.1:\d+", "127.0.0.1:[port]"),
    ]}, {
        insta::assert_snapshot!("front_page_http_500_error_view_pty", screen);
    });

    // The binary must still be alive and responsive: the global
    // quit-key callback is wired via `set_on_post_event`, so `q`
    // should fall through the error dialog and trigger a clean exit.
    let status = handle
        .shutdown()
        .expect("binary should exit cleanly after surfacing the error view");
    assert!(
        status.success(),
        "expected zero exit status, got {status:?}"
    );
}
