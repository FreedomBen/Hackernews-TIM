//! Integration tests for `SearchView` (docs/planning/TEST_PLAN.md Phase 2.2.3).
//!
//! Builds a real `SearchView` over [`FakeHnApi`], drives it through
//! [`PuppetHarness`], and asserts on rendered output, query/mode
//! state, and recorded `get_matched_stories` calls.
//!
//! `SearchView::retrieve_matched_stories` spawns a worker thread that
//! calls `HnApi::get_matched_stories` and posts a no-op callback to
//! `cb_sink` on completion. Tests that need to assert on the recorded
//! call wait via [`wait_for_call_count`] rather than relying on
//! `step_until_idle` alone, since the puppet harness only drains
//! pending input/callbacks — it does not block on spawned threads.
//!
//! Mode-switching tests pre-populate fixture matched-stories on the
//! fake, because the SearchView's `to_navigation_mode` Esc handler
//! calls `LinearLayout::set_focus_index(1)` on the inner StoryView —
//! and an empty StoryView refuses focus, so mode stays in Search.
//! The second `step_until_idle` after `wait_for_call_count` is what
//! drives the layout pass that pulls fetched stories from the
//! `MatchedStories` channel into the view.
//!
//! The "open story from results" scenario in the TEST_PLAN row is
//! deferred: it requires a comment-page payload queued for the
//! follow-up `get_page_data` call, which the current PuppetHarness
//! can't deterministically synchronize.

use std::time::{Duration, Instant};

use cursive::event::{Event, Key};
use cursive::view::Nameable;
use cursive::views::{NamedView, OnEventView};
use cursive::Cursive;

use hackernews_tim::client::fake::{FakeCall, FakeHnApi};
use hackernews_tim::client::HnApi;
use hackernews_tim::model::Story;
use hackernews_tim::test_support::{
    ensure_globals_initialised, leak_fake_api, make_story, PuppetHarness,
};
use hackernews_tim::view::search_view::{
    construct_search_main_view, construct_search_view, SearchView,
};

fn fixture_stories() -> Vec<Story> {
    vec![make_story(101, "Result one"), make_story(102, "Result two")]
}

/// Populate the fake with fixture results for every (query, by_date,
/// page) the mode-switching tests are likely to touch. Without
/// stories the inner StoryView refuses focus, which breaks the
/// `Esc → Navigation` mode flip.
fn populate_default_results(fake: &FakeHnApi) {
    let stories = fixture_stories();
    for query in ["q", "qd", "a", "ad", "/"] {
        for &by_date in &[false, true] {
            for page in 0..=2 {
                fake.set_matched_stories(query, by_date, page, stories.clone());
            }
        }
    }
}

/// Wait up to 1 second for the fake API to record at least
/// `expected` total calls. The view's search dispatch is on a
/// background thread, so the test must give the thread a chance to
/// run before inspecting the call log.
fn wait_for_call_count(fake: &FakeHnApi, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while fake.call_count() < expected {
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for >= {expected} calls; got {} ({:?})",
                fake.call_count(),
                fake.calls()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Drive the harness through a search dispatch: process the just-sent
/// input event, wait for the spawned worker thread to record the
/// fetch, then run another layout pass so the inner StoryView pulls
/// the stories off the `MatchedStories` channel.
fn settle_after_search(harness: &mut PuppetHarness, fake: &FakeHnApi, expected_total: usize) {
    harness.step_until_idle();
    wait_for_call_count(fake, expected_total);
    harness.step_until_idle();
}

fn build_named_search_view(siv: &mut Cursive, fake: &'static FakeHnApi) {
    let cb_sink = siv.cb_sink().clone();
    let api: &'static dyn HnApi = fake;
    let main_view = construct_search_main_view(api, cb_sink);
    let named: NamedView<_> = main_view.with_name("search_view_outer");
    siv.add_layer(named);
}

fn search_text(harness: &mut PuppetHarness) -> String {
    harness
        .cursive_mut()
        .call_on_name("search_view_outer", |v: &mut OnEventView<SearchView>| {
            v.get_inner_mut()
                .get_search_text_view_mut()
                .map(|t| t.get_text())
                .unwrap_or_default()
        })
        .expect("named search view should be present")
}

#[test]
fn renders_initial_state_snapshot() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    let cb_sink = siv.cb_sink().clone();
    let api: &'static dyn HnApi = fake;
    siv.add_layer(construct_search_view(api, cb_sink));
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("initial_state", harness.screen_text());
    });
}

