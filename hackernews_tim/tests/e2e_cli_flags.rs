//! docs/planning/TEST_PLAN.md §3.2.h — CLI flag scenarios (Linux-only). Covers
//! scenarios 3.2.10–3.2.13 (`-i`, `--init-config`, `--update-theme`,
//! `--migrate-auth`).
//!
//! See [`e2e_first_run.rs`] for the surrounding harness conventions
//! (TTY-gated flavor / auth prompts, `HOME` / `XDG_*` isolation,
//! `HN_ALGOLIA_BASE` / `HN_FIREBASE_BASE` / `HN_NEWS_BASE` overrides).

#![cfg(target_os = "linux")]

#[path = "e2e/mod.rs"]
mod helpers;

use std::time::Duration;

use serde_json::json;

use helpers::fakehn::FakeHnServer;
use helpers::keyring::{clear_mock_keyring, init_mock_keyring};
use helpers::{spawn_app, AppHandle, SpawnOptions, TestDirs, DEFAULT_WAIT};

use hackernews_tim::config::{self, Auth, AuthStorage, MigrationOutcome, KEYRING_SERVICE};

const FRONT_PAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(60);

/// Pre-write a minimal config TOML at the resolved `--config` path so
/// the binary skips `prompt_for_flavor`. The auth prompt still fires
/// because no auth file exists; callers handle that separately.
fn pre_write_default_config(dirs: &TestDirs) {
    let path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).expect("create config dir");
    // Empty TOML is enough — every field on `Config` is optional and
    // merges over `Config::default()`.
    std::fs::write(&path, "").expect("write config.toml");
}

/// Skip the auth prompt that `init_auth` fires when no auth file
/// exists. Sends an empty newline (default = N).
fn dismiss_auth_prompt(handle: &mut AppHandle) {
    handle
        .wait_for_text("No auth file found", DEFAULT_WAIT)
        .expect("auth prompt should print since no auth file exists");
    handle.send_keys("\n").expect("decline auth prompt");
}

// =====================================================================
// 3.2.10 — `-i <item_id>` direct entry
// =====================================================================

const DIRECT_ITEM_ID: u32 = 99001;
const DIRECT_ITEM_TITLE: &str = "direct-entry fixture story";

/// Mount the minimal endpoints `construct_and_add_new_comment_view`
/// needs to render an item with no kids:
///
/// * `/v0/item/<id>.json` (Firebase) — the root item, parsed as
///   `ItemResponse`.
/// * `/item` (news_base) — the rendered HTML page used by
///   `parse_vote_data_from_content`. An empty body is fine; the
///   matcher ignores the query string.
fn mount_direct_item(server: &FakeHnServer) {
    server.mount_get_json(
        format!("/v0/item/{DIRECT_ITEM_ID}.json"),
        200,
        json!({
            "id": DIRECT_ITEM_ID,
            "type": "story",
            "by": "alice",
            "title": DIRECT_ITEM_TITLE,
            "url": format!("https://example.com/{DIRECT_ITEM_ID}"),
            "score": 250,
            "descendants": 0,
            "time": 1_700_000_000_u64,
            "kids": [],
            "text": "",
        }),
    );
    server.mount_get_text("/item", 200, "<html><body></body></html>");
}

#[test]
fn dash_i_opens_directly_into_comment_view() {
    let server = FakeHnServer::start();
    mount_direct_item(&server);

    let dirs = TestDirs::new().expect("TestDirs::new");
    pre_write_default_config(&dirs);

    let opts = SpawnOptions::new()
        .algolia_base(server.algolia_base())
        .firebase_base(server.firebase_base())
        .news_base(server.news_base())
        .arg("-i")
        .arg(DIRECT_ITEM_ID.to_string())
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    dismiss_auth_prompt(&mut handle);

    handle
        .wait_for_text(
            &format!("Comment View - {DIRECT_ITEM_TITLE}"),
            FRONT_PAGE_RENDER_TIMEOUT,
        )
        .expect("comment view header should show the fixture item title");

    let screen = handle.screen();
    assert!(
        !screen.contains("Story View"),
        "expected to land directly on the comment view, not pass through a story view; saw:\n{screen}"
    );

    // Belt-and-suspenders: the binary should have hit Firebase for
    // exactly the item we passed via `-i`.
    let item_requests: Vec<_> = server
        .received_requests()
        .into_iter()
        .filter(|r| {
            r.method.as_str() == "GET" && r.url.path() == format!("/v0/item/{DIRECT_ITEM_ID}.json")
        })
        .collect();
    assert!(
        !item_requests.is_empty(),
        "expected at least one GET /v0/item/{DIRECT_ITEM_ID}.json; got requests: {:?}",
        server
            .received_requests()
            .iter()
            .map(|r| (r.method.as_str().to_string(), r.url.to_string()))
            .collect::<Vec<_>>()
    );

    let status = handle.shutdown().expect("binary should exit cleanly");
    assert!(status.success(), "expected success exit, got {status:?}");
}

