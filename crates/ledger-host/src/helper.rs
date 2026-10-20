//! `wanderbooks helper`: the small unconfined process a confined mode
//! keeps beside it (plan 3.9, "Starting the host cleanly").
//!
//! ```text
//! GOAL: a mode that confines itself (the viewer, the bridge) can still
//!       start the host cleanly and hand off the few things a sandbox
//!       forbids, through a channel that can ask for nothing else.
//! SOURCE: plan/served.md 3.9 (the helper: spawned over an inherited
//!         socket pair before any thread, a fixed message set; it never
//!         accepts a path or a command line).
//! INVARIANTS:
//!   1. It talks only over the socket it was handed (descriptor 3), one
//!      JSON line each way, and exits when that closes.
//!   2. It accepts exactly start_host, open_with (an attachment id),
//!      stage_backup and start_updater, none carrying a path or command;
//!      anything else is refused.
//!   3. start_host starts the host cleanly (start.rs).
//!   4. Spawned before the caller starts any thread.
//!   5. A message is exactly its version and one ask, on a line of at
//!      most MAX_LINE bytes; anything else ends the helper.
//! ```

use crate::knobs::{FLAG, Knobs};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};

/// This protocol's version.
pub const PROTO: u32 = 1;
/// The descriptor the helper is handed its socket on.
const FD: i32 = 3;
/// The longest line either side sends.
const MAX_LINE: u64 = 64 * 1024;

/// A message to the helper (INVARIANT 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// The sender's protocol version.
    pub proto: u32,
    /// What it asks.
    #[serde(flatten)]
    pub ask: HelperAsk,
}

/// What the helper can be asked: exactly these (INVARIANT 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperAsk {
    /// Start the books' host.
    StartHost,
    /// Open a document with the system's app for it.
    OpenWith {
        /// The attachment's id.
        attachment: u64,
    },
    /// Ask the owner for a backup file and stage it for a restore.
    StageBackup,
    /// Start the updater.
    StartUpdater,
}

/// The helper's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    /// Done.
    Done,
    /// Refused, with why.
    Refused(String),
}

/// The caller's end: a running helper.
pub struct Client {
    stream: std::sync::Mutex<UnixStream>,
    child: std::sync::Mutex<std::process::Child>,
}

impl Client {
    /// Spawn a helper for the root (`root`, else this account's), with
    /// these knobs. Call it before starting any thread (INVARIANT 4).
    pub fn spawn(root: Option<&Path>, knobs: &Knobs) -> Result<Self, String> {
        let (ours, theirs) = UnixStream::pair().map_err(|e| e.to_string())?;
        let exe = crate::start::this_executable()?;
        let mut c = std::process::Command::new(exe);
        c.arg("helper");
        if let Some(root) = root {
            let found = ledger_store::root::Root::find(Some(root)).map_err(|e| e.to_string())?;
            c.arg("--root").arg(found.path());
        }
        c.args(knobs.args())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null());
        let fd = theirs.as_raw_fd();
        // SAFETY: dup2 and fcntl are async-signal-safe; they give the child
        // its end of the pair as descriptor 3, without close-on-exec (which
        // a descriptor already numbered 3 would still have).
        unsafe {
            c.pre_exec(move || {
                let ok =
                    (fd == FD || libc::dup2(fd, FD) >= 0) && libc::fcntl(FD, libc::F_SETFD, 0) >= 0;
                if !ok {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = c
            .spawn()
            .map_err(|e| format!("can't start the helper: {e}"))?;
        drop(theirs);
        Ok(Self {
            stream: std::sync::Mutex::new(ours),
            child: std::sync::Mutex::new(child),
        })
    }

    /// Ask the helper one thing (blocking).
    pub fn ask(&self, ask: HelperAsk) -> Result<Answer, String> {
        let stream = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        let mut line =
            serde_json::to_string(&Message { proto: PROTO, ask }).map_err(|e| e.to_string())?;
        line.push('\n');
        (&*stream)
            .write_all(line.as_bytes())
            .map_err(|e| format!("the helper is gone: {}", e.kind()))?;
        let mut answer = String::new();
        std::io::BufReader::new(std::io::Read::take(&*stream, MAX_LINE))
            .read_line(&mut answer)
            .map_err(|e| format!("the helper didn't answer: {}", e.kind()))?;
        serde_json::from_str(answer.trim_end()).map_err(|_| "the helper is gone".to_string())
    }

    /// Start the books' host (INVARIANT 3).
    pub fn start_host(&self) -> Result<(), String> {
        match self.ask(HelperAsk::StartHost)? {
            Answer::Done => Ok(()),
            Answer::Refused(why) => Err(why),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Closing our end ends the helper (INVARIANT 1); reap it.
        let stream = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        let _ = stream.shutdown(std::net::Shutdown::Both);
        drop(stream);
        let _ = self.child.lock().unwrap_or_else(|e| e.into_inner()).wait();
    }
}

/// What the helper does with one message.
fn answer(ask: HelperAsk, root: Option<&Path>, knobs: &Knobs) -> Answer {
    match ask {
        HelperAsk::StartHost => match crate::start::start_host(root, knobs) {
            Ok(()) => Answer::Done,
            Err(why) => Answer::Refused(why),
        },
        HelperAsk::OpenWith { .. } => {
            Answer::Refused("opening documents comes with the viewer's menu".into())
        }
        HelperAsk::StageBackup => Answer::Refused(
            "restore from a terminal for now: `wanderbooks books restore BACKUP`".into(),
        ),
        HelperAsk::StartUpdater => Answer::Refused("updates come with the installers".into()),
    }
}

/// `wanderbooks helper`: serve the socket on descriptor 3 until it closes
/// (INVARIANT 1).
pub fn run(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut root: Option<PathBuf> = None;
    let mut knobs = Knobs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?)),
            FLAG => knobs = Knobs::parse(&args.next().ok_or("--test-knobs needs a value")?)?,
            "--help" | "-h" => {
                eprintln!("usage: {prog} [--root DIR]");
                eprintln!("  Started by a confined part of wanderbooks, over descriptor 3.");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                return Err(Box::new(crate::Exit(2)));
            }
        }
    }
    // SAFETY: descriptor 3 is ours by INVARIANT 1; if it isn't a socket,
    // the peer check below refuses it.
    let stream = unsafe { UnixStream::from_raw_fd(FD) };
    // Taken: nothing this helper starts inherits it.
    // SAFETY: fcntl on our own descriptor.
    unsafe { libc::fcntl(FD, libc::F_SETFD, libc::FD_CLOEXEC) };
    if !ledger_mcp::local::peer_is_us(&stream) {
        eprintln!("{prog}: descriptor 3 isn't a socket to this account's own process");
        return Err(Box::new(crate::Exit(2)));
    }
    serve(std::io::BufReader::new(&stream), &stream, |ask| {
        answer(ask, root.as_deref(), &knobs)
    });
    Ok(())
}

