//! TEST_PLAN.md §3.2.5 — article view + reader mode + link dialog
//! (Linux-only).
//!
//! Scenario:
//!
//! 1. Pre-write a config under `XDG_CONFIG_HOME/hackernews-tim/` that
//!    points `article_parse_command` and `url_open_command` at
//!    deterministic stub scripts. The article-parse stub prints
//!    canned `Article` JSON (HTML body with two `<a>` links); the
//!    browser stub appends its argument to a log file so we can
//!    assert which link was picked.
//! 2. Spawn the binary against a [`FakeHnServer`] configured with
//!    one fixture story whose URL points at `http://example.test/article`.
//!    The binary will run the article-parse stub against that URL —
//!    the stub ignores its argument and just emits canned JSON, so
//!    no real HTTP fetch happens.
//! 3. With the front page rendered, press `O` (default
//!    `open_article_in_article_view`) and wait for the article title
//!    to appear. That confirms reader mode rendered fixture content.
//! 4. Press `l` (default `open_link_dialog`) and wait for the second
//!    fixture link's number-prefixed entry. That confirms the link
//!    dialog enumerated the `<a href>` links extracted by the HTML
//!    parser.
//! 5. Esc closes the dialog. Type `2o` — the article view's typed-
//!    prefix `open_link_in_browser` shortcut — to invoke the browser
//!    stub with link 2's URL. Poll the stub's log file until the
//!    expected URL appears.
//! 6. Quit cleanly.
//!
//! Same `news.ycombinator.com` caveat as §3.2.1 / §3.2.b: the
//! unauthenticated front-page render hits real HN for vote state.
//! The article-view fetch is exercised entirely through the local
//! stub, so no production article URL is ever requested.

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::{spawn_app, AppHandle, SpawnOptions, TestDirs, DEFAULT_WAIT};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);
const ARTICLE_RENDER_TIMEOUT: Duration = Duration::from_secs(10);

const STORY_ID: u32 = 30001;
const STORY_TITLE: &str = "article fixture story";
const STORY_URL: &str = "http://example.test/article";

const ARTICLE_TITLE: &str = "Phase 3 reader mode title";
const LINK1: &str = "http://example.test/link-one";
const LINK2: &str = "http://example.test/link-two";

fn fixture_article_json() -> String {
    json!({
        "title": ARTICLE_TITLE,
        "url": STORY_URL,
        "content": format!(
            "<p>Reader mode body paragraph alpha.</p>\
             <p>Body paragraph beta with \
             <a href=\"{LINK1}\">first anchor</a> and \
             <a href=\"{LINK2}\">second anchor</a>.</p>"
        ),
        "author": "Article Author",
        "date_published": "2026-01-01"
    })
    .to_string()
}

/// Write the article-parse + browser stub scripts plus the article
/// fixture JSON they read. Returns the absolute paths to the two
/// scripts and to the browser stub's log file (which the stub
/// creates on its first invocation).
fn write_stub_scripts(dirs: &TestDirs) -> (PathBuf, PathBuf, PathBuf) {
    let scripts_dir = dirs.home.join("scripts");
    std::fs::create_dir_all(&scripts_dir).expect("create scripts dir");

    let article_fixture_path = dirs.home.join("article_fixture.json");
    std::fs::write(&article_fixture_path, fixture_article_json())
        .expect("write article fixture json");

    let browser_log_path = dirs.home.join("browser.log");

    let article_stub = scripts_dir.join("article_md.sh");
    std::fs::write(
        &article_stub,
        format!(
            "#!/bin/sh\nexec cat {fixture}\n",
            fixture = article_fixture_path.display(),
        ),
    )
    .expect("write article stub");
    chmod_exec(&article_stub);

    let browser_stub = scripts_dir.join("browser.sh");
    std::fs::write(
        &browser_stub,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {log}\n",
            log = browser_log_path.display(),
        ),
    )
    .expect("write browser stub");
    chmod_exec(&browser_stub);

    (article_stub, browser_stub, browser_log_path)
}

fn chmod_exec(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).expect("stat stub").permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("chmod 755 stub");
}

/// Pre-write a minimal config at the binary's default
/// `XDG_CONFIG_HOME/hackernews-tim/config.toml` so the first-run
/// flavor prompt is skipped. `Config` derives `ConfigParse`, which
/// merges parsed fields over the `Default` impl, so this partial
/// TOML only needs to override the two commands we care about.
fn write_test_config(dirs: &TestDirs, article_stub: &Path, browser_stub: &Path) {
    let config_dir = dirs.xdg_config_home.join("hackernews-tim");
    std::fs::create_dir_all(&config_dir).expect("create config dir");

    let toml = format!(
        "url_open_command = {{ command = \"{browser}\", options = [] }}\n\
         article_parse_command = {{ command = \"{article}\", options = [] }}\n",
        browser = browser_stub.display(),
        article = article_stub.display(),
    );
    std::fs::write(config_dir.join("config.toml"), toml).expect("write config");
}

