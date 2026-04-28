use super::find_bar::{self, FindSignal, FindStateRef};
use super::{article_view, async_view, help_view::HasHelpView, text_view, traits::*, utils};
use crate::prelude::*;
use crate::view::text_view::{StyledPaddingChar, TextPadding};

type SingleItemView = HideableView<PaddedView<text_view::TextView>>;

/// CommentView is a View displaying a list of comments in a HN story
pub struct CommentView {
    view: ScrollView<LinearLayout>,
    items: Vec<HnItem>,
    data: PageData,

    raw_command: String,

    find_state: FindStateRef,
}

pub enum NavigationDirection {
    Next,
    Previous,
}

impl ViewWrapper for CommentView {
    wrap_impl!(self.view: ScrollView<LinearLayout>);

    fn wrap_layout(&mut self, size: Vec2) {
        self.process_find_signal();
        self.view.layout(size);
    }
}

impl CommentView {
    pub fn new(data: PageData, find_state: FindStateRef) -> Self {
        let mut view = CommentView {
            view: LinearLayout::vertical()
                .child(HideableView::new(PaddedView::lrtb(
                    1,
                    1,
                    0,
                    1,
                    text_view::TextView::new(
                        data.root_item
                            .text(data.vote_state.get(&data.root_item.id.to_string())),
                    ),
                )))
                .scrollable(),
            items: vec![data.root_item.clone()],
            raw_command: String::new(),
            data,
            find_state,
        };

        view.try_update_comments();
        view
    }

