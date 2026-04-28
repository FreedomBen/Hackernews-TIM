// modules
mod init;
mod keybindings;
mod theme;

// re-export
pub use init::*;
pub use keybindings::*;
pub use theme::*;

use config_parser2::*;
use serde::Deserialize;

/// Inclusive bounds for [`Config::page_size`]. The floor keeps the
/// paging UX sane (a single-digit page size makes navigation tedious
/// and inflates per-page Algolia/Firebase round trips). The ceiling is
/// Algolia's maximum accepted `hitsPerPage`, which is also plenty for
/// a terminal viewport.
pub const MIN_PAGE_SIZE: usize = 5;
pub const MAX_PAGE_SIZE: usize = 100;
pub const DEFAULT_PAGE_SIZE: usize = 20;

/// Inclusive bounds for [`Config::search_page_size`]. The search view
/// is Algolia-only (no Firebase listing reconciliation), but a larger
/// cap here would overflow the terminal viewport with match previews
/// while offering little value — search results are usually winnowed
/// down by query, not by paging.
pub const MIN_SEARCH_PAGE_SIZE: usize = 5;
pub const MAX_SEARCH_PAGE_SIZE: usize = 30;
pub const DEFAULT_SEARCH_PAGE_SIZE: usize = 15;

#[derive(Debug, Deserialize, ConfigParse)]
/// Config is a struct storing the application's configurations
pub struct Config {
    pub use_page_scrolling: bool,
    pub use_pacman_loading: bool,
    pub use_hn_topcolor: bool,
    pub client_timeout: u64,
    /// Number of stories per TUI listing page. Clamped to
    /// [`MIN_PAGE_SIZE`]..=[`MAX_PAGE_SIZE`] on read via [`page_size`].
    pub page_size: u64,
    /// Number of results per search-view page. Clamped to
    /// [`MIN_SEARCH_PAGE_SIZE`]..=[`MAX_SEARCH_PAGE_SIZE`] on read via
    /// [`search_page_size`].
    pub search_page_size: u64,
    pub url_open_command: Command,
    pub article_parse_command: Command,

    pub theme: theme::Theme,
    pub keymap: keybindings::KeyMap,
}

/// Service name used for all of the app's keyring entries. Kept stable so
/// upgrades and `--migrate-auth` find existing items.
pub const KEYRING_SERVICE: &str = "hackernews-tim";

/// Where the user's HN credentials live.
///
/// `File` is the legacy plaintext-TOML behavior (still the default for
/// backward compatibility). `Keyring` stores the password and cached
/// session cookie in the OS credential manager (macOS Keychain, Windows
/// Credential Manager, or Linux Secret Service); the on-disk file then
/// becomes a small pointer carrying just `storage = "keyring"` and the
/// username.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthStorage {
    #[default]
    File,
    Keyring,
}

impl std::fmt::Display for AuthStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AuthStorage::File => "file",
            AuthStorage::Keyring => "keyring",
        })
    }
}

impl std::str::FromStr for AuthStorage {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "file" => Ok(AuthStorage::File),
            "keyring" => Ok(AuthStorage::Keyring),
            other => Err(format!(
                "unknown auth storage '{other}' (expected 'file' or 'keyring')"
            )),
        }
    }
}

#[derive(Debug, Clone)]
/// HackerNews user's authentication data
pub struct Auth {
    pub username: String,
    pub password: String,
    /// Cached HN session cookie value (the `user=` cookie). When present, the
    /// app uses it to restore a logged-in session instead of POSTing to
    /// `/login` on every startup — important because HN throttles repeated
    /// `/login` attempts from the same IP with a CAPTCHA challenge.
    pub session: Option<String>,
    /// Where this `Auth` was loaded from / should be written back to. Round
    /// trips so the in-app login dialog and the cookie-refresh path persist
    /// through the same backend the user originally chose.
    pub storage: AuthStorage,
}

