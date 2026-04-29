//! Integration tests for `get_login_dialog` (docs/planning/TEST_PLAN.md Phase 2.2.5).
//!
//! Drives the dialog through [`PuppetHarness`] over a [`FakeHnApi`],
//! using `call_on_name` to populate the username/password EditViews
//! and Tab+Enter to focus and activate the "Log in" button. Each test
//! pre-removes its temp auth file so a stale write from a previous
//! run can't false-pass the `path.exists()` assertions, and removes
//! it again on success to keep `/tmp` clean.
//!
//! Tab order in the rendered dialog is:
//! `username EditView → password EditView → Cancel button → Log in button`.
//! Three Tabs from the initial focus puts focus on Log in; Enter
//! activates its callback.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cursive::event::{Event, Key};
use cursive::views::EditView;
use cursive::Cursive;

use hackernews_tim::client::fake::{FakeCall, FakeHnApi};
use hackernews_tim::client::HnApi;
use hackernews_tim::config::{Auth, AuthStorage};
use hackernews_tim::test_support::{ensure_globals_initialised, leak_fake_api, PuppetHarness};
use hackernews_tim::view::login_dialog::get_login_dialog;

const USERNAME_ID: &str = "login_dialog_username";
const PASSWORD_ID: &str = "login_dialog_password";

/// Per-test auth file path under the system temp dir. The pid +
/// nanos suffix makes it unique enough that two parallel test runs
/// (cargo test runs each integration test in its own process, but
/// `--test-threads` can interleave) don't collide.
fn auth_path(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "hackernews_tim_login_test_{}_{nanos}_{suffix}.toml",
        std::process::id()
    ))
}

fn add_login_dialog(siv: &mut Cursive, fake: &'static FakeHnApi, path: &Path) {
    let api: &'static dyn HnApi = fake;
    siv.add_layer(get_login_dialog(api, path.to_path_buf(), AuthStorage::File));
}

fn set_field(harness: &mut PuppetHarness, name: &str, value: &str) {
    harness
        .cursive_mut()
        .call_on_name(name, |v: &mut EditView| {
            v.set_content(value);
        })
        .expect("named EditView should be present");
}

/// Tab to the "Log in" button (3 tabs from initial focus on the
/// username EditView) and press Enter to activate it.
fn click_log_in(harness: &mut PuppetHarness) {
    harness.send(Event::Key(Key::Tab));
    harness.send(Event::Key(Key::Tab));
    harness.send(Event::Key(Key::Tab));
    harness.send(Event::Key(Key::Enter));
    harness.step_until_idle();
}

#[test]
fn empty_input_shows_status_and_keeps_dialog() {
    ensure_globals_initialised();
    let path = auth_path("empty");
    let _ = std::fs::remove_file(&path);

    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_login_dialog(&mut siv, fake, &path);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    let layers_before = harness.cursive_mut().screen_mut().len();

    click_log_in(&mut harness);

    let text = harness.screen_text();
    assert!(
        text.contains("Username and password are required."),
        "expected status message; got:\n{text}"
    );
    assert!(
        text.contains("Log in to Hacker News"),
        "login dialog should still be visible; got:\n{text}"
    );
    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        layers_before,
        "dialog must not be popped on validation failure"
    );
    assert_eq!(
        fake.call_count(),
        0,
        "FakeHnApi.login must not be called with empty input; got {:?}",
        fake.calls()
    );
    assert!(
        !path.exists(),
        "auth file must not be written on validation failure"
    );
}

#[test]
fn successful_login_writes_auth_file_and_swaps_dialog() {
    ensure_globals_initialised();
    let path = auth_path("success");
    let _ = std::fs::remove_file(&path);

    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    fake.set_session_cookie(Some("alice&abcdef0123".to_string()));
    add_login_dialog(&mut siv, fake, &path);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    set_field(&mut harness, USERNAME_ID, "alice");
    set_field(&mut harness, PASSWORD_ID, "hunter2");
    harness.step_until_idle();

    click_log_in(&mut harness);

    // The fake recorded the login attempt with the right credentials.
    let calls = fake.calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, FakeCall::Login(u, p) if u == "alice" && p == "hunter2")),
        "expected FakeCall::Login(alice, hunter2); got {calls:?}"
    );

    // The auth file was written and round-trips through Auth::from_file.
    assert!(
        path.exists(),
        "auth file at {} should exist after successful login",
        path.display()
    );
    let parsed = Auth::from_file(&path).expect("auth file should be parseable");
    assert_eq!(parsed.username, "alice");
    assert_eq!(parsed.password, "hunter2");
    assert_eq!(parsed.session.as_deref(), Some("alice&abcdef0123"));

    // The login dialog was popped and replaced with a "Login successful" info dialog.
    let text = harness.screen_text();
    assert!(
        text.contains("Login successful"),
        "expected the success info dialog; got:\n{text}"
    );
    assert!(
        !text.contains("Log in to Hacker News"),
        "login dialog should no longer be visible; got:\n{text}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn failed_login_shows_inline_error_and_keeps_dialog() {
    ensure_globals_initialised();
    let path = auth_path("fail");
    let _ = std::fs::remove_file(&path);

    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    fake.fail_login();
    add_login_dialog(&mut siv, fake, &path);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    let layers_before = harness.cursive_mut().screen_mut().len();

    set_field(&mut harness, USERNAME_ID, "alice");
    set_field(&mut harness, PASSWORD_ID, "wrongpw");
    harness.step_until_idle();

    click_log_in(&mut harness);

    let calls = fake.calls();
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, FakeCall::Login(u, p) if u == "alice" && p == "wrongpw")),
        "expected FakeCall::Login(alice, wrongpw); got {calls:?}"
    );

    let text = harness.screen_text();
    assert!(
        text.contains("Login failed"),
        "expected inline failure status; got:\n{text}"
    );
    assert!(
        text.contains("Log in to Hacker News"),
        "login dialog should remain on screen after failure; got:\n{text}"
    );
    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        layers_before,
        "dialog must not be popped on login failure"
    );
    assert!(
        !path.exists(),
        "auth file must not be written when login fails"
    );
}

#[test]
fn cancel_button_pops_the_dialog_without_login_call() {
    ensure_globals_initialised();
    let path = auth_path("cancel");
    let _ = std::fs::remove_file(&path);

    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_login_dialog(&mut siv, fake, &path);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    assert_eq!(harness.cursive_mut().screen_mut().len(), 1);

    // Two Tabs put focus on Cancel; Enter activates it.
    harness.send(Event::Key(Key::Tab));
    harness.send(Event::Key(Key::Tab));
    harness.send(Event::Key(Key::Enter));
    harness.step_until_idle();

    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        0,
        "Cancel should pop the dialog layer"
    );
    assert_eq!(
        fake.call_count(),
        0,
        "Cancel must not call the HN API; got {:?}",
        fake.calls()
    );
    assert!(!path.exists(), "Cancel must not write the auth file");
}
