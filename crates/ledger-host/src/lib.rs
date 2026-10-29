//! The books open while something holds them (plan 3.9, D-21, D-23).
//!
//! Nothing runs in the background: the first holder (the app, an agent's
//! bridge, `wanderbooks serve`, or a command that needs the books) starts
//! the **host**, which starts the books server and holds it open while
//! any lease on its socket lasts, and closes it 10 s after the last one
//! ends. Each part is a mode of the one `wanderbooks` executable:
//! [`host::run`] (`wanderbooks host`), [`bridge::run`] (`wanderbooks
//! mcp`), and the owner's commands in [`commands`] (`serve`, `close`,
//! `books`, `token`, `connect`, `doctor`, `passphrase`, and what the
//! viewer's Books menu does). Agents' tokens are kept in files under the data root
//! ([`tokens`]).

#[cfg(unix)]
pub mod bridge;
#[cfg(unix)]
pub mod commands;
#[cfg(unix)]
pub mod host;
pub mod knobs;
pub mod lease;
#[cfg(unix)]
pub mod opener;
#[cfg(unix)]
mod start;
pub mod tokens;
#[cfg(unix)]
pub mod update;

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
