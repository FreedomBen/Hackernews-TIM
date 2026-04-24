# Known follow-ups

Work intentionally deferred from the find-on-page feature (commits
`8474986` and `4fb95bb`). Each entry is self-contained so a future
agent can pick it up without re-reading the original conversation.

## 1. Article view: jump to match

**What exists today.** `ArticleView` (`hackernews_tui/src/view/article_view.rs`)
supports `/` and `Ctrl-f` to open the find dialog, highlights matches
live, and clears on `Esc`. But `Enter` (which sends
`FindSignal::JumpNext`) is a no-op — `process_find_signal` matches both
`JumpNext` and `JumpPrev` arms to `{}`. Match navigation (`n`/`N`) is
not bound on this view.

**Why it was deferred.** The article body is rendered as ONE big
`StyledString` inside a single `text_view::TextView`, wrapped in a
`PaddedView` inside a `LinearLayout` inside a `ScrollView`. Jumping to
"the next match" means scrolling that `ScrollView` so the row
containing the match becomes visible. That requires a byte-offset →
row-index lookup that doesn't currently exist anywhere in the
codebase.

**What to add.**

1. **`TextView::row_for_byte_offset`** in
   `hackernews_tui/src/view/text_view.rs`. Our custom `TextView`
   already stores `rows: Vec<lines::spans::Row>` computed from the
   source. Each `Row` contains segments with `.start`/`.end` byte
   offsets into the source string. Add:

   ```rust
   pub fn row_for_byte_offset(&self, offset: usize) -> Option<usize> {
       self.rows.iter().position(|row| {
           row.segments.iter().any(|seg| seg.start <= offset && offset < seg.end)
       })
   }
   ```

   `TextView.rows` is currently private — this method is the public
   surface.

2. **Track match byte offsets** in `ArticleView`. Extend `FindState`
   or use a local field. A local `Vec<usize>` of match start offsets
   is probably cleanest (no state-shape churn). Compute it inside
   `apply_find_query` alongside the existing highlight walk — the
   `highlight_matches` helper in
   `hackernews_tui/src/view/find_bar.rs` already discovers each
   match's byte range internally; consider returning the ranges as a
   third tuple element or write a parallel helper that returns just
   the ranges. Current signature:

   ```rust
   pub fn highlight_matches(content: &StyledString, query: &str, match_style: Style)
       -> (StyledString, usize)
   ```

   Proposed: return `(StyledString, Vec<(usize, usize)>)` and keep
   `count = ranges.len()`. All existing callers can just use
   `ranges.len()`.

3. **Wire `FindSignal::JumpNext` / `JumpPrev`** in `ArticleView::process_find_signal`:
   - Find the next/prev match offset relative to the current scroll y.
   - Call `self.view.get_inner_mut().get_child_mut(2)...` to get the
     inner `TextView`, call `row_for_byte_offset(offset)` to get a row
     index.
   - The `ScrollView` scroll offset is in LinearLayout coordinates.
     Add the heights of child 0 (title) and child 1 (metadata) plus
     the `PaddedView`'s top padding (currently `1`) to get the
     absolute y of a given TextView row. Get child heights via
     `required_size`.
   - Scroll with `self.view.set_offset(Vec2::new(0, target_y))` or
     similar — check Cursive's `ScrollView` API for the exact call.

4. **Add match-nav bindings.** Add `find_next_match` and
   `find_prev_match` to `ArticleViewKeyMap` in
   `hackernews_tui/src/config/keybindings.rs` (defaults `n` and `N`,
   matching the other views). Wire them as context-dependent
   `on_pre_event_inner` handlers in `construct_article_main_view` —
   register them BEFORE any existing `n`/`N` binding in that view
   (currently none, so ordering only matters relative to the global
   scroll handlers that `on_scroll_events()` adds). Return `None` when
   `state.match_ids` (or the local match list) is empty so the keys
   fall through to scroll if the user hasn't started a find session.

**Testing.** `highlight_matches` already has unit tests in
`view/find_bar.rs`. Add tests for the new `row_for_byte_offset` method
(construct a TextView, call `layout`, verify offset → row mapping).
For the scroll math, manual browser testing is the only realistic
path — the TUI test infrastructure doesn't simulate layout.

