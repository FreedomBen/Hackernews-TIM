//! docs/planning/TEST_PLAN.md §3.2.9 — custom keymap from TOML (Linux-only).
//!
//! Pre-write a config TOML with a `[[keymap.custom_keymaps]]` block
//! bound to `Z`, `tag = "ask_hn"`, `by_date = false`. The binary
//! starts with the front page (StorySortMode::None for `front_page`),
//! we wait for it to render, then press `Z`. The custom-keymap
//! handler in `view::set_up_global_callbacks` reads `by_date = false`
//! as `StorySortMode::Points`, so the new view's nav strip shows
//! `3.ask_hn (by_point)` — a unique substring proving:
//!
//! 1. The TOML block was parsed and registered at `init_ui` time.
//! 2. Pressing `Z` dispatched to `construct_and_add_new_story_view`
//!    with `tag = "ask_hn"` and `sort_mode = Points`.
//!
//! As a belt-and-suspenders check we also assert the fake server saw
//! a `GET /api/v1/search?tags=ask_hn` request — proving the same fact
//! from the HTTP side.
//!
//! Same `news.ycombinator.com` caveat as §3.2.1 / §3.2.b applies for
//! the unauthenticated front-page render's vote-state probe.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, SpawnOptions, TestDirs, DEFAULT_WAIT};

const FRONT_PAGE_STORY_ID: u32 = 60001;
const FRONT_PAGE_STORY_TITLE: &str = "front-page fixture for custom keymap";
const ASK_HN_STORY_ID: u32 = 60002;
const ASK_HN_STORY_TITLE: &str = "ask hn fixture for custom keymap";

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const TAG_SWITCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Drop a config TOML at the resolved `--config` path with one
/// `[[keymap.custom_keymaps]]` block bound to `Z` (tag `ask_hn`,
/// sort `Points`). Pre-writing the file skips `prompt_for_flavor`,
/// so only the auth prompt fires at startup.
fn write_test_config_with_custom_keymap(dirs: &TestDirs) {
    let path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).expect("create config dir");
    // `numeric_filters` has no `#[serde(default)]` on its parent so
    // we have to provide it; an empty inline table is enough since
    // every interval is `#[serde(default)]`.
    let toml = "\
[[keymap.custom_keymaps]]
key = \"Z\"
tag = \"ask_hn\"
by_date = false
numeric_filters = { elapsed_days_interval = {}, points_interval = {}, num_comments_interval = {} }
";
    std::fs::write(&path, toml).expect("write config.toml");
}

fn fixture_hit(id: u32, title: &str) -> serde_json::Value {
    json!({
        "objectID": id.to_string(),
        "author": "alice",
        "url": format!("https://example.com/{id}"),
        "story_text": null,
        "points": 100,
        "num_comments": 5,
        "created_at_i": 1_700_000_000_u64,
        "_highlightResult": { "title": { "value": title } },
        "dead": false,
        "flagged": false,
    })
}

/// Mount enough of the fake HN backend that:
///
/// * The initial front-page render (`StorySortMode::None` for
///   `"front_page"`) succeeds — that path fetches
///   `/v0/topstories.json` for IDs and `/api/v1/search?tags=story,(story_<id>,)`
///   for the bodies, and `reorder_stories_based_on_ids` panics on a
///   hit whose id isn't in the topstories list, so we list both
///   fixture IDs there.
/// * The post-`Z` ask_hn render (`StorySortMode::Points`) succeeds —
///   that path goes straight to `/api/v1/search?tags=ask_hn` and
///   returns the hits as-is, no Firebase round-trip.
fn mount_endpoints(server: &FakeHnServer) {
    server.mount_get_json(
        "/v0/topstories.json",
        200,
        json!([FRONT_PAGE_STORY_ID, ASK_HN_STORY_ID]),
    );
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [
                fixture_hit(FRONT_PAGE_STORY_ID, FRONT_PAGE_STORY_TITLE),
                fixture_hit(ASK_HN_STORY_ID, ASK_HN_STORY_TITLE),
            ]
        }),
    );
}

#[test]
fn custom_keymap_z_opens_ask_hn_story_view() {
    let server = FakeHnServer::start();
    mount_endpoints(&server);

    let dirs = TestDirs::new().expect("TestDirs::new");
    write_test_config_with_custom_keymap(&dirs);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base())
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    // Config is pre-written, so `prompt_for_flavor` does not fire.
    // No auth file exists, so `prompt_for_auth` does fire — decline.
    handle
        .wait_for_text("No auth file found", DEFAULT_WAIT)
        .expect("auth prompt should print since no auth file exists");
    handle.send_keys("\n").expect("decline auth prompt");

    // Wait for the front page to render. Active tag = `1.front_page`,
    // sort = `None` -> no `(by_*)` suffix.
    handle
        .wait_for_text(FRONT_PAGE_STORY_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("front-page fixture should render before pressing Z");

    let pre_screen = handle.screen();
    assert!(
        pre_screen.contains("1.front_page"),
        "front_page tag should be present in nav strip before Z; saw:\n{pre_screen}"
    );
    assert!(
        !pre_screen.contains("(by_point)"),
        "no by_point suffix should be visible on the front page; saw:\n{pre_screen}"
    );

    // Cursive can drop the first key if it arrives the same instant
    // raw mode engages — mirror the small sleep used in §3.2.6 / §3.2.7.
    std::thread::sleep(Duration::from_millis(200));
    handle.send_keys("Z").expect("send Z (custom keymap)");

    // The new story view's nav strip highlights the active tag with
    // a `(by_point)` suffix when sort_mode = Points (see
    // `construct_story_view_top_bar`). `3.ask_hn (by_point)` is the
    // active label per `STORY_TAGS` (front_page=1, story=2, ask_hn=3).
    handle
        .wait_for_text("3.ask_hn (by_point)", TAG_SWITCH_TIMEOUT)
        .expect("ask_hn (by_point) active tag label should appear after Z");

    let post_screen = handle.screen();
    assert!(
        post_screen.contains("3.ask_hn (by_point)"),
        "expected '3.ask_hn (by_point)' active tag after Z; saw:\n{post_screen}"
    );

    // Belt-and-suspenders: the binary should have hit Algolia with
    // `tags=ask_hn` (the get_stories_by_tag path for sort=Points).
    let ask_hn_search_requests: Vec<_> = server
        .received_requests()
        .into_iter()
        .filter(|r| {
            r.method.as_str() == "GET"
                && r.url.path() == "/api/v1/search"
                && r.url.query().unwrap_or("").contains("tags=ask_hn")
        })
        .collect();
    assert!(
        !ask_hn_search_requests.is_empty(),
        "expected at least one /api/v1/search?tags=ask_hn request after Z; \
         got requests: {:?}",
        server
            .received_requests()
            .iter()
            .map(|r| (r.method.as_str().to_string(), r.url.to_string()))
            .collect::<Vec<_>>()
    );

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");
}
