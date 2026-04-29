//! TEST_PLAN.md §3.2.7-§3.2.8 — vote / reply happy paths against
//! a fake HN backend (Linux-only).
//!
//! Both scenarios pre-write a config + auth file with a session
//! cookie under the test's isolated `XDG_CONFIG_HOME` so the binary
//! boots straight into the front-page story view (no flavor / no
//! auth prompt). `verify_session` is satisfied by mounting `/news`
//! with an HTML body containing `href="logout"`; the same mount
//! also covers `get_listing_vote_state` for the front page tag.
//!
//! 3.2.7: press the upvote key on the focused story; assert the
//! background thread issues `GET /vote?id=<id>&how=up&auth=<token>`
//! to the fake server. The auth token is recovered from a hand-built
//! `<a id='up_<id>' ... auth=<token>>` anchor served at `/item`,
//! matching the regex in `parse_vote_data_from_content`.
//!
//! 3.2.8: press the reply key, then a stub editor (passed via
//! `VISUAL`) overwrites the scaffold with a fixed body. Assert the
//! binary's resulting `POST /comment` carries `parent`, `hmac`,
//! `goto`, and `text` matching the form scraped from `/item`.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::{Duration, Instant};

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, SpawnOptions, TestDirs};

const STORY_ID: u32 = 30001;
const STORY_TITLE: &str = "vote fixture story";
const FIXTURE_USERNAME: &str = "alice";
const FIXTURE_PASSWORD: &str = "hunter2";
const FIXTURE_SESSION: &str = "alice&deadbeef0123";
const FIXTURE_AUTH_TOKEN: &str = "abc123def";
const FIXTURE_HMAC: &str = "rephmac1";
const REPLY_BODY: &str = "Test reply body";

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const VOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REPLY_POST_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Drop a minimal config TOML at the resolved `--config` path so
/// `prompt_for_flavor` doesn't fire on startup. Every `Config` field
/// is optional (per the `ConfigParse` derive), so an empty file
/// merges cleanly over `Config::default()`.
fn write_test_config(dirs: &TestDirs) {
    let path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).expect("create config dir");
    std::fs::write(&path, "# minimal e2e config\n").expect("write config.toml");
}

/// Drop an auth TOML pointing at the file backend with a pre-canned
/// session cookie. The binary's `attempt_login` path takes the
/// `with_cached_session` branch, hits `/news` once for
/// `verify_session`, and skips the `/login` POST entirely.
fn write_test_auth(dirs: &TestDirs) {
    let path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("hn-auth.toml");
    std::fs::create_dir_all(path.parent().unwrap()).expect("create auth dir");
    let toml_str = format!(
        "storage = \"file\"\nusername = \"{FIXTURE_USERNAME}\"\npassword = \"{FIXTURE_PASSWORD}\"\nsession = \"{FIXTURE_SESSION}\"\n"
    );
    std::fs::write(&path, toml_str).expect("write hn-auth.toml");
}

