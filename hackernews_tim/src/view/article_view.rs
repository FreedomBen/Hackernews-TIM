use super::find_bar::{self, FindSignal, FindStateRef};
use super::text_view::TextView as HnTextView;
use super::{async_view, help_view::HasHelpView, link_dialog, traits::*, utils};
use crate::prelude::*;

/// ArticleView is a View used to display the content of a web page in reader mode
pub struct ArticleView {
    article: Article,
    links: Vec<String>,
    width: usize,

    view: ScrollView<LinearLayout>,

    raw_command: String,

    find_state: FindStateRef,
    /// The un-highlighted content as produced by the last re-parse.
    /// `apply_find_query` layers highlights on top of this; `clear`
    /// restores the displayed text back to it.
    base_content: Option<StyledString>,
    /// Source-byte ranges of the current find matches, in document
    /// order. Populated by `apply_find_query`; consumed by the
    /// jump-to-match handlers.
    match_ranges: Vec<(usize, usize)>,
    /// Index of the last match jumped to. `None` until the user first
    /// invokes `find_next_match` or `find_prev_match`.
    current_match: Option<usize>,
}

impl ViewWrapper for ArticleView {
    wrap_impl!(self.view: ScrollView<LinearLayout>);

    fn wrap_layout(&mut self, size: Vec2) {
        if self.width != size.x {
            // got a new width since the last time the article view is rendered,
            // re-parse the article using the new width

            self.width = size.x;

            match self.article.parse(self.width.saturating_sub(5)) {
                Ok(result) => {
                    self.base_content = Some(result.content.clone());
                    self.set_article_content(result.content);
                    self.links = result.links;
                    // A fresh parse wipes any existing highlights; re-apply
                    // them from the active query if a find session is live.
                    let query = self.find_state.borrow().query.clone();
                    if !query.is_empty() {
                        self.apply_find_query(&query);
                    }
                }
                Err(err) => {
                    warn!("failed to parse the article: {}", err);
                }
            }
        }

        // Run layout before draining find signals: jump-to-match
        // translates byte offsets to row indices via TextView row data,
        // which only exists after `TextView::layout` runs.
        self.with_view_mut(|v| v.layout(size));
        self.process_find_signal();
        // Update/Clear signals replace the inner TextView content;
        // its cached rows still point into the old bytes. Re-layout so
        // the next draw indexes the fresh content rather than panicking
        // off the end of a stale span.
        self.with_view_mut(|v| v.layout(size));
    }

    fn wrap_take_focus(&mut self, _: Direction) -> Result<EventResult, CannotFocus> {
        Ok(EventResult::Consumed(None))
    }
}

impl ArticleView {
    pub fn new(article: Article, find_state: FindStateRef) -> Self {
        let component_style = &config::get_config_theme().component_style;
        let unknown = "[unknown]".to_string();
        let desc = format!(
            "by: {}, date_published: {}",
            article.author.as_ref().unwrap_or(&unknown),
            article.date_published.as_ref().unwrap_or(&unknown),
        );

        let view = LinearLayout::vertical()
            .child(TextView::new(&article.title).center().full_width())
            .child(
                TextView::new(StyledString::styled(desc, component_style.metadata))
                    .center()
                    .full_width(),
            )
            .child(PaddedView::lrtb(1, 1, 1, 1, HnTextView::new("")))
            .scrollable();

        ArticleView {
            article,
            links: vec![],
            width: 0,

            view,
            raw_command: "".to_string(),

            find_state,
            base_content: None,
            match_ranges: Vec::new(),
            current_match: None,
        }
    }

    /// Snapshot of the link list as collected by the most recent
    /// parse pass. Test-only accessor — production code reads
    /// `self.links` directly.
    #[cfg(any(test, feature = "test-support"))]
    pub fn links_for_test(&self) -> Vec<String> {
        self.links.clone()
    }

