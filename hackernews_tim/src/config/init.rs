use std::io::{self, IsTerminal, Write};
use std::path::Path;

use super::AuthStorage;

const LIGHT_CONFIG: &str = include_str!("../../../examples/config.toml");
const DARK_CONFIG: &str = include_str!("../../../examples/config-dark.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFlavor {
    Light,
    Dark,
}

impl ConfigFlavor {
    pub fn contents(self) -> &'static str {
        match self {
            ConfigFlavor::Light => LIGHT_CONFIG,
            ConfigFlavor::Dark => DARK_CONFIG,
        }
    }
}

impl std::str::FromStr for ConfigFlavor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "light" => Ok(ConfigFlavor::Light),
            "dark" => Ok(ConfigFlavor::Dark),
            other => Err(format!(
                "unknown config flavor '{other}' (expected 'light' or 'dark')"
            )),
        }
    }
}

pub fn write_default_config(path: &Path, flavor: ConfigFlavor) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, flavor.contents())?;
    Ok(())
}

/// Replace the `[theme]` table in the user's config with the one from the
/// embedded default for `flavor`, preserving all other sections and any
/// comments outside the theme table.
///
/// Errors if `path` does not exist — callers should direct the user to
/// `--init-config` in that case.
pub fn update_theme_in_place(path: &Path, flavor: ConfigFlavor) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!(
            "config file {} does not exist; run with --init-config <light|dark> to create one first",
            path.display()
        );
    }

    let existing_str = std::fs::read_to_string(path)?;
    let mut existing: toml_edit::DocumentMut = existing_str
        .parse()
        .map_err(|err| anyhow::anyhow!("failed to parse existing config: {err}"))?;

    let default: toml_edit::DocumentMut = flavor
        .contents()
        .parse()
        .map_err(|err| anyhow::anyhow!("failed to parse embedded default config: {err}"))?;

    let default_theme = default
        .get("theme")
        .ok_or_else(|| anyhow::anyhow!("embedded default is missing the [theme] table"))?
        .clone();

    existing["theme"] = default_theme;

    std::fs::write(path, existing.to_string())?;
    Ok(())
}

/// One-time migration helper. If `target` doesn't exist but any of
/// `sources` does, copy the first existing source into `target`,
/// creating `target`'s parent directory if needed. Prints a one-line
/// user-visible note on success so the user knows the original file is
/// still on disk and can be removed manually.
///
/// Failures are logged and swallowed — the caller continues as if the
/// migration had not happened, matching the rest of the startup flow
/// (config parse failures, auth failures, etc. are also best-effort).
///
/// `std::fs::copy` preserves the source file's permission bits, so a
/// legacy `hn-auth.toml` stays at mode `0600` after the copy.
pub fn migrate_legacy_file(target: &Path, sources: &[std::path::PathBuf]) {
    if target.exists() {
        return;
    }
    let Some(source) = sources.iter().find(|p| p.as_path() != target && p.exists()) else {
        return;
    };
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    "Failed to create {} while migrating {}: {err}",
                    parent.display(),
                    source.display()
                );
                return;
            }
        }
    }
    match std::fs::copy(source, target) {
        Ok(_) => {
            eprintln!(
                "Copied legacy {} to {} (original kept).",
                source.display(),
                target.display(),
            );
        }
        Err(err) => tracing::warn!(
            "Failed to copy {} to {}: {err}",
            source.display(),
            target.display()
        ),
    }
}

/// Outcome of [`prompt_for_auth`].
pub enum AuthPromptResult {
    /// The user declined to log in. No file should be written.
    Skip,
    /// The user entered credentials — still need to verify + write.
    Credentials {
        username: String,
        password: String,
        storage: AuthStorage,
    },
}

/// Interactively ask the user whether to log in to Hacker News, and if so
/// collect a credential-storage choice plus username + password (password
/// input is masked).
///
/// Returns `None` when stdin/stdout is not a TTY or the interaction fails,
/// so the caller can silently fall back to "no auth".
pub fn prompt_for_auth() -> Option<AuthPromptResult> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return None;
    }

    loop {
        print!("No auth file found. Log in to Hacker News now? [y/N]: ");
        if io::stdout().flush().is_err() {
            return None;
        }

        let mut buf = String::new();
        if io::stdin().read_line(&mut buf).is_err() {
            return None;
        }

        match buf.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => break,
            "n" | "no" | "" => return Some(AuthPromptResult::Skip),
            _ => eprintln!("Please enter 'y' or 'n'."),
        }
    }

    let storage = prompt_for_auth_storage()?;

    print!("Username: ");
    io::stdout().flush().ok()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username).ok()?;
    let username = username.trim().to_string();
    if username.is_empty() {
        eprintln!("Empty username, skipping login.");
        return Some(AuthPromptResult::Skip);
    }

    let password = match rpassword::prompt_password("Password: ") {
        Ok(p) => p,
        Err(err) => {
            eprintln!("Failed to read password: {err}. Skipping login.");
            return Some(AuthPromptResult::Skip);
        }
    };
    if password.is_empty() {
        eprintln!("Empty password, skipping login.");
        return Some(AuthPromptResult::Skip);
    }

    Some(AuthPromptResult::Credentials {
        username,
        password,
        storage,
    })
}

