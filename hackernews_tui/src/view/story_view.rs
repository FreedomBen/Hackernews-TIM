use std::collections::HashMap;
use std::sync::mpsc;

use super::find_bar::{self, FindSignal, FindStateRef};
use super::{
    article_view, async_view, comment_view, help_view::HasHelpView, text_view, traits::*, utils,
};
use crate::client::StoryNumericFilters;
use crate::prelude::*;

static STORY_TAGS: [&str; 5] = ["front_page", "story", "ask_hn", "show_hn", "job"];

type VoteUpdate = (u32, VoteData);

/// StoryView is a View displaying a list stories corresponding
/// to a particular category (top stories, newest stories, most popular stories, etc).
pub struct StoryView {
    pub stories: Vec<Story>,

    view: ScrollView<LinearLayout>,
    raw_command: String,

    starting_id: usize,
    max_id_len: usize,

    // Vote state for stories visible on the HN listing page is fetched
    // eagerly at construction time so the arrows are visible as soon as
    // the list renders. Stories not on that page (later pagination, tags
    // with no HN equivalent) fall back to a lazy per-item fetch on first
    // vote. Completed votes land on `vote_receiver` from a background
    // thread; `wrap_layout` drains it into `vote_state` and repaints the
    // relevant row.
    vote_state: HashMap<u32, VoteData>,
    vote_sender: mpsc::Sender<VoteUpdate>,
    vote_receiver: mpsc::Receiver<VoteUpdate>,
    cb_sink: CbSink,

    find_state: FindStateRef,
}

impl ViewWrapper for StoryView {
    wrap_impl!(self.view: ScrollView<LinearLayout>);

    fn wrap_layout(&mut self, size: Vec2) {
        self.try_update_vote_state();
        self.process_find_signal();
        self.view.layout(size);
    }
}

impl StoryView {
    pub fn new(
        stories: Vec<Story>,
        starting_id: usize,
        cb_sink: CbSink,
        initial_vote_state: HashMap<u32, VoteData>,
        find_state: FindStateRef,
    ) -> Self {
        let max_id_len = Self::compute_max_id_len(stories.len(), starting_id);
        let view =
            Self::construct_story_list(&stories, starting_id, max_id_len, &initial_vote_state);
        let (vote_sender, vote_receiver) = mpsc::channel();
        StoryView {
            view,
            stories,
            raw_command: String::new(),
            starting_id,
            max_id_len,
            vote_state: initial_vote_state,
            vote_sender,
            vote_receiver,
            cb_sink,
            find_state,
        }
    }

    /// The width of the widest story-index prefix, used to align the
    /// metadata lines under each title.
    fn compute_max_id_len(n: usize, starting_id: usize) -> usize {
        let max_id = starting_id + n + 1;
        let mut width = 0;
        let mut pw = 1;
        while pw <= max_id {
            pw *= 10;
            width += 1;
        }
        width
    }

    fn construct_story_list(
        stories: &[Story],
        starting_id: usize,
        max_id_len: usize,
        vote_state: &HashMap<u32, VoteData>,
    ) -> ScrollView<LinearLayout> {
        LinearLayout::vertical()
            .with(|s| {
                stories.iter().enumerate().for_each(|(i, story)| {
                    // initialize the story text with its ID
                    let mut story_text = StyledString::styled(
                        format!("{1:>0$}. ", max_id_len, starting_id + i + 1),
                        config::get_config_theme().component_style.metadata,
                    );
                    story_text.append(Self::get_story_text(
                        max_id_len,
                        story,
                        vote_state.get(&story.id),
                    ));

                    s.add_child(text_view::TextView::new(story_text));
                })
            })
            .scrollable()
    }