/// Mirror of the on-disk auth file. `password` and `session` are only
/// present when `storage == file`; for `storage = "keyring"` they're
/// fetched from the OS credential manager keyed by the username.
#[derive(Deserialize)]
struct AuthFileRepr {
    storage: Option<String>,
    username: String,
    password: Option<String>,
    session: Option<String>,
}

fn keyring_password_account(username: &str) -> String {
    username.to_string()
}

fn keyring_session_account(username: &str) -> String {
    format!("{username}:session")
}

fn keyring_set(account: &str, value: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)?;
    entry.set_password(value)?;
    Ok(())
}

fn keyring_get(account: &str) -> anyhow::Result<Option<String>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Best-effort delete: a missing entry isn't an error, since we use this
/// to clean up after a migration that may or may not have left orphans.
fn keyring_delete(account: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Probe the OS keyring for read+write+delete access. Used before we
/// commit a user to keyring storage on first run, so a misconfigured
/// headless system (no Secret Service / locked Keychain) gets caught
/// while we can still cleanly fall back to file storage.
pub fn keyring_available() -> bool {
    const PROBE_ACCOUNT: &str = "__probe__";
    let entry = match keyring::Entry::new(KEYRING_SERVICE, PROBE_ACCOUNT) {
        Ok(e) => e,
        Err(_) => return false,
    };
    if entry.set_password("ok").is_err() {
        return false;
    }
    let read_ok = entry.get_password().is_ok();
    let _ = entry.delete_credential();
    read_ok
}

impl Config {
    /// parse config from a file
    pub fn from_file<P>(file: P) -> anyhow::Result<Self>
    where
        P: AsRef<std::path::Path>,
    {
        let config_str = std::fs::read_to_string(file)?;
        let value = toml::from_str::<toml::Value>(&config_str)?;
        let mut config = Self::default();
        config.parse(value)?;
        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            use_page_scrolling: true,
            use_pacman_loading: true,
            use_hn_topcolor: true,
            #[cfg(all(unix, not(target_os = "macos")))]
            url_open_command: Command {
                command: "xdg-open".to_string(),
                options: vec![],
            },
            #[cfg(target_os = "macos")]
            url_open_command: Command {
                command: "open".to_string(),
                options: vec![],
            },
            #[cfg(target_os = "windows")]
            url_open_command: Command {
                command: "start".to_string(),
                options: vec![],
            },
            article_parse_command: Command {
                command: "article_md".to_string(),
                options: vec!["--format".to_string(), "html".to_string()],
            },
            client_timeout: 32,
            page_size: DEFAULT_PAGE_SIZE as u64,
            search_page_size: DEFAULT_SEARCH_PAGE_SIZE as u64,
            theme: theme::Theme::default(),
            keymap: keybindings::KeyMap::default(),
        }
    }
}