// =====================================================================
// 3.2.11 — `--init-config light` / `--init-config dark`
// =====================================================================

/// Embedded copies of the default configs the binary writes via
/// `--init-config`. Asserting byte-for-byte equality catches any
/// drift between the embedded TOML and the in-tree examples.
const EMBEDDED_LIGHT_CONFIG: &str = include_str!("../../examples/config.toml");
const EMBEDDED_DARK_CONFIG: &str = include_str!("../../examples/config-dark.toml");

/// Drive `--init-config <flavor>`: spawn, wait for the success line,
/// wait for the binary to exit, then return the path the config was
/// written to plus its on-disk contents.
fn run_init_config(flavor: &str) -> (std::path::PathBuf, String) {
    let dirs = TestDirs::new().expect("TestDirs::new");
    let expected_path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("config.toml");
    assert!(
        !expected_path.exists(),
        "config file should not exist yet at {}",
        expected_path.display()
    );

    let opts = SpawnOptions::new()
        .arg("--init-config")
        .arg(flavor)
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    let success_marker = format!("Wrote default {flavor} config to");
    handle
        .wait_for_text(&success_marker, Duration::from_secs(10))
        .unwrap_or_else(|e| panic!("expected success line for --init-config {flavor}: {e}"));

    let status = handle
        .wait_for_exit(Duration::from_secs(5))
        .expect("binary should exit on its own after writing the config");
    assert!(
        status.success(),
        "--init-config {flavor} should exit 0; got {status:?}"
    );

    let written =
        std::fs::read_to_string(&expected_path).expect("written config should be readable");
    (expected_path, written)
}

#[test]
fn init_config_light_writes_embedded_light_default() {
    let (path, written) = run_init_config("light");
    assert_eq!(
        written,
        EMBEDDED_LIGHT_CONFIG,
        "config at {} should match the embedded light default byte-for-byte",
        path.display()
    );
}

#[test]
fn init_config_dark_writes_embedded_dark_default() {
    let (path, written) = run_init_config("dark");
    assert_eq!(
        written,
        EMBEDDED_DARK_CONFIG,
        "config at {} should match the embedded dark default byte-for-byte",
        path.display()
    );
}

// =====================================================================
// 3.2.12 — `--update-theme dark`
// =====================================================================

#[test]
fn update_theme_dark_swaps_only_theme_table() {
    let dirs = TestDirs::new().expect("TestDirs::new");
    let config_path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("config.toml");
    std::fs::create_dir_all(config_path.parent().unwrap()).expect("create config dir");

    // Seed with the embedded light default. The post-update file
    // should retain every non-`[theme]` table from this seed
    // unchanged, with `[theme]` swapped for the dark version.
    std::fs::write(&config_path, EMBEDDED_LIGHT_CONFIG).expect("seed light config");

    let opts = SpawnOptions::new()
        .arg("--update-theme")
        .arg("dark")
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    handle
        .wait_for_text("Updated dark theme in", Duration::from_secs(10))
        .expect("binary should announce the theme update");

    let status = handle
        .wait_for_exit(Duration::from_secs(5))
        .expect("binary should exit on its own after updating the theme");
    assert!(
        status.success(),
        "--update-theme dark should exit 0; got {status:?}"
    );

    let updated = std::fs::read_to_string(&config_path).expect("updated config readable");
    let updated_value: toml::Value =
        toml::from_str(&updated).expect("updated config should parse as TOML");
    let light_value: toml::Value =
        toml::from_str(EMBEDDED_LIGHT_CONFIG).expect("light default should parse as TOML");
    let dark_value: toml::Value =
        toml::from_str(EMBEDDED_DARK_CONFIG).expect("dark default should parse as TOML");

    // The new `[theme]` table must match the embedded dark default's.
    assert_eq!(
        updated_value.get("theme"),
        dark_value.get("theme"),
        "[theme] table should be the dark default after --update-theme dark"
    );

    // Every other top-level key must equal what the light default
    // shipped with — that's the "all other tables are byte-identical"
    // requirement from docs/planning/TEST_PLAN.md table 3.2.12, asserted via
    // structural equality so re-serialization of the dark theme block
    // doesn't trip a literal byte diff.
    let updated_table = updated_value.as_table().expect("toml root is a table");
    let light_table = light_value.as_table().expect("light root is a table");
    for (key, value) in light_table {
        if key == "theme" {
            continue;
        }
        assert_eq!(
            updated_table.get(key),
            Some(value),
            "non-theme top-level key '{key}' should be preserved verbatim"
        );
    }
    for key in updated_table.keys() {
        assert!(
            light_table.contains_key(key),
            "unexpected new top-level key '{key}' after --update-theme"
        );
    }
}

