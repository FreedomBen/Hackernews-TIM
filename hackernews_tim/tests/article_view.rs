//! Integration tests for `ArticleView` (TEST_PLAN.md Phase 2.2.4).
//!
//! Builds an `ArticleView` over an in-memory [`Article`] (no network),
//! drives it through [`PuppetHarness`], and asserts on rendered output,
//! link extraction, and dialog-overlay behavior.
//!
//! `ArticleView::wrap_layout` parses `Article::content` on the first
//! layout pass — so the snapshot tests must `step_until_idle` before
//! reading `screen_text`.
//!
//! The "type `5 o`" scenario from the TEST_PLAN row is omitted here
//! because the `o` keypath spawns the configured `url_open_command`
//! (`xdg-open` / `open`) in a thread, which would launch a real
//! browser during `cargo test`. The bounds half of that path is
//! covered by the unit tests for `view::utils::nth_link` and
//! `open_ith_link_in_browser`.

use cursive::event::{Event, Key};
use cursive::view::Nameable;
use cursive::views::{NamedView, OnEventView};
use cursive::Cursive;

use hackernews_tim::client::fake::{FakeCall, FakeHnApi};
use hackernews_tim::client::{init_test_user_info, HnApi};
use hackernews_tim::config::init_test_config;
use hackernews_tim::model::Article;
use hackernews_tim::test_support::PuppetHarness;
use hackernews_tim::view::article_view::{construct_article_main_view, ArticleView};

fn ensure_globals_initialised() {
    init_test_config();
    init_test_user_info(None);
}

fn make_fake_api() -> &'static FakeHnApi {
    Box::leak(Box::new(FakeHnApi::new()))
}

fn fixture_article() -> Article {
    Article {
        title: "Sample article".to_string(),
        url: "https://example.com/post/42".to_string(),
        content: r#"<html><body>
            <p>First paragraph with a <a href="https://one.example">first link</a>.</p>
            <p>Second paragraph mentions <a href="https://two.example">two</a>.</p>
            <p>Third paragraph references <a href="https://three.example">three</a>.</p>
        </body></html>"#
            .to_string(),
        author: Some("alice".to_string()),
        date_published: Some("2024-01-02".to_string()),
    }
}

/// Long fixture used by the scroll test. Plain paragraphs with no
/// links keep the rendered body simple while still tall enough to
/// exceed the puppet's default 40-row viewport.
fn long_article() -> Article {
    let mut content = String::from("<html><body>");
    for i in 0..60 {
        content.push_str(&format!(
            "<p>Paragraph number {i} of the long fixture article body.</p>"
        ));
    }
    content.push_str("</body></html>");
    Article {
        title: "Long article".to_string(),
        url: "https://example.com/long".to_string(),
        content,
        author: None,
        date_published: None,
    }
}

fn add_named_article_view(siv: &mut Cursive, article: Article, fake: &'static FakeHnApi) {
    let api: &'static dyn HnApi = fake;
    let main_view = construct_article_main_view(api, article);
    let named: NamedView<OnEventView<ArticleView>> = main_view.with_name("article_view_outer");
    siv.add_layer(named);
}

fn article_links(harness: &mut PuppetHarness) -> Vec<String> {
    harness
        .cursive_mut()
        .call_on_name("article_view_outer", |v: &mut OnEventView<ArticleView>| {
            v.get_inner().links_for_test()
        })
        .expect("named article view should be present")
}