    /// Get the text summarizing basic information about a story.
    ///
    /// When `vote` is `Some` and the user has voted in a direction, a
    /// coloured arrow is inserted before the points count.
    fn get_story_text(max_id_len: usize, story: &Story, vote: Option<&VoteData>) -> StyledString {
        let component_style = &config::get_config_theme().component_style;
        let mut story_text = story.styled_title();

        if let Ok(url) = url::Url::parse(&story.url) {
            if let Some(domain) = url.domain() {
                story_text.append_styled(format!(" ({domain})"), component_style.link);
            }
        }

        story_text.append_plain("\n");

        // left-align the story's metadata by `max_id_len+2`, which is the
        // maximum width of a string `{story_id}. `.
        story_text.append_styled(
            format!("{:width$}", " ", width = max_id_len + 2),
            component_style.metadata,
        );

        if let Some(vd) = vote {
            match vd.vote {
                Some(VoteDirection::Up) => story_text.append_styled("▲ ", component_style.upvote),
                Some(VoteDirection::Down) => {
                    story_text.append_styled("▼ ", component_style.downvote)
                }
                None => {}
            }
        }

        story_text.append_styled(
            format!(
                "{} points | by {} | {} ago | {} comments",
                story.points,
                story.author,
                crate::utils::get_elapsed_time_as_text(story.time),
                story.num_comments,
            ),
            component_style.metadata,
        );
        story_text
    }

    inner_getters!(self.view: ScrollView<LinearLayout>);

    /// Toggle or apply a vote in the given direction for the currently
    /// focused story. The story-list endpoints don't include vote state,
    /// so a background thread fetches the story's HN page to recover the
    /// auth token, submits the vote, and returns the resulting
    /// [`VoteData`] via a channel. The UI picks it up on the next layout.
    fn apply_vote(&mut self, direction: VoteDirection, client: &'static client::HNClient) {
        let id = self.get_focus_index();
        if id >= self.stories.len() {
            return;
        }
        let story_id = self.stories[id].id;
        let sender = self.vote_sender.clone();
        let cb_sink = self.cb_sink.clone();

        std::thread::spawn(move || {
            let vd = match client.get_vote_data_for_item(story_id) {
                Ok(Some(vd)) => vd,
                Ok(None) => {
                    warn!(
                        "no vote data found for story (id={}) — is the user logged in?",
                        story_id
                    );
                    return;
                }
                Err(err) => {
                    warn!(
                        "failed to fetch vote data for story (id={}): {}",
                        story_id, err
                    );
                    return;
                }
            };

            // Ignore downvote attempts on items the user lacks privilege for.
            if direction == VoteDirection::Down
                && !vd.can_downvote
                && vd.vote != Some(VoteDirection::Down)
            {
                warn!("downvote not available for story (id={})", story_id);
                return;
            }

            // Re-pressing the same direction rescinds the vote; anything
            // else replaces the current vote with the new direction.
            let new_vote = if vd.vote == Some(direction) {
                None
            } else {
                Some(direction)
            };

            if let Err(err) = client.vote(story_id, &vd.auth, new_vote) {
                error!("failed to vote HN story (id={}): {}", story_id, err);
                return;
            }

            let updated = VoteData {
                auth: vd.auth,
                vote: new_vote,
                can_downvote: vd.can_downvote,
            };
            let _ = sender.send((story_id, updated));
            // Wake Cursive so `wrap_layout` runs and picks up the update.
            let _ = cb_sink.send(Box::new(|_| {}));
        });
    }

    /// Drain completed vote updates from the channel and repaint their rows.
    fn try_update_vote_state(&mut self) {
        while let Ok((story_id, vd)) = self.vote_receiver.try_recv() {
            self.vote_state.insert(story_id, vd);
            if let Some(idx) = self.stories.iter().position(|s| s.id == story_id) {
                self.refresh_story_row(idx);
            }
        }
    }

    /// Move focus by approximately half a viewport's worth of rows,
    /// mirroring vim's Ctrl-D / Ctrl-U semantics. The auto-scroll hook in
    /// `on_set_focus_index` keeps the new focus visible.
    fn move_focus_half_page(&mut self, forward: bool) -> Option<EventResult> {
        let (half_page, width) = {
            let size = self.get_inner().get_scroller().last_available_size();
            ((size.y / 2).max(1), size.x.max(1))
        };
        let constraint = Vec2::new(width, 1);
        let n = self.len();
        if n == 0 {
            return None;
        }
        let current = self.get_focus_index();
        let target = if forward {
            let mut accum = 0usize;
            let mut i = current;
            while i + 1 < n && accum < half_page {
                i += 1;
                if let Some(item) = self.get_item_mut(i) {
                    accum += item.required_size(constraint).y;
                }
            }
            i
        } else {
            let mut accum = 0usize;
            let mut i = current;
            while i > 0 && accum < half_page {
                i -= 1;
                if let Some(item) = self.get_item_mut(i) {
                    accum += item.required_size(constraint).y;
                }
            }
            i
        };
        self.set_focus_index(target)
    }