    /// Poll the shared find state and apply any pending signal. Called
    /// from `wrap_layout` so the UI reacts on the layout pass that
    /// follows a find dialog keystroke.
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
            Some(FindSignal::JumpNext) => {
                self.jump_to_next_match();
            }
            Some(FindSignal::JumpPrev) => {
                self.jump_to_prev_match();
            }
            None => {}
        }
    }

    /// Re-highlight every item with the new `query` and publish the
    /// matched indices on the shared state. Non-matching items are still
    /// re-rendered from their base text so stale highlights clear.
    fn apply_find_query(&mut self, query: &str) {
        let style: Style = config::get_config_theme()
            .component_style
            .matched_highlight
            .into();
        let mut matches = Vec::new();
        for id in 0..self.items.len() {
            let base = self.items[id].text(self.get_vote_status(self.items[id].id));
            let (new_text, ranges) = find_bar::highlight_matches(&base, query, style);
            if !ranges.is_empty() {
                matches.push(id);
            }
            self.get_item_view_mut(id)
                .get_inner_mut()
                .get_inner_mut()
                .set_content(new_text);
        }
        self.find_state.borrow_mut().match_ids = matches;
    }

    /// Restore each item's canonical text from its state-based renderer.
    fn clear_find_highlights(&mut self) {
        for id in 0..self.items.len() {
            self.update_item_text_content(id);
        }
    }

    /// Move focus to the next matched item strictly after the current
    /// focus. Wraps to the first match when none follow — including the
    /// case where the current focus is itself the last match, so
    /// pressing `n` on the final hit cycles back to the first.
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
                .find(|&&i| i > current)
                .copied()
                .or_else(|| state.match_ids.first().copied())
        };
        if let Some(target) = target {
            self.set_focus_index(target);
        }
    }

    /// Move focus to the previous matched item strictly before the
    /// current focus. Wraps to the last match when none precede.
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

    /// Check the comment receiver channel if there are new comments loaded
    /// then update the internal comment data accordingly.
    pub fn try_update_comments(&mut self) {
        let mut new_comments = vec![];
        // limit the number of top comments updated each time
        let mut limit = 5;
        while !self.data.comment_receiver.is_empty() && limit > 0 {
            if let Ok(mut comments) = self.data.comment_receiver.try_recv() {
                new_comments.append(&mut comments);
            }
            limit -= 1;
        }

        if new_comments.is_empty() {
            return;
        }

        let mut new_items = new_comments
            .into_iter()
            .map(Into::<HnItem>::into)
            .collect::<Vec<_>>();

        new_items.iter().for_each(|item| {
            let text_view = text_view::TextView::new(item.text(self.get_vote_status(item.id)));
            self.add_item(HideableView::new(PaddedView::lrtb(
                item.level * 2 + 1,
                1,
                0,
                1,
                if item.level > 0 {
                    // get the padding style (color) based on the comment's height
                    //
                    // We use base 16 colors to display the comment's padding
                    let c = config::Color::from((item.level % 16) as u8);
                    text_view
                        .padding(TextPadding::default().left(StyledPaddingChar::new('▎', c.into())))
                } else {
                    // add top padding for top comments, use the first color in the 16 base colors
                    let c = config::Color::from(0);
                    text_view
                        .padding(TextPadding::default().top(StyledPaddingChar::new('▔', c.into())))
                },
            )));
        });
        self.items.append(&mut new_items);

        // update the view's layout
        self.layout(
            self.get_inner_scroll_view()
                .get_scroller()
                .last_outer_size(),
        )
    }

    /// Return the id of the first item (`direction` dependent),
    /// whose level is less than or equal `max_level`.
    pub fn find_item_id_by_max_level(
        &self,
        start_id: usize,
        max_level: usize,
        direction: NavigationDirection,
    ) -> usize {
        match direction {
            NavigationDirection::Next => (start_id + 1..self.len())
                .find(|&id| self.items[id].level <= max_level)
                .unwrap_or_else(|| self.len()),
            NavigationDirection::Previous => (0..start_id)
                .rfind(|&id| self.items[id].level <= max_level)
                .unwrap_or(start_id),
        }
    }

    /// Return the id of the next visible item (`direction` dependent)
    pub fn find_next_visible_item(&self, start_id: usize, direction: NavigationDirection) -> usize {
        match direction {
            NavigationDirection::Next => (start_id + 1..self.len())
                .find(|&id| self.get_item_view(id).is_visible())
                .unwrap_or_else(|| self.len()),
            NavigationDirection::Previous => (0..start_id)
                .rfind(|&id| self.get_item_view(id).is_visible())
                .unwrap_or(start_id),
        }
    }

    /// Return the id of the next/previous sibling of the item at
    /// `start_id`, wrapping within the sibling group. Two items are
    /// siblings when they share the same immediate parent — i.e. they
    /// sit at the same indentation level inside the same parent's
    /// subtree. Items in the parent's subtree are stored in DFS
    /// preorder, so the parent is the nearest preceding item with a
    /// strictly lower level, and the subtree ends at the next item
    /// whose level is `<=` the parent's. Top-level (level 0) items
    /// keep their historical scope (all level-0 items, no wrap) by
    /// delegating to `find_item_id_by_max_level`.
    pub fn find_sibling(&self, start_id: usize, direction: NavigationDirection) -> usize {
        let level = self.items[start_id].level;

        if level == 0 {
            return self.find_item_id_by_max_level(start_id, 0, direction);
        }

        let Some(parent_idx) = (0..start_id).rfind(|&i| self.items[i].level < level) else {
            return start_id;
        };
        let parent_level = self.items[parent_idx].level;

        let subtree_end = ((parent_idx + 1)..self.len())
            .find(|&i| self.items[i].level <= parent_level)
            .unwrap_or_else(|| self.len());

        let siblings: Vec<usize> = ((parent_idx + 1)..subtree_end)
            .filter(|&i| self.items[i].level == level)
            .collect();

        if siblings.len() <= 1 {
            return start_id;
        }

        let pos = siblings.iter().position(|&i| i == start_id).unwrap_or(0);
        let new_pos = match direction {
            NavigationDirection::Next => (pos + 1) % siblings.len(),
            NavigationDirection::Previous => (pos + siblings.len() - 1) % siblings.len(),
        };
        siblings[new_pos]
    }

    fn get_vote_status(&self, item_id: u32) -> Option<&VoteData> {
        self.data.vote_state.get(&item_id.to_string())
    }

    /// Toggle or apply a vote in the given direction for the currently
    /// focused item. The HTTP request runs on a background thread and the
    /// UI reflects the new vote immediately (optimistic update); a failed
    /// request is logged but the local state is not rolled back.
    fn apply_vote(&mut self, direction: VoteDirection, client: &'static client::HNClient) -> bool {
        let id = self.get_focus_index();
        let item_id = self.items[id].id;

        let (new_vote, auth) = {
            let Some(vd) = self.data.vote_state.get_mut(&item_id.to_string()) else {
                return false;
            };
            // Ignore downvote attempts on items the user lacks privilege for.
            if direction == VoteDirection::Down
                && !vd.can_downvote
                && vd.vote != Some(VoteDirection::Down)
            {
                return false;
            }
            // Re-pressing the same direction rescinds the vote; anything
            // else replaces the current vote with the new direction.
            let new_vote = if vd.vote == Some(direction) {
                None
            } else {
                Some(direction)
            };
            vd.vote = new_vote;
            (new_vote, vd.auth.clone())
        };

        std::thread::spawn(move || {
            if let Err(err) = client.vote(item_id, &auth, new_vote) {
                tracing::error!("Failed to vote HN item (id={item_id}): {err}");
            }
        });

        self.update_item_text_content(id);
        true
    }

    /// Toggle the viewer's vouch on the focused item.
    ///
    /// Absence of a [`VouchData`] entry means HN didn't render a vouch
    /// link for the item on this page — either the item isn't dead, the
    /// viewer lacks vouch privilege, or the viewer authored it. In any of
    /// those cases the key no-ops. The HTTP request is fire-and-forget;
    /// the local `vouched` flag flips optimistically so an immediate
    /// second keypress rescinds rather than re-vouches.
    fn apply_vouch(&mut self, client: &'static client::HNClient) -> bool {
        let id = self.get_focus_index();
        let item_id = self.items[id].id;

        let (rescind, auth) = {
            let Some(vd) = self.data.vouch_state.get_mut(&item_id.to_string()) else {
                return false;
            };
            let rescind = vd.vouched;
            vd.vouched = !rescind;
            (rescind, vd.auth.clone())
        };

        std::thread::spawn(move || {
            if let Err(err) = client.vouch(item_id, &auth, rescind) {
                tracing::error!("Failed to vouch HN item (id={item_id}): {err}");
            }
        });

        true
    }

    fn get_item_view(&self, id: usize) -> &SingleItemView {
        self.get_item(id)
            .unwrap()
            .downcast_ref::<SingleItemView>()
            .unwrap()
    }

    fn get_item_view_mut(&mut self, id: usize) -> &mut SingleItemView {
        self.get_item_mut(id)
            .unwrap()
            .downcast_mut::<SingleItemView>()
            .unwrap()
    }

    /// Toggle the collapsing state of items whose levels are greater than the `min_level`.
    fn toggle_items_collapse_state(&mut self, start_id: usize, min_level: usize) {
        // This function will be called recursively until it's unable to find any items.
        //
        // Note: collapsed item's state is unchanged, we only toggle its visibility.
        // Also, the state and visibility of such item's children are unaffected as they should already
        // be in a hidden state (as result of that item's collapsed state).
        if start_id == self.len() || self.items[start_id].level <= min_level {
            return;
        }
        match self.items[start_id].display_state {
            DisplayState::Hidden => {
                self.items[start_id].display_state = DisplayState::Normal;
                self.get_item_view_mut(start_id).unhide();
                self.toggle_items_collapse_state(start_id + 1, min_level)
            }
            DisplayState::Normal => {
                self.items[start_id].display_state = DisplayState::Hidden;
                self.get_item_view_mut(start_id).hide();
                self.toggle_items_collapse_state(start_id + 1, min_level)
            }
            DisplayState::Minimized => {
                let component = self.get_item_view_mut(start_id);
                if component.is_visible() {
                    component.hide();
                } else {
                    component.unhide();
                }

                // skip toggling all children of the current item
                let next_id = self.find_item_id_by_max_level(
                    start_id,
                    self.items[start_id].level,
                    NavigationDirection::Next,
                );
                self.toggle_items_collapse_state(next_id, min_level)
            }
        };
    }

    /// Toggle the collapsing state of currently focused item and its children
    pub fn toggle_collapse_focused_item(&mut self) {
        let id = self.get_focus_index();
        match self.items[id].display_state {
            DisplayState::Hidden => {
                panic!(
                    "invalid collapse state `Collapsed` when calling `toggle_collapse_focused_item`"
                );
            }
            DisplayState::Minimized => {
                self.toggle_items_collapse_state(id + 1, self.items[id].level);
                self.items[id].display_state = DisplayState::Normal;
            }
            DisplayState::Normal => {
                self.toggle_items_collapse_state(id + 1, self.items[id].level);
                self.items[id].display_state = DisplayState::Minimized;
            }
        };
        self.update_item_text_content(id);
    }

    /// Move focus by approximately half a viewport's worth of rows,
    /// mirroring vim's Ctrl-D / Ctrl-U semantics. Hidden (collapsed)
    /// items are skipped via `find_next_visible_item`; the auto-scroll
    /// hook in `on_set_focus_index` keeps the new focus visible.
    fn move_focus_half_page(&mut self, forward: bool) -> Option<EventResult> {
        let (half_page, width) = {
            let size = self
                .get_inner_scroll_view()
                .get_scroller()
                .last_available_size();
            ((size.y / 2).max(1), size.x.max(1))
        };
        let constraint = Vec2::new(width, 1);
        let n = self.len();
        if n == 0 {
            return None;
        }
        let current = self.get_focus_index();
        let mut target = current;
        let mut accum = 0usize;
        loop {
            let dir = if forward {
                NavigationDirection::Next
            } else {
                NavigationDirection::Previous
            };
            let next = self.find_next_visible_item(target, dir);
            if next == target || next == n {
                break;
            }
            target = next;
            if let Some(item) = self.get_item_mut(target) {
                accum += item.required_size(constraint).y;
            }
            if accum >= half_page {
                break;
            }
        }
        self.set_focus_index(target)
    }

    /// Update the `id`-th item's text content based on its state-based text
    pub fn update_item_text_content(&mut self, id: usize) {
        let new_content = self.items[id].text(self.get_vote_status(self.items[id].id));
        self.get_item_view_mut(id)
            .get_inner_mut()
            .get_inner_mut()
            .set_content(new_content);
    }

    inner_getters!(self.view: ScrollView<LinearLayout>);
}

