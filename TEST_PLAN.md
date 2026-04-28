# Test Plan

A roadmap for expanding test coverage in this workspace. Grouped into phases by
infrastructure cost, not by importance — Phase 1 needs nothing new, Phase 2
needs one focused refactor.

## Current state (baseline)

- **117 unit tests** in `hackernews_tim` + **6 integration tests** in
  `config_parser` — all passing on Linux/macOS/Windows in CI.
- Coverage is concentrated in:
  - `client/mod.rs` (71 tests) — HTML scraping, comment parsing, vote-state
    inference, login classification.
  - `config/mod.rs` + `config/init.rs` (25 tests) — auth round-trips, file
    permissions, theme replacement, keyring pointer files.
  - `view/find_bar.rs`, `view/comment_view.rs`, `model.rs`,
    `client/model.rs`, `config/theme.rs` — small pure helpers.
- Untested or thinly tested: `client/query.rs`, `config/keybindings.rs`,
  `parser/{html,article,rcdom}.rs`, `utils.rs`, `reply_editor.rs`, every view
  module except `find_bar` and the `parse_link_index` helper in `comment_view`.

---

## Progress

Tick items as the tests land in `make test`. Sub-section grain is
deliberately coarse — the tables further down remain the source of
truth for individual cases.

### Phase 1 (pure-logic tests)

- [x] 1.1 `client/query.rs` — URL/query construction
- [x] 1.2 `client/mod.rs` — additional private helpers
- [x] 1.3 `config/keybindings.rs` — typed key parsing
- [x] 1.4 `parser/html.rs` — HN comment HTML rendering
- [x] 1.5 `parser/article.rs` + `parser/rcdom.rs` — reader-mode rendering
- [x] 1.6 `utils.rs` — formatting helpers
- [x] 1.7 `reply_editor.rs` — scaffold I/O
- [x] 1.8 View-module helpers (lift, then test)
- [x] 1.9 Doctests (optional, low priority)
- [x] Phase 1 acceptance — `make test` + `cargo clippy -- -D warnings` green; test count roughly doubles (117 → 243)

### Phase 2 (view-level tests)

- [x] 2.1.1 Introduce `HnApi` trait + `FakeHnApi` test double
- [x] 2.1.2 Wire up the Cursive puppet backend + `tests/support` helpers
- [x] 2.1.3 Add `insta` snapshot library
- [x] 2.2.1 StoryView tests
- [x] 2.2.2 CommentView tests
- [ ] 2.2.3 SearchView tests
- [ ] 2.2.4 ArticleView tests
- [ ] 2.2.5 LinkDialog / HelpView / LoginDialog / find-bar tests
- [ ] 2.2.6 Global navigation + post-event hook tests
- [ ] 2.3 Fixture discipline (`tests/fixtures/`, `FakeHnApi` per-test config)
- [ ] Phase 2 acceptance — view tests run with no network; `HnApi` is the only data dependency in `view/*`

---

## Phase 1 — Pure-logic tests (no new infrastructure)

Goal: lift untested pure functions into the existing `#[cfg(test)] mod tests`
pattern. Any function whose inputs are values (no `&Cursive`, no network, no
globals) belongs here. Where a function reads a global (e.g.
`get_config_theme()`, `get_user_info()`), refactor it to take the value as a
parameter so the test can pass a fixture.

### 1.1 `client/query.rs` — URL/query construction