    fn refresh_story_row(&mut self, id: usize) {
        let text = self.story_row_text(id);
        self.set_story_row_content(id, text);
    }

    /// Build the rendered `StyledString` for the `id`-th story row,
    /// incorporating the numeric prefix and any vote-arrow icon.
    fn story_row_text(&self, id: usize) -> StyledString {
        let max_id_len = self.max_id_len;
        let starting_id = self.starting_id;
        let story = &self.stories[id];
        let vote = self.vote_state.get(&story.id);

        let mut text = StyledString::styled(
            format!("{1:>0$}. ", max_id_len, starting_id + id + 1),
            config::get_config_theme().component_style.metadata,
        );
        text.append(Self::get_story_text(max_id_len, story, vote));
        text
    }

    /// Replace the rendered content of the `id`-th row's inner `TextView`.
    fn set_story_row_content(&mut self, id: usize, text: StyledString) {
        let linear = self.get_inner_list_mut();
        if let Some(child) = linear.get_child_mut(id) {
            if let Some(tv) = child.downcast_mut::<text_view::TextView>() {
                tv.set_content(text);
            }
        }
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
        let style: Style = config::get_config_theme()
            .component_style
            .matched_highlight
            .into();
        let mut matches = Vec::new();
        for id in 0..self.stories.len() {
            let base = self.story_row_text(id);
            let (new_text, ranges) = find_bar::highlight_matches(&base, query, style);
            if !ranges.is_empty() {
                matches.push(id);
            }
            self.set_story_row_content(id, new_text);
        }
        self.find_state.borrow_mut().match_ids = matches;
    }

    fn clear_find_highlights(&mut self) {
        for id in 0..self.stories.len() {
            self.refresh_story_row(id);
        }
    }

    fn jump_to_next_match(&mut self) {
        let current = self.get_focus_index();
        let target = {
            let state = self.find_state.borrow();
            if state.match_ids.is_empty() {
                return;
            }
            state
                .match_ids
                .iter()
                .find(|&&i| i >= current)
                .copied()
                .or_else(|| state.match_ids.first().copied())
        };
        if let Some(target) = target {
            self.set_focus_index(target);
        }
    }

    fn jump_to_prev_match(&mut self) {
        let current = self.get_focus_index();
        let target = {
            let state = self.find_state.borrow();
            if state.match_ids.is_empty() {
                return;
            }
            state
                .match_ids
                .iter()
                .rev()
                .find(|&&i| i < current)
                .copied()
                .or_else(|| state.match_ids.last().copied())
        };
        if let Some(target) = target {
            self.set_focus_index(target);
        }
    }
}

impl ListViewContainer for StoryView {
    fn get_inner_list(&self) -> &LinearLayout {
        self.get_inner().get_inner()
    }

    fn get_inner_list_mut(&mut self) -> &mut LinearLayout {
        self.get_inner_mut().get_inner_mut()
    }

    fn on_set_focus_index(&mut self, old_id: usize, new_id: usize) {
        let direction = old_id <= new_id;

        // enable auto-scrolling when changing the focused index of the view
        self.scroll(direction);
    }
}

impl ScrollViewContainer for StoryView {
    type ScrollInner = LinearLayout;

    fn get_inner_scroll_view(&self) -> &ScrollView<LinearLayout> {
        self.get_inner()
    }

    fn get_inner_scroll_view_mut(&mut self) -> &mut ScrollView<LinearLayout> {
        self.get_inner_mut()
    }
}

