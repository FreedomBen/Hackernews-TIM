//! docs/planning/TEST_PLAN.md §3.2.4 — search view workflow (Linux-only).
//!
//! Scenario:
//!
//! 1. Spawn the binary with isolated `HOME` / `XDG_*` and `HN_*_BASE`
//!    pointing at a [`FakeHnServer`]. Dismiss the first-run flavor
//!    and auth prompts the same way [`e2e_first_run.rs`] /
//!    [`e2e_navigation.rs`] do.
//! 2. Wait for the front-page render so we know Cursive is up and
//!    raw mode is on (otherwise the PTY's IXON would swallow Ctrl-S).
//! 3. Send the configured `goto_search_view` key (default Ctrl-S) and
//!    wait for the search view's centered "Search View" description
//!    plus the "Search:" bar prefix — neither appears on the story
//!    view top bar, so they are an unambiguous "search view opened"
//!    signal.
//! 4. The freshly-opened search view starts with an empty inner
//!    `StoryView` (`vec![]` is passed to
//!    `story_view::construct_story_main_view`), so no fixture rows
//!    are visible yet. Typing the first character fires
//!    `retrieve_matched_stories` against the fake `/api/v1/search`
//!    endpoint; the fixture hits then re-render in the inner view.
//! 5. Press `Esc` (default `to_navigation_mode`) and assert the
//!    search bar is no longer the focused input by typing another
//!    character and confirming the search bar text didn't grow.
//!    This catches a regression where the mode-switch keymap
//!    silently fell through.
//! 6. Quit cleanly.
//!
//! Same `news.ycombinator.com` caveat as §3.2.1 / §3.2.b applies for
//! the unauthenticated front-page render's vote-state probe; the
//! search results path itself is fully covered by `wiremock`.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, AppHandle, SpawnOptions, DEFAULT_WAIT};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const SEARCH_RENDER_TIMEOUT: Duration = Duration::from_secs(10);

// IDs distinct from the navigation suite so a future cross-test
// regression is easier to localise. `wiremock` matches on `path` only,
// so the same `/api/v1/search` mock answers both the front-page
// bootstrap and the typed-query search request.
const STORY1_ID: u32 = 20001;
const STORY2_ID: u32 = 20002;

const STORY1_TITLE: &str = "search fixture story alpha";
const STORY2_TITLE: &str = "search fixture story beta";

fn mount_search_fixtures(server: &FakeHnServer) {
    server.mount_get_json("/v0/topstories.json", 200, json!([STORY1_ID, STORY2_ID]));
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [
                fixture_hit(STORY1_ID, STORY1_TITLE, "alice", 200, 8),
                fixture_hit(STORY2_ID, STORY2_TITLE, "bob", 90, 4),
            ]
        }),
    );
}

fn fixture_hit(
    id: u32,
    title: &str,
    author: &str,
    points: u32,
    num_comments: u32,
) -> serde_json::Value {
    json!({
        "objectID": id.to_string(),
        "author": author,
        "url": format!("https://example.com/{id}"),
        "story_text": null,
        "points": points,
        "num_comments": num_comments,
        "created_at_i": 1_700_000_000_u64,
        "_highlightResult": { "title": { "value": title } },
        "dead": false,
        "flagged": false,
    })
}

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
fn search_view_opens_and_renders_typed_query_results() {
    let server = FakeHnServer::start();
    mount_search_fixtures(&server);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    dismiss_first_run_prompts(&mut handle);

    // Front page must render before we send Ctrl-S — Cursive flips the
    // PTY out of cooked mode (which would otherwise eat ^S as XOFF)
    // during its first event-loop tick, and that's only guaranteed
    // once a draw has happened.
    handle
        .wait_for_text(STORY1_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("front-page fixture story should render before opening search");

    // Ctrl-S — `goto_search_view` (GlobalKeyMap default).
    handle.send_keys("\x13").expect("send Ctrl-S");

    // The centered "Search View" description row only appears on the
    // search view's title bar; the front page's story-view top bar
    // has no description row. The "Search:" prefix likewise lives
    // only inside the search bar.
    handle
        .wait_for_text("Search View", SEARCH_RENDER_TIMEOUT)
        .expect("search view title should render after Ctrl-S");
    handle
        .wait_for_text("Search:", SEARCH_RENDER_TIMEOUT)
        .expect("search bar prefix should be visible in the search view");

    // Empty inner StoryView right after open: typing the first char
    // is what fires `retrieve_matched_stories`. Send a multi-char
    // query so we cover both the initial-keystroke trigger and the
    // per-keystroke re-fetch.
    handle.send_keys("rust").expect("type search query");

    handle
        .wait_for_text(STORY1_TITLE, SEARCH_RENDER_TIMEOUT)
        .expect("first fixture hit should render in the search results");
    handle
        .wait_for_text(STORY2_TITLE, SEARCH_RENDER_TIMEOUT)
        .expect("second fixture hit should render in the search results");

    let after_query = handle.screen();
    assert!(
        after_query.contains("Search View"),
        "search view title should still be visible after results render; saw:\n{after_query}"
    );
    assert!(
        after_query.contains("rust"),
        "the typed query should be visible in the search bar; saw:\n{after_query}"
    );

    // docs/planning/TEST_PLAN.md §3.2.4 acceptance: PTY-rendered search-view snapshot
    // with the typed query visible and both fixture hits in the results.
    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("search_view_with_results_pty", after_query);
    });

    // Esc — `to_navigation_mode` (SearchViewKeyMap default). After the
    // switch, character keys should no longer feed the search bar; they
    // route to the inner StoryView (where they're either bound — `j`/`k`
    // navigate stories — or ignored). We type `z` (unbound everywhere)
    // and assert it does NOT show up appended to "rust".
    handle
        .send_keys("\x1b")
        .expect("send Esc (to_navigation_mode)");
    std::thread::sleep(Duration::from_millis(200));
    handle.send_keys("z").expect("send z");
    std::thread::sleep(Duration::from_millis(200));

    let after_esc = handle.screen();
    assert!(
        !after_esc.contains("rustz"),
        "Esc should have left search mode so 'z' does not extend the query; saw:\n{after_esc}"
    );
    assert!(
        after_esc.contains("Search View"),
        "search view should still be the active view after Esc; saw:\n{after_esc}"
    );

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");
}