/// Mount `/v0/topstories.json` + `/api/v1/search` to render exactly
/// one fixture story on the front page, and `/news` so both
/// `verify_session` (looks for `href="logout"`) and
/// `get_listing_vote_state` (parsed for vote arrows; an empty match
/// set is fine) succeed.
fn mount_front_page(server: &FakeHnServer) {
    server.mount_get_json("/v0/topstories.json", 200, json!([STORY_ID]));
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [{
                "objectID": STORY_ID.to_string(),
                "author": FIXTURE_USERNAME,
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
    server.mount_get_text(
        "/news",
        200,
        "<html><body><a href=\"logout?goto=news&amp;auth=zzz\">logout</a></body></html>",
    );
    // Login fallback in case `verify_session` fails for any reason —
    // mirrors the success body shape used by `e2e_login.rs`.
    server.mount_post_with_user_cookie(
        "/login",
        200,
        "<html><body><a href=\"logout?goto=news&amp;auth=zzz\">logout</a></body></html>",
        FIXTURE_SESSION,
    );
}

/// Mount `/item` returning HTML that `parse_vote_data_from_content`
/// can scrape: an `<a id='up_<id>' ... auth=<token>>` anchor (the
/// upvote regex). The lazy per-item fetch in
/// `StoryView::apply_vote` hits this on every keypress, regardless
/// of whether listing vote state was pre-populated.
fn mount_item_with_vote_data(server: &FakeHnServer) {
    let html = format!(
        "<html><body>\n\
         <a href=\"logout?goto=news&amp;auth=zzz\">logout</a>\n\
         <a id='up_{STORY_ID}' href='vote?id={STORY_ID}&amp;how=up&amp;auth={FIXTURE_AUTH_TOKEN}'>up</a>\n\
         </body></html>"
    );
    server.mount_get_text("/item", 200, html);
}

/// Mount `/vote` so the binary's `GET /vote?id=...&how=up&auth=...`
/// gets a 200 (the response body is unused). The request itself is
/// what we assert on via `received_requests`.
fn mount_vote_endpoint(server: &FakeHnServer) {
    server.mount_get_text("/vote", 200, "");
}

/// Mount `/item` returning HTML containing both a vote-data anchor
/// (so `parse_vote_data_from_content` resolves an auth token) AND a
/// reply form with a hidden `hmac` input that `parse_reply_form`
/// scrapes via `extract_hidden_input`. Used by §3.2.8 — the binary
/// fetches `/item` once for the hmac before POSTing `/comment`, and
/// again when Cursive re-opens the comment view after `wait_for_enter`.
fn mount_item_with_reply_form(server: &FakeHnServer) {
    let html = format!(
        "<html><body>\n\
         <a href=\"logout?goto=news&amp;auth=zzz\">logout</a>\n\
         <a id='up_{STORY_ID}' href='vote?id={STORY_ID}&amp;how=up&amp;auth={FIXTURE_AUTH_TOKEN}'>up</a>\n\
         <form action=\"comment\" method=\"post\">\n\
         <input type=\"hidden\" name=\"parent\" value=\"{STORY_ID}\">\n\
         <input type=\"hidden\" name=\"goto\" value=\"item?id={STORY_ID}\">\n\
         <input type=\"hidden\" name=\"hmac\" value=\"{FIXTURE_HMAC}\">\n\
         <textarea name=\"text\"></textarea>\n\
         </form>\n\
         </body></html>"
    );
    server.mount_get_text("/item", 200, html);
}

/// Mount `/comment` so `post_reply`'s POST gets a body that
/// `classify_post_reply_response` treats as success (none of the
/// HN error markers present).
fn mount_comment_endpoint(server: &FakeHnServer) {
    server.mount_post_text("/comment", 200, "<html><body>posted</body></html>");
}

/// Resolve the absolute path to the stub editor checked into the
/// repo at `tests/e2e/scripts/reply_stub.sh`. Cargo runs tests with
/// `CARGO_MANIFEST_DIR` set to the crate root, so this works from
/// any CWD.
fn reply_stub_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/e2e/scripts/reply_stub.sh")
}

#[test]
fn vote_happy_path_sends_get_to_vote_endpoint() {
    let server = FakeHnServer::start();
    mount_front_page(&server);
    mount_item_with_vote_data(&server);
    mount_vote_endpoint(&server);

    let dirs = TestDirs::new().expect("TestDirs::new");
    write_test_config(&dirs);
    write_test_auth(&dirs);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base())
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app");

    handle
        .wait_for_text(STORY_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("front-page fixture should render");

    // Cursive sometimes drops a key sent the same instant raw mode
    // engages. Mirror the small sleep used in `e2e_login.rs`.
    std::thread::sleep(Duration::from_millis(200));
    handle
        .send_keys("v")
        .expect("send v (default upvote keybinding)");

    let deadline = Instant::now() + VOTE_REQUEST_TIMEOUT;
    let mut last_paths: Vec<String> = Vec::new();
    let query = loop {
        let requests = server.received_requests();
        if let Some(req) = requests
            .iter()
            .find(|r| r.method.as_str() == "GET" && r.url.path() == "/vote")
        {
            break req.url.query().unwrap_or("").to_string();
        }
        if Instant::now() >= deadline {
            last_paths = requests
                .iter()
                .map(|r| format!("{} {}", r.method, r.url.path()))
                .collect();
            break String::new();
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    assert!(
        !query.is_empty(),
        "timed out after {VOTE_REQUEST_TIMEOUT:?} waiting for GET /vote; \
         observed requests: {last_paths:?}\n--- screen ---\n{}",
        handle.screen()
    );
    assert!(
        query.contains(&format!("id={STORY_ID}")),
        "vote URL should carry id={STORY_ID}; saw query: {query:?}"
    );
    assert!(
        query.contains("how=up"),
        "vote URL should carry how=up; saw query: {query:?}"
    );
    assert!(
        query.contains(&format!("auth={FIXTURE_AUTH_TOKEN}")),
        "vote URL should carry auth={FIXTURE_AUTH_TOKEN}; saw query: {query:?}"
    );

    let status = handle.shutdown().expect("clean exit");
    assert!(status.success(), "expected success exit, got {status:?}");
}

#[test]
fn reply_happy_path_posts_to_comment_endpoint() {
    let server = FakeHnServer::start();
    mount_front_page(&server);
    mount_item_with_reply_form(&server);
    mount_comment_endpoint(&server);

    let stub = reply_stub_path();
    assert!(
        stub.exists(),
        "stub editor script missing at {} — `tests/e2e/scripts/reply_stub.sh` must be checked in with the +x bit set",
        stub.display()
    );

    let dirs = TestDirs::new().expect("TestDirs::new");
    write_test_config(&dirs);
    write_test_auth(&dirs);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base())
        .env("VISUAL", &stub)
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app");

    handle
        .wait_for_text(STORY_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("front-page fixture should render");

    // Sleep mirrors the post-render race avoidance in §3.2.6 / §3.2.7.
    std::thread::sleep(Duration::from_millis(200));
    handle
        .send_keys("r")
        .expect("send r (default story-view reply keybinding)");

    // Cursive quits, the stub editor runs, post_reply succeeds, and
    // main prints the success line before blocking on wait_for_enter.
    handle
        .wait_for_text(
            &format!("Reply posted to item {STORY_ID}"),
            REPLY_POST_TIMEOUT,
        )
        .expect("binary should announce a successful reply");

    // Dismiss wait_for_enter; binary re-opens Cursive at the
    // comment view (we don't assert on its render — the §3.2.8
    // contract is the POST itself).
    handle.send_keys("\n").expect("dismiss wait_for_enter");

    // Inspect the recorded /comment POST. Body is form-urlencoded:
    // "Test reply body" → "Test+reply+body".
    let requests = server.received_requests();
    let comment_posts: Vec<_> = requests
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/comment")
        .collect();
    assert_eq!(
        comment_posts.len(),
        1,
        "expected exactly one POST /comment; observed: {:?}",
        requests
            .iter()
            .map(|r| format!("{} {}", r.method, r.url.path()))
            .collect::<Vec<_>>()
    );
    let body = std::str::from_utf8(&comment_posts[0].body).expect("/comment body should be UTF-8");
    assert!(
        body.contains(&format!("parent={STORY_ID}")),
        "POST /comment should carry parent={STORY_ID}; saw body: {body:?}"
    );
    assert!(
        body.contains(&format!("hmac={FIXTURE_HMAC}")),
        "POST /comment should carry hmac={FIXTURE_HMAC}; saw body: {body:?}"
    );
    assert!(
        body.contains("goto=item"),
        "POST /comment should carry goto=item?id=...; saw body: {body:?}"
    );
    // ureq's `send_form` percent-encodes spaces as `+`.
    let encoded_body = REPLY_BODY.replace(' ', "+");
    assert!(
        body.contains(&format!("text={encoded_body}")),
        "POST /comment should carry text={encoded_body:?}; saw body: {body:?}"
    );

    let status = handle.shutdown().expect("clean exit");
    assert!(status.success(), "expected success exit, got {status:?}");
}