    /// Number of currently active find matches. Test-only accessor
    /// — production code reads `self.find_state.match_ids` via the
    /// shared `FindStateRef`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn match_ids_len_for_test(&self) -> usize {
        self.find_state.borrow().match_ids.len()
    }

    /// Update the content of the article
    pub fn set_article_content(&mut self, new_content: StyledString) {
        self.view
            .get_inner_mut()
            .get_child_mut(2)
            .expect("The article view should have 3 children")
            .downcast_mut::<PaddedView<HnTextView>>()
            .expect("The 3rd child of the article view should be a padded text view")
            .get_inner_mut()
            .set_content(new_content)
    }

    /// Poll the shared find state and apply any pending signal.
    fn process_find_signal(&mut self) {
        let signal = self.find_state.borrow_mut().pending.take();
        match signal {
            Some(FindSignal::Update) => {
                let query = self.find_state.borrow().query.clone();
                self.apply_find_query(&query);
            }
            Some(FindSignal::Clear) => {
                self.clear_find_highlights();
                let mut state = self.find_state.borrow_mut();
                state.query.clear();
                state.match_ids.clear();
            }
            Some(FindSignal::JumpNext) => self.jump_to_next_match(),
            Some(FindSignal::JumpPrev) => self.jump_to_prev_match(),
            None => {}
        }
    }

    fn apply_find_query(&mut self, query: &str) {
        let Some(base) = self.base_content.clone() else {
            return;
        };
        let style: Style = config::get_config_theme()
            .component_style
            .matched_highlight
            .into();
        let (highlighted, ranges) = find_bar::highlight_matches(&base, query, style);
        self.match_ranges = ranges;
        self.current_match = None;
        // Publish match presence so outer keymap logic can fall through
        // `n`/`N` to scroll bindings when no session is active.
        self.find_state.borrow_mut().match_ids = (0..self.match_ranges.len()).collect();
        self.set_article_content(highlighted);
    }

    fn clear_find_highlights(&mut self) {
        self.match_ranges.clear();
        self.current_match = None;
        if let Some(base) = self.base_content.clone() {
            self.set_article_content(base);
        }
    }

    fn jump_to_next_match(&mut self) {
        if self.match_ranges.is_empty() {
            return;
        }
        let next = match self.current_match {
            None => 0,
            Some(i) => (i + 1) % self.match_ranges.len(),
        };
        self.current_match = Some(next);
        let (offset, _) = self.match_ranges[next];
        self.scroll_to_source_offset(offset);
    }

    fn jump_to_prev_match(&mut self) {
        if self.match_ranges.is_empty() {
            return;
        }
        let len = self.match_ranges.len();
        let prev = match self.current_match {
            None => len - 1,
            Some(0) => len - 1,
            Some(i) => i - 1,
        };
        self.current_match = Some(prev);
        let (offset, _) = self.match_ranges[prev];
        self.scroll_to_source_offset(offset);
    }

    /// Scroll the ScrollView so the row containing `offset` in the
    /// article body lands at (or near) the top of the viewport. The
    /// article body is child 2 of the inner LinearLayout, wrapped in a
    /// `PaddedView` with a 1-row top padding.
    fn scroll_to_source_offset(&mut self, offset: usize) {
        let body_row = {
            let linear = self.view.get_inner();
            let Some(padded) = linear
                .get_child(2)
                .and_then(|c| c.downcast_ref::<PaddedView<HnTextView>>())
            else {
                return;
            };
            match padded.get_inner().row_for_byte_offset(offset) {
                Some(r) => r,
                None => return,
            }
        };

        let width = self.width.max(1);
        let constraint = Vec2::new(width, 1);
        let prefix_height = {
            let linear = self.view.get_inner_mut();
            let mut h = 0usize;
            for i in 0..2 {
                if let Some(child) = linear.get_child_mut(i) {
                    h += child.required_size(constraint).y;
                }
            }
            // +1 for the PaddedView's top padding on child 2.
            h + 1
        };

        let target_y = prefix_height + body_row;
        self.view
            .get_scroller_mut()
            .set_offset(Vec2::new(0, target_y));
    }

    inner_getters!(self.view: ScrollView<LinearLayout>);
}

impl ScrollViewContainer for ArticleView {
    type ScrollInner = LinearLayout;

    fn get_inner_scroll_view(&self) -> &ScrollView<LinearLayout> {
        self.get_inner()
    }

    fn get_inner_scroll_view_mut(&mut self) -> &mut ScrollView<LinearLayout> {
        self.get_inner_mut()
    }
}