// =====================================================================
// 3.2.13 — `--migrate-auth file` ↔ `--migrate-auth keyring`
// =====================================================================
//
// The keyring round-trip is exercised in-process (`config::migrate_auth`
// directly), since `keyring::set_default_credential_builder` mutates a
// process-global slot that does not propagate to children spawned via
// `spawn_app` (see `helpers::keyring` for the rationale). The
// spawned-binary side is covered by a NoOp scenario that only touches
// the file backend, which proves the CLI plumbing — argument parsing,
// success line, exit code — without needing the keyring at all.

const MIGRATE_USERNAME: &str = "migrate_user";
const MIGRATE_PASSWORD: &str = "hunter2";
const MIGRATE_SESSION: &str = "migrate_user&abcdef0123";

fn keyring_password_account(username: &str) -> String {
    username.to_string()
}

fn keyring_session_account(username: &str) -> String {
    format!("{username}:session")
}

fn seed_file_backed_auth(path: &std::path::Path) {
    std::fs::create_dir_all(path.parent().unwrap()).expect("create auth dir");
    Auth {
        username: MIGRATE_USERNAME.to_string(),
        password: MIGRATE_PASSWORD.to_string(),
        session: Some(MIGRATE_SESSION.to_string()),
        storage: AuthStorage::File,
    }
    .write_to_file(path)
    .expect("seed file-backed auth");
}