#[test]
fn renders_article_with_links_snapshot() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    add_named_article_view(&mut siv, fixture_article(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    insta::assert_snapshot!("article_with_links", harness.screen_text());
}

#[test]
fn empty_article_does_not_panic() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    let article = Article {
        title: "Empty".to_string(),
        url: "https://example.com/empty".to_string(),
        content: String::new(),
        author: None,
        date_published: None,
    };
    add_named_article_view(&mut siv, article, fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let text = harness.screen_text();
    assert!(
        !text.is_empty(),
        "expected the article chrome to render even with empty content"
    );
}

#[test]
fn parses_links_in_document_order() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    add_named_article_view(&mut siv, fixture_article(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let links = article_links(&mut harness);
    assert_eq!(
        links,
        vec![
            "https://one.example".to_string(),
            "https://two.example".to_string(),
            "https://three.example".to_string(),
        ],
        "expected links in document order from the first parse pass"
    );
}

#[test]
fn down_key_scrolls_article_body() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    add_named_article_view(&mut siv, long_article(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    let before = harness.screen_text();
    // PageDown is provided by `on_scroll_events()` and shifts the
    // viewport down a full page — enough to make the rendered text
    // strictly different from the initial frame.
    harness.send(Event::Key(Key::PageDown));
    harness.step_until_idle();
    let after = harness.screen_text();

    assert_ne!(
        before, after,
        "expected PageDown to scroll the article body; got identical frames"
    );
}

#[test]
fn l_opens_link_dialog_with_parsed_links() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    add_named_article_view(&mut siv, fixture_article(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('l'));
    harness.step_until_idle();

    let text = harness.screen_text();
    assert!(
        text.contains("https://one.example"),
        "link dialog should list the first link; got:\n{text}"
    );
    assert!(
        text.contains("https://three.example"),
        "link dialog should list the third link; got:\n{text}"
    );
    assert_eq!(
        fake.call_count(),
        0,
        "opening the link dialog should not call into the HN API"
    );
    assert!(
        fake.calls()
            .iter()
            .all(|c| !matches!(c, FakeCall::GetArticle(_))),
        "no get_article call should fire just from showing the dialog"
    );
}

/// Plain-text fixture used by find-dialog tests — three identical
/// occurrences of "alpha", no link styling. Counting matches against
/// [`fixture_article`] is brittle because the parsed link markers
/// (`first link [1]`) embed a literal "first" inside one span.
fn plain_article() -> Article {
    Article {
        title: "Plain article".to_string(),
        url: "https://example.com/plain".to_string(),
        content: "<html><body><p>alpha bravo charlie alpha delta alpha echo</p></body></html>"
            .to_string(),
        author: None,
        date_published: None,
    }
}

#[test]
fn slash_opens_find_dialog() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    add_named_article_view(&mut siv, fixture_article(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('/'));
    harness.step_until_idle();
    assert!(
        harness.screen_text().contains("Find"),
        "find dialog title should be visible after slash; got:\n{}",
        harness.screen_text()
    );
}

#[test]
fn typing_query_populates_match_ids() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    add_named_article_view(&mut siv, plain_article(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('/'));
    harness.step_until_idle();

    // The plain fixture body contains "alpha" three times; typing it
    // should populate match ranges via `apply_find_query`. Drain
    // between typing and Enter so the `Update` signal fires its
    // layout pass before `JumpNext` overwrites `pending`.
    for c in "alpha".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();

    let match_count = harness
        .cursive_mut()
        .call_on_name("article_view_outer", |v: &mut OnEventView<ArticleView>| {
            v.get_inner().match_ids_len_for_test()
        })
        .expect("named article view should be present");
    assert_eq!(
        match_count, 3,
        "expected three matches for 'alpha' in the plain fixture"
    );
}

#[test]
fn esc_in_find_dialog_clears_highlights() {
    ensure_globals_initialised();
    let mut siv = Cursive::new();
    let fake = make_fake_api();
    add_named_article_view(&mut siv, plain_article(), fake);
    let mut harness = PuppetHarness::new(siv);
    harness.step_until_idle();

    harness.send(Event::Char('/'));
    harness.step_until_idle();
    for c in "alpha".chars() {
        harness.send(Event::Char(c));
    }
    harness.step_until_idle();
    harness.send(Event::Key(Key::Enter));
    harness.step_until_idle();

    // Re-open and dismiss with Esc — the close-dialog signal should
    // clear the match_ids set on the outer ArticleView.
    harness.send(Event::Char('/'));
    harness.step_until_idle();
    harness.send(Event::Key(Key::Esc));
    harness.step_until_idle();

    let match_count = harness
        .cursive_mut()
        .call_on_name("article_view_outer", |v: &mut OnEventView<ArticleView>| {
            v.get_inner().match_ids_len_for_test()
        })
        .expect("named article view should be present");
    assert_eq!(
        match_count, 0,
        "Esc on the find dialog should clear the match_ids set"
    );
}
