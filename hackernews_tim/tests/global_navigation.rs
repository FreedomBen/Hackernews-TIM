//! Integration tests for global navigation + post-event hooks
//! (TEST_PLAN.md Phase 2.2.6).
//!
//! Drives the global keymap callbacks installed by
//! [`set_up_global_callbacks`] through [`PuppetHarness`] over a
//! [`FakeHnApi`]. The `F1`–`F5` story-tag shortcuts each call
//! `construct_and_add_new_story_view`, which spawns a background
//! thread that records `HnApi::get_stories_by_tag`; the test waits
//! for that recorded call via [`wait_for_matching_call`] rather than
//! relying on `step_until_idle` alone, since the puppet harness only
//! drains pending input/callbacks — it does not block on spawned
//! threads (same pattern used by the SearchView tests). The
//! prefetch's parallel `get_listing_vote_state` call is ignored.
//!
//! `F6` (`goto_my_threads_view`) reads the global `USER_INFO` slot —
//! we install a logged-in fixture in the shared setup since
//! `OnceCell::set` is one-shot per binary. The logged-out branch of
//! that callback is intentionally not exercised here for the same
//! reason; it would require a second integration-test binary.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use cursive::event::{Event, Key};
use cursive::views::TextView;
use cursive::Cursive;

use hackernews_tim::client::fake::{FakeCall, FakeHnApi};
use hackernews_tim::client::{
    init_test_user_info, HnApi, StoryNumericFilters, StorySortMode, UserInfo,
};
use hackernews_tim::config::{init_test_config_with, AuthStorage, Config, CustomKeyMap, Keys};
use hackernews_tim::test_support::PuppetHarness;
use hackernews_tim::view::set_up_global_callbacks;

const CUSTOM_KEY: char = 'r';
const CUSTOM_TAG: &str = "rust_news";
const TEST_USERNAME: &str = "alice_navtest";

fn ensure_globals_initialised() {
    static SETUP: OnceLock<()> = OnceLock::new();
    SETUP.get_or_init(|| {
        let mut config = Config::default();
        config.keymap.custom_keymaps.push(CustomKeyMap {
            key: Keys::new(vec![Event::Char(CUSTOM_KEY)]),
            tag: CUSTOM_TAG.to_string(),
            by_date: true,
            numeric_filters: StoryNumericFilters::default(),
        });
        init_test_config_with(config);
        init_test_user_info(Some(UserInfo {
            username: TEST_USERNAME.to_string(),
            karma: Some(123),
            showdead: false,
        }));
    });
}

fn make_fake_api() -> &'static FakeHnApi {
    Box::leak(Box::new(FakeHnApi::new()))
}

fn build_harness_with_callbacks(fake: &'static FakeHnApi) -> PuppetHarness {
    let mut siv = Cursive::new();
    // A placeholder layer keeps the screen non-empty; nothing in the
    // assertions depends on its content, but it gives Cursive
    // something to render and lets layer-count comparisons start at 1.
    siv.add_layer(TextView::new("nav-test placeholder"));
    let api: &'static dyn HnApi = fake;
    set_up_global_callbacks(
        &mut siv,
        api,
        PathBuf::from("/tmp/hackernews_tim_global_navigation_test.toml"),
        AuthStorage::File,
    );
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();
    harness
}

/// Wait up to ~2s for the fake to record any call satisfying `pred`.
/// Mirrors the search_view tests' `wait_for_call_count`, but on a
/// predicate so we can match a specific tag/sort tuple regardless of
/// the unrelated `GetListingVoteState` call the StoryView prefetch
/// kicks off in parallel via `rayon::join`.
fn wait_for_matching_call<F>(fake: &FakeHnApi, pred: F)
where
    F: Fn(&FakeCall) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if fake.calls().iter().any(&pred) {
            return;
        }
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for matching call; got: {:?}",
                fake.calls()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_get_stories_by_tag(fake: &FakeHnApi, tag: &str, expected_sort: StorySortMode) {
    let tag = tag.to_string();
    wait_for_matching_call(fake, move |call| {
        matches!(
            call,
            FakeCall::GetStoriesByTag(t, sort, _, _)
                if t == &tag && *sort == expected_sort
        )
    });
}

#[test]
fn f1_opens_front_page_story_view() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    harness.send(Event::Key(Key::F1));
    harness.step_until_idle();

    assert_get_stories_by_tag(fake, "front_page", StorySortMode::None);
}

#[test]
fn f2_opens_all_stories_view() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    harness.send(Event::Key(Key::F2));
    harness.step_until_idle();

    // tag == "story" → set_up_switch_story_view_shortcut picks
    // StorySortMode::Date.
    assert_get_stories_by_tag(fake, "story", StorySortMode::Date);
}

#[test]
fn f3_opens_ask_hn_view() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    harness.send(Event::Key(Key::F3));
    harness.step_until_idle();

    assert_get_stories_by_tag(fake, "ask_hn", StorySortMode::None);
}

#[test]
fn f4_opens_show_hn_view() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    harness.send(Event::Key(Key::F4));
    harness.step_until_idle();

    assert_get_stories_by_tag(fake, "show_hn", StorySortMode::None);
}

#[test]
fn f5_opens_jobs_view() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    harness.send(Event::Key(Key::F5));
    harness.step_until_idle();

    // tag == "job" → StorySortMode::Date.
    assert_get_stories_by_tag(fake, "job", StorySortMode::Date);
}

#[test]
fn f6_opens_threads_view_when_logged_in() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    harness.send(Event::Key(Key::F6));
    harness.step_until_idle();

    wait_for_matching_call(fake, |call| {
        matches!(
            call,
            FakeCall::GetUserThreadsPage(name, 0) if name == TEST_USERNAME
        )
    });
}

#[test]
fn custom_keymap_opens_configured_story_view() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    harness.send(Event::Char(CUSTOM_KEY));
    harness.step_until_idle();

    // by_date = true → StorySortMode::Date.
    assert_get_stories_by_tag(fake, CUSTOM_TAG, StorySortMode::Date);
}

#[test]
fn open_help_dialog_pushes_help_layer() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    let layers_before = harness.cursive_mut().screen_mut().len();
    harness.send(Event::Char('?'));
    harness.step_until_idle();

    assert_eq!(
        harness.cursive_mut().screen_mut().len(),
        layers_before + 1,
        "? should push a help dialog layer"
    );
    let text = harness.screen_text();
    assert!(
        text.to_lowercase().contains("help"),
        "help dialog should render help content; got:\n{text}"
    );
}

#[test]
fn quit_key_signals_cursive_shutdown() {
    ensure_globals_initialised();
    let fake = make_fake_api();
    let mut harness = build_harness_with_callbacks(fake);

    assert!(
        harness.cursive_mut().is_running(),
        "cursive should be running before quit"
    );

    harness.send(Event::Char('q'));
    harness.step_until_idle();

    assert!(
        !harness.cursive_mut().is_running(),
        "pressing q should clear Cursive's running flag"
    );
}
