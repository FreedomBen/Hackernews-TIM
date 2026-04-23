// modules
pub mod client;
pub mod config;
pub mod model;
pub mod parser;
pub mod prelude;
pub mod utils;
pub mod view;

const DEFAULT_CONFIG_FILE: &str = "hn-tui.toml";
const DEFAULT_AUTH_FILE: &str = "hn-auth.toml";
const DEFAULT_LOG_FILE: &str = "hn-tui.log";

use clap::*;
use prelude::*;

fn run(
    client: &'static client::HNClient,
    start_id: Option<u32>,
    auth_file: std::path::PathBuf,
    login_status: client::StartupLoginStatus,
) {
    // setup the application's UI
    let s = view::init_ui(client, start_id, auth_file, login_status);

    // use `cursive_buffered_backend` crate to fix the flickering issue
    // when using `cursive` with `crossterm_backend` (See https://github.com/gyscos/Cursive/issues/142)
    let crossterm_backend = backends::crossterm::Backend::init().unwrap();
    let buffered_backend = Box::new(cursive_buffered_backend::BufferedBackend::new(
        crossterm_backend,
    ));
    let mut app = CursiveRunner::new(s, buffered_backend);

    app.run();
}

/// initialize application logging
fn init_logging(log_dir_str: &str) {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "hackernews_tui=info")
    }

    let log_dir = std::path::PathBuf::from(log_dir_str);
    if !log_dir.exists() {
        std::fs::create_dir_all(&log_dir)
            .unwrap_or_else(|_| panic!("{}", "failed to create a log folder: {log_dir_str}"));
    }

    let log_file = std::fs::File::create(log_dir.join(DEFAULT_LOG_FILE)).unwrap_or_else(|err| {
        panic!("failed to create application's log file: {err}");
    });

    tracing_subscriber::fmt::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(log_file))
        .init();
}

