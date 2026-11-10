//! `wanderbooks licenses`: the third-party notices are part of the binary
//! and print as plain text, and the mode behaves like every other one.

use std::process::Command;

fn wanderbooks() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
}

#[test]
fn licenses_prints_the_notices_and_nothing_on_stderr() {
    let out = wanderbooks().arg("licenses").output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        out.stderr.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.starts_with("wanderbooks "),
        "the version line comes first: {:?}",
        &text[..text.len().min(80)]
    );
    for needle in [
        "MIT OR Apache-2.0",
        "Apache License",
        "SIL OPEN FONT LICENSE",
        "rust_decimal",
        "egui",
        "rusqlite",
        "Inter Project Authors",
        "Bitstream",
        // A crate whose own licence file cargo-about cannot match (two
        // licences in one file) still travels with its copyright line.
        "LICENCE FILES AS PACKAGED",
        "Kang Seonghoon",
        "Packaged with no licence file",
    ] {
        assert!(text.contains(needle), "{needle:?} is not in the notices");
    }
}

#[test]
fn the_printed_notices_are_the_file_at_the_repository_root() {
    let notices = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../THIRD-PARTY-NOTICES.txt"
    ));
    let out = wanderbooks().arg("licenses").output().unwrap();
    let done = out
        .stdout
        .windows(2)
        .position(|w| w == b"\n\n")
        .expect("the header line and an empty line");
    assert_eq!(&out.stdout[done + 2..], notices);
}

#[test]
fn licenses_takes_no_arguments() {
    let out = wanderbooks().args(["licenses", "extra"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("usage: wanderbooks licenses"),
        "{out:?}"
    );
}

#[test]
fn licenses_comes_first_like_every_mode() {
    let out = wanderbooks()
        .args(["--root", "x", "licenses"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("must come first"),
        "{out:?}"
    );
}