/// Ask where to store the upcoming credentials. Defaults to file storage
/// (capital `F` in the prompt) so users who want the legacy plaintext
/// behavior can just hit enter. When the user picks `keyring`, the OS
/// credential manager is probed up front; on failure the user is offered
/// a one-shot fall back to file storage rather than committing to a
/// backend that won't work.
fn prompt_for_auth_storage() -> Option<AuthStorage> {
    println!("Where should credentials be stored?");
    println!("  [F]ile    plaintext TOML on disk (mode 0600 on Unix)");
    println!("  [k]eyring OS credential manager (Keychain / Credential Manager / Secret Service)");

    let storage = loop {
        print!("Storage [F/k]: ");
        if io::stdout().flush().is_err() {
            return None;
        }

        let mut buf = String::new();
        if io::stdin().read_line(&mut buf).is_err() {
            return None;
        }

        match buf.trim().to_ascii_lowercase().as_str() {
            "f" | "file" | "" => break AuthStorage::File,
            "k" | "keyring" => break AuthStorage::Keyring,
            _ => eprintln!("Please enter 'f' or 'k'."),
        }
    };

    if storage == AuthStorage::Keyring && !super::keyring_available() {
        eprintln!(
            "OS keyring isn't usable on this system (probe failed — likely no Secret \
             Service available, or Keychain access denied)."
        );
        loop {
            print!("Fall back to file storage? [Y/n]: ");
            if io::stdout().flush().is_err() {
                return None;
            }
            let mut buf = String::new();
            if io::stdin().read_line(&mut buf).is_err() {
                return None;
            }
            match buf.trim().to_ascii_lowercase().as_str() {
                "y" | "yes" | "" => return Some(AuthStorage::File),
                "n" | "no" => return None,
                _ => eprintln!("Please enter 'y' or 'n'."),
            }
        }
    }

    Some(storage)
}

