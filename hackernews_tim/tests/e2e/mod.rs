//! Phase 3 PTY end-to-end harness — Linux-only.
//!
//! This file is intentionally **not** at the top of `tests/`, so Cargo
//! does not treat it as its own integration test crate. Each
//! `tests/e2e_*.rs` binary pulls these helpers in via:
//!
//! ```ignore
//! #![cfg(target_os = "linux")]
//!
//! #[path = "e2e/mod.rs"]
//! mod helpers;
//! ```
//!
//! See TEST_PLAN.md §3.1.1 for the surrounding plan.

#![cfg(target_os = "linux")]
#![allow(dead_code)] // Helpers are shared across many e2e binaries; not all are used by every one.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{
    Child, CommandBuilder, ExitStatus, MasterPty, NativePtySystem, PtySize, PtySystem,
};

/// Default visible terminal size for spawned binaries. Big enough that
/// the front-page story view fits without wrapping; small enough that
/// snapshots stay reviewable.
pub const SCREEN_COLS: u16 = 120;
pub const SCREEN_ROWS: u16 = 40;

/// Default timeout for [`AppHandle::wait_for_text`].
pub const DEFAULT_WAIT: Duration = Duration::from_secs(5);

/// Default key sent by [`AppHandle::shutdown`] when the caller hasn't
/// configured a different quit sequence.
pub const DEFAULT_QUIT_KEY: &str = "q";

/// Placeholder URL handed to `HN_ALGOLIA_BASE` / `HN_FIREBASE_BASE`
/// when no fake backend is configured. Picks an unrouteable port so a
/// stray real request fails loudly instead of escaping to production.
const DEFAULT_BLACKHOLE_BASE: &str = "http://127.0.0.1:1";

/// Path to the freshly-built debug binary. Cargo sets
/// `CARGO_BIN_EXE_<name>` for every binary target in the package while
/// building integration tests, so this works without a separate build
/// step.
pub fn binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hackernews_tim"))
}

/// Owns the temp directories handed to a spawned binary. Dropping
/// `TestDirs` removes them.
pub struct TestDirs {
    _temp: tempfile::TempDir,
    pub home: PathBuf,
    pub xdg_config_home: PathBuf,
    pub xdg_data_home: PathBuf,
    pub log_dir: PathBuf,
}

impl TestDirs {
    pub fn new() -> std::io::Result<Self> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().to_path_buf();
        let home = root.join("home");
        let xdg_config_home = home.join(".config");
        let xdg_data_home = home.join(".local").join("share");
        let log_dir = root.join("logs");
        std::fs::create_dir_all(&xdg_config_home)?;
        std::fs::create_dir_all(&xdg_data_home)?;
        std::fs::create_dir_all(&log_dir)?;
        Ok(Self {
            _temp: temp,
            home,
            xdg_config_home,
            xdg_data_home,
            log_dir,
        })
    }
}

/// Inputs to [`spawn_app`].
pub struct SpawnOptions {
    pub args: Vec<OsString>,
    pub env: HashMap<OsString, OsString>,
    pub cwd: Option<PathBuf>,
    pub size: PtySize,
    pub algolia_base: Option<String>,
    pub firebase_base: Option<String>,
    pub dirs: Option<TestDirs>,
}

impl SpawnOptions {
    pub fn new() -> Self {
        Self {
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            size: PtySize {
                rows: SCREEN_ROWS,
                cols: SCREEN_COLS,
                pixel_width: 0,
                pixel_height: 0,
            },
            algolia_base: None,
            firebase_base: None,
            dirs: None,
        }
    }

    pub fn arg<S: Into<OsString>>(mut self, a: S) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn env<K, V>(mut self, k: K, v: V) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env.insert(k.into(), v.into());
        self
    }

    pub fn algolia_base(mut self, base: impl Into<String>) -> Self {
        self.algolia_base = Some(base.into());
        self
    }

    pub fn firebase_base(mut self, base: impl Into<String>) -> Self {
        self.firebase_base = Some(base.into());
        self
    }

    pub fn dirs(mut self, dirs: TestDirs) -> Self {
        self.dirs = Some(dirs);
        self
    }

    pub fn size(mut self, rows: u16, cols: u16) -> Self {
        self.size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        self
    }
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Handle to a binary running inside a PTY, with a background thread
/// feeding the PTY's master-end output into a `vt100::Parser`.
pub struct AppHandle {
    _master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    reader_handle: Option<thread::JoinHandle<()>>,
    writer: Box<dyn Write + Send>,
    pub dirs: TestDirs,
    closed: bool,
}

impl AppHandle {
    /// Write UTF-8 / control sequences to the PTY's master end.
    pub fn send_keys(&mut self, keys: &str) -> std::io::Result<()> {
        if !keys.is_empty() {
            self.writer.write_all(keys.as_bytes())?;
            self.writer.flush()?;
        }
        Ok(())
    }