#[test]
fn migrate_auth_round_trips_file_keyring_file() {
    init_mock_keyring();
    // The stateful mock store survives across tests in the same
    // binary (the `keyring` default-builder slot is process-global);
    // start each run with an empty slate.
    clear_mock_keyring();

    let dirs = TestDirs::new().expect("TestDirs::new");
    let auth_path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("hn-auth.toml");

    seed_file_backed_auth(&auth_path);

    // --- File → Keyring ---------------------------------------------
    let outcome =
        config::migrate_auth(&auth_path, AuthStorage::Keyring).expect("migrate to keyring");
    assert!(
        matches!(
            outcome,
            MigrationOutcome::Migrated {
                from: AuthStorage::File,
                to: AuthStorage::Keyring,
            }
        ),
        "expected Migrated File → Keyring; got {outcome:?}"
    );

    // The file should now be a pointer (no plaintext password).
    let pointer_text = std::fs::read_to_string(&auth_path).expect("read pointer file");
    let pointer: toml::Value = toml::from_str(&pointer_text).expect("pointer parses as TOML");
    assert_eq!(
        pointer.get("storage").and_then(|v| v.as_str()),
        Some("keyring"),
        "pointer should declare keyring storage; full file:\n{pointer_text}"
    );
    assert_eq!(
        pointer.get("username").and_then(|v| v.as_str()),
        Some(MIGRATE_USERNAME),
        "pointer should carry the username; full file:\n{pointer_text}"
    );
    assert!(
        pointer.get("password").is_none(),
        "keyring pointer must not contain a plaintext password; saw:\n{pointer_text}"
    );

    // The keyring should hold both the password and session under the
    // documented service / account names.
    let pw_entry =
        ::keyring::Entry::new(KEYRING_SERVICE, &keyring_password_account(MIGRATE_USERNAME))
            .expect("password Entry");
    assert_eq!(
        pw_entry.get_password().expect("get password from keyring"),
        MIGRATE_PASSWORD,
    );
    let session_entry =
        ::keyring::Entry::new(KEYRING_SERVICE, &keyring_session_account(MIGRATE_USERNAME))
            .expect("session Entry");
    assert_eq!(
        session_entry
            .get_password()
            .expect("get session from keyring"),
        MIGRATE_SESSION,
    );

    // --- Keyring → Keyring (NoOp) ----------------------------------
    let outcome =
        config::migrate_auth(&auth_path, AuthStorage::Keyring).expect("noop migrate to keyring");
    assert!(
        matches!(
            outcome,
            MigrationOutcome::NoOp {
                storage: AuthStorage::Keyring,
            }
        ),
        "second migrate to keyring should be a NoOp; got {outcome:?}"
    );

    // --- Keyring → File --------------------------------------------
    let outcome =
        config::migrate_auth(&auth_path, AuthStorage::File).expect("migrate back to file");
    assert!(
        matches!(
            outcome,
            MigrationOutcome::Migrated {
                from: AuthStorage::Keyring,
                to: AuthStorage::File,
            }
        ),
        "expected Migrated Keyring → File; got {outcome:?}"
    );

    let restored_text = std::fs::read_to_string(&auth_path).expect("read restored file");
    let restored: toml::Value = toml::from_str(&restored_text).expect("restored parses as TOML");
    assert_eq!(
        restored.get("username").and_then(|v| v.as_str()),
        Some(MIGRATE_USERNAME),
    );
    assert_eq!(
        restored.get("password").and_then(|v| v.as_str()),
        Some(MIGRATE_PASSWORD),
        "restored file must contain plaintext password; saw:\n{restored_text}"
    );
    assert_eq!(
        restored.get("session").and_then(|v| v.as_str()),
        Some(MIGRATE_SESSION),
    );
    // `storage` defaults to File; the writer either omits it or writes
    // "file". Accept either to avoid coupling the test to formatting.
    let storage_value = restored.get("storage").and_then(|v| v.as_str());
    assert!(
        matches!(storage_value, None | Some("file")),
        "restored file should declare file storage (or omit it); saw {storage_value:?}"
    );

    // Keyring entries should be cleaned up — `get_password` errors on
    // a deleted credential under the mock backend.
    let pw_entry =
        ::keyring::Entry::new(KEYRING_SERVICE, &keyring_password_account(MIGRATE_USERNAME))
            .expect("password Entry post-migrate");
    assert!(
        pw_entry.get_password().is_err(),
        "keyring password should be cleared after migrating back to file"
    );
    let session_entry =
        ::keyring::Entry::new(KEYRING_SERVICE, &keyring_session_account(MIGRATE_USERNAME))
            .expect("session Entry post-migrate");
    assert!(
        session_entry.get_password().is_err(),
        "keyring session should be cleared after migrating back to file"
    );

    // --- File → File (NoOp) ----------------------------------------
    let outcome =
        config::migrate_auth(&auth_path, AuthStorage::File).expect("noop migrate to file");
    assert!(
        matches!(
            outcome,
            MigrationOutcome::NoOp {
                storage: AuthStorage::File,
            }
        ),
        "second migrate to file should be a NoOp; got {outcome:?}"
    );
}

#[test]
fn migrate_auth_file_to_file_via_binary_prints_noop() {
    let dirs = TestDirs::new().expect("TestDirs::new");
    let auth_path = dirs
        .xdg_config_home
        .join("hackernews-tim")
        .join("hn-auth.toml");
    seed_file_backed_auth(&auth_path);
    let pre_text = std::fs::read_to_string(&auth_path).expect("read seeded auth");

    // Pre-write an empty config to skip `prompt_for_flavor` — though
    // the `--migrate-auth` branch exits before the flavor prompt, the
    // safety net keeps the test independent of that ordering.
    pre_write_default_config(&dirs);

    let opts = SpawnOptions::new()
        .arg("--migrate-auth")
        .arg("file")
        .dirs(dirs);
    let mut handle = spawn_app(opts).expect("spawn_app should succeed");

    handle
        .wait_for_text(
            "is already using file storage; nothing to migrate.",
            Duration::from_secs(10),
        )
        .expect("binary should print the NoOp message");

    let status = handle
        .wait_for_exit(Duration::from_secs(5))
        .expect("binary should exit on its own after the NoOp branch");
    assert!(
        status.success(),
        "--migrate-auth file (already file) should exit 0; got {status:?}"
    );

    // The auth file should be byte-identical — a NoOp must not rewrite
    // the file on disk.
    let post_text = std::fs::read_to_string(&auth_path).expect("read auth post-NoOp");
    assert_eq!(
        post_text, pre_text,
        "NoOp migrate must not modify the auth file"
    );
}