#[test]
fn construction_does_not_panic_with_empty_state() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let text = harness.screen_text();
    assert!(text.contains("Search:"), "expected search bar in:\n{text}");
    assert_eq!(fake.call_count(), 0, "no fetch should run before input");
}

#[test]
fn typing_query_updates_search_text() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    for c in "rust".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();

    assert_eq!(search_text(&mut harness), "rust");
}

#[test]
fn typing_query_triggers_get_matched_stories_call() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('q'));
    harness.step_until_idle();

    wait_for_call_count(fake, 1);
    let calls = fake.calls();
    assert!(
        matches!(
            &calls[0],
            FakeCall::GetMatchedStories(query, by_date, page)
                if query == "q" && !*by_date && *page == 0
        ),
        "expected GetMatchedStories(\"q\", false, 0); got {calls:?}"
    );
}

#[test]
fn esc_then_d_in_navigation_mode_toggles_by_date() {
    // In Search mode, 'd' is a literal query character. After Esc
    // (to_navigation_mode), 'd' triggers cycle_sort_mode, which
    // toggles `by_date` and re-fetches.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    populate_default_results(fake);
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('q'));
    settle_after_search(&mut harness, fake, 1);

    harness.send(Event::Key(Key::Esc));
    harness.send(Event::Char('d'));
    settle_after_search(&mut harness, fake, 2);

    let calls = fake.calls();
    assert!(
        matches!(
            &calls[1],
            FakeCall::GetMatchedStories(query, by_date, page)
                if query == "q" && *by_date && *page == 0
        ),
        "second call should toggle by_date=true; got {calls:?}"
    );
}

#[test]
fn next_page_in_navigation_advances_page() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    populate_default_results(fake);
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('q'));
    settle_after_search(&mut harness, fake, 1);

    harness.send(Event::Key(Key::Esc));
    harness.send(Event::Char('n'));
    settle_after_search(&mut harness, fake, 2);

    let calls = fake.calls();
    assert!(
        matches!(
            &calls[1],
            FakeCall::GetMatchedStories(query, by_date, page)
                if query == "q" && !*by_date && *page == 1
        ),
        "next_page should bump page=1; got {calls:?}"
    );
}

#[test]
fn prev_page_at_zero_does_not_refetch() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    populate_default_results(fake);
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('q'));
    settle_after_search(&mut harness, fake, 1);

    harness.send(Event::Key(Key::Esc));
    harness.send(Event::Char('p'));
    harness.step_until_idle();
    // Give any spurious thread a chance to record (none should).
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        fake.call_count(),
        1,
        "prev_page at page 0 should not refetch; got {:?}",
        fake.calls()
    );
}

#[test]
fn i_returns_to_search_mode_so_chars_become_input() {
    // Round-trip: Esc to Navigation, 'i' back to Search, then 'd'
    // becomes a literal char in the query (rather than triggering
    // cycle_sort_mode).
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    populate_default_results(fake);
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('a'));
    settle_after_search(&mut harness, fake, 1);
    assert_eq!(search_text(&mut harness), "a");

    harness.send(Event::Key(Key::Esc));
    harness.send(Event::Char('i'));
    harness.send(Event::Char('d'));
    settle_after_search(&mut harness, fake, 2);

    assert_eq!(search_text(&mut harness), "ad");
}

#[test]
fn find_dialog_only_opens_in_navigation_mode() {
    // In Search mode, '/' is just a query character — no find dialog
    // layer is added. After Esc (Navigation mode), '/' opens the
    // find-on-page dialog as a new layer.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = leak_fake_api();
    populate_default_results(fake);
    build_named_search_view(&mut siv, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let layers_initial = harness.cursive_mut().screen_mut().len();
    assert_eq!(layers_initial, 1);

    harness.send(Event::Char('/'));
    settle_after_search(&mut harness, fake, 1);
    let layers_after_search_slash = harness.cursive_mut().screen_mut().len();
    assert_eq!(
        layers_after_search_slash, 1,
        "'/' in Search mode should not open a dialog"
    );
    assert_eq!(search_text(&mut harness), "/");

    harness.send(Event::Key(Key::Esc));
    harness.send(Event::Char('/'));
    harness.step_until_idle();

    let layers_after_nav_slash = harness.cursive_mut().screen_mut().len();
    assert_eq!(
        layers_after_nav_slash, 2,
        "'/' in Navigation mode should add a find dialog layer"
    );
}
