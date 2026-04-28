# Hackernews-TIM Improvements over upstream hackernews-TUI

This fork has diverged from upstream `hackernews-TUI`. The sections below summarize every change authored by Benjamin Porter. None have been upstreamed.

## ⚠️ Breaking / behavior changes

Please review these before merging upstream. Most ship with automatic migration, but user-visible defaults, paths, and names have shifted.

- **Project renamed** from `hackernews-TUI` → `Hackernews-TIM`. Binary/crate is `hackernews_tim`.
- **Config file renamed** `hn-tui.toml` → `config.toml`. Legacy name is auto-migrated in place on first run.
- **Config/auth files relocated** from `$XDG_CONFIG_HOME/` (and `$HOME/.config/`) into a `hackernews-tim/` subdirectory. Legacy files are copied on first run (originals preserved).
- **Log file** moved from the config dir into the `hackernews-tim` cache subdirectory.
- **Default dark theme** selection colors were subdued — users with the shipped dark theme will see a different highlight color.
- **New default keybindings** were added (see below). Existing bindings are unchanged, but arrow keys, `Ctrl+u`/`Ctrl+d`, `/`, `n`, `N`, and `F6` are now claimed by the app in list/article views.
- `toml` dependency bumped to 1.1.0; `config_parser` tests updated for the 1.x `FromStr` behavior change.

## New features

### Authentication & identity
- Interactive HN login with first-run prompt and in-app login dialog.
- Session cookie is cached in the auth file to avoid re-logging in every startup; auth files are upgraded in place.
- Startup login outcome is reported to the user; bad credentials are no longer silently saved (hardened success detection).
- Logged-in **username and karma** shown in view title bars.
- User's HN profile **topcolor** is applied to the title bar.
- Authenticated user's own stories/comments marked with an orange `*`.
- **Point counts** shown next to your own comments on the byline (e.g. `* 3 points by you 1h ago`), parsed from HN's `score_<id>` span on authenticated comment-tree fetches.
- "Open your own HN comments" global keybind (browser); plus an in-TUI threads view (see below).
- HN `showdead` profile setting is honored when authenticated.

### Voting
- **Upvote/downvote stories** from the story list.
- **Downvote comments** (up arrow already existed); down arrow now renders in the comment view.
- Up-arrow indicator shown next to voteable stories in the story view.
- Vote state is pre-fetched for story lists so arrows render on every open and on every page.
- Past votes are read from HN's post-vote markup.
- **Vouch** for dead stories and comments.

### Comments & replies
- Comments loaded from the HN web page when authenticated (surfaces dead/flagged content authenticated users can see).
- Reply-via-editor flow in the comment view; reply keybind in the story view; reply promoted into `CommentViewKeyMap`.
- Edit-your-own-comment flow on the comment view.
- Aborted replies and edits (empty editor body, or comment edit with no changes) now print an explicit message instead of silently dropping the action.
- `[dead]` and `[flagged]` badges prefix the byline of such items; their bodies are faded.

### Threads view
- **F6 / `goto_my_threads_view`**: in-TUI version of HN's `/threads?id=<u>` page, rendered through the existing `CommentView`. Requires authentication; falls back to the same "Log in first" dialog as the existing browser shortcut.
- Replies under each user comment are expanded by fetching the full subtree from the Firebase `/items/{id}` endpoint. Subtrees fan out in parallel via rayon while preserving listing order, so each user comment shows at level 0 with its descendants below. On per-subtree fetch failure the bare comment is still shown.
- Each entry is prefixed with a `re: <story title>` link header so you can jump to the parent thread via the link dialog.
- A bare `o` / `O` in the comment view (no numeric prefix) now defaults to opening link 1. In the threads view, every item in each user-comment subtree (the user's level-0 comment plus all replies) carries a `parent_story_url`, so a bare `o` / `O` from any focused item in the subtree opens the parent thread — replies don't need a visible `re:` header to inherit the same shortcut.

### Global navigation strip
- All top-level views (story, comment, article, search, threads) now render a `[Y] Hacker News | 1.front_page | … | search (^S) | 6.threads` strip in the title bar, mirrored from the F1–F6 / Ctrl-S keybindings. Previously this strip only appeared on the story view.
- Each entry is a focusable `NavLink` button: arrow keys / `h` / `l` move between entries, `Enter` triggers the corresponding view switch, and the active entry is rendered with reverse-video as a "you are here" indicator.
- `j` / `k` (and arrow keys) shift focus across the title-bar / main-view / footer boundary; pressing `k` / Up at the topmost item of the story or comment list now falls through to the title bar instead of consuming the event.
- Title-bar text views (separators, user info) are marked `no_wrap` so the strip stays on a single row even with a long username.

### Find-on-page
- New `/` find-on-page feature across comment, story, article, and search views.
- `n` / `N` jump forward/back through matches; paging handlers are gated so these keys don't collide.
- `n` advances strictly past the current match (and wraps from last to first); `N` doubles as a sibling-prev shortcut when no find session is active.
- `p` mirrors `N` while a find session is active (jumps to the previous match); without an active session it continues to do sibling-prev navigation.
- `Esc` exits find mode outside the dialog.

### Navigation
- Arrow keys bound alongside `h/j/k/l` in story and comment views.
- `Ctrl+u` / `Ctrl+d` for half-page scrolling; `PageUp`/`PageDown` now move focus by half a page in list views.
- `n` / `p` in the comment view are sibling-aware: at depth ≥ 1 they cycle (with wrap) only among comments sharing the same parent, instead of jumping across subtrees at the same indentation. Top-level scope is unchanged.

### Config
- `--init-config <light|dark>` flag writes a default config and exits; first-run prompt offers the same.
- `--update-theme <light|dark>` flag replaces just the `[theme]` table of an existing config in place (mutually exclusive with `--init-config`).
- **Story listing page size** is configurable.
- **Search-view page size** is configurable.

## Build, tooling, packaging

- Added top-level `Makefile` with build, install (honors `PREFIX`), lint, clippy, format, test, docker, and cross-compile targets; `make help` auto-generates from `##` comments.
- Added `hackernews_tui(1)` man page; installed by `make install`.
- Added `CLAUDE.md` with build commands and architecture overview.
- Silenced all compiler and clippy warnings across the workspace; normalized view files with `rustfmt`.
- Test fixture CSRF tokens replaced with synthetic values.
- Snapshot test added for `parse_reply_form` against a real HN response.

## Commit count

72 commits by Benjamin Porter on `main`, dated 2026-04-23 to 2026-04-28.