**Docs.** Update `README.md` (ArticleView section), `examples/hn-tui.toml`,
`examples/hn-tui-dark.toml` with the new keymap entries. Update
`HasHelpView for ArticleView` in `view/help_view.rs`.

## 2. Find-on-page inside SearchView

**What exists today.** `SearchView`
(`hackernews_tui/src/view/search_view.rs`) wraps a `StoryView`
constructed by `construct_story_main_view`, not by
`construct_story_view` — the outer paging wrapper is bypassed because
SearchView supplies its own paging via Algolia. The `find_in_view`
keybinding is registered on the OUTER `construct_story_view` wrapper,
so pressing `/` inside a SearchView hits nothing; the character lands
in the search query text box (if SearchView is in Search mode) or is
ignored (if in Navigation mode).

**Why it was deferred.** In Search mode `/` belongs to the query
input; overriding that would break typing `/` into a search. Find-on-
page for matched stories is arguably redundant when the user can
refine the Algolia query. But it is a real gap vs. a regular
StoryView.

**What to add.**

Option A: bind `find_in_view` in Navigation mode only. Inside
`construct_search_main_view`, the `SearchViewMode::Navigation` arm of
the catch-all `on_pre_event_inner` dispatch can intercept
`search_view_keymap.find_in_view` before falling through. When
matched, call `find_bar::construct_find_dialog` with a `FindStateRef`
owned by SearchView. The existing inner `StoryView` already owns a
`FindStateRef` (passed as the sixth arg to
`construct_story_main_view`); SearchView needs to reuse it. Change
the two call sites in `search_view.rs:54` and `search_view.rs:149` to
pass a stored `self.find_state` instead of creating a fresh one each
time.

Option B: add a different keybinding (e.g. `Ctrl-g`) so there's no
ambiguity with query-input typing. Simpler but splits muscle memory
across views.

**Scope of the change.** `search_view.rs` struct gets a
`find_state: FindStateRef` field initialized in `SearchView::new`.
The inner `StoryView` gets that same state cloned into it. A new
`on_pre_event_inner` handler (gated on `mode == Navigation`) opens
the dialog. Match-nav (`n`/`N`) is already handled inside
`construct_story_main_view`; it will work once the find dialog runs
against the inner StoryView's find state.

**Caveats.**
- The inner `StoryView` gets rebuilt every time
  `update_stories_view` runs (on each Algolia response). Right now
  the find state is passed into the NEW StoryView, which means any
  active find session is lost when the user types into the search
  box. Pre-condition: every rebuild should clear find state
  (signal `Clear`) OR SearchView should persist find state across
  rebuilds by passing the same `Rc<RefCell<FindState>>` to each new
  StoryView. Probably the latter — otherwise mid-session behaviour is
  surprising.
- Because StoryView now requires a `FindStateRef` on construction,
  SearchView constructing one per rebuild already works — but cloning
  the same shared state into each rebuild matches the persistent-
  session model.

**Testing.** Manual, since SearchView's async paths aren't easily
unit-testable.

## General notes

- The find infrastructure lives in
  `hackernews_tui/src/view/find_bar.rs`. It exports `FindState`,
  `FindStateRef`, `FindSignal::{Update, Clear, JumpNext, JumpPrev}`,
  `highlight_matches`, and `construct_find_dialog`. Reuse these; do
  not duplicate the span-walking highlight logic.
- Existing view integrations to reference:
  - `CommentView` (full-featured, multi-item, list nav) —
    `hackernews_tui/src/view/comment_view.rs`.
  - `StoryView` (list + outer-wrapper key dispatch) —
    `hackernews_tui/src/view/story_view.rs`.
  - `ArticleView` (single-content, highlight-only) —
    `hackernews_tui/src/view/article_view.rs`.
- The `matched_highlight` style comes from
  `config::get_config_theme().component_style.matched_highlight` and
  needs `.into()` to convert from the project's `config::theme::Style`
  to `cursive::theme::Style`.
- Always add `find_*` keybinding entries to BOTH `examples/hn-tui.toml`
  and `examples/hn-tui-dark.toml`, plus the relevant README shortcut
  table and the `HasHelpView` impl for the target view.
