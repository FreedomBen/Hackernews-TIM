//! Integration tests for `LinkDialog` (docs/planning/TEST_PLAN.md Phase 2.2.5).
//!
//! Builds the dialog over a fixed list of fixture links via
//! [`construct_link_dialog_main_view`] (the resize-wrapped public
//! [`get_link_dialog`] hides the outer type behind `impl View`, so it
//! can't be wrapped in a [`NamedView`] for inner-state queries),
//! drives it through [`PuppetHarness`], and asserts on rendered
//! output and focus tracking.
//!
//! The "Enter triggers `open_ith_link_in_browser`" scenario from the
//! TEST_PLAN row is omitted because that path spawns the configured
//! `url_open_command` (`xdg-open` / `open`) in a thread, which would
//! launch a real browser during `cargo test`. The bounds half of
//! that path is covered by the unit tests for `view::utils::nth_link`
//! and `open_ith_link_in_browser`.
//!
//! `j`/`k` (and the equivalent arrow keys) drive
//! `LinearLayout::set_focus_index` through the link-dialog keymap,
//! and the `prev` handler guards against decrementing past 0 — both
//! of which we assert here through
//! [`LinkDialog::focused_index_for_test`].

use cursive::event::{Event, Key};
use cursive::view::Nameable;
use cursive::views::{NamedView, OnEventView};
use cursive::Cursive;

use hackernews_tim::client::fake::FakeHnApi;
use hackernews_tim::client::HnApi;
use hackernews_tim::test_support::{ensure_globals_initialised, leak_fake_api, PuppetHarness};
use hackernews_tim::view::link_dialog::{construct_link_dialog_main_view, LinkDialog};

const NAME: &str = "link_dialog_outer";

fn fixture_links() -> Vec<String> {
    vec![
        "https://example.com/one".to_string(),
        "https://example.com/two".to_string(),
        "https://example.com/three".to_string(),
        "https://example.com/four".to_string(),
        "https://example.com/five".to_string(),
    ]
}

fn add_named_link_dialog(siv: &mut Cursive, links: &[String], fake: &'static FakeHnApi) {
    let api: &'static dyn HnApi = fake;
    let main = construct_link_dialog_main_view(api, links);
    let named: NamedView<OnEventView<LinkDialog>> = main.with_name(NAME);
    siv.add_layer(named);
}

fn focused_index(harness: &mut PuppetHarness) -> usize {
    harness
        .cursive_mut()
        .call_on_name(NAME, |v: &mut OnEventView<LinkDialog>| {
            v.get_inner().focused_index_for_test()
        })
        .expect("named link dialog should be present")
}

#[test]
fn renders_dialog_with_five_links_snapshot() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_named_link_dialog(&mut siv, &fixture_links(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    insta::assert_snapshot!("dialog_five_links", harness.screen_text());
}

#[test]
fn j_advances_focus_and_k_retreats() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_named_link_dialog(&mut siv, &fixture_links(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    assert_eq!(
        focused_index(&mut harness),
        0,
        "initial focus is the first link"
    );

    harness.send(Event::Char('j'));
    harness.send(Event::Char('j'));
    harness.send(Event::Char('j'));
    harness.step_until_idle();
    assert_eq!(
        focused_index(&mut harness),
        3,
        "three j's advance focus to index 3"
    );

    harness.send(Event::Char('k'));
    harness.step_until_idle();
    assert_eq!(focused_index(&mut harness), 2, "k retreats one row");
}

#[test]
fn arrow_keys_drive_the_same_focus_path_as_j_k() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_named_link_dialog(&mut siv, &fixture_links(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Key(Key::Down));
    harness.send(Event::Key(Key::Down));
    harness.step_until_idle();
    assert_eq!(focused_index(&mut harness), 2);

    harness.send(Event::Key(Key::Up));
    harness.step_until_idle();
    assert_eq!(focused_index(&mut harness), 1);
}

#[test]
fn k_at_top_does_not_underflow() {
    // The `prev` handler in `construct_link_dialog_main_view` guards
    // `focus_id > 0` before decrementing; this test pins that guard.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_named_link_dialog(&mut siv, &fixture_links(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('k'));
    harness.send(Event::Char('k'));
    harness.step_until_idle();

    assert_eq!(
        focused_index(&mut harness),
        0,
        "k from the top must stay at 0"
    );
}

#[test]
fn esc_pops_the_dialog_layer() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_named_link_dialog(&mut siv, &fixture_links(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    assert_eq!(harness.cursive_mut().screen_mut().len(), 1);
    harness.send(Event::Key(Key::Esc));
    harness.step_until_idle();
    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        0,
        "Esc should pop the dialog layer"
    );
}

#[test]
fn renders_first_and_last_link_text() {
    // Light visual smoke check that the dialog actually renders the
    // numbered list and the link text (shorten_url returns the URL
    // unchanged for these short fixtures, so the URL appears verbatim).
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    add_named_link_dialog(&mut siv, &fixture_links(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let text = harness.screen_text();
    assert!(
        text.contains("1.") && text.contains("https://example.com/one"),
        "first link not rendered; got:\n{text}"
    );
    assert!(
        text.contains("5.") && text.contains("https://example.com/five"),
        "fifth link not rendered; got:\n{text}"
    );
}