| Target                                         | What to assert                                                                                        |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `StorySortMode::next("front_page", _)`         | `None.next("front_page") == None`; `Date` and `Points` inputs panic on the `front_page should have no sort mode` assert. |
| `StorySortMode::next("story", _)`              | Starts at `Date`, cycles `Date → Points → Date` (never None for `story` / `job`); `None` input panics on the `story/job should have a sort mode` assert. |
| `StorySortMode::next("ask_hn", _)`             | Cycles `None → Date → Points → None` (same path as every non-`front_page`/`story`/`job` tag).        |
| `FilterInterval::query("points")`              | Empty when both bounds None; emits `,points>=N` for start only; `,points<N` for end only; both bounds combined. |
| `FilterInterval::desc("points")`               | Renders `points: [start:end]` with empty strings for missing bounds.                                   |
| `StoryNumericFilters::query()`                 | Empty when all intervals empty; otherwise prefixed with `&numericFilters=` and trailing comma stripped. |
| `StoryNumericFilters::query()` time conversion | `elapsed_days` start/end convert to a *reversed* `created_at_i` interval (start ↔ end swap) using `from_day_offset_to_time_offset_in_secs`. |
| `StoryNumericFilters::desc()`                  | Concatenates the three sub-descriptions.                                                              |
| `Display for StoryNumericFilters`              | Equals `desc()` output.                                                                               |

### 1.2 `client/mod.rs` — additional private helpers

The 71 existing tests cover comment parsing, vote/vouch state, login
classification, listing-path mapping, HN page-window math, reply-form
parsing, threads score extraction, and profile parsing (karma, topcolor,
showdead). These private helpers are still untested:

| Target                                  | What to assert                                                                                  |
| --------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `showdead_query_suffix(sep)`            | Builds the `&` / `?` prefixed `showdead=yes` query suffix; returns empty string when disabled. (Tested via the pure helper `build_showdead_query_suffix(sep, enabled)` so the global `USER_INFO` is not in the loop.) |
| `extract_textarea(body, "text")`        | Returns inner text of `<textarea name="text">…</textarea>`; `None` when missing; raw HTML returned verbatim — entity decoding is the caller's job (`fetch_edit_form` runs `decode_html` on the result). |
| `extract_hidden_input(body, "hmac")`    | Returns the `value=` of `<input type="hidden" name="hmac">`; `None` when missing.               |
| `classify_post_reply_response(body)`    | `Ok` on success page; `Err` with specific messages for known failure pages.                      |

Fixture HTML lives next to existing fixtures (already established pattern in
`client/mod.rs` tests).

### 1.3 `config/keybindings.rs` — typed key parsing

| Target                                        | What to assert                                                                                |
| --------------------------------------------- | --------------------------------------------------------------------------------------------- |
| `Keys::deserialize` from TOML strings         | Parses single-key (`"q"`), ctrl/alt modifier (`"C-c"` / `"M-x"`), special key (`"esc"`, `"backspace"`), and arrays of keys (which act as an OR — any one event triggers). Unknown strings return a deserialize error. (Prefix chords like `2 l` are handled at the view layer in `parse_link_index`, not by `Keys`.) |
| `Display for Keys`                            | Round-trips to the same canonical form for each variant.                                      |
| `From<Keys> for event::EventTrigger`          | Each variant produces a trigger that matches the expected `cursive::event::Event`.            |
| `KeyMap` defaults                             | Each section's `Default` impl populates the documented bindings (snapshot a few critical ones — `quit`, `goto_front_page_view`, `open_help_dialog`). |
| `CustomKeyMap` deserialization                | Parses `[[keymap.custom_keymaps]]` blocks with `key`, `tag`, `by_date`, `numeric_filters` correctly. |
| ConfigParse merge                             | A partial TOML overlays only the fields present, leaving defaults for the rest.               |

### 1.4 `parser/html.rs` — HN comment HTML rendering

`parse_hn_html_text(text, style, base_link_id) -> HTMLTextParsedResult` is
pure. Build a small set of fixture HTML snippets and assert on the resulting
`StyledString` source + the extracted `links` vector.

