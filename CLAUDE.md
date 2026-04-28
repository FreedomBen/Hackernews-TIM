# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Agent Instructions

- After making changes, commit them.  Do not push

## Commands

All workflows go through the root `Makefile` (wraps cargo). Run `make help` for the full list.

| Task                    | Command                                                  |
| ----------------------- | -------------------------------------------------------- |
| Build release           | `make release` (or `make build` / `make all`)            |
| Build debug             | `make debug`                                             |
| Run the app             | `make run` (debug build, runs `hackernews_tim` crate)    |
| Type-check everything   | `make check`                                             |
| Run tests               | `make test` (workspace)                                  |
| Single test             | `cargo test -p <crate> <test_name>` (e.g. `-p hackernews_tim`) |
| Format                  | `make fmt` / `make fmt-check`                            |
| Lint                    | `make clippy` (runs with `-D warnings`) / `make lint`    |
| Install                 | `make install` (honors `PREFIX`, default `/usr/local`)   |
| Docker image            | `make docker-build` / `make docker-run`                  |
| Cross-compile           | `make cross-build CROSS_TARGET=<triple>` (uses `Cross.toml` + `ci/Dockerfile-cross`) |

CI (`.github/workflows/ci.yml`) runs `cargo test`, `cargo fmt --all -- --check`, and `cargo clippy -- -D warnings` on macOS, Windows, and Linux — keep these green before pushing.

Run the app against a custom config or start item:
`cargo run -p hackernews_tim -- -c path/to/config.toml -a path/to/hn-auth.toml -l /tmp -i <item_id>`

## Workspace layout

Three-crate Cargo workspace (`Cargo.toml` at root):

- `hackernews_tim/` — the binary; depends on its sibling crates via path. Also exposes a thin library shim (`src/lib.rs`) that just re-declares `pub mod` for the binary's modules so `cargo test --doc` can exercise documentation examples on the public surface. The binary in `src/main.rs` uses these modules through the library (`use hackernews_tim::{client, config, ...};`), so module source compiles once.
- `config_parser/` (crate name `config_parser2`) — runtime half of the custom TOML parser. Exposes the `ConfigParse` trait.
- `config_parser_derive/` — proc-macro crate providing `#[derive(ConfigParse)]`. Pulled in transitively by `config_parser2`.

When bumping versions, both `config_parser2` and `config_parser_derive` publish to crates.io independently; the `hackernews_tim` crate references `config_parser2` by both version and path, so local changes are picked up without a publish.

## Architecture

The application is a Cursive TUI that layers custom views on top of Hacker News data fetched from two upstream APIs.

