//! Integration tests for `CommentView` (docs/planning/TEST_PLAN.md Phase 2.2.2).
//!
//! Drives a real `CommentView` over a hand-built comment tree fed
//! through the `crossbeam_channel` that backs `PageData`. The view's
//! `try_update_comments` runs on construction, so sending into the
//! channel before `CommentView::new` populates the tree as soon as
//! the view is rendered.
//!
//! Tests at this layer exercise the *navigation logic* of
//! `CommentView` — `find_sibling`, `find_item_id_by_max_level`,
//! `toggle_collapse_focused_item` — and the rendering output.
//! End-to-end keymap-driven scenarios (sending `n`/`u`/`Tab` through
//! the puppet harness) are deferred: the catch-all
//! `on_pre_event_inner(any, ...)` in `construct_comment_main_view`
//! resets `LinearLayout` focus in this test environment in a way that
//! production-runtime users don't see, and untangling it isn't worth
//! the cost relative to testing the underlying logic directly. The
//! production keymap → method wiring is small enough to read at a
//! glance — see `view::comment_view::construct_comment_main_view`.
//!
//! Scenarios covered:
//!
//! - Snapshot of a small comment tree (1 root story + 4 comments,
//!   2 levels of nesting).
//! - `find_sibling` navigation: skipping children for top-level
//!   navigation; walking forward and backward at the same level.
//! - `find_item_id_by_max_level` parent navigation: from a level-1
//!   reply, jumping up to the level-0 parent.
//! - `toggle_collapse_focused_item`: Hidden state for children;
//!   re-toggle restores them.
//! - Dead/flagged comment styling makes it through to the rendered
//!   screen text.

use std::collections::HashMap;

use crossbeam_channel::unbounded;
use cursive::view::Nameable;
use cursive::views::NamedView;
use cursive::Cursive;

use hackernews_tim::model::{Comment, HnItem, PageData, Story};
use hackernews_tim::test_support::{ensure_globals_initialised, PuppetHarness};
use hackernews_tim::view::comment_view::{CommentView, NavigationDirection};
use hackernews_tim::view::find_bar::FindState;
use hackernews_tim::view::traits::ListViewContainer;

fn fixture_root_story() -> Story {
    Story {
        id: 9000,
        url: "https://example.com/post".to_string(),
        author: "alice".to_string(),
        points: 42,
        num_comments: 4,
        time: 1_700_000_000,
        title: "An interesting post".to_string(),
        content: String::new(),
        dead: false,
        flagged: false,
    }
}

fn fixture_comment(id: u32, level: usize, author: &str, content: &str) -> Comment {
    // HN comments are rendered with <p> separators between paragraphs
    // and no opening/closing wrap, so leaving `content` plain mirrors
    // the production format and avoids leaking `</p>` into the parsed
    // output (which `parse_hn_html_text` doesn't strip).
    Comment {
        id,
        level,
        n_children: 0,
        author: author.to_string(),
        time: 1_700_000_000,
        content: content.to_string(),
        dead: false,
        flagged: false,
        points: None,
        parent_story_id: None,
    }
}

/// 1 root + 4 comments, 2 levels of nesting:
///
/// ```text
///   0: root story (level 0)
///   1: bob (level 0)
///   2:   carol (level 1)
///   3:   dan (level 1)
///   4: erin (level 0)
/// ```
fn fixture_comments() -> Vec<Comment> {
    vec![
        Comment {
            n_children: 2,
            ..fixture_comment(101, 0, "bob", "First top-level comment")
        },
        fixture_comment(102, 1, "carol", "Reply to bob"),
        fixture_comment(103, 1, "dan", "Another reply to bob"),
        fixture_comment(104, 0, "erin", "Second top-level comment"),
    ]
}

fn build_page_data(comments: Vec<Comment>) -> PageData {
    let (sender, receiver) = unbounded();
    sender.send(comments).expect("send into fresh channel");
    drop(sender);
    PageData {
        title: "An interesting post".to_string(),
        url: "https://example.com/post".to_string(),
        root_item: HnItem::from(fixture_root_story()),
        comment_receiver: receiver,
        vote_state: HashMap::new(),
        vouch_state: HashMap::new(),
    }
}

/// Construct a `CommentView` directly (bypassing
/// `construct_comment_main_view`) and add it as a `NamedView` so
/// tests can poke at its public API via `call_on_name`.
fn add_comment_view(siv: &mut Cursive, comments: Vec<Comment>) {
    let data = build_page_data(comments);
    let view = CommentView::new(data, FindState::new_ref());
    let named: NamedView<CommentView> = view.with_name("comment_view");
    siv.add_layer(named);
}

fn item_count(harness: &mut PuppetHarness) -> usize {
    harness
        .cursive_mut()
        .call_on_name("comment_view", |v: &mut CommentView| v.len())
        .expect("comment_view named view should be present")
}

