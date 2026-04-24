// modules
mod init;
mod keybindings;
mod theme;

// re-export
pub use init::*;
pub use keybindings::*;
pub use theme::*;

use config_parser2::*;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize, Serialize, Clone)]
/// HackerNews user's authentication data
pub struct Auth {
    pub username: String,
    pub password: String,
    /// Cached HN session cookie value (the `user=` cookie). When present, the
    /// app uses it to restore a logged-in session instead of POSTing to
    /// `/login` on every startup — important because HN throttles repeated
    /// `/login` attempts from the same IP with a CAPTCHA challenge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
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
    /// parse auth from a file
    pub fn from_file<P>(file: P) -> anyhow::Result<Self>
    where
        P: AsRef<std::path::Path>,
    {
        let auth_str = std::fs::read_to_string(file)?;
        let mut auth = toml::from_str::<Self>(&auth_str)?;
        // Treat the hand-editable placeholder (`session = ""`) the same as a
        // missing field so downstream code only has to match on the `Some`
        // case to mean "we have a cached cookie to try".
        if auth.session.as_deref() == Some("") {
            auth.session = None;
        }
        Ok(auth)
    }

    /// Serialize auth to TOML and write it to `file`, creating any missing
    /// parent directories. On Unix the file is chmod'd to `0600` so other
    /// local users can't read the credentials.
    ///
    /// The `session` key is always emitted (with an empty string when no
    /// cookie is cached yet) alongside a comment block explaining how to
    /// paste a browser-side cookie in by hand. This matters because HN
    /// serves a CAPTCHA after repeated `/login` attempts, and the TUI
    /// can't solve it — so users who get stuck need a clear way to
    /// bootstrap the session from a browser login.
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
        std::fs::write(path, self.to_annotated_toml())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn to_annotated_toml(&self) -> String {
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
pub fn backport_auth_file(path: &std::path::Path, auth: &Auth) -> anyhow::Result<bool> {
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
    use super::Auth;

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
        }
        .write_to_file(&path)
        .expect("write should succeed");
        assert!(path.exists());

        std::fs::remove_dir_all(&dir).ok();
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