| Fixture                                           | What to assert                                                                       |
| ------------------------------------------------- | ------------------------------------------------------------------------------------ |
| Plain text paragraph                              | Source text matches; no links collected.                                             |
| `<p>` with a single `<a href="…">label</a>`       | Source contains `[1]` link marker; `links[0]` equals href; `base_link_id` offsets the marker. |
| Multiple links                                    | Markers numbered sequentially from `base_link_id + 1`.                               |
| `<pre><code>` block                               | Code spans are preserved verbatim, including whitespace.                             |
| `<i>` / `<b>` styling                             | Style is applied over the wrapped span.                                              |
| HTML entities (`&amp;`, `&#x27;`, `&gt;`)         | Decoded in output.                                                                   |
| Nested elements                                   | Inner text preserved without doubled spans.                                          |
| Empty/whitespace input                            | Returns empty `StyledString` and empty `links`.                                      |

### 1.5 `parser/article.rs` + `parser/rcdom.rs` — reader-mode rendering

Harder to fixture (depends on `readable-readability`), but doable. Note that
`Article::parse(&self, max_width)` only renders `self.content` — the
`title`/`url` fields on `Article` are populated by the upstream
`readable-readability` pipeline that constructs the struct.

- Assert that constructing `Article` from a fixture HTML page extracts the
  expected `title`/`url` (commit a real-world article HTML under
  `tests/fixtures/`).
- Assert `Article::parse(max_width)` renders the body into an
  `HTMLTextParsedResult` whose `StyledString` byte-for-byte contains
  expected paragraph separators and link markers.
- Assert link extraction in the result is in document order.

If the readability output is too brittle to snapshot, restrict assertions to
*invariants* (link count, title presence, no-panic on malformed input).

`parser/rcdom.rs` is a vendored copy of the html5ever sample DOM and is
exercised transitively by `Article::parse` — it is not directly unit
tested.

### 1.6 `utils.rs` — formatting helpers

| Target                                         | What to assert                                                                          |
| ---------------------------------------------- | --------------------------------------------------------------------------------------- |
| `get_time_offset_in_text(offset)` (private)    | Returns `"X seconds"`, `"X minutes"`, `"X hours"`, `"X days"`, `"X months"`, `"X years"` at boundary values; pluralization via `format_plural`. (This is the pure inner half — `get_elapsed_time_as_text` reads `SystemTime::now()` and is not pure; one smoke test for the now-relative path is enough.) |
| `from_day_offset_to_time_offset_in_secs(d)`    | `0 → 0`, `1 → 86400`, `7 → 604800`.                                                     |
| `shorten_url(url)`                             | Returns input unchanged when length ≤ 50; otherwise renders `first40 + "..." + last10`. (The function does not strip the scheme or `www.` — only truncates.) |
| `decode_html("&amp;&lt;&gt;&#x27;")`           | Returns `&<>'`.                                                                         |
| `combine_styled_strings([a, b])`               | Concatenation preserves spans of each input.                                            |

### 1.7 `reply_editor.rs` — scaffold I/O

These touch the filesystem but only via temp paths — same pattern as the
existing `config::tests::auth_write_*` tests.

| Target                              | What to assert                                                                                       |
| ----------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `write_scaffold(path, parent)`      | File contains the parent text quoted with `# > ` prefixes (the whole quoted block is also a Git-style comment), plus the scissors line `# ------ >8 ------` and instructional comments; an empty parent renders the `# > (empty)` placeholder. |
| `write_edit_scaffold(path, text)`   | File contains the current text verbatim, followed by the scissors line and edit-mode instructions. |
| `read_and_strip(path)`              | Returns text above the scissors line as `Result<String>`; trims surrounding whitespace; returns an empty string when only the scaffold remains. |
| `scratch_path()`                    | Returns a path under the system temp dir matching `hn-reply-{pid}-{nanos}.md`; consecutive calls return distinct paths. |

### 1.8 View-module helpers (lift, then test)

Each of these is currently a private method or function on a view but contains
no Cursive runtime dependency. Lift to a module-level pure function (or a free
helper that takes config/user as parameters), then test:

| Source                                      | Helper                              | Suggested refactor                                                          |
| ------------------------------------------- | ----------------------------------- | --------------------------------------------------------------------------- |
| `view/story_view.rs:138`                    | `get_story_text`                    | Take `&ComponentStyle` and `Option<&str>` (current username) as parameters; remove `get_config_theme()` / `get_user_info()` calls. Test row formatting for: anonymous user, story with vote up/down/none, own-story marker, missing URL, custom domain. |
| `view/story_view.rs:93`                     | `compute_max_id_len`                | Already pure — just call it from a `tests` module. Test: `compute_max_id_len(0, _)`, single page, page boundary.       |
| `view/story_view.rs:374`                    | `story_row_text`                    | Lift the formatting half (excluding `self.stories[id]` lookup) into a pure helper.                                     |
| `view/help_view.rs:183,199,234`             | `default_*_commands` factories      | Already pure — assert each list is non-empty and contains expected keymaps (e.g. `goto_front_page_view` is present in `default_view_navigation_commands`). |
| `view/utils.rs:362`                         | `open_ith_link_in_browser`          | Split the index-validation half from the side-effect half; test the validator with `links=[]`, `i=0`, `i > links.len()`. |
| `view/comment_view.rs:521`                  | `parse_link_index` (already tested) | Add edge cases: leading zeros (e.g. `"007"` → `Some(7)`), integer overflow (`"9".repeat(40)` → `None`), negative input (`"-5"` → `None`). |

### 1.9 Doctests (optional, low priority)

Add `///` examples to the public surface of `client::query`, `utils`, and
`config::keybindings::Keys` parsing. `cargo test --doc` will exercise them.

### Phase 1 acceptance

- All Phase 1 tests live in `#[cfg(test)] mod tests` blocks within their
  respective source files (or `tests/fixtures/` for HTML fixtures).
- `make test` stays green on all three CI platforms.
- `cargo clippy -- -D warnings` stays green (test code included).
- Total test count roughly doubles (target: ~250 unit tests).

---

## Phase 2 — View-level tests with Cursive's puppet backend

Goal: drive complete views without a real terminal. Inject events, force a
layout pass, and assert on observable view state and rendered output.

### 2.1 Infrastructure (do this first, in this order)

#### 2.1.1 Introduce an `HnApi` trait

Currently every view takes `client: &'static client::HNClient`. The puppet
backend can render any `View`, but tests can't construct views without a
network-bound client.

- Define a trait `HnApi` in `client/mod.rs` exposing the methods views call:
  `get_stories_by_tag`, `get_article`, `get_page_content`,
  `get_listing_vote_state`, `get_listing_vouch_state`, `vote`, `vouch`,
  `parse_vote_data`, `login`, etc. (Audit `view/*` for the actual call set.)
- Implement `HnApi for HNClient`.
- Change view constructors to take `&'static dyn HnApi` (or a generic
  `&'static C: HnApi` if monomorphization matters).
- Provide a `FakeHnApi` test double in `hackernews_tim/src/client/fake.rs`
  (gated `#[cfg(any(test, feature = "test-support"))]`) that returns
  hand-built fixtures and records interactions.

This refactor is the bulk of Phase 2 cost. Phase 1 should not block on it.

#### 2.1.2 Wire up the puppet backend

`cursive_core` ships a `puppet` backend that renders to an in-memory cell
buffer and accepts events programmatically.

- `cursive_core` is already a regular dependency (`0.3.7`); enable its
  puppet backend for tests (verify the feature name against the 0.3.7
  source — likely `puppet-backend` — and either add the feature to the
  existing dependency line or duplicate the crate as a dev-dependency
  with the feature enabled). If the feature isn't exposed, fall back to
  building views in isolation and calling
  `View::layout`/`View::on_event`/`View::draw` against a constructed
  `Printer` with a stub backend.
- Create `hackernews_tim/tests/support/mod.rs` (or a `test-support` module
  inside the crate) with helpers:
  - `build_cursive_with(backend) -> Cursive`
  - `step_until_idle(&mut Cursive)` — drives async-view loading to completion.
  - `screen_text(&Cursive) -> String` — flattens the puppet's cell buffer
    into a visible-text snapshot for `insta` comparison.
  - `send(&mut Cursive, Event)` — convenience wrapper.