impl Auth {
    /// Parse auth from `file`, fetching the password (and any cached
    /// session cookie) from the OS keyring when the file declares
    /// `storage = "keyring"`. Errors if the keyring is unavailable or
    /// has no entry for this user — callers handle the failure as
    /// "no auth", per the project's best-effort auth posture.
    pub fn from_file<P>(file: P) -> anyhow::Result<Self>
    where
        P: AsRef<std::path::Path>,
    {
        let auth_str = std::fs::read_to_string(file)?;
        let repr: AuthFileRepr = toml::from_str(&auth_str)?;
        let storage: AuthStorage = repr
            .storage
            .as_deref()
            .unwrap_or("file")
            .parse()
            .map_err(|err: String| anyhow::anyhow!(err))?;
        match storage {
            AuthStorage::File => {
                let password = repr
                    .password
                    .ok_or_else(|| anyhow::anyhow!("auth file is missing the `password` field"))?;
                // Treat the hand-editable placeholder (`session = ""`) the
                // same as a missing field so downstream code only has to
                // match on the `Some` case to mean "we have a cached
                // cookie to try".
                let session = repr.session.filter(|s| !s.is_empty());
                Ok(Auth {
                    username: repr.username,
                    password,
                    session,
                    storage,
                })
            }
            AuthStorage::Keyring => {
                let username = repr.username;
                let password =
                    keyring_get(&keyring_password_account(&username))?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "no password found in OS keyring for user '{username}' \
                             (service '{KEYRING_SERVICE}')"
                        )
                    })?;
                let session =
                    keyring_get(&keyring_session_account(&username))?.filter(|s| !s.is_empty());
                Ok(Auth {
                    username,
                    password,
                    session,
                    storage,
                })
            }
        }
    }

    /// Persist auth using the backend named by `self.storage`, creating any
    /// missing parent directories for the file. On Unix the file is chmod'd
    /// to `0600` so other local users can't read it.
    ///
    /// For `File`: writes the full annotated TOML (username, password, and
    /// the always-emitted `session` placeholder + cookie-paste guidance,
    /// for users stuck behind HN's CAPTCHA).
    ///
    /// For `Keyring`: writes the password (and session cookie, if any) to
    /// the OS keyring first, then writes a small pointer file containing
    /// just `storage = "keyring"` and `username = ...`. Keyring-first
    /// ordering means a crash mid-write leaves the user with a still-valid
    /// previous file rather than a pointer to nothing.
    pub fn write_to_file<P>(&self, file: P) -> anyhow::Result<()>
    where
        P: AsRef<std::path::Path>,
    {
        let path = file.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        match self.storage {
            AuthStorage::File => {
                std::fs::write(path, self.to_annotated_file_toml())?;
            }
            AuthStorage::Keyring => {
                keyring_set(&keyring_password_account(&self.username), &self.password)?;
                match self.session.as_deref() {
                    Some(s) if !s.is_empty() => {
                        keyring_set(&keyring_session_account(&self.username), s)?;
                    }
                    _ => {
                        // Cookie cleared → drop any stale entry so it can't
                        // shadow a later explicit re-cache.
                        keyring_delete(&keyring_session_account(&self.username))?;
                    }
                }
                std::fs::write(path, self.to_annotated_keyring_toml())?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn to_annotated_file_toml(&self) -> String {
        let mut doc = toml_edit::DocumentMut::new();
        doc["username"] = toml_edit::value(self.username.as_str());
        doc["password"] = toml_edit::value(self.password.as_str());
        doc["session"] = toml_edit::value(self.session.as_deref().unwrap_or(""));

        if let Some(mut key) = doc.as_table_mut().key_mut("session") {
            key.leaf_decor_mut().set_prefix(concat!(
                "\n",
                "# `session` is the value of Hacker News's `user=` cookie.\n",
                "# The TUI fills this in automatically after a successful\n",
                "# login so later runs can skip the `/login` POST, which HN\n",
                "# throttles with a CAPTCHA after a few attempts.\n",
                "#\n",
                "# If you get stuck at the CAPTCHA, populate it by hand:\n",
                "#   1. Sign in to https://news.ycombinator.com/ in a browser.\n",
                "#   2. Open DevTools -> Application/Storage -> Cookies ->\n",
                "#      https://news.ycombinator.com.\n",
                "#   3. Copy the value of the cookie named `user` (looks like\n",
                "#      `yourname&abcdef0123...`).\n",
                "#   4. Paste it between the quotes below.\n",
            ));
        }

        doc.to_string()
    }

    fn to_annotated_keyring_toml(&self) -> String {
        let mut doc = toml_edit::DocumentMut::new();
        doc["storage"] = toml_edit::value("keyring");
        doc["username"] = toml_edit::value(self.username.as_str());

        if let Some(mut key) = doc.as_table_mut().key_mut("storage") {
            key.leaf_decor_mut().set_prefix(format!(
                "# Credentials for this user are stored in the OS keyring\n\
                 # (service \"{KEYRING_SERVICE}\"). The password lives at\n\
                 # account = \"<username>\" and the cached session cookie at\n\
                 # account = \"<username>:session\".\n\
                 #\n\
                 # Run `hackernews_tim --migrate-auth file` to switch back to\n\
                 # plaintext-on-disk storage.\n"
            ));
        }

        doc.to_string()
    }
}

/// Outcome of a [`migrate_auth`] call. The CLI uses this to print a
/// helpful message — distinguishing "nothing to do" from "moved" lets
/// the user know whether the keyring was actually touched.
#[derive(Debug, Clone, Copy)]
pub enum MigrationOutcome {
    NoOp { storage: AuthStorage },
    Migrated { from: AuthStorage, to: AuthStorage },
}

/// Move the credentials at `auth_path` between storage backends. Reads
/// the current auth (which may pull secrets from the keyring), persists
/// it to the new backend, and — when migrating away from the keyring —
/// cleans up the now-orphaned keyring entries so a future
/// `--migrate-auth keyring` doesn't pick up stale data.
pub fn migrate_auth(
    auth_path: &std::path::Path,
    target: AuthStorage,
) -> anyhow::Result<MigrationOutcome> {
    if !auth_path.exists() {
        anyhow::bail!(
            "auth file {} does not exist; log in once first (run the app or press `L` in the TUI) before migrating",
            auth_path.display()
        );
    }

    let mut auth = Auth::from_file(auth_path)?;
    let from = auth.storage;
    if from == target {
        return Ok(MigrationOutcome::NoOp { storage: target });
    }

    let username = auth.username.clone();
    auth.storage = target;
    auth.write_to_file(auth_path)?;

    if from == AuthStorage::Keyring {
        keyring_delete(&keyring_password_account(&username))?;
        keyring_delete(&keyring_session_account(&username))?;
    }

    Ok(MigrationOutcome::Migrated { from, to: target })
}

/// If `path` is an auth file written by an older version (no `session = `
/// line), rewrite it in the current annotated format so the cookie-paste
/// guidance is visible the next time the user opens it. Returns `true` when
/// a rewrite happened.
///
/// The match is deliberately conservative: any `session =` line (even one
/// the user has already edited or blanked out) is treated as "already
/// migrated" so we never overwrite intentional hand-edits. Only a file that
/// has never carried the field at all gets upgraded.
///
/// No-op for keyring-backed auth files — those don't carry a `session`
/// field on disk to begin with (the cookie lives in the OS keyring), so
/// there's nothing to backport.
pub fn backport_auth_file(path: &std::path::Path, auth: &Auth) -> anyhow::Result<bool> {
    if auth.storage != AuthStorage::File {
        return Ok(false);
    }
    let existing = std::fs::read_to_string(path)?;
    let already_has_session =
        regex::Regex::new(r"(?m)^\s*session\s*=").is_ok_and(|rg| rg.is_match(&existing));
    if already_has_session {
        return Ok(false);
    }
    auth.write_to_file(path)?;
    Ok(true)
}

#[derive(Debug, Deserialize, Clone)]
pub struct Command {
    pub command: String,
    pub options: Vec<String>,
}

config_parser_impl!(Command);

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{} {}", self.command, self.options.join(" ")))
    }
}

