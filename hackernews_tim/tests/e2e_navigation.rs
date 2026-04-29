//! TEST_PLAN.md §3.2.b — front-page navigation and comment view
//! drilldown (Linux-only). Covers scenarios 3.2.2 and 3.2.3.
//!
//! See [`e2e_first_run.rs`] for the surrounding harness conventions
//! (TTY-gated flavor / auth prompts, `HOME` / `XDG_*` isolation,
//! `HN_ALGOLIA_BASE` / `HN_FIREBASE_BASE` / `HN_NEWS_BASE`
//! overrides). All three base URLs point at the in-process
//! [`FakeHnServer`], so no traffic escapes to the real HN hosts.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, AppHandle, SpawnOptions, DEFAULT_WAIT};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);

// Fixture story IDs. Both the Algolia (`/api/v1/...`) and Firebase
// (`/v0/item/...json`) endpoints are mocked, and `HN_NEWS_BASE`
// routes the binary's `news.ycombinator.com` traffic
// (`/item?id=<id>` for vote-state HTML) at the same fake server, so
// these IDs need not collide with real HN items.
const STORY1_ID: u32 = 10001;
const STORY2_ID: u32 = 10002;
const STORY3_ID: u32 = 10003;

const STORY1_TITLE: &str = "navigation fixture story one";
const STORY2_TITLE: &str = "navigation fixture story two";
const STORY3_TITLE: &str = "navigation fixture story three";