fn mount_article_fixtures(server: &FakeHnServer) {
    server.mount_get_json("/v0/topstories.json", 200, json!([STORY_ID]));
    server.mount_get_json(
        "/api/v1/search",
        200,
        json!({
            "hits": [
                {
                    "objectID": STORY_ID.to_string(),
                    "author": "alice",
                    "url": STORY_URL,
                    "story_text": null,
                    "points": 100,
                    "num_comments": 5,
                    "created_at_i": 1_700_000_000_u64,
                    "_highlightResult": { "title": { "value": STORY_TITLE } },
                    "dead": false,
                    "flagged": false,
                }
            ]
        }),
    );
}

/// The pre-written config skips `prompt_for_flavor`, but the auth
/// file is still missing at its default path so `prompt_for_auth`
/// fires. Step through it the same way the other e2e suites do.
fn dismiss_auth_prompt_only(handle: &mut AppHandle) {
    handle
        .wait_for_text("No auth file found", DEFAULT_WAIT)
        .expect("auth prompt should fire when only the config is pre-written");
    handle.send_keys("\n").expect("skip auth (default = N)");
}

/// Poll until `path` exists and is non-empty, or `timeout` elapses.
/// `url_open_command` runs in a detached thread, so the file appears
/// some time after the keypress that triggered it.
fn wait_for_browser_log(path: &Path, timeout: Duration) -> Result<String, String> {
    let start = Instant::now();
    loop {
        if let Ok(s) = std::fs::read_to_string(path) {
            if !s.is_empty() {
                return Ok(s);
            }
        }
        if start.elapsed() > timeout {
            return Err(format!(
                "timed out after {:?} waiting for browser stub to write {}",
                timeout,
                path.display()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn article_view_renders_and_link_dialog_picks_url_open_command() {
    let server = FakeHnServer::start();
    mount_article_fixtures(&server);

    let dirs = TestDirs::new().expect("create TestDirs");
    let (article_stub, browser_stub, browser_log) = write_stub_scripts(&dirs);
    write_test_config(&dirs, &article_stub, &browser_stub);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    dismiss_auth_prompt_only(&mut handle);

    handle
        .wait_for_text(STORY_TITLE, FRONT_PAGE_RENDER_TIMEOUT)
        .expect("fixture story should render on the front page");

    // Sanity: focus must be drawn before `O` dispatches —
    // `open_article_in_article_view` reads `s.stories[focus_index].url`.
    handle
        .focused_row()
        .expect("focus should be drawn before sending O");

    // `O` — `open_article_in_article_view` (StoryViewKeyMap default).
    handle.send_keys("O").expect("send O (open article view)");

    // The article-parse stub returns canned JSON, so the title from
    // that fixture is the first signal that reader mode rendered.
    handle
        .wait_for_text(ARTICLE_TITLE, ARTICLE_RENDER_TIMEOUT)
        .expect("article title from the article-parse stub should render");

    // TEST_PLAN.md §3.2.5 acceptance: PTY-rendered article view snapshot.
    // Allow a beat for the async-view loading frame to be replaced by
    // the parsed body before the snapshot is taken.
    std::thread::sleep(Duration::from_millis(150));
    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("article_view_after_open_pty", handle.screen());
    });

    // `l` — `open_link_dialog` (ArticleViewKeyMap default). The
    // dialog enumerates `<a href>` links extracted from the body, so
    // entry 2 with the second fixture URL confirms both that the
    // dialog opened and that the parser collected both links.
    handle.send_keys("l").expect("send l (open link dialog)");
    let dialog_needle = format!("2. {LINK2}");
    handle
        .wait_for_text(&dialog_needle, DEFAULT_WAIT)
        .expect("link dialog should enumerate both fixture links");

    // TEST_PLAN.md §3.2.5 acceptance: PTY-rendered link-dialog snapshot.
    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("article_link_dialog_pty", handle.screen());
    });

    // Close the dialog with Esc, then use the article view's typed-
    // prefix shortcut: digit chars accumulate into `raw_command`,
    // and `o` (`open_link_in_browser`) parses that buffer and
    // launches `url_open_command` with the chosen link's URL. Give
    // Cursive a beat to pop the dialog layer before the next
    // keystroke routes back to the article view.
    handle.send_keys("\x1b").expect("send Esc (close dialog)");
    std::thread::sleep(Duration::from_millis(150));
    handle
        .send_keys("2o")
        .expect("send 2o (open link 2 in browser)");

    let log = wait_for_browser_log(&browser_log, ARTICLE_RENDER_TIMEOUT)
        .expect("browser stub should be invoked with the picked URL");
    assert!(
        log.lines().any(|line| line.trim() == LINK2),
        "browser stub should have been invoked with {LINK2:?}; got log:\n{log}"
    );

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");
}