static CONFIG: once_cell::sync::OnceCell<Config> = once_cell::sync::OnceCell::new();

/// Load the configuration from a file, returning an owned `Config` without
/// sealing it into the global. Callers can mutate the returned value (for
/// example to apply a per-user HN topcolor override) before handing it to
/// [`init_config`]. If the file can't be read or parsed, the default config
/// is returned and the failure is logged.
pub fn load_config_file(config_file_str: &str) -> Config {
    let config_file = std::path::PathBuf::from(config_file_str);

    match Config::from_file(config_file) {
        Err(err) => {
            tracing::error!(
                "failed to load configurations from the file {config_file_str}: {err:#}\
                 \nUse the default configurations instead",
            );
            Config::default()
        }
        Ok(config) => config,
    }
}

/// Seal the given config into the global. Must be called exactly once, before
/// any call to [`get_config`]. Panics on a second invocation.
pub fn init_config(config: Config) {
    tracing::info!("application's configurations: {:?}", config);
    CONFIG.set(config).unwrap_or_else(|_| {
        panic!("failed to set up the application's configurations");
    });
}

pub fn get_config() -> &'static Config {
    CONFIG.get().unwrap()
}

/// Idempotently install [`Config::default`] into the global so tests that
/// call into code paths reading `get_config_theme()` etc. don't panic. Safe
/// to call from many test modules; the first caller wins and the rest are
/// no-ops because all tests share the same default config snapshot.
///
/// Available to integration tests via the `test-support` feature.
#[cfg(any(test, feature = "test-support"))]
pub fn init_test_config() {
    let _ = CONFIG.set(Config::default());
}

