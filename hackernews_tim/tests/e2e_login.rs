//! docs/planning/TEST_PLAN.md §3.2.6 — in-app login flow against a fake HN
//! backend (Linux-only).
//!
//! Scenario:
//!
//! 1. Spawn the binary with isolated `HOME` / `XDG_*` and all three
//!    base URLs (`HN_ALGOLIA_BASE`, `HN_FIREBASE_BASE`,
//!    `HN_NEWS_BASE`) pointing at an in-process [`FakeHnServer`].
//!    Dismiss the first-run flavor prompt; decline the auth prompt
//!    (we're going to log in from inside the TUI instead).
//! 2. Wait for the front-page render so Cursive is in raw mode and
//!    the global `open_login_dialog` keybinding is active.
//! 3. Press the default `L` (open_login_dialog) to surface the login
//!    dialog. Type the username, Tab to the password field, type the
//!    password, then Tab×2 to focus the "Log in" button and Enter to
//!    activate.
//! 4. The fake server's `/login` POST returns 200 with a body
//!    containing `href="logout` (so `classify_login_response` treats
//!    the response as success) and a `Set-Cookie: user=<token>`
//!    header. The binary's cookie jar records the cookie under the
//!    fake host, so [`HNClient::current_session_cookie`] can read it
//!    back when the dialog persists the auth file.
//! 5. Assert the binary surfaces a "Login successful" / "Logged in
//!    as alice" info dialog, then confirm the auth file on disk
//!    contains the typed credentials and the captured session cookie.
//! 6. Quit cleanly.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, AppHandle, SpawnOptions, DEFAULT_WAIT};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const LOGIN_RESULT_TIMEOUT: Duration = Duration::from_secs(10);

const FIXTURE_USERNAME: &str = "alice";
const FIXTURE_PASSWORD: &str = "hunter2";
const FIXTURE_SESSION_COOKIE: &str = "alice&abcdef0123";

const STORY_ID: u32 = 30001;
const STORY_TITLE: &str = "login fixture story";

/// Mount the topstories / search endpoints so the front page renders
/// with one fixture row before the user opens the login dialog.
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

/// Step the binary through the first-run prompts. Decline the auth
/// prompt — the test logs in from inside the TUI rather than at
/// startup.
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
fn login_dialog_succeeds_against_fake_backend() {
    let server = FakeHnServer::start();
    mount_front_page(&server);

    // The binary POSTs `acct=&pw=` to `/login`; respond with a body
    // carrying `href="logout` (the marker `classify_login_response`
    // treats as a success page) and a `Set-Cookie: user=...` header
    // that the binary's cookie jar will pick up. ureq follows the
    // 200 response without redirecting, so the body — not a chained
    // GET — is what's classified.
    server.mount_post_with_user_cookie(
        "/login",
        200,
        "<html><body><a href=\"logout?goto=news&auth=abc\">logout</a></body></html>",
        FIXTURE_SESSION_COOKIE,
    );

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base());
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    let auth_file = handle
        .dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("hn-auth.toml");

    dismiss_first_run_prompts(&mut handle);

    handle
        .wait_for_text(STORY_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("front-page fixture should render before login dialog opens");

    // `L` opens the login dialog (default `open_login_dialog`
    // binding in `GlobalKeyMap::default`). Cursive sometimes drops a
    // key sent the same instant the post-event hooks finish wiring
    // up; the small sleep avoids the race without polling.
    std::thread::sleep(Duration::from_millis(200));
    handle.send_keys("L").expect("send L (open_login_dialog)");

    handle
        .wait_for_text("Log in to Hacker News", DEFAULT_WAIT)
        .expect("login dialog title should appear");

    // docs/planning/TEST_PLAN.md §3.2.6 acceptance: PTY-rendered login dialog snapshot
    // — empty username/password fields, taken before any text is typed.
    // Brief sleep lets the dialog finish drawing before capture.
    std::thread::sleep(Duration::from_millis(150));
    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("login_dialog_pty", handle.screen());
    });

    // Initial focus is the username EditView. Tab order is:
    // username → password → Cancel → Log in. Type each field then
    // Tab twice from password to land on "Log in".
    handle.send_keys(FIXTURE_USERNAME).expect("type username");
    handle.send_keys("\t").expect("Tab to password field");
    handle.send_keys(FIXTURE_PASSWORD).expect("type password");
    handle.send_keys("\t\t").expect("Tab to Log in button");
    handle.send_keys("\r").expect("Enter activates Log in");

    handle
        .wait_for_text(
            &format!("Logged in as {FIXTURE_USERNAME}"),
            LOGIN_RESULT_TIMEOUT,
        )
        .expect("post-login info dialog should announce the username");

    let success_screen = handle.screen();
    assert!(
        success_screen.contains("Login successful"),
        "expected the success info dialog title; saw:\n{success_screen}"
    );
    assert!(
        !success_screen.contains("Log in to Hacker News"),
        "login dialog should be popped on success; saw:\n{success_screen}"
    );

    // The fake server should have observed exactly one POST /login
    // carrying `acct=alice&pw=hunter2`.
    let login_requests: Vec<_> = server
        .received_requests()
        .into_iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/login")
        .collect();
    assert_eq!(
        login_requests.len(),
        1,
        "expected exactly one /login POST; got {}",
        login_requests.len()
    );
    let body =
        std::str::from_utf8(&login_requests[0].body).expect("login form body should be UTF-8");
    assert!(
        body.contains(&format!("acct={FIXTURE_USERNAME}")),
        "expected acct={FIXTURE_USERNAME} in form body; saw {body:?}"
    );
    assert!(
        body.contains(&format!("pw={FIXTURE_PASSWORD}")),
        "expected pw={FIXTURE_PASSWORD} in form body; saw {body:?}"
    );

    // Auth file persisted with the typed credentials and the cookie
    // captured from `Set-Cookie: user=<value>`. Read directly via
    // toml — the `Auth` struct itself is not exposed for tests.
    assert!(
        auth_file.exists(),
        "expected auth file at {}",
        auth_file.display()
    );
    let auth_text = std::fs::read_to_string(&auth_file).expect("auth file should be readable");
    let auth: toml::Value = toml::from_str(&auth_text).expect("auth file should parse as TOML");
    assert_eq!(
        auth.get("username").and_then(|v| v.as_str()),
        Some(FIXTURE_USERNAME),
        "auth.username; full file:\n{auth_text}"
    );
    assert_eq!(
        auth.get("password").and_then(|v| v.as_str()),
        Some(FIXTURE_PASSWORD),
        "auth.password; full file:\n{auth_text}"
    );
    assert_eq!(
        auth.get("session").and_then(|v| v.as_str()),
        Some(FIXTURE_SESSION_COOKIE),
        "auth.session should match the cookie set by /login; full file:\n{auth_text}"
    );

    // Dismiss the success info dialog (Esc → close_dialog) before
    // sending the quit key, so `q` reaches the front-page story view
    // rather than getting absorbed by the dialog.
    handle
        .send_keys("\x1b")
        .expect("send Esc to close info dialog");
    std::thread::sleep(Duration::from_millis(150));

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");
}