/// Interactively ask the user which default config flavor to write.
///
/// Returns `None` if either stdin or stdout is not a TTY, or if the user skips.
pub fn prompt_for_flavor() -> Option<ConfigFlavor> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return None;
    }

    loop {
        print!("No config file found. Create one now? [l]ight / [d]ark / [s]kip: ");
        if io::stdout().flush().is_err() {
            return None;
        }

        let mut buf = String::new();
        if io::stdin().read_line(&mut buf).is_err() {
            return None;
        }

        match buf.trim().to_ascii_lowercase().as_str() {
            "l" | "light" => return Some(ConfigFlavor::Light),
            "d" | "dark" => return Some(ConfigFlavor::Dark),
            "s" | "skip" | "" => return None,
            _ => eprintln!("Please enter 'l', 'd', or 's'."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flavor_from_str_accepts_both_cases() {
        assert_eq!(
            "light".parse::<ConfigFlavor>().unwrap(),
            ConfigFlavor::Light
        );
        assert_eq!(
            "LIGHT".parse::<ConfigFlavor>().unwrap(),
            ConfigFlavor::Light
        );
        assert_eq!("Dark".parse::<ConfigFlavor>().unwrap(), ConfigFlavor::Dark);
        assert!("midnight".parse::<ConfigFlavor>().is_err());
    }

    #[test]
    fn embedded_configs_are_nonempty_and_distinct() {
        assert!(!LIGHT_CONFIG.is_empty());
        assert!(!DARK_CONFIG.is_empty());
        assert_ne!(LIGHT_CONFIG, DARK_CONFIG);
    }

    #[test]
    fn embedded_configs_declare_faded_component_style() {
        assert!(
            LIGHT_CONFIG.contains("faded = "),
            "light config should declare a `faded` component style entry"
        );
        assert!(
            DARK_CONFIG.contains("faded = "),
            "dark config should declare a `faded` component style entry"
        );
    }

    #[test]
    fn write_default_config_creates_parent_dirs() {
        let tmp =
            std::env::temp_dir().join(format!("hackernews_tim_init_test_{}", std::process::id()));
        let path = tmp.join("nested").join("config.toml");
        let _ = std::fs::remove_dir_all(&tmp);

        write_default_config(&path, ConfigFlavor::Dark).expect("write should succeed");
        let contents = std::fs::read_to_string(&path).expect("file should exist");
        assert_eq!(contents, DARK_CONFIG);

        std::fs::remove_dir_all(&tmp).ok();
    }

    fn update_theme_tmp(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hackernews_tim_update_theme_test_{}_{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn update_theme_errors_when_config_missing() {
        let path = update_theme_tmp("missing");
        let _ = std::fs::remove_file(&path);

        let err = update_theme_in_place(&path, ConfigFlavor::Dark)
            .expect_err("should error when config file does not exist");
        let msg = format!("{err:#}");
        assert!(msg.contains("does not exist"), "unexpected error: {msg}");
        assert!(msg.contains("--init-config"), "unexpected error: {msg}");
    }

    #[test]
    fn update_theme_replaces_theme_and_preserves_other_sections() {
        let path = update_theme_tmp("replaces");
        let _ = std::fs::remove_file(&path);

        let original = r##"# user comment above general
use_page_scrolling = false
client_timeout = 99

[theme.palette]
background = "#123456"
foreground = "#abcdef"

[keymap.global_keymap]
quit = "Q"
"##;
        std::fs::write(&path, original).unwrap();

        update_theme_in_place(&path, ConfigFlavor::Dark).expect("update should succeed");

        let updated = std::fs::read_to_string(&path).unwrap();

        // Theme values now come from the dark default.
        assert!(
            updated.contains(r##"background = "#1d1f21""##),
            "expected dark background, got:\n{updated}"
        );
        // Old theme values are gone.
        assert!(
            !updated.contains("#123456"),
            "old palette leaked through:\n{updated}"
        );
        // Non-theme sections and top-level comment are preserved.
        assert!(updated.contains("# user comment above general"));
        assert!(updated.contains("use_page_scrolling = false"));
        assert!(updated.contains("client_timeout = 99"));
        assert!(updated.contains(r##"quit = "Q""##));

        std::fs::remove_file(&path).ok();
    }

    fn migration_tmp(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hackernews_tim_migrate_test_{}_{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn migrate_legacy_file_noops_when_target_exists() {
        let tmp = migration_tmp("target_exists");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let target = tmp.join("config.toml");
        std::fs::write(&target, "target content").unwrap();

        let legacy_dir = tmp.join("legacy");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        let source = legacy_dir.join("config.toml");
        std::fs::write(&source, "legacy content").unwrap();

        migrate_legacy_file(&target, &[source]);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "target content");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn migrate_legacy_file_copies_when_target_missing() {
        let tmp = migration_tmp("copies");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let target = tmp.join("subdir").join("config.toml");
        let source = tmp.join("config.toml");
        std::fs::write(&source, "legacy content").unwrap();

        assert!(
            !target.exists(),
            "precondition: target should not exist yet"
        );
        migrate_legacy_file(&target, std::slice::from_ref(&source));

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "legacy content");
        assert!(source.exists(), "source should still exist after copy");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn migrate_legacy_file_uses_first_existing_source() {
        let tmp = migration_tmp("first_existing");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let target = tmp.join("subdir").join("config.toml");
        let missing = tmp.join("a").join("config.toml");
        let present = tmp.join("b").join("config.toml");
        std::fs::create_dir_all(present.parent().unwrap()).unwrap();
        std::fs::write(&present, "from second").unwrap();

        migrate_legacy_file(&target, &[missing, present]);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "from second");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn migrate_legacy_file_noops_when_no_source_exists() {
        let tmp = migration_tmp("no_source");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let target = tmp.join("subdir").join("config.toml");
        let missing = tmp.join("nonexistent.toml");

        migrate_legacy_file(&target, &[missing]);
        assert!(!target.exists());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn update_theme_adds_theme_when_missing() {
        let path = update_theme_tmp("adds");
        let _ = std::fs::remove_file(&path);

        let original = "client_timeout = 10\n";
        std::fs::write(&path, original).unwrap();

        update_theme_in_place(&path, ConfigFlavor::Light).expect("update should succeed");

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("client_timeout = 10"));
        assert!(
            updated.contains("[theme.palette]") || updated.contains("palette"),
            "expected theme section to be added:\n{updated}"
        );

        std::fs::remove_file(&path).ok();
    }
}