/// The story-listing page size, clamped to
/// [`MIN_PAGE_SIZE`]..=[`MAX_PAGE_SIZE`]. Read lazily so a user-facing
/// value out of range silently gets pulled into a working range rather
/// than panicking or failing config parse.
pub fn page_size() -> usize {
    clamp_page_size(get_config().page_size as usize)
}

pub(crate) fn clamp_page_size(raw: usize) -> usize {
    raw.clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE)
}

/// Results per search-view page, clamped to
/// [`MIN_SEARCH_PAGE_SIZE`]..=[`MAX_SEARCH_PAGE_SIZE`].
pub fn search_page_size() -> usize {
    clamp_search_page_size(get_config().search_page_size as usize)
}

pub(crate) fn clamp_search_page_size(raw: usize) -> usize {
    raw.clamp(MIN_SEARCH_PAGE_SIZE, MAX_SEARCH_PAGE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::{Auth, AuthStorage};

    fn tmp_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hackernews_tim_auth_test_{}_{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn auth_write_then_read_round_trips() {
        let path = tmp_path("round_trip");
        let _ = std::fs::remove_file(&path);

        let original = Auth {
            username: "alice".to_string(),
            password: "hunter2".to_string(),
            session: None,
            storage: AuthStorage::File,
        };
        original.write_to_file(&path).expect("write should succeed");

        let parsed = Auth::from_file(&path).expect("read should succeed");
        assert_eq!(parsed.username, original.username);
        assert_eq!(parsed.password, original.password);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auth_write_then_read_preserves_session() {
        let path = tmp_path("session_round_trip");
        let _ = std::fs::remove_file(&path);

        let original = Auth {
            username: "alice".to_string(),
            password: "hunter2".to_string(),
            session: Some("alice&abcdef123456".to_string()),
            storage: AuthStorage::File,
        };
        original.write_to_file(&path).expect("write should succeed");

        let parsed = Auth::from_file(&path).expect("read should succeed");
        assert_eq!(parsed.session, original.session);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auth_write_always_emits_session_placeholder_with_guidance() {
        // Users who get stuck behind HN's CAPTCHA need a documented slot to
        // paste a browser cookie into — so the `session` key is always
        // written, with an explanatory comment, even when the app has no
        // cached cookie yet.
        let path = tmp_path("annotated_write");
        let _ = std::fs::remove_file(&path);

        Auth {
            username: "alice".to_string(),
            password: "hunter2".to_string(),
            session: None,
            storage: AuthStorage::File,
        }
        .write_to_file(&path)
        .expect("write should succeed");

        let written = std::fs::read_to_string(&path).expect("read should succeed");
        assert!(
            written.contains("session = \"\""),
            "expected empty session line, got:\n{written}"
        );
        assert!(
            written.contains("user="),
            "expected the `user=` cookie hint in the guidance, got:\n{written}"
        );
        assert!(
            written.contains("news.ycombinator.com"),
            "expected a link to HN in the guidance, got:\n{written}"
        );

        // And the round-trip still works: empty session normalises to None.
        let parsed = Auth::from_file(&path).expect("read should succeed");
        assert_eq!(parsed.session, None);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn backport_rewrites_legacy_file_with_session_line() {
        // Old files (v0.13 and earlier) only had `username`/`password` —
        // backport should replay them through the annotated writer so the
        // user sees the cookie-paste guidance.
        let path = tmp_path("backport_legacy");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "username = \"bob\"\npassword = \"pw\"\n")
            .expect("seed write should succeed");

        let auth = super::Auth::from_file(&path).expect("read should succeed");
        let rewrote = super::backport_auth_file(&path, &auth).expect("backport should succeed");
        assert!(rewrote, "expected legacy file to be rewritten");

        let new_body = std::fs::read_to_string(&path).expect("read should succeed");
        assert!(
            new_body.contains("session = \"\""),
            "expected empty session placeholder, got:\n{new_body}"
        );
        assert!(
            new_body.contains("news.ycombinator.com"),
            "expected cookie-paste guidance, got:\n{new_body}"
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn backport_noops_when_session_line_already_present() {
        // If the file already has a session line — empty or populated —
        // don't touch it, so user edits (and existing caches) are preserved.
        let path = tmp_path("backport_noop");
        let _ = std::fs::remove_file(&path);
        let body = "username = \"bob\"\npassword = \"pw\"\nsession = \"\"\n";
        std::fs::write(&path, body).expect("seed write should succeed");

        let auth = super::Auth::from_file(&path).expect("read should succeed");
        let rewrote = super::backport_auth_file(&path, &auth).expect("backport should succeed");
        assert!(!rewrote, "expected already-migrated file to be left alone");

        let unchanged = std::fs::read_to_string(&path).expect("read should succeed");
        assert_eq!(unchanged, body);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auth_read_tolerates_missing_session() {
        // A file written by an older version has no `session` field; parsing
        // must still succeed and leave `session` as None.
        let path = tmp_path("missing_session");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "username = \"bob\"\npassword = \"pw\"\n")
            .expect("seed file should succeed");

        let parsed = Auth::from_file(&path).expect("read should succeed");
        assert_eq!(parsed.username, "bob");
        assert_eq!(parsed.session, None);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auth_write_creates_parent_dirs() {
        let dir = tmp_path("parent_dirs");
        let path = dir.join("nested").join("hn-auth.toml");
        let _ = std::fs::remove_dir_all(&dir);

        Auth {
            username: "bob".to_string(),
            password: "pw".to_string(),
            session: None,
            storage: AuthStorage::File,
        }
        .write_to_file(&path)
        .expect("write should succeed");
        assert!(path.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn auth_storage_parse_round_trip() {
        use super::AuthStorage;
        assert_eq!("file".parse::<AuthStorage>().unwrap(), AuthStorage::File);
        assert_eq!("File".parse::<AuthStorage>().unwrap(), AuthStorage::File);
        assert_eq!(
            "keyring".parse::<AuthStorage>().unwrap(),
            AuthStorage::Keyring
        );
        assert_eq!(
            "KEYRING".parse::<AuthStorage>().unwrap(),
            AuthStorage::Keyring
        );
        assert!("plaintext".parse::<AuthStorage>().is_err());

        assert_eq!(AuthStorage::File.to_string(), "file");
        assert_eq!(AuthStorage::Keyring.to_string(), "keyring");
    }

    #[test]
    fn auth_default_storage_is_file() {
        // Backward compat: existing on-disk auth files without a `storage`
        // key must continue to load as file-backed.
        use super::AuthStorage;
        assert_eq!(AuthStorage::default(), AuthStorage::File);
    }

    #[test]
    fn auth_from_file_defaults_storage_to_file() {
        // Files written by previous app versions don't declare a `storage`
        // key. They should keep loading exactly as before.
        let path = tmp_path("legacy_no_storage_key");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            "username = \"bob\"\npassword = \"pw\"\nsession = \"\"\n",
        )
        .expect("seed write should succeed");

        let parsed = Auth::from_file(&path).expect("read should succeed");
        assert_eq!(parsed.username, "bob");
        assert_eq!(parsed.password, "pw");
        assert_eq!(parsed.session, None);
        assert_eq!(parsed.storage, super::AuthStorage::File);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auth_keyring_pointer_file_round_trip() {
        // We can write the keyring pointer body and parse it back without
        // touching the keyring — we just verify the file's `storage` and
        // `username` are recognized. Reading credentials from the keyring
        // is exercised by manual integration testing (see README).
        use super::AuthStorage;
        let path = tmp_path("keyring_pointer_format");
        let _ = std::fs::remove_file(&path);

        let body = "storage = \"keyring\"\nusername = \"alice\"\n";
        std::fs::write(&path, body).expect("seed write should succeed");

        let repr_str = std::fs::read_to_string(&path).expect("read should succeed");
        let repr: super::AuthFileRepr =
            toml::from_str(&repr_str).expect("pointer file should parse");
        let storage: AuthStorage = repr.storage.as_deref().unwrap_or("file").parse().unwrap();
        assert_eq!(storage, AuthStorage::Keyring);
        assert_eq!(repr.username, "alice");
        assert!(repr.password.is_none());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn backport_noops_for_keyring_storage_auth() {
        // The session cookie for keyring-backed auth lives in the OS
        // keyring, not the file, so the file legitimately has no `session`
        // line. Backport must leave the pointer file alone.
        use super::AuthStorage;
        let path = tmp_path("backport_keyring_noop");
        let _ = std::fs::remove_file(&path);

        let pointer_body = "storage = \"keyring\"\nusername = \"alice\"\n";
        std::fs::write(&path, pointer_body).expect("seed write should succeed");

        let auth = super::Auth {
            username: "alice".to_string(),
            password: "ignored".to_string(),
            session: None,
            storage: AuthStorage::Keyring,
        };
        let rewrote = super::backport_auth_file(&path, &auth).expect("backport should succeed");
        assert!(
            !rewrote,
            "keyring-backed auth files should not be backported"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), pointer_body);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn auth_keyring_write_emits_pointer_with_guidance() {
        // Even though we can't easily exercise the keyring side under
        // `cargo test`, we can validate the pointer body is written with
        // the documented `storage`/`username`-only schema and a comment
        // that points the user at the migration command.
        let body = super::Auth {
            username: "carol".to_string(),
            password: "pw".to_string(),
            session: None,
            storage: super::AuthStorage::Keyring,
        }
        .to_annotated_keyring_toml();

        assert!(
            body.contains("storage = \"keyring\""),
            "missing storage marker:\n{body}"
        );
        assert!(
            body.contains("username = \"carol\""),
            "missing username:\n{body}"
        );
        assert!(
            !body.contains("password ="),
            "password leaked into pointer file:\n{body}"
        );
        assert!(
            !body.contains("session ="),
            "session cookie leaked into pointer file:\n{body}"
        );
        assert!(
            body.contains("--migrate-auth file"),
            "missing migration hint:\n{body}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn auth_write_sets_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let path = tmp_path("perms");
        let _ = std::fs::remove_file(&path);

        Auth {
            username: "carol".to_string(),
            password: "pw".to_string(),
            session: None,
            storage: AuthStorage::File,
        }
        .write_to_file(&path)
        .expect("write should succeed");

        let mode = std::fs::metadata(&path)
            .expect("stat should succeed")
            .permissions()
            .mode();
        // Only compare the low 9 bits (rwx for u/g/o); the file-type bits above
        // are platform-defined and not what we're asserting on.
        assert_eq!(mode & 0o777, 0o600, "expected 0600, got {:o}", mode & 0o777);

        std::fs::remove_file(&path).ok();
    }
}
