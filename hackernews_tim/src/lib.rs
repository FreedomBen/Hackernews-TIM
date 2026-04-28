//! Internal library shim for the `hackernews_tim` binary.
//!
//! Exists primarily so `cargo test --doc` can exercise documentation
//! examples on the public surface (see TEST_PLAN.md §1.9). The real
//! entry point is the binary at `src/main.rs`, which uses these
//! modules through the library.

pub mod client;
pub mod config;
pub mod model;
pub mod parser;
pub mod prelude;
pub mod reply_editor;
pub mod utils;
pub mod view;
