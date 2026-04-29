//! End-to-end integration tests for the find-on-page dialog
//! (`view::find_bar`) driven through a `StoryView` host (docs/planning/TEST_PLAN.md
//! Phase 2.2.5).
//!
//! These tests exercise the dialog *UI*: opening with `/`, typing
//! into the EditableTextView, pressing Esc/Enter to dismiss. The host
//! `StoryView`'s `wrap_layout` polls `FindState::pending` on every
//! pass, so each test runs `step_until_idle` after each action so the
//! signal landed by the dialog (`Update`, `Clear`, `JumpNext`) is
//! consumed.
//!
//! Story-view-side mechanics (JumpNext moves focus, Clear empties
//! `match_ids`, external mutation flows in) are already covered by
//! `tests/story_view.rs` via direct `FindState::pending` pokes. This
//! file is the complementary half: signals arriving via the dialog's
//! keystrokes rather than direct mutation. The `n`/`N` keypress
//! handlers are wired in `construct_story_view` (not
//! `construct_story_main_view` which we use here for nameability),
//! so the keypress→JumpNext path is left to the trivially-traceable
//! production wiring.
//!
//! Fixture titles are crafted so the query "alpha" matches rows 0,
//! 2, and 4 of the five-row main view.
//!
//! Inspecting `match_ids` requires access to the inner `StoryView`,
//! so we name the `OnEventView<StoryView>` returned by
//! `construct_story_main_view` and reach in through `call_on_name`.

use std::collections::HashMap;

use cursive::event::{Event, Key};
use cursive::view::Nameable;
use cursive::views::{NamedView, OnEventView};
use cursive::Cursive;

use hackernews_tim::client::HnApi;
use hackernews_tim::model::Story;
use hackernews_tim::test_support::{
    ensure_globals_initialised, leak_fake_api, make_story, PuppetHarness,
};
use hackernews_tim::view::find_bar::{FindState, FindStateRef};
use hackernews_tim::view::story_view::{construct_story_main_view, StoryView};
use hackernews_tim::view::traits::ListViewContainer;

const NAME: &str = "story_view_for_find_bar";

/// Five stories where "alpha" matches rows 0, 2, and 4.
fn fixture_stories() -> Vec<Story> {
    vec![
        make_story(101, "alpha first"),
        make_story(102, "bravo second"),
        make_story(103, "alpha third"),
        make_story(104, "charlie fourth"),
        make_story(105, "alpha fifth"),
    ]
}

/// Build a named `OnEventView<StoryView>` and return the shared
/// `FindStateRef` so the test can poke it externally if needed.
fn build_named_main_view(siv: &mut Cursive) -> FindStateRef {
    let cb_sink = siv.cb_sink().clone();
    let api: &'static dyn HnApi = leak_fake_api();
    let find_state = FindState::new_ref();
    let main_view = construct_story_main_view(
        fixture_stories(),
        api,
        0,
        cb_sink,
        HashMap::new(),
        find_state.clone(),
    );
    let named: NamedView<OnEventView<StoryView>> = main_view.with_name(NAME);
    siv.add_layer(named);
    find_state
}

fn match_ids_len(harness: &mut PuppetHarness) -> usize {
    harness
        .cursive_mut()
        .call_on_name(NAME, |v: &mut OnEventView<StoryView>| {
            v.get_inner().match_ids_len_for_test()
        })
        .expect("named story view should be present")
}

fn focus_index(harness: &mut PuppetHarness) -> usize {
    harness
        .cursive_mut()
        .call_on_name(NAME, |v: &mut OnEventView<StoryView>| {
            v.get_inner().get_focus_index()
        })
        .expect("named story view should be present")
}

#[test]
fn slash_opens_find_dialog_as_new_layer() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    let layers_before = harness.cursive_mut().screen_mut().len();

    harness.send(Event::Char('/'));
    harness.step_until_idle();

    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        layers_before + 1,
        "/ should push the find dialog as a new layer"
    );
    assert!(
        harness.screen_text().contains("Find"),
        "find dialog should be on screen; got:\n{}",
        harness.screen_text()
    );
}

#[test]
fn typing_through_dialog_updates_match_ids_live() {
    // Each character typed into the dialog should set
    // `pending = Some(Update)`, which the host's `wrap_layout` polls
    // and converts into a fresh `apply_find_query` pass.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('/'));
    harness.step_until_idle();
    assert_eq!(
        match_ids_len(&mut harness),
        0,
        "no matches should be tracked before any character is typed"
    );

    // After "a", every fixture row contains the letter — five matches.
    harness.send(Event::Char('a'));
    harness.step_until_idle();
    assert_eq!(
        match_ids_len(&mut harness),
        5,
        "typing 'a' should match all five stories"
    );

    // Narrowing to "alpha" cuts it down to the three alpha rows.
    for c in "lpha".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();
    assert_eq!(
        match_ids_len(&mut harness),
        3,
        "typing 'alpha' should narrow matches to the three alpha rows"
    );
}

#[test]
fn enter_in_dialog_pops_layer_and_jumps_to_first_match() {
    // Move focus to row 1 first so that JumpNext from "alpha" — which
    // matches rows 0/2/4 — has somewhere visibly forward to land.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('j'));
    harness.step_until_idle();
    assert_eq!(focus_index(&mut harness), 1);

    let layers_before = harness.cursive_mut().screen_mut().len();
    harness.send(Event::Char('/'));
    harness.step_until_idle();
    for c in "alpha".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();

    harness.send(Event::Key(Key::Enter));
    harness.step_until_idle();

    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        layers_before,
        "Enter should pop the find dialog layer"
    );
    assert_eq!(
        focus_index(&mut harness),
        2,
        "Enter from focus 1 should jump to the next match (row 2)"
    );
    assert_eq!(
        match_ids_len(&mut harness),
        3,
        "match_ids must persist after Enter so subsequent n/N can navigate"
    );
}

#[test]
fn esc_in_dialog_pops_layer_and_clears_match_ids() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    let layers_before = harness.cursive_mut().screen_mut().len();

    harness.send(Event::Char('/'));
    harness.step_until_idle();
    for c in "alpha".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();
    assert_eq!(match_ids_len(&mut harness), 3);

    harness.send(Event::Key(Key::Esc));
    harness.step_until_idle();

    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        layers_before,
        "Esc should pop the find dialog layer"
    );
    assert_eq!(
        match_ids_len(&mut harness),
        0,
        "Esc should clear match_ids via the FindSignal::Clear path"
    );
}

#[test]
fn typing_with_no_matches_leaves_match_ids_empty() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('/'));
    for c in "zzz".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();

    assert_eq!(
        match_ids_len(&mut harness),
        0,
        "no fixture title contains 'zzz' so match_ids should stay empty"
    );

    // Enter with no matches must still be safe (`jump_to_next_match`
    // early-returns on an empty match set).
    harness.send(Event::Key(Key::Enter));
    harness.step_until_idle();
}

#[test]
fn backspace_in_dialog_widens_match_set() {
    // Typing "alpha" narrows to 3, then backspacing back to "a"
    // widens to 5 again — confirms the dialog's `del_char` keypath
    // also sets `pending = Some(Update)`.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let _find_state = build_named_main_view(&mut siv);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('/'));
    for c in "alpha".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();
    assert_eq!(match_ids_len(&mut harness), 3);

    for _ in 0..4 {
        harness.send(Event::Key(Key::Backspace));
    }
    harness.step_until_idle();

    assert_eq!(
        match_ids_len(&mut harness),
        5,
        "after backspacing back to 'a', all five stories should match again"
    );
}