#### 2.1.3 Snapshot library

Add `insta` as a dev-dependency. Snapshot tests live in
`hackernews_tim/tests/snapshots/`.

### 2.2 Per-view tests

Each item below is one or more integration tests under `hackernews_tim/tests/`.

#### 2.2.1 StoryView

| Scenario                                                                                                                              | Assertion                                                                                                       |
| ------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Render front page with 3 fixture stories                                                                                              | Snapshot matches (story IDs aligned, metadata strings correct, theme styles applied).                          |
| Send `j`, `j`, `k`                                                                                                                    | Focused row is row 1.                                                                                            |
| Send half-page-down (`Ctrl-D` or configured key)                                                                                      | Focus advances by `page_size / 2`.                                                                              |
| Send vote-up key while not logged in                                                                                                  | No state change; warning logged (capture via `tracing-test`).                                                   |
| Send vote-up while logged in (with `FakeHnApi` returning a `VoteData`)                                                                | Row re-renders with the upvote indicator.                                                                       |
| Open find bar (`/`), type a query                                                                                                     | `FindState.match_ids` populated; matching rows show highlight style.                                            |
| Send find-jump-next                                                                                                                   | Focus moves to next match's row.                                                                                |
| Switch sort mode (cycle key)                                                                                                          | Title bar updates; `FakeHnApi` recorded a fetch with the new `StorySortMode`.                                   |
| Goto previous view (Backspace) on the only view                                                                                       | No-op (no panic).                                                                                               |

#### 2.2.2 CommentView

| Scenario                                                                                              | Assertion                                                                                                 |
| ----------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Render with fixture comment tree (10 comments, 3 levels of nesting)                                   | Snapshot matches; per-comment points shown when present.                                                  |
| Navigate `j`/`k`                                                                                      | Focus moves linearly through comments in document order.                                                  |
| Navigate `n`/`p` (next-sibling / prev-sibling)                                                        | Skips children of current comment.                                                                        |
| Navigate `o` (parent)                                                                                 | Jumps to parent in the same view; from the threads view, opens the parent thread (per recent commit `e1cb79f`). |
| Collapse current subtree                                                                              | Children are hidden; expand restores them.                                                                |
| Send reply key while logged out                                                                       | Login dialog opens.                                                                                       |
| Send reply key while logged in                                                                        | `FakeHnApi.run_editor_for_reply` invoked; on success, comment appended (or whatever the current behavior is). |
| Open link dialog from a comment with multiple links                                                   | Dialog renders with numbered list; selecting index 2 calls `open_ith_link_in_browser` with the right URL. |
| Type `2 l` (typed-prefix link open)                                                                   | `parse_link_index("2") = Some(2)`; the second link is opened.                                             |
| Dead/flagged comments                                                                                 | Render with the right style and points are missing/correct.                                               |

#### 2.2.3 SearchView

| Scenario                                                                                              | Assertion                                                                                            |
| ----------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| Render initial state                                                                                  | Empty results panel + query input focused.                                                           |
| Type query, submit                                                                                    | `FakeHnApi` records a search call with the query; results panel renders fixture stories.            |
| Cycle filter (numeric filter key)                                                                     | Title bar updates with new `StoryNumericFilters::desc()`.                                            |
| Open story from results                                                                               | Comment view pushed onto the stack; preserves search state when popped.                              |

#### 2.2.4 ArticleView

| Scenario                                       | Assertion                                                                                          |
| ---------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Render with fixture parsed `Article`           | Snapshot matches; link markers numbered from 1.                                                    |
| Scroll down with `j`                           | Visible content shifts by 1 line.                                                                  |
| Open link dialog                               | LinkDialog populated from `Article.links`.                                                         |
| Type `5 o` (open link 5)                       | Browser-open helper called with `Article.links[4]`.                                                |
| Find-on-page                                   | Highlight + jump-to-match work, including jump-to-byte-range from `find_bar::highlight_matches`.   |

