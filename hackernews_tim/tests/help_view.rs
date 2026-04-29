//! Snapshot tests for the per-view [`HelpView`] dialogs (docs/planning/TEST_PLAN.md
//! Phase 2.2.5).
//!
//! Each parent view that supports the help dialog implements
//! `HasHelpView::construct_help_view`. These tests construct the help
//! dialog directly (no parent view) and snapshot the rendered screen
//! to catch drift between the keymap defaults documented in the help
//! text and the actual key bindings.
//!
//! The puppet uses a large viewport so the longest help dialogs (the
//! StoryView and SearchView versions both span many groups) render
//! without being clipped — clipped output would still pass the
//! snapshot assertion but would silently lose lines on regression.
//!
//! These snapshots are *expected* to churn whenever a new keybinding
//! is added or a description is reworded; that's the point — review
//! via `cargo insta review` keeps the docs honest.

use cursive::{Cursive, Vec2};

use hackernews_tim::client::init_test_user_info;
use hackernews_tim::config::init_test_config;
use hackernews_tim::test_support::PuppetHarness;
use hackernews_tim::view::article_view::ArticleView;
use hackernews_tim::view::comment_view::CommentView;
use hackernews_tim::view::help_view::{DefaultHelpView, HasHelpView};
use hackernews_tim::view::link_dialog::LinkDialog;
use hackernews_tim::view::search_view::SearchView;
use hackernews_tim::view::story_view::StoryView;

fn ensure_globals_initialised() {
    init_test_config();
    init_test_user_info(None);
}

/// Help dialogs are tall — give them a viewport big enough that no
/// rows fall off the bottom. The dialog itself sizes to its content.
fn help_size() -> Vec2 {
    Vec2::new(140, 80)
}

fn render_help<H: HasHelpView>() -> String {
    let mut siv = Cursive::new();
    siv.add_layer(H::construct_help_view());
    let mut harness = PuppetHarness::with_size(siv, help_size());
    harness.step_until_idle();
    harness.screen_text()
}

#[test]
fn default_help_view_snapshot() {
    ensure_globals_initialised();
    insta::assert_snapshot!("default_help", render_help::<DefaultHelpView>());
}

#[test]
fn story_view_help_snapshot() {
    ensure_globals_initialised();
    insta::assert_snapshot!("story_view_help", render_help::<StoryView>());
}

#[test]
fn comment_view_help_snapshot() {
    ensure_globals_initialised();
    insta::assert_snapshot!("comment_view_help", render_help::<CommentView>());
}

#[test]
fn search_view_help_snapshot() {
    ensure_globals_initialised();
    insta::assert_snapshot!("search_view_help", render_help::<SearchView>());
}

#[test]
fn article_view_help_snapshot() {
    ensure_globals_initialised();
    insta::assert_snapshot!("article_view_help", render_help::<ArticleView>());
}

#[test]
fn link_dialog_help_snapshot() {
    ensure_globals_initialised();
    insta::assert_snapshot!("link_dialog_help", render_help::<LinkDialog>());
}

#[test]
fn each_help_view_includes_quit_and_help_lines() {
    // Cross-cutting invariant: every help dialog should surface the
    // global keymap's `quit` and `open_help_dialog` rows. Catches the
    // case where a HasHelpView impl forgets to include
    // `default_other_commands()` somewhere in its CommandGroup list.
    ensure_globals_initialised();
    let renders = [
        ("DefaultHelpView", render_help::<DefaultHelpView>()),
        ("StoryView", render_help::<StoryView>()),
        ("CommentView", render_help::<CommentView>()),
        ("SearchView", render_help::<SearchView>()),
        ("ArticleView", render_help::<ArticleView>()),
        ("LinkDialog", render_help::<LinkDialog>()),
    ];
    for (name, text) in renders {
        assert!(
            text.contains("Quit the application"),
            "{name} help is missing the quit row; got:\n{text}"
        );
        assert!(
            text.contains("Open the help dialog"),
            "{name} help is missing the help-dialog row; got:\n{text}"
        );
    }
}
