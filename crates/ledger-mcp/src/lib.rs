//! `ledger-mcp`: the AI-first bookkeeping interface. It is served one way
//! (plan D-44): by the books server, `wanderbooks server` ([`local`]),
//! every book of the data root on the root's local sockets, which agents
//! reach through the bridge `wanderbooks mcp --book ID`. Files come in as
//! tool input (inline, at most 256 KiB) or as uploads, and the files the
//! books make go out as downloads.

pub mod access;
pub mod admin;
pub mod backup;
mod handbook;
#[cfg(unix)]
pub mod local;
pub mod opening;
mod policy;
pub mod scopes;
mod server;
mod settings;
mod tin_files;
mod types;

pub use server::{NOT_RUN_CLOSING, NOT_RUN_CLOSING_CODE};

/// How a mode of the one executable stops when it can't go on: the
/// reason is already printed on stderr, and the number is the process's
/// exit code (2: a bad argument). A library never ends the process
/// itself, so the books it opened are closed properly on the way out;
/// the executable turns this into the exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit(pub i32);

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}", self.0)
    }
}

impl std::error::Error for Exit {}

#[cfg(test)]
mod exit_tests {
    use super::Exit;

    #[test]
    fn an_exit_says_its_code() {
        assert_eq!(Exit(2).to_string(), "exit code 2");
        assert_eq!(Exit(1).to_string(), "exit code 1");
    }
}
