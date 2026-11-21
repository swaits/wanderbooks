//! The line the Connect page shows to copy (commands.rs INVARIANT 8; plan
//! P15 unit A4).
//!
//! ```text
//! GOAL: a command pasted into a terminal runs as it reads: whatever a
//!       path holds, a POSIX shell gets back exactly the parts it was
//!       given; PowerShell's quoting doubles what it must; a part that
//!       cannot be written safely for both PowerShell and cmd.exe is
//!       refused, in words, never shown.
//! SOURCE: WB-app-1 (the window shows an exact command), POSIX shell
//!         command language, "Quoting": single quotes keep every
//!         character literal but the single quote itself.
//! INVARIANTS:
//!   1. For any parts, `sh` reads the Unix line back as those parts.
//!   2. Plain parts are left as they are.
//!   3. PowerShell: a `'` is doubled; a part with a character cmd.exe
//!      acts on and that needs quoting is refused.
//! ```

use ledger_host::commands::{quote_unix, quote_windows, shell_line};
#[cfg(unix)]
use std::process::Command;

/// The parts a POSIX shell reads from `line` as the words of a command,
/// by `printf` (which prints each word then a NUL).
#[cfg(unix)]
fn sh_words(line: &str) -> Vec<String> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("printf '%s\\0' {line}"))
        .output()
        .expect("sh runs");
    assert!(out.status.success(), "sh refused: {line}");
    let mut words: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .map(|w| String::from_utf8(w.to_vec()).unwrap())
        .collect();
    assert_eq!(words.pop().as_deref(), Some(""), "ends with a NUL");
    words
}

/// A small deterministic generator (no extra dependency).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Random text weighted to what a shell cares about.
fn random_part(rng: &mut Rng) -> String {
    const SPECIAL: &[char] = &[
        ' ', '\t', '\n', '$', '`', ';', '&', '|', '<', '>', '(', ')', '{', '}', '[', ']', '*', '?',
        '~', '#', '!', '\'', '"', '\\', '%', '^', '=', ',', ':', '/', '.', '-', '_', '+', '@', 'a',
        'Z', '0', 'é', '日', '😀',
    ];
    let len = (rng.next() % 14) as usize;
    (0..len)
        .map(|_| SPECIAL[(rng.next() % SPECIAL.len() as u64) as usize])
        .collect()
}

#[cfg(unix)]
#[test]
fn a_posix_shell_reads_back_exactly_the_parts_whatever_they_hold() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..150 {
        let n = 1 + (rng.next() % 4) as usize;
        let parts: Vec<String> = (0..n).map(|_| random_part(&mut rng)).collect();
        let line = shell_line(&parts, false).unwrap();
        assert_eq!(sh_words(&line), parts, "line: {line}");
    }
    // The ones that bite, by name.
    for part in [
        "/home/a b/$HOME/`id`/;rm -rf x/&|'q'\"",
        "it's",
        "''",
        "",
        "-",
        "a\nb",
        "~/x",
        "*",
    ] {
        let parts = vec![part.to_string(), "mcp".to_string()];
        let line = shell_line(&parts, false).unwrap();
        assert_eq!(sh_words(&line), parts, "line: {line}");
    }
}

#[test]
fn plain_parts_are_left_as_they_are() {
    assert_eq!(quote_unix("mcp"), "mcp");
    assert_eq!(
        quote_unix("/opt/wander-books/bin_1.2/x@y:z,w=1+2%3"),
        "/opt/wander-books/bin_1.2/x@y:z,w=1+2%3"
    );
    assert_eq!(quote_unix("a b"), "'a b'");
    assert_eq!(quote_unix("it's"), "'it'\\''s'");
    assert_eq!(quote_unix(""), "''");
    let line = shell_line(
        &[
            "/opt/wb".into(),
            "mcp".into(),
            "--book".into(),
            "acme".into(),
        ],
        false,
    );
    assert_eq!(line.unwrap(), "/opt/wb mcp --book acme");
}

#[test]
fn powershell_quoting_doubles_quotes_and_refuses_what_cmd_would_act_on() {
    // Plain: the same line in PowerShell and cmd.exe.
    let plain = shell_line(&["C:\\wb\\wanderbooks.exe".into(), "mcp".into()], true).unwrap();
    assert_eq!(plain, "C:\\wb\\wanderbooks.exe mcp");
    // A space: PowerShell's single quotes, and `&` to run a quoted program.
    let spaced = shell_line(&["C:\\Program Files\\wb\\x.exe".into(), "mcp".into()], true).unwrap();
    assert_eq!(spaced, "& 'C:\\Program Files\\wb\\x.exe' mcp");
    // A quote is doubled (also the curly ones PowerShell reads as quotes).
    assert_eq!(quote_windows("it's").unwrap(), ("'it''s'".into(), true));
    assert_eq!(
        quote_windows("a\u{2019}b").unwrap(),
        ("'a\u{2019}\u{2019}b'".into(), true)
    );
    // `$` and a backtick are literal in single quotes.
    assert_eq!(quote_windows("$x`y").unwrap(), ("'$x`y'".into(), true));
    // What cmd.exe acts on: refused, in words.
    for bad in [
        "a&b", "a|b", "a<b", "a>b", "a^b", "50%", "a\"b", "hi!", "a\nb",
    ] {
        let why = quote_windows(bad).unwrap_err();
        assert!(why.contains("cmd.exe"), "{bad}: {why}");
        assert!(shell_line(&[bad.to_string()], true).is_err());
    }
}