pub fn construct_article_main_view(
    client: &'static dyn client::HnApi,
    article: Article,
) -> OnEventView<ArticleView> {
    let is_suffix_key = |c: &Event| -> bool {
        let article_view_keymap = config::get_article_view_keymap();
        article_view_keymap.open_link_in_browser.has_event(c)
            || article_view_keymap.open_link_in_article_view.has_event(c)
    };

    let article_view_keymap = config::get_article_view_keymap().clone();
    let find_state = find_bar::FindState::new_ref();
    let find_state_for_key = find_state.clone();
    let find_state_for_next = find_state.clone();
    let find_state_for_prev = find_state.clone();
    let find_state_for_esc = find_state.clone();

    OnEventView::new(ArticleView::new(article, find_state))
        .on_pre_event_inner(EventTrigger::from_fn(|_| true), move |s, e| {
            match *e {
                Event::Char(c) if c.is_ascii_digit() => {
                    s.raw_command.push(c);
                }
                _ => {
                    if !is_suffix_key(e) {
                        s.raw_command.clear();
                    }
                }
            };
            None
        })
        .on_pre_event_inner(article_view_keymap.open_link_dialog, move |s, _| {
            Some(EventResult::with_cb({
                let links = s.links.clone();
                move |s| {
                    s.add_layer(link_dialog::get_link_dialog(client, &links));
                }
            }))
        })
        .on_pre_event_inner(article_view_keymap.open_link_in_browser, |s, _| {
            match s.raw_command.parse::<usize>() {
                Ok(num) => {
                    s.raw_command.clear();
                    utils::open_ith_link_in_browser(&s.links, num)
                }
                Err(_) => None,
            }
        })
        .on_pre_event_inner(
            article_view_keymap.open_link_in_article_view,
            move |s, _| match s.raw_command.parse::<usize>() {
                Ok(num) => {
                    s.raw_command.clear();
                    utils::open_ith_link_in_article_view(client, &s.links, num)
                }
                Err(_) => None,
            },
        )
        .on_pre_event_inner(article_view_keymap.open_article_in_browser, |s, _| {
            utils::open_url_in_browser(&s.article.url);
            Some(EventResult::Consumed(None))
        })
        .on_pre_event(config::get_global_keymap().open_help_dialog.clone(), |s| {
            s.add_layer(ArticleView::construct_on_event_help_view())
        })
        // Open the find-on-page dialog. Enter in the dialog sends
        // JumpNext, which scrolls to the next match.
        .on_pre_event(article_view_keymap.find_in_view.clone(), move |s| {
            {
                let mut state = find_state_for_key.borrow_mut();
                state.query.clear();
                state.pending = Some(FindSignal::Clear);
            }
            s.add_layer(find_bar::construct_find_dialog(find_state_for_key.clone()));
        })
        // Exit find-on-page: clear tracked matches so `n`/`N` revert to
        // their default bindings. Returns `None` when no session is
        // active so Esc keeps its usual meaning elsewhere.
        .on_pre_event_inner(
            config::get_global_keymap().close_dialog.clone(),
            move |_, _| {
                let mut state = find_state_for_esc.borrow_mut();
                if state.match_ids.is_empty() {
                    return None;
                }
                state.match_ids.clear();
                state.pending = Some(FindSignal::Clear);
                Some(EventResult::Consumed(None))
            },
        )
        // Context-dependent match nav: `n`/`N` jump between matches
        // only while a find session is active. Registered as
        // `on_pre_event_inner` so returning None lets other scroll
        // bindings pick up the event.
        .on_pre_event_inner(article_view_keymap.find_next_match.clone(), move |_, _| {
            let mut state = find_state_for_next.borrow_mut();
            if state.match_ids.is_empty() {
                return None;
            }
            state.pending = Some(FindSignal::JumpNext);
            Some(EventResult::Consumed(None))
        })
        .on_pre_event_inner(article_view_keymap.find_prev_match.clone(), move |_, _| {
            let mut state = find_state_for_prev.borrow_mut();
            if state.match_ids.is_empty() {
                return None;
            }
            state.pending = Some(FindSignal::JumpPrev);
            Some(EventResult::Consumed(None))
        })
        .on_scroll_events()
}

/// Construct an article view of an article
pub fn construct_article_view(client: &'static dyn client::HnApi, article: Article) -> impl View {
    let desc = format!("Article View - {}", article.title);
    let main_view = construct_article_main_view(client, article).full_height();

    let mut view = LinearLayout::vertical()
        .child(utils::construct_view_title_bar(client, &desc))
        .child(main_view)
        .child(utils::construct_footer_view::<ArticleView>());
    view.set_focus_index(1)
        .unwrap_or(EventResult::Consumed(None));

    view
}

/// Retrieve an article from a given `url` and construct an article view of that article
pub fn construct_and_add_new_article_view(
    client: &'static dyn client::HnApi,
    s: &mut Cursive,
    url: &str,
) {
    let async_view = async_view::construct_article_view_async(client, s, url);
    s.screen_mut().add_transparent_layer(Layer::new(async_view))
}