pub fn construct_story_main_view(
    stories: Vec<Story>,
    client: &'static client::HNClient,
    starting_id: usize,
    cb_sink: CbSink,
    initial_vote_state: HashMap<u32, VoteData>,
    find_state: FindStateRef,
) -> OnEventView<StoryView> {
    let is_suffix_key =
        |c: &Event| -> bool { config::get_story_view_keymap().goto_story.has_event(c) };

    let story_view_keymap = config::get_story_view_keymap().clone();
    let scroll_keymap = config::get_scroll_keymap().clone();

    let find_state_for_key = find_state.clone();

    OnEventView::new(StoryView::new(
        stories,
        starting_id,
        cb_sink,
        initial_vote_state,
        find_state,
    ))
    // number parsing
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

        // don't allow the inner `LinearLayout` child view to handle the event
        // because of its pre-defined `on_event` function
        Some(EventResult::Ignored)
    })
    // story navigation shortcuts
    .on_pre_event_inner(story_view_keymap.prev_story, |s, _| {
        let id = s.get_focus_index();
        if id == 0 {
            None
        } else {
            s.set_focus_index(id - 1)
        }
    })
    .on_pre_event_inner(story_view_keymap.next_story, |s, _| {
        let id = s.get_focus_index();
        s.set_focus_index(id + 1)
    })
    .on_pre_event_inner(story_view_keymap.goto_story_comment_view, move |s, _| {
        let id = s.get_focus_index();
        // the story struct hasn't had any comments inside yet,
        // so it can be cloned without greatly affecting performance
        let item_id = s.stories[id].id;
        Some(EventResult::with_cb({
            move |s| comment_view::construct_and_add_new_comment_view(s, client, item_id, false)
        }))
    })
    // Reply to the focused story. Same mechanism as the comment view's
    // reply keybind: stash a PendingAction and quit; main::run's outer
    // loop handles the editor spawn and HN POST. `return_to_id` points
    // at the story itself so the TUI re-opens on its comment view where
    // the just-posted reply will appear (after Algolia propagation).
    .on_pre_event_inner(story_view_keymap.reply.clone(), |s, _| {
        let id = s.get_focus_index();
        let story = s.stories[id].clone();
        let parent_id = story.id;
        let parent_content = HnItem::from(story).plain_text();
        Some(EventResult::with_cb(move |siv| {
            siv.set_user_data(crate::reply_editor::PendingAction::ReplyTo {
                parent_id,
                parent_content: parent_content.clone(),
                return_to_id: parent_id,
            });
            siv.quit();
        }))
    })
    // vote shortcuts
    .on_pre_event_inner(story_view_keymap.upvote, move |s, _| {
        s.apply_vote(VoteDirection::Up, client);
        Some(EventResult::Consumed(None))
    })
    .on_pre_event_inner(story_view_keymap.downvote, move |s, _| {
        s.apply_vote(VoteDirection::Down, client);
        Some(EventResult::Consumed(None))
    })
    // open external link shortcuts
    .on_pre_event_inner(story_view_keymap.open_article_in_browser, move |s, _| {
        let id = s.get_focus_index();
        utils::open_url_in_browser(s.stories[id].get_url().as_ref());
        Some(EventResult::Consumed(None))
    })
    .on_pre_event_inner(
        story_view_keymap.open_article_in_article_view,
        move |s, _| {
            let id = s.get_focus_index();
            let url = s.stories[id].url.clone();
            if !url.is_empty() {
                Some(EventResult::with_cb({
                    move |s| article_view::construct_and_add_new_article_view(client, s, &url)
                }))
            } else {
                Some(EventResult::Consumed(None))
            }
        },
    )
    .on_pre_event_inner(story_view_keymap.open_story_in_browser, move |s, _| {
        let url = s.stories[s.get_focus_index()].story_url();
        utils::open_url_in_browser(&url);
        Some(EventResult::Consumed(None))
    })
    .on_pre_event_inner(story_view_keymap.goto_story, move |s, _| {
        match s.raw_command.parse::<usize>() {
            Ok(number) => {
                s.raw_command.clear();
                if number < starting_id + 1 {
                    return None;
                }
                let number = number - 1 - starting_id;
                if number < s.len() {
                    s.set_focus_index(number).unwrap();
                    Some(EventResult::Consumed(None))
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    })
    // Open the find-on-page dialog. Stale highlights from a previous
    // session are cleared before opening so the view doesn't briefly
    // show last-search highlights with an empty new query.
    .on_pre_event(story_view_keymap.find_in_view.clone(), move |s| {
        {
            let mut state = find_state_for_key.borrow_mut();
            state.query.clear();
            state.pending = Some(FindSignal::Clear);
        }
        s.add_layer(find_bar::construct_find_dialog(find_state_for_key.clone()));
    })
    // vim-style half-page cursor movement
    .on_pre_event_inner(scroll_keymap.page_down, |s, _| s.move_focus_half_page(true))
    .on_pre_event_inner(scroll_keymap.page_up, |s, _| s.move_focus_half_page(false))
    .on_scroll_events()
}

fn get_story_view_title_bar(tag: &'static str, sort_mode: client::StorySortMode) -> impl View {
    let style = config::get_config_theme().component_style.title_bar;
    let mut title = StyledString::styled(
        "[Y]",
        Style::from(style).combine(ColorStyle::front(
            config::get_config_theme().palette.light_white,
        )),
    );
    title.append_styled(" Hacker News", style);

    for (i, item) in STORY_TAGS.iter().enumerate() {
        title.append_styled(" | ", style);
        if *item == tag {
            let sort_mode_desc = match sort_mode {
                client::StorySortMode::None => "",
                client::StorySortMode::Date => " (by_date)",
                client::StorySortMode::Points => " (by_point)",
            };
            title.append_styled(
                format!("{}.{}{}", i + 1, item, sort_mode_desc),
                Style::from(style)
                    .combine(config::get_config_theme().component_style.current_story_tag),
            );
        } else {
            title.append_styled(format!("{}.{}", i + 1, item), style);
        }
    }
    title.append_styled(" | ", style);

    let user_info = utils::build_user_info_text(style.into());

    PaddedView::lrtb(
        0,
        0,
        0,
        1,
        Layer::with_color(
            LinearLayout::horizontal()
                .child(TextView::new(title))
                .child(TextView::new(StyledString::new()).full_width())
                .child(TextView::new(user_info)),
            style.into(),
        ),
    )
}

/// Construct a story view given a list of stories.
#[allow(clippy::too_many_arguments)]
pub fn construct_story_view(
    stories: Vec<Story>,
    initial_vote_state: HashMap<u32, VoteData>,
    client: &'static client::HNClient,
    tag: &'static str,
    sort_mode: client::StorySortMode,
    page: usize,
    numeric_filters: client::StoryNumericFilters,
    cb_sink: CbSink,
) -> impl View {
    let starting_id = client::STORY_LIMIT * page;
    let find_state = find_bar::FindState::new_ref();
    let find_state_for_next = find_state.clone();
    let find_state_for_prev = find_state.clone();
    let find_state_for_esc = find_state.clone();
    let main_view = construct_story_main_view(
        stories,
        client,
        starting_id,
        cb_sink,
        initial_vote_state,
        find_state,
    )
    .full_height();

    let mut view = LinearLayout::vertical()
        .child(get_story_view_title_bar(tag, sort_mode))
        .child(main_view)
        .child(utils::construct_footer_view::<StoryView>());
    view.set_focus_index(1)
        .unwrap_or(EventResult::Consumed(None));

    let current_tag_pos = STORY_TAGS
        .iter()
        .position(|t| *t == tag)
        .unwrap_or_else(|| panic!("unkwnown tag {tag}"));

    let story_view_keymap = config::get_story_view_keymap().clone();
    let find_state_for_next_page = find_state_for_next.clone();
    let find_state_for_prev_page = find_state_for_next.clone();
    let find_next_for_next_page = story_view_keymap.find_next_match.clone();
    let find_prev_for_prev_page = story_view_keymap.find_prev_match.clone();

    // Because we re-use the story main view to construct a search view,
    // some of the story keymaps need to be handled here instead of by the main view like
    // for comment views or article views.

    OnEventView::new(view)
        .on_pre_event(config::get_global_keymap().open_help_dialog.clone(), |s| {
            s.add_layer(StoryView::construct_on_event_help_view())
        })
        // Exit find-on-page: clear tracked matches so `n`/`N` revert to
        // their default paging bindings. Returns `None` when no session
        // is active so Esc keeps its usual meaning elsewhere.
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
        // Context-dependent match navigation: when a find session is
        // active, `n`/`N` jump between matches; otherwise they fall
        // through to the paging handlers below. Cursive runs every
        // matching pre-event callback, so the paging handlers also
        // gate themselves on the find state to avoid double-firing.
        .on_pre_event_inner(story_view_keymap.find_next_match.clone(), move |_, _| {
            let mut state = find_state_for_next.borrow_mut();
            if state.match_ids.is_empty() {
                return None;
            }
            state.pending = Some(FindSignal::JumpNext);
            Some(EventResult::Consumed(None))
        })
        .on_pre_event_inner(story_view_keymap.find_prev_match.clone(), move |_, _| {
            let mut state = find_state_for_prev.borrow_mut();
            if state.match_ids.is_empty() {
                return None;
            }
            state.pending = Some(FindSignal::JumpPrev);
            Some(EventResult::Consumed(None))
        })
        .on_pre_event(story_view_keymap.cycle_sort_mode, move |s| {
            // disable "search_by_date" for front_page stories
            if tag == "front_page" {
                return;
            }
            construct_and_add_new_story_view(
                s,
                client,
                tag,
                sort_mode.next(tag),
                0,
                numeric_filters,
                true,
            );
        })
        // story tag navigation
        .on_pre_event(story_view_keymap.next_story_tag, move |s| {
            let next_tag = STORY_TAGS[(current_tag_pos + 1) % STORY_TAGS.len()];
            construct_and_add_new_story_view(
                s,
                client,
                next_tag,
                if next_tag == "story" || next_tag == "job" {
                    client::StorySortMode::Date
                } else {
                    client::StorySortMode::None
                },
                0,
                StoryNumericFilters::default(),
                false,
            );
        })
        .on_pre_event(story_view_keymap.prev_story_tag, move |s| {
            let prev_tag = STORY_TAGS[(current_tag_pos + STORY_TAGS.len() - 1) % STORY_TAGS.len()];
            construct_and_add_new_story_view(
                s,
                client,
                prev_tag,
                if prev_tag == "story" || prev_tag == "job" {
                    client::StorySortMode::Date
                } else {
                    client::StorySortMode::None
                },
                0,
                StoryNumericFilters::default(),
                false,
            );
        })
        // Paging. Gated on the find state because the default bindings
        // overlap with `find_next_match`/`find_prev_match` (both `n`/`N`)
        // and Cursive fires every matching pre-event handler on one
        // keypress — without the gate, a match jump would be stomped by
        // a page rebuild.
        .on_pre_event_inner(story_view_keymap.prev_page, move |_, e| {
            if find_prev_for_prev_page.has_event(e)
                && !find_state_for_prev_page.borrow().match_ids.is_empty()
            {
                return None;
            }
            Some(EventResult::with_cb(move |s| {
                if page > 0 {
                    construct_and_add_new_story_view(
                        s,
                        client,
                        tag,
                        sort_mode,
                        page - 1,
                        numeric_filters,
                        true,
                    );
                }
            }))
        })
        .on_pre_event_inner(story_view_keymap.next_page, move |_, e| {
            if find_next_for_next_page.has_event(e)
                && !find_state_for_next_page.borrow().match_ids.is_empty()
            {
                return None;
            }
            Some(EventResult::with_cb(move |s| {
                construct_and_add_new_story_view(
                    s,
                    client,
                    tag,
                    sort_mode,
                    page + 1,
                    numeric_filters,
                    true,
                );
            }))
        })
}

/// Retrieve a list of stories satisfying some conditions and construct a story view displaying them.
pub fn construct_and_add_new_story_view(
    s: &mut Cursive,
    client: &'static client::HNClient,
    tag: &'static str,
    sort_mode: client::StorySortMode,
    page: usize,
    numeric_filters: client::StoryNumericFilters,
    pop_layer: bool,
) {
    let async_view =
        async_view::construct_story_view_async(s, client, tag, sort_mode, page, numeric_filters);
    if pop_layer {
        s.pop_layer();
    }
    s.screen_mut().add_transparent_layer(Layer::new(async_view));
}