/// Answer each message read from `reader` on `writer`, a line each, with
/// `reply` for the asks of this version, until the input ends or a line
/// is too long or cut off (INVARIANTS 1, 5).
fn serve<R: BufRead, W: Write>(mut reader: R, mut writer: W, reply: impl Fn(HelperAsk) -> Answer) {
    loop {
        // INVARIANT 5: never more than a line's worth read.
        let mut line = String::new();
        match std::io::Read::take(&mut reader, MAX_LINE + 1).read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) if !line.ends_with('\n') => break,
            Ok(_) => {}
        }
        let parsed = crate::lease::strict_pair(line.trim_end())
            .and_then(|v| serde_json::from_value::<Message>(v).map_err(|e| e.to_string()));
        let answer = match parsed {
            Ok(m) if m.proto == PROTO => reply(m.ask),
            Ok(_) => Answer::Refused("the helper is another version: restart".into()),
            Err(e) => Answer::Refused(format!("not understood: {e}")),
        };
        let mut text = serde_json::to_string(&answer).unwrap_or_default();
        text.push('\n');
        if writer.write_all(text.as_bytes()).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exactly_four_asks_and_none_carries_a_path() {
        for ok in [
            json!({"proto": 1, "start_host": null}),
            json!({"proto": 1, "open_with": {"attachment": 7}}),
            json!({"proto": 1, "stage_backup": null}),
            json!({"proto": 1, "start_updater": null}),
        ] {
            assert!(
                serde_json::from_value::<Message>(ok.clone()).is_ok(),
                "{ok}"
            );
        }
        for bad in [
            json!({"proto": 1, "open_with": {"attachment": 7, "path": "/etc/passwd"}}),
            json!({"proto": 1, "open_with": {"path": "/x"}}),
            json!({"proto": 1, "run": {"command": "sh"}}),
            json!({"proto": 1, "start_host": {"root": "/x"}}),
            json!({"proto": 1}),
        ] {
            assert!(
                serde_json::from_value::<Message>(bad.clone()).is_err(),
                "{bad}"
            );
        }
        // Two asks in one message: refused, not half-read.
        let two = r#"{"proto":1,"start_host":null,"start_updater":null}"#;
        assert!(crate::lease::strict_pair(two).is_err());
    }

    /// Serve `input`; the answers, a line each.
    fn served(input: &[u8]) -> Vec<String> {
        let mut out: Vec<u8> = Vec::new();
        serve(input, &mut out, |ask| match ask {
            HelperAsk::StartHost => Answer::Done,
            _ => Answer::Refused("no".into()),
        });
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn each_line_is_answered_and_a_bad_one_ends_it() {
        let ok = r#"{"proto":1,"start_host":null}"#;
        let other = r#"{"proto":2,"start_host":null}"#;
        let got = served(format!("{ok}\n{other}\nnonsense\n{ok}\n").as_bytes());
        assert_eq!(got[0], r#""done""#);
        assert!(got[1].contains("another version"), "{}", got[1]);
        assert!(got[2].contains("not understood"), "{}", got[2]);
        assert_eq!(got[3], r#""done""#);
        assert_eq!(got.len(), 4);
        // A line at the limit is read; one byte more ends the helper,
        // and so does one cut off without its newline.
        let at = format!("{ok}{}\n", " ".repeat(MAX_LINE as usize - ok.len()));
        assert_eq!(served(at.as_bytes()), [r#""done""#]);
        let over = format!(
            "{ok}{}\n{ok}\n",
            " ".repeat(MAX_LINE as usize - ok.len() + 1)
        );
        assert!(served(over.as_bytes()).is_empty());
        assert!(served(ok.as_bytes()).is_empty());
        // An answer that can't be written ends it.
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let asked = std::cell::Cell::new(0);
        serve(format!("{ok}\n{ok}\n").as_bytes(), Broken, |_| {
            asked.set(asked.get() + 1);
            Answer::Done
        });
        assert_eq!(asked.get(), 1, "nothing more after the first answer failed");
    }

    #[test]
    fn what_isnt_built_yet_is_refused() {
        let k = Knobs::default();
        for ask in [
            HelperAsk::OpenWith { attachment: 1 },
            HelperAsk::StageBackup,
            HelperAsk::StartUpdater,
        ] {
            assert!(matches!(answer(ask, None, &k), Answer::Refused(_)));
        }
    }
}