    /// Snapshot the visible terminal as a string. Trailing whitespace
    /// and trailing blank lines are stripped to keep `insta` snapshots
    /// stable.
    pub fn screen(&self) -> String {
        let parser = self.parser.lock().expect("vt100 parser mutex poisoned");
        screen_text(parser.screen())
    }

    /// Poll [`screen`] every 50 ms until `needle` appears or `timeout`
    /// elapses. On timeout, the error includes the last screen for
    /// post-mortem.
    pub fn wait_for_text(&self, needle: &str, timeout: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if self.screen().contains(needle) {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "timed out after {:?} waiting for {needle:?}\n--- screen ---\n{}",
                    timeout,
                    self.screen()
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Send the default quit key (`q`), wait for exit, return status.
    pub fn shutdown(&mut self) -> Result<ExitStatus, String> {
        self.shutdown_with(DEFAULT_QUIT_KEY)
    }

    /// Send `keys`, then wait for exit. Subsequent calls send nothing
    /// but still wait for exit.
    pub fn shutdown_with(&mut self, keys: &str) -> Result<ExitStatus, String> {
        if !self.closed {
            let _ = self.send_keys(keys);
            self.closed = true;
        }
        self.wait_for_exit(Duration::from_secs(5))
    }

    /// Block until the child process exits or `timeout` fires. On
    /// success, the reader thread is joined so `screen()` reflects all
    /// output the child emitted.
    pub fn wait_for_exit(&mut self, timeout: Duration) -> Result<ExitStatus, String> {
        let start = Instant::now();
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    if let Some(h) = self.reader_handle.take() {
                        let _ = h.join();
                    }
                    return Ok(status);
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        return Err(format!(
                            "process did not exit within {:?}\n--- screen ---\n{}",
                            timeout,
                            self.screen()
                        ));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("try_wait failed: {e}")),
            }
        }
    }
}

impl Drop for AppHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.reader_handle.take() {
            let _ = h.join();
        }
    }
}

fn screen_text(screen: &vt100::Screen) -> String {
    let (rows, cols) = screen.size();
    let mut lines: Vec<String> = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut line = String::with_capacity(cols as usize);
        for c in 0..cols {
            if let Some(cell) = screen.cell(r, c) {
                line.push_str(&cell.contents());
            }
        }
        lines.push(line.trim_end().to_string());
    }
    while matches!(lines.last(), Some(l) if l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Spawn the `hackernews_tim` debug binary in a PTY, applying the
/// always-set isolation env vars (`HOME`, `XDG_CONFIG_HOME`,
/// `XDG_DATA_HOME`, `HN_ALGOLIA_BASE`, `HN_FIREBASE_BASE`) plus a
/// `-l <log_dir>` arg. Returns an [`AppHandle`] whose reader thread is
/// already streaming output into a `vt100::Parser`.
pub fn spawn_app(mut opts: SpawnOptions) -> Result<AppHandle, String> {
    let dirs = match opts.dirs.take() {
        Some(d) => d,
        None => TestDirs::new().map_err(|e| format!("TestDirs::new failed: {e}"))?,
    };

    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(opts.size)
        .map_err(|e| format!("openpty failed: {e}"))?;

    let mut cmd = CommandBuilder::new(binary_path());
    cmd.env_clear();
    if let Ok(path) = std::env::var("PATH") {
        cmd.env("PATH", path);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("HOME", &dirs.home);
    cmd.env("XDG_CONFIG_HOME", &dirs.xdg_config_home);
    cmd.env("XDG_DATA_HOME", &dirs.xdg_data_home);
    cmd.env(
        "HN_ALGOLIA_BASE",
        opts.algolia_base
            .clone()
            .unwrap_or_else(|| DEFAULT_BLACKHOLE_BASE.into()),
    );
    cmd.env(
        "HN_FIREBASE_BASE",
        opts.firebase_base
            .clone()
            .unwrap_or_else(|| DEFAULT_BLACKHOLE_BASE.into()),
    );
    for (k, v) in &opts.env {
        cmd.env(k, v);
    }
    cmd.arg("-l");
    cmd.arg(&dirs.log_dir);
    for arg in &opts.args {
        cmd.arg(arg);
    }
    match opts.cwd.as_ref() {
        Some(cwd) => cmd.cwd(cwd),
        None => cmd.cwd(&dirs.home),
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("spawn_command failed: {e}"))?;
    drop(pair.slave);

    let parser = Arc::new(Mutex::new(vt100::Parser::new(
        opts.size.rows,
        opts.size.cols,
        100,
    )));
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("try_clone_reader failed: {e}"))?;
    let parser_clone = parser.clone();
    let reader_handle = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut p = parser_clone
                        .lock()
                        .expect("vt100 parser mutex poisoned");
                    p.process(&buf[..n]);
                }
                Err(_) => break,
            }
        }
    });
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("take_writer failed: {e}"))?;

    Ok(AppHandle {
        _master: pair.master,
        child,
        parser,
        reader_handle: Some(reader_handle),
        writer,
        dirs,
        closed: false,
    })
}