**Entry flow (`hackernews_tim/src/main.rs`)**: `main` → `init_app_dirs` (uses `dirs_next`, returns the app-specific `hackernews-tim` subdirectory under the user's config dir plus a list of legacy dirs — currently `$XDG_CONFIG_HOME` and `$HOME/.config` — where pre-subdir files lived) → `parse_args` (clap, flags `-c/-a/-l/-i/--init-config/--update-theme/--migrate-auth`; `--init-config`, `--update-theme`, and `--migrate-auth` are mutually exclusive) → `init_logging` (tracing + `RUST_LOG`, default `hackernews_tim=info`, writes `hn-tui.log`) → unless `--init-config` was given, call `config::migrate_legacy_file` for the config and auth files when the user didn't override the path: if the new default location is empty but a legacy copy exists, copy it in (original preserved). The config migration also looks for a legacy `hn-tui.toml` (the pre-rename name) inside the app subdir itself, in addition to the pre-subdir legacy dirs → if `--init-config <light|dark>` was given, write the embedded default to the resolved `--config` path and exit → else if `--update-theme <light|dark>` was given, replace just the `[theme]` table in the existing `--config` file via `config::update_theme_in_place` (errors if the file is missing) and exit → else if `--migrate-auth <file|keyring>` was given, call `config::migrate_auth` to move credentials at the resolved `--auth` path between the plaintext-TOML backend and the OS keyring (errors if the auth file is missing) and exit → else if the config file is missing and stdin/stdout are both TTYs, prompt via `config::prompt_for_flavor` to write one → `config::load_config` (populates a global) → `init_auth`, which either reads `hn-auth.toml` or prompts via `config::prompt_for_auth` on first run (which also asks where to store credentials — file or OS keyring) → `run`, which builds an `HNClient`, optionally logs in, calls `view::init_ui`, then wraps the backend in `cursive_buffered_backend` to avoid flicker (see gyscos/Cursive#142).

**HN client (`src/client/`)**: single `HNClient` wrapping `ureq::Agent`, cached in a `once_cell`. Talks to HN Algolia (`https://hn.algolia.com/api/v1`) for search/listings and HN Firebase (`https://hacker-news.firebaseio.com/v0`) for official/live data. `query.rs` holds filter/sort types (`StorySortMode`, `StoryNumericFilters`) re-exported at the `client` module root. `model.rs` holds the deserialized item types.

**Config (`src/config/`)**: `Config` derives `ConfigParse` from the workspace's own `config_parser2` crate, so every field is optional in TOML and merged over `Config::default()`. Submodules: `theme.rs` (palette + styles, consumed in `view::init_ui` to patch the Cursive palette), `keybindings.rs` (typed `KeyMap` with a global section, per-view sections, and user-defined `custom_keymaps` for extra story tags), `init.rs` (embeds `examples/config.toml` and `examples/config-dark.toml` via `include_str!`, exposes the `ConfigFlavor` enum, `write_default_config`, and `update_theme_in_place` — which swaps the `[theme]` table of an existing config using `toml_edit` — plus the TTY-gated first-run prompts `prompt_for_flavor` and `prompt_for_auth`). The `Auth` struct carries an `AuthStorage` enum (`File` | `Keyring`); the file backend writes plaintext TOML, while the keyring backend writes only `storage = "keyring"` and `username` to disk and stores the password (and cached `user=` cookie) in the OS credential manager via the `keyring` crate, under service name `KEYRING_SERVICE = "hackernews-tim"` (password account = the username, session-cookie account = `<username>:session`). `migrate_auth(path, target)` reads the current auth and rewrites it through the chosen backend, cleaning up orphaned keyring entries when migrating away from the keyring. Globals are accessed via `config::get_config()`, `config::get_config_theme()`, `config::get_global_keymap()`. Defaults live in `examples/config.toml` (light) and `examples/config-dark.toml` (dark); docs live in `docs/config.md`.

**Views (`src/view/`)**: one module per screen — `story_view`, `comment_view`, `article_view`, `search_view`, `help_view` — plus infrastructure (`async_view` for loading states, `link_dialog`, `result_view`, `text_view`, `fn_view_wrapper`, `traits`, `utils`). `view::init_ui` installs global keymap callbacks via `Cursive::set_on_post_event`, iterates `config.keymap.custom_keymaps` to create extra Story View shortcuts, and either opens a comment view (if `-i` was passed) or the front-page story view.

**HTML/article parsing (`src/parser/`)**: uses `html5ever` + `markup5ever` + `tendril` + a vendored `rcdom.rs`. The in-terminal reader mode (`article_view`) runs `readable-readability` over the fetched page.

**Prelude (`src/prelude.rs`)**: re-exports `client`, `config`, `model`, `utils`, common Cursive types, `anyhow::Result`, and tracing macros. Most modules `use crate::prelude::*;` instead of importing directly.

## Conventions specific to this repo

- Keymap bindings support arrays (multiple keys per command) and prefix chords like `{story_id} g`; when adding a new command, wire it in `config/keybindings.rs`, document it in `README.md`'s shortcut tables, and update `docs/config.md`.
- Story tags recognized by `set_up_switch_story_view_shortcut`: `front_page`, `story`, `ask_hn`, `show_hn`, `job`. Tag equals `story` or `job` means the view defaults to `StorySortMode::Date`; everything else defaults to `StorySortMode::None`. Custom keymaps switch between `Date` and `Points` via `by_date`.
- `url_open_command` and `article_parse_command` are platform-conditional at compile time (`xdg-open` on Unix, `open` on macOS) — keep `#[cfg]` guards aligned when touching defaults.
- Authentication is best-effort: failures log a warning and the app continues unauthenticated. Don't turn auth errors into hard failures. The first-run auth prompt (`prompt_for_auth`) and config prompt (`prompt_for_flavor`) both no-op when stdin or stdout is not a TTY — preserve that behavior when editing, or non-interactive runs (CI, pipes, `-i <item>` scripts) will hang.
- `Makefile` targets must stay in sync with `make help` (the help awk scans `##` comments) — add `## description` to every new `.PHONY` target.