#### 2.2.5 LinkDialog, HelpView, LoginDialog, Find bar

| View                | Tests                                                                                                                                                              |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `LinkDialog`        | Snapshot of dialog with 5 links; arrow-key navigation moves focus; Enter triggers `open_ith_link_in_browser` for the focused index.                                |
| `HelpView`          | Snapshot the help for each parent view (StoryView, CommentView, SearchView, ArticleView, LinkDialog, DefaultHelpView). Catches drift between defaults and docs. |
| `LoginDialog`       | Username/password inputs route to fake `HnApi::login`; success dismisses the dialog and sets `get_user_info()`; failure surfaces an error inline.                  |
| `find_bar` end-to-end | Open with `/`, type query, see live highlight; `n` / `N` jump forward/back; `Esc` clears highlights and `match_ids`.                                              |

#### 2.2.6 Global navigation + post-event hooks

| Scenario                                                          | Assertion                                                                                                          |
| ----------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `F1` (default `goto_front_page_view`)                             | Active view becomes a StoryView for the front page.                                                                |
| `F2` / `F3` / `F4` / `F5` / `F6` (defaults for `goto_all_stories_view`, `goto_ask_hn_view`, `goto_show_hn_view`, `goto_jobs_view`, `goto_my_threads_view`) | Each opens the corresponding view.                                              |
| Custom keymap registered via TOML config                          | Pressing the configured key opens a StoryView with the configured tag and sort mode.                               |
| `?` opens help dialog                                             | HelpView matches the current parent view's commands.                                                               |
| Quit (`q`)                                                        | Cursive event loop signals shutdown.                                                                               |

### 2.3 Fixture discipline

- Network responses live under `tests/fixtures/` as raw HTML / JSON.
- `FakeHnApi` is configured per-test by handing it a map of expected calls →
  fixture responses, or by setting up a `wiremock` server when tests want to
  exercise the real `HNClient`'s URL construction end-to-end (Phase 1.1
  covers URL construction in isolation, so wiremock is rarely worth it here).

### Phase 2 acceptance

- View-level tests run under `cargo test` with no network access.
- Snapshot tests are reviewable: changes to a snapshot require an explicit
  `cargo insta review` step.
- StoryView, CommentView, SearchView, ArticleView, LinkDialog, HelpView all
  have at least one rendering snapshot and one event-driven test.
- The `HnApi` trait is the only public API surface that views depend on for
  data; `&'static HNClient` no longer appears in `view/*`.

---

## Out of scope (for now)

- **Tier 3 / PTY end-to-end tests.** Driving the real binary under
  `portable-pty` + `vt100` would catch keymap wiring, `cursive_buffered_backend`
  interactions, and the full `init_ui` graph, but the cost (PTY harness,
  Windows CI flakiness, fixture HN backend) outweighs the benefit until
  Phase 2 lands.
- **`config_parser_derive` proc-macro tests.** Currently exercised
  transitively via `config_parser/tests/test.rs`. Adding `trybuild` for
  compile-fail tests is a future improvement, not urgent.
- **Property-based tests.** `proptest` over comment-tree builders or HTML
  fuzz inputs would harden the parser, but only after the example-based
  Phase 1 tests catch the obvious cases.
- **Performance / regression benchmarks.** Out of scope for this plan.

---

## Running

| Task                          | Command                                          |
| ----------------------------- | ------------------------------------------------ |
| Run everything                | `make test`                                      |
| One crate                     | `cargo test -p hackernews_tim`                   |
| One module                    | `cargo test -p hackernews_tim view::find_bar`    |
| Single test                   | `cargo test -p hackernews_tim parse_link_index`  |
| Update snapshots (Phase 2)    | `cargo insta review`                             |
| Doctests                      | `cargo test --doc -p hackernews_tim`             |
