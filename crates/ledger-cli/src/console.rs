//! The program's console on Windows (plan P15 unit C1, WB-pkg-2).
//!
//! The installed program is one executable that is the window, the books
//! server child and the assistants' bridge. Built for the Windows GUI
//! subsystem (`windows_subsystem = "windows"` in `main.rs`), a double-click
//! or a Start Menu entry opens no console window. The same file still has
//! to be a command line: a person who types `wanderbooks books list` in a
//! terminal needs the words printed there, and an assistant that starts
//! `wanderbooks mcp --book X` hands it pipes, which a GUI-subsystem
//! program receives as they are.
//!
//! ```text
//! GOAL: no console window when the program starts as an app; its words
//!       reach the terminal that started it as a command; a pipe handed to
//!       it is never replaced.
//! SOURCE: WB-pkg-2 (one executable); Microsoft's documentation of
//!         AttachConsole(ATTACH_PARENT_PROCESS) and SetStdHandle.
//! INVARIANTS:
//!   1. A standard handle the parent gave (a pipe, a file) is left as it
//!      is; only a missing one (null or invalid) is pointed at the
//!      parent's console (CONIN$ for input, CONOUT$ for output and
//!      errors).
//!   2. With no parent console (a Start Menu launch, an assistant that
//!      gave pipes and has no console) nothing is attached and nothing is
//!      opened: the program never makes a console window of its own.
//! SCENARIOS:
//!   1. IF an assistant starts `mcp` with three pipes: SHOULD speak over
//!      them, unchanged.
//!   2. IF a terminal starts `books list`: SHOULD print into the terminal.
//!   3. IF the Start Menu starts it with no words: SHOULD open the window
//!      and no console.
//! ```
//!
//! Not run on Windows by the author (the unit was written on Linux); the
//! `release` workflow's Windows job builds it, and the first install is
//! the evidence for the scenarios.

/// Point the missing standard handles at the parent's console, if there
/// is one (INVARIANTS 1 and 2). A no-op everywhere but Windows.
#[cfg(windows)]
pub fn attach_parent_console() {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };

    // SAFETY: plain Win32 calls with no pointers of ours; a failure is a
    // value, and the handles opened below are given to the process, which
    // keeps them open for its life.
    unsafe {
        let missing = |id| {
            let handle = GetStdHandle(id);
            handle.is_null() || handle == INVALID_HANDLE_VALUE
        };
        let wanted = [
            (STD_INPUT_HANDLE, "CONIN$", true),
            (STD_OUTPUT_HANDLE, "CONOUT$", false),
            (STD_ERROR_HANDLE, "CONOUT$", false),
        ];
        if !wanted.iter().any(|(id, _, _)| missing(*id)) {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        for (id, device, input) in wanted {
            if !missing(id) {
                continue;
            }
            let opened = std::fs::OpenOptions::new()
                .read(input)
                .write(!input)
                .open(device);
            if let Ok(file) = opened {
                SetStdHandle(id, file.into_raw_handle());
            }
        }
    }
}

/// Nothing to do where a program is a console program already.
#[cfg(not(windows))]
pub fn attach_parent_console() {}