impl ListViewContainer for CommentView {
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

impl ScrollViewContainer for CommentView {
    type ScrollInner = LinearLayout;

    fn get_inner_scroll_view(&self) -> &ScrollView<LinearLayout> {
        self.get_inner()
    }

    fn get_inner_scroll_view_mut(&mut self) -> &mut ScrollView<LinearLayout> {
        self.get_inner_mut()
    }
}

/// Resolve the link-open numeric prefix typed by the user. Empty input
/// (a bare `o`/`O`) defaults to link 1 so the threads view's "re: parent
/// thread" header opens with a single keystroke. A non-empty buffer
/// containing junk returns `None`, leaving the event for downstream
/// handlers.
fn parse_link_index(raw_command: &str) -> Option<usize> {
    if raw_command.is_empty() {
        Some(1)
    } else {
        raw_command.parse::<usize>().ok()
    }
}

fn construct_comment_main_view(client: &'static client::HNClient, data: PageData) -> impl View {
    let is_suffix_key = |c: &Event| -> bool {
        let comment_view_keymap = config::get_comment_view_keymap();
        comment_view_keymap.open_link_in_browser.has_event(c)
            || comment_view_keymap.open_link_in_article_view.has_event(c)
    };

    let comment_view_keymap = config::get_comment_view_keymap().clone();
    let scroll_keymap = config::get_scroll_keymap().clone();

    let article_url = data.url.clone();
    let page_url = format!("{}/item?id={}", client::HN_HOST_URL, data.root_item.id);

    let find_state = find_bar::FindState::new_ref();
    let find_state_for_key = find_state.clone();
    let find_state_for_next = find_state.clone();
    let find_state_for_prev = find_state.clone();
    let find_state_for_esc = find_state.clone();
    let find_state_for_ntl = find_state.clone();
    let find_state_for_ptl = find_state.clone();
    let find_state_for_sib_back = find_state.clone();
    let find_next_for_ntl = comment_view_keymap.find_next_match.clone();

    OnEventView::new(CommentView::new(data, find_state))
        .on_pre_event_inner(EventTrigger::from_fn(|_| true), move |s, e| {
            s.try_update_comments();

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
        .on_pre_event_inner(comment_view_keymap.upvote, move |s, _| {
            s.apply_vote(VoteDirection::Up, client);
            Some(EventResult::Consumed(None))
        })
        .on_pre_event_inner(comment_view_keymap.downvote, move |s, _| {
            s.apply_vote(VoteDirection::Down, client);
            Some(EventResult::Consumed(None))
        })
        .on_pre_event_inner(comment_view_keymap.vouch, move |s, _| {
            s.apply_vouch(client);
            Some(EventResult::Consumed(None))
        })
        // Reply to the focused item. Stashes the request into Cursive
        // user data and quits; `main::run`'s outer loop reads it, spawns
        // `$EDITOR`, then re-enters the TUI.
        .on_pre_event_inner(comment_view_keymap.reply.clone(), |s, _| {
            let id = s.get_focus_index();
            let parent_id = s.items[id].id;
            let parent_content = s.items[id].plain_text();
            let return_to_id = s.data.root_item.id;
            Some(EventResult::with_cb(move |siv| {
                siv.set_user_data(crate::reply_editor::PendingAction::ReplyTo {
                    parent_id,
                    parent_content: parent_content.clone(),
                    return_to_id,
                });
                siv.quit();
            }))
        })
        // Edit the focused comment. Only fires if the focused item is
        // authored by the logged-in user — otherwise the key falls through
        // silently (HN would reject the edit anyway, but pre-gating avoids
        // making the user type a full edit just to hit that rejection).
        .on_pre_event_inner(comment_view_keymap.edit.clone(), |s, _| {
            let id = s.get_focus_index();
            let item = &s.items[id];
            let me = client::get_user_info().map(|ui| ui.username.as_str())?;
            let author = item.author.as_deref()?;
            if author != me {
                return None;
            }
            let comment_id = item.id;
            let return_to_id = s.data.root_item.id;
            Some(EventResult::with_cb(move |siv| {
                siv.set_user_data(crate::reply_editor::PendingAction::EditComment {
                    comment_id,
                    return_to_id,
                });
                siv.quit();
            }))
        })
        // comment navigation shortcuts. Returning `None` when the cursor
        // is already at the top/bottom lets the event bubble up to the
        // outer `LinearLayout::vertical` so it can shift focus into the
        // nav strip or footer.
        .on_pre_event_inner(comment_view_keymap.prev_comment, |s, _| {
            let current = s.get_focus_index();
            let target = s.find_next_visible_item(current, NavigationDirection::Previous);
            if target == current {
                None
            } else {
                s.set_focus_index(target)
            }
        })
        .on_pre_event_inner(comment_view_keymap.next_comment, |s, _| {
            let current = s.get_focus_index();
            let target = s.find_next_visible_item(current, NavigationDirection::Next);
            if target == current || target >= s.len() {
                None
            } else {
                s.set_focus_index(target)
            }
        })
        .on_pre_event_inner(comment_view_keymap.next_leq_level_comment, move |s, _| {
            let id = s.get_focus_index();
            let next_id =
                s.find_item_id_by_max_level(id, s.items[id].level, NavigationDirection::Next);
            s.set_focus_index(next_id)
        })
        .on_pre_event_inner(comment_view_keymap.prev_leq_level_comment, move |s, _| {
            let id = s.get_focus_index();
            let next_id =
                s.find_item_id_by_max_level(id, s.items[id].level, NavigationDirection::Previous);
            s.set_focus_index(next_id)
        })
        // Exit find-on-page: clear tracked matches so `n`/`N` revert to
        // their default comment-navigation bindings. Returns `None` when
        // no session is active so Esc keeps its usual meaning elsewhere.
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
        // Context-dependent match navigation: `n`/`N` jump between find
        // matches when a session is active, otherwise fall through to the
        // existing next/prev_top_level_comment bindings below.
        .on_pre_event_inner(comment_view_keymap.find_next_match.clone(), move |_, _| {
            let mut state = find_state_for_next.borrow_mut();
            if state.match_ids.is_empty() {
                return None;
            }
            state.pending = Some(FindSignal::JumpNext);
            Some(EventResult::Consumed(None))
        })
        .on_pre_event_inner(comment_view_keymap.find_prev_match.clone(), move |_, _| {
            let mut state = find_state_for_prev.borrow_mut();
            if state.match_ids.is_empty() {
                return None;
            }
            state.pending = Some(FindSignal::JumpPrev);
            Some(EventResult::Consumed(None))
        })
        // Cursive runs every matching pre-event callback, not just the
        // first — so on the default binding (`n`) both this handler and
        // `find_next_match` fire. When a find session is active and the
        // event also matches `find_next_match`, step aside so the match
        // jump doesn't land on a stale focus.
        .on_pre_event_inner(comment_view_keymap.next_top_level_comment, move |s, e| {
            if find_next_for_ntl.has_event(e) && !find_state_for_ntl.borrow().match_ids.is_empty() {
                return None;
            }
            let id = s.get_focus_index();
            let next_id = s.find_sibling(id, NavigationDirection::Next);
            s.set_focus_index(next_id)
        })
        // When a find session is active, the prev-top-level key (default
        // `p`) doubles as a prev-match shortcut, mirroring `N`. With no
        // matches highlighted it falls back to sibling-prev navigation.
        .on_pre_event_inner(comment_view_keymap.prev_top_level_comment, move |s, _| {
            {
                let mut state = find_state_for_ptl.borrow_mut();
                if !state.match_ids.is_empty() {
                    state.pending = Some(FindSignal::JumpPrev);
                    return Some(EventResult::Consumed(None));
                }
            }
            let id = s.get_focus_index();
            let next_id = s.find_sibling(id, NavigationDirection::Previous);
            s.set_focus_index(next_id)
        })
        // Mirror the vim `n`/`N` pairing for sibling cycling: the
        // `find_prev_match` key (default `N`) doubles as a sibling-prev
        // shortcut whenever no find session is active. The earlier
        // `find_prev_match` handler already consumes the event when
        // matches exist, so this runs only in the no-match case.
        .on_pre_event_inner(comment_view_keymap.find_prev_match.clone(), move |s, _| {
            if !find_state_for_sib_back.borrow().match_ids.is_empty() {
                return None;
            }
            let id = s.get_focus_index();
            let next_id = s.find_sibling(id, NavigationDirection::Previous);
            s.set_focus_index(next_id)
        })
        .on_pre_event_inner(comment_view_keymap.parent_comment, move |s, _| {
            let id = s.get_focus_index();
            if s.items[id].level > 0 {
                let next_id = s.find_item_id_by_max_level(
                    id,
                    s.items[id].level - 1,
                    NavigationDirection::Previous,
                );
                s.set_focus_index(next_id)
            } else {
                Some(EventResult::Consumed(None))
            }
        })
        // open external link shortcuts. Bare `o`/`O` (no numeric prefix)
        // opens link 1, so pressing `o` on a focused comment opens its
        // first link — that's how the threads view's `re: parent thread`
        // header is documented to work. In the threads view, every item
        // (root comment and replies) carries `parent_story_url`; when the
        // buffer is empty we open that directly so a bare `o`/`O` jumps
        // back to the parent thread regardless of which item is focused
        // and regardless of whether the focused item has any links of
        // its own.
        .on_pre_event_inner(comment_view_keymap.open_link_in_browser, |s, _| {
            let id = s.get_focus_index();
            if s.raw_command.is_empty() {
                if let Some(url) = s.items[id].parent_story_url.clone() {
                    utils::open_url_in_browser(&url);
                    return Some(EventResult::Consumed(None));
                }
            }
            let num = parse_link_index(&s.raw_command)?;
            s.raw_command.clear();
            utils::open_ith_link_in_browser(&s.items[id].links, num)
        })
        .on_pre_event_inner(
            comment_view_keymap.open_link_in_article_view,
            move |s, _| {
                let id = s.get_focus_index();
                if s.raw_command.is_empty() {
                    if let Some(url) = s.items[id].parent_story_url.clone() {
                        return Some(EventResult::with_cb(move |siv| {
                            article_view::construct_and_add_new_article_view(client, siv, &url)
                        }));
                    }
                }
                let num = parse_link_index(&s.raw_command)?;
                s.raw_command.clear();
                utils::open_ith_link_in_article_view(client, &s.items[id].links, num)
            },
        )
        .on_pre_event_inner(comment_view_keymap.open_comment_in_browser, move |s, _| {
            let id = s.items[s.get_focus_index()].id;
            let url = format!("{}/item?id={}", client::HN_HOST_URL, id);
            utils::open_url_in_browser(&url);
            Some(EventResult::Consumed(None))
        })
        // other commands
        .on_pre_event_inner(comment_view_keymap.toggle_collapse_comment, move |s, _| {
            s.toggle_collapse_focused_item();
            Some(EventResult::Consumed(None))
        })
        .on_pre_event(comment_view_keymap.open_article_in_browser, {
            let url = article_url.clone();
            move |_| {
                utils::open_url_in_browser(&url);
            }
        })
        .on_pre_event(comment_view_keymap.open_article_in_article_view, {
            let url = article_url;
            move |s| {
                if !url.is_empty() {
                    article_view::construct_and_add_new_article_view(client, s, &url)
                }
            }
        })
        .on_pre_event(comment_view_keymap.open_story_in_browser, {
            let url = page_url;
            move |_| {
                utils::open_url_in_browser(&url);
            }
        })
        .on_pre_event(config::get_global_keymap().open_help_dialog.clone(), |s| {
            s.add_layer(CommentView::construct_on_event_help_view());
        })
        // Open the find-on-page dialog. Stale highlights from a previous
        // session are cleared before opening so the view doesn't briefly
        // show last-search highlights with an empty new query.
        .on_pre_event(comment_view_keymap.find_in_view.clone(), move |s| {
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
        .full_height()
}

pub fn construct_comment_view(
    client: &'static client::HNClient,
    data: PageData,
    nav: utils::NavTarget,
) -> impl View {
    let title = format!("Comment View - {}", data.title,);
    let main_view = construct_comment_main_view(client, data);

    let mut view = LinearLayout::vertical()
        .child(utils::construct_view_title_bar_with_nav(
            client, &title, nav,
        ))
        .child(main_view)
        .child(utils::construct_footer_view::<CommentView>());
    view.set_focus_index(1)
        .unwrap_or(EventResult::Consumed(None));

    // Vim-style cross-row focus: translate `k`/`j` into `Up`/`Down`
    // when the inner main view didn't consume them (cursor at the top
    // of the comment list, etc.) so the outer layout can shift focus
    // to the nav strip or footer.
    OnEventView::new(view)
        .on_event_inner(Event::Char('k'), |inner, _| {
            Some(inner.on_event(Event::Key(Key::Up)))
        })
        .on_event_inner(Event::Char('j'), |inner, _| {
            Some(inner.on_event(Event::Key(Key::Down)))
        })
}

/// Retrieve comments in a Hacker News item and construct a comment view of that item
pub fn construct_and_add_new_comment_view(
    s: &mut Cursive,
    client: &'static client::HNClient,
    item_id: u32,
    pop_layer: bool,
) {
    let async_view = async_view::construct_comment_view_async(s, client, item_id);
    if pop_layer {
        s.pop_layer();
    }
    s.screen_mut().add_transparent_layer(Layer::new(async_view));
}

/// Build a comment view of the given user's recent comments — the in-TUI
/// equivalent of HN's `/threads?id=<u>` page. Each entry includes a "re:
/// <story title>" header link to the parent thread.
pub fn construct_and_add_new_threads_view(
    s: &mut Cursive,
    client: &'static client::HNClient,
    username: String,
    page: usize,
) {
    let async_view = async_view::construct_threads_view_async(s, client, username, page);
    s.screen_mut().add_transparent_layer(Layer::new(async_view));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_link_index_defaults_to_one_when_empty() {
        assert_eq!(parse_link_index(""), Some(1));
    }

    #[test]
    fn parse_link_index_uses_typed_prefix() {
        assert_eq!(parse_link_index("3"), Some(3));
        assert_eq!(parse_link_index("12"), Some(12));
    }

    #[test]
    fn parse_link_index_returns_none_for_garbage() {
        assert_eq!(parse_link_index("x"), None);
        assert_eq!(parse_link_index("1a"), None);
    }
}
