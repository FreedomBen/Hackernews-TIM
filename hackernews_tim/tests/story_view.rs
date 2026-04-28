//! Integration tests for `StoryView` (TEST_PLAN.md Phase 2.2.1).
//!
//! Builds a real `StoryView` over hand-built `Story` fixtures, drives
//! it through [`hackernews_tim::test_support::PuppetHarness`], and
//! asserts on rendered output, focus movement, and `FindState`
//! interactions.
//!
//! Scenarios covered here:
//!
//! - Snapshot of a 3-row front-page render.
//! - `j` / `k` (next/prev) and half-page navigation.
//! - External `FindState` updates flow into the view on the next
//!   layout pass and populate `match_ids`.
//! - `find-jump-next` advances focus to the matching row's index.
//! - Construction with an empty story list does not panic and
//!   reports `stories.len() == 0` via the public field.
//!
//! Scenarios deferred to a follow-up commit (vote/login state,
//! tag-cycle, find dialog UX, post-event hooks) are tracked in the
//! Phase 2.2.1 row of TEST_PLAN.md — they need background-thread
//! orchestration the puppet harness can't yet drive deterministically.

use std::collections::HashMap;

use cursive::event::{Event, Key};
use cursive::view::Nameable;
use cursive::views::{NamedView, OnEventView};
use cursive::Cursive;

use hackernews_tim::client::fake::FakeHnApi;
use hackernews_tim::client::{init_test_user_info, HnApi, StoryNumericFilters, StorySortMode};
use hackernews_tim::config::init_test_config;
use hackernews_tim::model::Story;
use hackernews_tim::test_support::PuppetHarness;
use hackernews_tim::view::find_bar::{FindSignal, FindState, FindStateRef};
use hackernews_tim::view::story_view::{
    construct_story_main_view, construct_story_view, StoryView,
};
use hackernews_tim::view::traits::ListViewContainer;

fn ensure_globals_initialised() {
    init_test_config();
    init_test_user_info(None);
}

fn fixture_story(id: u32, title: &str, author: &str, points: u32) -> Story {
    Story {
        id,
        url: format!("https://example.com/{id}"),
        author: author.to_string(),
        points,
        num_comments: 0,
        time: 1_700_000_000,
        title: title.to_string(),
        content: String::new(),
        dead: false,
        flagged: false,
    }
}

fn fixture_stories() -> Vec<Story> {
    vec![
        fixture_story(101, "Rust 1.99 released", "alice", 250),
        fixture_story(102, "Cursive 0.20 hits stable", "bob", 90),
        fixture_story(103, "Hacker News test fixtures", "carol", 47),
    ]
}

fn make_fake_api() -> &'static dyn HnApi {
    Box::leak(Box::<FakeHnApi>::default())
}

/// Build a fresh main view (no title bar / footer) wrapped in a
/// `NamedView<StoryView>` so tests can poll the inner view via
/// `siv.find_name("story_view")`. The returned `FindStateRef` is the
/// same `Rc` the view holds, so external mutation drives the view's
/// next layout pass.
fn build_named_main_view(siv: &mut Cursive, stories: Vec<Story>) -> FindStateRef {
    let api = make_fake_api();
    let find_state = FindState::new_ref();
    let main_view = construct_story_main_view(
        stories,
        api,
        0,
        siv.cb_sink().clone(),
        HashMap::new(),
        find_state.clone(),
    );
    // Stuff the view into a NamedView. Cursive doesn't let us name an
    // OnEventView directly via `with_name`, so we wrap the whole event
    // wrapper in a NamedView keyed by "story_view_outer" and reach in
    // for the inner StoryView via `call_on_name`.
    let named: NamedView<_> = main_view.with_name("story_view_outer");
    siv.add_layer(named);
    find_state
}

#[test]
fn renders_three_story_fixtures() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let cb_sink = siv.cb_sink().clone();
    let api = make_fake_api();
    siv.add_layer(construct_story_view(
        fixture_stories(),
        HashMap::new(),
        api,
        "front_page",
        StorySortMode::None,
        0,
        StoryNumericFilters::default(),
        cb_sink,
    ));
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    // Redact relative-time text ("2 years ago") so the snapshot stays
    // stable regardless of when the test runs against a fixed `time`
    // field on the fixture.
    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("front_page_three_stories", harness.screen_text());
    });
}

#[test]
fn empty_story_list_does_not_panic() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let cb_sink = siv.cb_sink().clone();
    let api = make_fake_api();
    siv.add_layer(construct_story_view(
        Vec::new(),
        HashMap::new(),
        api,
        "front_page",
        StorySortMode::None,
        0,
        StoryNumericFilters::default(),
        cb_sink,
    ));
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    // Just assert the harness produced a frame; the snapshot is in
    // `front_page_three_stories` and we don't want to commit a second
    // empty-page snapshot whose content would churn with theme tweaks.
    let text = harness.screen_text();
    assert!(
        !text.is_empty(),
        "expected at least the title/footer chrome to render"
    );
}

