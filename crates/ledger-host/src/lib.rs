//! The books open while something holds them (plan 3.9, D-21, D-23; plan
//! P15 3.1, D-77, D-84).
//!
//! On the desktop the app is the host: [`app_server`] starts the books
//! server as the window's child and closes it by closing its input.
//! Nothing else on the desktop starts a detached host. The headless
//! `wanderbooks serve` keeps one, as before: its host starts the books
//! server and holds it open while any lease on its socket lasts, and
//! closes it 10 s after the last one ends. Each part is a mode of the one
//! `wanderbooks` executable: [`host::run`] (`wanderbooks host`),
//! [`bridge::run`] (`wanderbooks mcp`, a line to whatever runs, in words
//! when nothing does), and the owner's commands in [`commands`] (`serve`,
//! `close`, `books`, `token`, `connect`, `doctor`, `passphrase`, and what
//! the viewer's Books menu does), which talk to the running app's (or
//! host's) server, or with nothing running run a server of their own for
//! the length of the work, or say "Start Wanderbooks first". Agents'
//! tokens are kept in files under the data root ([`tokens`]).

pub mod app_server;
pub mod bridge;
pub mod channel;
pub mod commands;
pub mod extension;
pub mod host;
pub mod knobs;
pub mod lease;
pub mod opener;
mod start;
pub mod tokens;
#[cfg(unix)]
pub mod update;
pub mod web;

/// The version of the program being run, set by its `main` (this crate's
/// own is the fallback: the packages rewrite only the program's, whose
/// `Cargo.toml` is the one source of the version, WB-pkg-6).
static PROGRAM_VERSION: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Say which version this program is.
pub fn set_program_version(version: &'static str) {
    let _ = PROGRAM_VERSION.set(version);
}

/// This program's version.
pub fn program_version() -> &'static str {
    PROGRAM_VERSION
        .get()
        .copied()
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// How a mode stops when it can't go on: the reason is already printed,
/// and the number is the process's exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit(pub i32);

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}", self.0)
    }
}

impl std::error::Error for Exit {}