/// Mount the `/v0/topstories.json` + `/api/v1/search` endpoints with
/// three fixture stories. Returns the handle so individual tests can
/// add further mocks (e.g. per-item Firebase responses for §3.2.3).
fn mount_three_stories(server: &FakeHnServer) {
    server.mount_get_json(
        "/v0/topstories.json",
        200,
        json!([STORY1_ID, STORY2_ID, STORY3_ID]),
    );
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [
                fixture_hit(STORY1_ID, STORY1_TITLE, "alice", 250, 12),
                fixture_hit(STORY2_ID, STORY2_TITLE, "bob", 150, 7),
                fixture_hit(STORY3_ID, STORY3_TITLE, "carol", 90, 3),
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

/// Step the binary through the first-run prompts so the subsequent
/// front-page render is the only thing under test.
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
fn front_page_navigates_with_j_k_and_ctrl_d() {
    let server = FakeHnServer::start();
    mount_three_stories(&server);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    dismiss_first_run_prompts(&mut handle);

    handle
        .wait_for_text(STORY1_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("first fixture story should render");

    // All three rows should be visible at once on a 120×40 screen.
    let initial_screen = handle.screen();
    for title in [STORY1_TITLE, STORY2_TITLE, STORY3_TITLE] {
        assert!(
            initial_screen.contains(title),
            "expected {title:?} on the rendered front page; saw:\n{initial_screen}"
        );
    }

    // TEST_PLAN.md §3.2.2 acceptance: PTY-rendered front-page snapshot.
    // Filter "X units ago" since `created_at_i` is a fixed epoch but the
    // displayed offset drifts as wall-clock time advances.
    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("front_page_three_stories_pty", initial_screen);
    });

    let row_story1 = handle
        .focused_row()
        .expect("front page should have a focused row at startup");

    // `j` — next_story.
    handle.send_keys("j").expect("send j");
    let row_story2 = wait_for_focus_change(&handle, row_story1);
    assert!(
        row_story2 > row_story1,
        "j should advance focus from row {row_story1} (saw {row_story2})\nscreen:\n{}",
        handle.screen()
    );

    // Second `j` lands on story 3.
    handle.send_keys("j").expect("send j");
    let row_story3 = wait_for_focus_change(&handle, row_story2);
    assert!(
        row_story3 > row_story2,
        "second j should advance focus from row {row_story2} (saw {row_story3})"
    );

    // `k` — prev_story. Should return to story 2's row exactly.
    handle.send_keys("k").expect("send k");
    let row_after_k = wait_for_focus_change(&handle, row_story3);
    assert_eq!(
        row_after_k,
        row_story2,
        "k should move focus back to row {row_story2} (saw {row_after_k})\n--- screen ---\n{}",
        handle.screen()
    );

    // `Ctrl-D` — page_down. With only three stories the half-page jump
    // saturates at the last entry; we just assert focus didn't slide
    // backwards.
    handle.send_keys("\x04").expect("send Ctrl-D");
    std::thread::sleep(Duration::from_millis(200));
    let row_after_ctrl_d = handle
        .focused_row()
        .expect("focus row should still resolve after Ctrl-D");
    assert!(
        row_after_ctrl_d >= row_after_k,
        "Ctrl-D should not move focus backwards (was {row_after_k}, now {row_after_ctrl_d})"
    );

    // Front page must still render every fixture story after the
    // navigation sequence — guards against a crash that wiped the
    // screen mid-test.
    let final_screen = handle.screen();
    for title in [STORY1_TITLE, STORY2_TITLE, STORY3_TITLE] {
        assert!(
            final_screen.contains(title),
            "expected {title:?} to remain visible after navigation; saw:\n{final_screen}"
        );
    }

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");
}

/// Poll [`AppHandle::focused_row`] until it differs from `prior` or
/// 1 s elapses. Cursive redraws on the next tick after the key event,
/// so a same-row reading immediately after `send_keys` is expected.
fn wait_for_focus_change(handle: &AppHandle, prior: u16) -> u16 {
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    loop {
        if let Some(now) = handle.focused_row() {
            if now != prior {
                return now;
            }
        }
        if std::time::Instant::now() >= deadline {
            return handle.focused_row().unwrap_or(prior);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn drill_into_comments_and_back_to_front_page() {
    let server = FakeHnServer::start();
    mount_three_stories(&server);

    // The comment view's `get_page_data` first hits the official
    // Firebase `/v0/item/{id}.json` route to load the root item.
    // `kids: []` keeps the test simple — no per-comment Algolia mocks
    // needed.
    server.mount_get_json(
        "/v0/item/10001.json",
        200,
        json!({
            "id": STORY1_ID,
            "type": "story",
            "by": "alice",
            "title": STORY1_TITLE,
            "url": format!("https://example.com/{STORY1_ID}"),
            "score": 250,
            "descendants": 0,
            "time": 1_700_000_000_u64,
            "kids": [],
            "text": "",
        }),
    );

    // The unauthenticated comment-view path also fetches the rendered
    // HN HTML page via `get_page_content` to scrape vote state. An
    // empty `<body>` is enough — `parse_vote_data_from_content` finds
    // no `<a id='up_...'>` anchors and returns an empty map, while
    // `lazy_load_comments` loads from the (empty) `kids` list.
    server.mount_get_text("/item", 200, "<html><body></body></html>");

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    dismiss_first_run_prompts(&mut handle);

    handle
        .wait_for_text(STORY1_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("first fixture story should render on the front page");

    // Sanity check: the focused row is on the first story so Enter
    // will drill into STORY1_ID (mocked at /v0/item/10001.json).
    handle
        .focused_row()
        .expect("focus must be drawn before Enter dispatches goto_story_comment_view");

    // Enter — `goto_story_comment_view` (StoryViewKeyMap default).
    handle.send_keys("\r").expect("send Enter");

    // Comment view title bar reads "Comment View - <story title>".
    handle
        .wait_for_text(
            &format!("Comment View - {STORY1_TITLE}"),
            FRONT_PAGE_RENDER_TIMEOUT,
        )
        .expect("comment view header should show the drilled story title");

    // `goto_previous_view` defaults to `Backspace` or `Ctrl-P`. Give the
    // async comment view a beat to finish wiring its post-event hooks
    // after the title bar appears, otherwise the back-nav key races the
    // initial draw and gets swallowed.
    std::thread::sleep(Duration::from_millis(200));
    handle
        .send_keys("\x7f")
        .expect("send Backspace (goto_previous_view)");

    handle
        .wait_for_text(STORY2_TITLE, DEFAULT_WAIT)
        .expect("front page (story 2 row) should be visible again after going back");

    // The other fixture rows should still be present — the front
    // page was restored, not just the focused row.
    let final_screen = handle.screen();
    for title in [STORY2_TITLE, STORY3_TITLE] {
        assert!(
            final_screen.contains(title),
            "expected {title:?} on the restored front page; saw:\n{final_screen}"
        );
    }
    assert!(
        !final_screen.contains("Comment View - "),
        "comment view title should be gone after Backspace; saw:\n{final_screen}"
    );

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");
}