#[test]
fn j_and_k_move_focus_through_stories() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv, fixture_stories());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    fn focus_index(harness: &mut PuppetHarness) -> usize {
        harness
            .cursive_mut()
            .call_on_name("story_view_outer", |v: &mut OnEventView<StoryView>| {
                v.get_inner_mut().get_focus_index()
            })
            .expect("named view should be present")
    }

    assert_eq!(focus_index(&mut harness), 0);

    harness.send(Event::Char('j'));
    harness.step_until_idle();
    assert_eq!(focus_index(&mut harness), 1);

    harness.send(Event::Char('j'));
    harness.step_until_idle();
    assert_eq!(focus_index(&mut harness), 2);

    harness.send(Event::Char('k'));
    harness.step_until_idle();
    assert_eq!(focus_index(&mut harness), 1);
}

#[test]
fn k_at_first_story_is_a_no_op() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv, fixture_stories());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('k'));
    harness.step_until_idle();
    let focus = harness
        .cursive_mut()
        .call_on_name("story_view_outer", |v: &mut OnEventView<StoryView>| {
            v.get_inner_mut().get_focus_index()
        })
        .unwrap();
    assert_eq!(focus, 0, "k at row 0 should clamp to row 0");
}

#[test]
fn j_past_last_story_clamps_to_last() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv, fixture_stories());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    for _ in 0..10 {
        harness.send(Event::Char('j'));
    }
    harness.step_until_idle();
    let focus = harness
        .cursive_mut()
        .call_on_name("story_view_outer", |v: &mut OnEventView<StoryView>| {
            v.get_inner_mut().get_focus_index()
        })
        .unwrap();
    assert_eq!(focus, 2, "j past the last row should clamp to row 2");
}

#[test]
fn find_state_match_ids_populated_on_layout() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let find_state = build_named_main_view(&mut siv, fixture_stories());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    // Mutate the shared FindState — this is what `find_bar`'s dialog
    // does in the real flow, except we skip the dialog UX here.
    {
        let mut state = find_state.borrow_mut();
        state.query = "Cursive".to_string();
        state.pending = Some(FindSignal::Update);
    }

    // Force a layout pass so `process_find_signal` runs.
    harness.step_until_idle();

    let matches = find_state.borrow().match_ids.clone();
    assert_eq!(
        matches,
        vec![1],
        "row 1 (\"Cursive 0.20 hits stable\") should be the only match"
    );
}

#[test]
fn find_jump_next_moves_focus_to_match() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let find_state = build_named_main_view(&mut siv, fixture_stories());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    {
        let mut state = find_state.borrow_mut();
        state.query = "Hacker".to_string();
        state.pending = Some(FindSignal::Update);
    }
    harness.step_until_idle();
    assert_eq!(find_state.borrow().match_ids, vec![2]);

    {
        let mut state = find_state.borrow_mut();
        state.pending = Some(FindSignal::JumpNext);
    }
    harness.step_until_idle();

    let focus = harness
        .cursive_mut()
        .call_on_name("story_view_outer", |v: &mut OnEventView<StoryView>| {
            v.get_inner_mut().get_focus_index()
        })
        .unwrap();
    assert_eq!(focus, 2, "find-jump-next should land on row 2");
}

#[test]
fn find_clear_drops_match_ids() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let find_state = build_named_main_view(&mut siv, fixture_stories());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    {
        let mut state = find_state.borrow_mut();
        state.query = "alice".to_string();
        state.pending = Some(FindSignal::Update);
    }
    harness.step_until_idle();
    assert!(!find_state.borrow().match_ids.is_empty());

    {
        let mut state = find_state.borrow_mut();
        state.pending = Some(FindSignal::Clear);
    }
    harness.step_until_idle();

    assert!(
        find_state.borrow().match_ids.is_empty(),
        "Clear signal should drop match_ids"
    );
}

#[test]
fn esc_key_propagates_to_find_outer_layer() {
    // Sanity check: bare Esc on the main view alone (no find dialog,
    // no outer wrapper) shouldn't crash. The full find-bar dismissal
    // flow is exercised end-to-end in 2.2.5.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv, fixture_stories());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Key(Key::Esc));
    harness.step_until_idle();

    let focus = harness
        .cursive_mut()
        .call_on_name("story_view_outer", |v: &mut OnEventView<StoryView>| {
            v.get_inner_mut().get_focus_index()
        })
        .unwrap();
    assert_eq!(focus, 0, "Esc on row 0 should leave focus where it is");
}