/// parse command line arguments
fn parse_args(config_dir: std::path::PathBuf, cache_dir: std::path::PathBuf) -> ArgMatches {
    Command::new("hackernews-tui")
        .version(env!("CARGO_PKG_VERSION"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .arg(
            Arg::new("auth")
                .short('a')
                .long("auth")
                .value_name("FILE")
                .default_value(config_dir.join(DEFAULT_AUTH_FILE).into_os_string())
                .help("Path to the application's authentication file")
                .next_line_help(true),
        )
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("FILE")
                .default_value(config_dir.join(DEFAULT_CONFIG_FILE).into_os_string())
                .help("Path to the application's config file")
                .next_line_help(true),
        )
        .arg(
            Arg::new("log")
                .short('l')
                .long("log")
                .value_name("FOLDER")
                .default_value(cache_dir.into_os_string())
                .help("Path to a folder to store application's logs")
                .next_line_help(true),
        )
        .arg(
            Arg::new("start_id")
                .short('i')
                .value_parser(clap::value_parser!(u32))
                .help("The Hacker News item's id to start the application with")
                .next_line_help(true),
        )
        .arg(
            Arg::new("init_config")
                .long("init-config")
                .value_name("THEME")
                .value_parser(["light", "dark"])
                .help("Write a default config file to the --config path then exit (light or dark)")
                .next_line_help(true),
        )
        .arg(
            Arg::new("update_theme")
                .long("update-theme")
                .value_name("THEME")
                .value_parser(["light", "dark"])
                .conflicts_with("init_config")
                .help(
                    "Replace the [theme] section of the existing --config file with the latest \
                     default (light or dark) then exit. Other sections and surrounding comments \
                     are preserved.",
                )
                .next_line_help(true),
        )
        .get_matches()
}

fn init_app_dirs() -> (std::path::PathBuf, std::path::PathBuf) {
    let mut config_dir = dirs_next::config_dir().expect("failed to get user's config dir");
    let cache_dir = dirs_next::cache_dir().expect("failed to get user's cache dir");
    let home_dir = dirs_next::home_dir().expect("failed to get user's home dir");

    // Try to find application's config file in the user's config dir.
    // If not found, fallback to use `$HOME/.config` (for backward compability reason)
    if !config_dir.join(DEFAULT_CONFIG_FILE).exists() {
        config_dir = home_dir.join(".config");
    }

    (config_dir, cache_dir)
}

fn init_auth(auth_path: &std::path::Path) -> Option<config::Auth> {
    if !auth_path.exists() {
        match config::prompt_for_auth() {
            None | Some(config::AuthPromptResult::Skip) => return None,
            Some(config::AuthPromptResult::Credentials { username, password }) => {
                let session = match client::verify_credentials(&username, &password) {
                    Ok(session) => session,
                    Err(err) => {
                        eprintln!("Login failed: {err:#}. Starting without auth.");
                        return None;
                    }
                };
                let auth = config::Auth {
                    username,
                    password,
                    session,
                };
                if let Err(err) = auth.write_to_file(auth_path) {
                    eprintln!("Failed to write auth to {}: {err:#}", auth_path.display());
                    return None;
                }
                println!("Wrote auth to {}", auth_path.display());
                return Some(auth);
            }
        }
    }

    match config::Auth::from_file(auth_path) {
        Ok(auth) => {
            match config::backport_auth_file(auth_path, &auth) {
                Ok(true) => tracing::info!(
                    "Upgraded {} to the current auth-file format (added session line)",
                    auth_path.display()
                ),
                Ok(false) => {}
                Err(err) => tracing::warn!(
                    "Failed to upgrade auth file {} in place: {err:#}",
                    auth_path.display()
                ),
            }
            Some(auth)
        }
        Err(err) => {
            tracing::warn!(
                "Failed to get authentication from {}: {err}",
                auth_path.display()
            );
            None
        }
    }
}

/// Persist `auth` back to `auth_path`, logging (but not aborting on) failure.
/// Called whenever the startup flow refreshes the cached session cookie so
/// the next run can skip the `/login` POST.
fn save_auth(auth_path: &std::path::Path, auth: &config::Auth) {
    if let Err(err) = auth.write_to_file(auth_path) {
        tracing::warn!(
            "Failed to persist updated session to {}: {err:#}",
            auth_path.display()
        );
    }
}

/// Build the HN client and establish an authenticated session, preferring
/// the cached session cookie over a password POST to `/login`.
///
/// Flow:
/// 1. No credentials → return an anonymous client.
/// 2. Cached session present → build a client seeded with it and verify
///    against HN. If still valid, done. If the verification call doesn't
///    come back cleanly (stale cookie, HN hiccup), fall through.
/// 3. Fall back to password login on a fresh client. On success, persist
///    the fresh session cookie so the next run can skip this whole dance.
fn build_client_and_log_in(
    config: &config::Config,
    auth: &Option<config::Auth>,
    auth_path: &std::path::Path,
) -> (client::HNClient, bool, client::StartupLoginStatus) {
    let Some(auth) = auth else {
        let client = client::HNClient::with_timeout(config.client_timeout)
            .expect("failed to build HN client");
        return (client, false, client::StartupLoginStatus::NotAttempted);
    };

    if let Some(session) = auth.session.as_deref().filter(|s| !s.is_empty()) {
        match client::HNClient::with_cached_session(config.client_timeout, session) {
            Ok(client) if client.verify_session() => {
                tracing::info!(
                    "Restored HN session for {} from cached cookie",
                    auth.username
                );
                return (client, true, client::StartupLoginStatus::NotAttempted);
            }
            Ok(_) => {
                tracing::info!(
                    "Cached HN session for {} is stale; falling back to password login",
                    auth.username
                );
            }
            Err(err) => tracing::warn!("failed to build HN client with cached session: {err}"),
        }
    }

    let client =
        client::HNClient::with_timeout(config.client_timeout).expect("failed to build HN client");
    match client.login(&auth.username, &auth.password) {
        Ok(()) => {
            let updated = config::Auth {
                session: client.current_session_cookie(),
                ..auth.clone()
            };
            if updated.session != auth.session {
                save_auth(auth_path, &updated);
            }
            (
                client,
                true,
                client::StartupLoginStatus::Success {
                    username: auth.username.clone(),
                },
            )
        }
        Err(err) => {
            tracing::warn!("Failed to login, user={}: {err}", auth.username);
            let status = client::StartupLoginStatus::from_login_error(&err);
            (client, false, status)
        }
    }
}

fn main() {
    let (config_dir, cache_dir) = init_app_dirs();
    let args = parse_args(config_dir, cache_dir);

    init_logging(
        args.get_one::<String>("log")
            .expect("`log` argument should have a default value"),
    );

    let config_file_str = args
        .get_one::<String>("config")
        .expect("`config` argument should have a default value");
    let config_path = std::path::Path::new(config_file_str);

    if let Some(theme) = args.get_one::<String>("init_config") {
        let flavor: config::ConfigFlavor = theme
            .parse()
            .expect("clap value_parser restricts this to 'light' or 'dark'");
        match config::write_default_config(config_path, flavor) {
            Ok(()) => {
                println!("Wrote default {theme} config to {}", config_path.display());
                std::process::exit(0);
            }
            Err(err) => {
                eprintln!(
                    "Failed to write config to {}: {err:#}",
                    config_path.display()
                );
                std::process::exit(1);
            }
        }
    }

    if let Some(theme) = args.get_one::<String>("update_theme") {
        let flavor: config::ConfigFlavor = theme
            .parse()
            .expect("clap value_parser restricts this to 'light' or 'dark'");
        match config::update_theme_in_place(config_path, flavor) {
            Ok(()) => {
                println!("Updated {theme} theme in {}", config_path.display());
                std::process::exit(0);
            }
            Err(err) => {
                eprintln!(
                    "Failed to update theme in {}: {err:#}",
                    config_path.display()
                );
                std::process::exit(1);
            }
        }
    }

    if !config_path.exists() {
        if let Some(flavor) = config::prompt_for_flavor() {
            match config::write_default_config(config_path, flavor) {
                Ok(()) => println!("Wrote config to {}", config_path.display()),
                Err(err) => eprintln!(
                    "Failed to write config to {}: {err:#}",
                    config_path.display()
                ),
            }
        }
    }

    // Parse the config as a value so we can still mutate it (e.g. apply the
    // user's HN `topcolor`) before sealing it into the global.
    let mut config = config::load_config_file(config_file_str);

    let auth_file_str = args
        .get_one::<String>("auth")
        .expect("`auth` argument should have a default value");
    let auth_path = std::path::PathBuf::from(auth_file_str);
    let auth = init_auth(&auth_path);

    // Build the HN client early so we can log in and (optionally) fetch the
    // user's `topcolor` before the theme is sealed. Same client instance is
    // then installed as the global so the session cookies carry over.
    //
    // Prefer a cached session cookie if the auth file has one: HN throttles
    // repeated `/login` POSTs from the same IP with a CAPTCHA, and the TUI
    // can't solve it. We only fall back to the password when no cached
    // session exists or the cached one has expired.
    let (hn_client, logged_in, login_status) = build_client_and_log_in(&config, &auth, &auth_path);

    let mut user_info: Option<client::UserInfo> = None;
    if let Some(auth) = &auth {
        if logged_in {
            let profile = hn_client.fetch_profile_info(&auth.username);
            if config.use_hn_topcolor {
                if let Some(hex) = profile.topcolor.as_deref() {
                    if config.theme.apply_hn_topcolor(hex) {
                        tracing::info!("Applied HN topcolor override: #{hex}");
                    }
                }
            }
            user_info = Some(client::UserInfo {
                username: auth.username.clone(),
                karma: profile.karma,
            });
        }
    }

    config::init_config(config);
    client::init_user_info(user_info);
    let client = client::install_client(hn_client);

    let start_id = args.get_one::<u32>("start_id").cloned();
    run(client, start_id, auth_path, login_status);
}