#[test]
fn renders_small_comment_tree_snapshot() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    add_comment_view(&mut siv, fixture_comments());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    insta::with_settings!({filters => vec![
        (r"\d+ \w+ ago", "[time ago]"),
    ]}, {
        insta::assert_snapshot!("small_comment_tree", harness.screen_text());
    });
}

#[test]
fn try_update_comments_loads_tree_into_view() {
    // CommentView::new calls try_update_comments internally, so the
    // queued comments are already attached by the time this test
    // inspects the view. Confirm the count matches: 1 root + 4
    // comments = 5 visible items.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    add_comment_view(&mut siv, fixture_comments());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    assert_eq!(item_count(&mut harness), 5);
}

#[test]
fn find_sibling_skips_children_at_top_level() {
    // From the root story (id=0, level 0), find_sibling Next should
    // land on bob (id=1). From bob (id=1, level 0), Next should skip
    // carol/dan (level 1) and land on erin (id=4).
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    add_comment_view(&mut siv, fixture_comments());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness
        .cursive_mut()
        .call_on_name("comment_view", |v: &mut CommentView| {
            assert_eq!(
                v.find_sibling(0, NavigationDirection::Next),
                1,
                "find_sibling Next from root should land on bob"
            );
            assert_eq!(
                v.find_sibling(1, NavigationDirection::Next),
                4,
                "find_sibling Next from bob should skip carol/dan and land on erin"
            );
            assert_eq!(
                v.find_sibling(4, NavigationDirection::Previous),
                1,
                "find_sibling Previous from erin should land on bob"
            );
        });
}

#[test]
fn find_sibling_walks_within_a_level_one_subtree() {
    // carol (id=2, level 1) and dan (id=3, level 1) are siblings under
    // bob's subtree. find_sibling Next/Previous should walk between
    // them without escaping the subtree.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    add_comment_view(&mut siv, fixture_comments());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness
        .cursive_mut()
        .call_on_name("comment_view", |v: &mut CommentView| {
            assert_eq!(v.find_sibling(2, NavigationDirection::Next), 3);
            assert_eq!(v.find_sibling(3, NavigationDirection::Previous), 2);
        });
}

#[test]
fn find_item_id_by_max_level_jumps_to_parent() {
    // The `parent_comment` keymap routes through
    // `find_item_id_by_max_level(id, level - 1, Previous)`. From
    // dan (id=3, level 1), the parent is bob (id=1, level 0).
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    add_comment_view(&mut siv, fixture_comments());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness
        .cursive_mut()
        .call_on_name("comment_view", |v: &mut CommentView| {
            assert_eq!(
                v.find_item_id_by_max_level(3, 0, NavigationDirection::Previous),
                1,
                "from dan (level 1), seeking max_level 0 backwards should land on bob"
            );
            assert_eq!(
                v.find_item_id_by_max_level(2, 0, NavigationDirection::Previous),
                1,
                "from carol (level 1), seeking max_level 0 backwards should land on bob"
            );
        });
}

#[test]
fn toggle_collapse_hides_and_restores_subtree() {
    // bob has two replies (carol, dan). Collapsing bob should hide
    // both children from the rendered screen. Re-toggling restores
    // them.
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    add_comment_view(&mut siv, fixture_comments());
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let initial = harness.screen_text();
    assert!(
        initial.contains("Reply to bob"),
        "initial render should include carol's reply:\n{initial}"
    );

    harness
        .cursive_mut()
        .call_on_name("comment_view", |v: &mut CommentView| {
            // Move focus to bob (id=1).
            v.set_focus_index(1);
            v.toggle_collapse_focused_item();
        });
    harness.step_until_idle();

    let after_collapse = harness.screen_text();
    assert!(
        !after_collapse.contains("Reply to bob"),
        "carol's reply should be hidden after collapsing bob:\n{after_collapse}"
    );
    assert!(
        after_collapse.contains("more)"),
        "collapsed subtree should show '(N more)' marker:\n{after_collapse}"
    );

    harness
        .cursive_mut()
        .call_on_name("comment_view", |v: &mut CommentView| {
            v.toggle_collapse_focused_item();
        });
    harness.step_until_idle();

    let after_restore = harness.screen_text();
    assert!(
        after_restore.contains("Reply to bob"),
        "carol's reply should be visible again after a second toggle:\n{after_restore}"
    );
}

#[test]
fn dead_comment_renders_with_dead_marker() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let comments = vec![Comment {
        dead: true,
        ..fixture_comment(201, 0, "spammer", "Some dead content")
    }];
    add_comment_view(&mut siv, comments);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let text = harness.screen_text();
    assert!(
        text.contains("[dead]"),
        "dead comment metadata should include '[dead]':\n{text}"
    );
}

#[test]
fn flagged_comment_renders_with_flagged_marker() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let comments = vec![Comment {
        flagged: true,
        ..fixture_comment(202, 0, "controversial", "Some flagged content")
    }];
    add_comment_view(&mut siv, comments);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let text = harness.screen_text();
    assert!(
        text.contains("[flagged]"),
        "flagged comment metadata should include '[flagged]':\n{text}"
    );
}
