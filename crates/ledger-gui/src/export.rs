//! Files the viewer makes for you (report PDFs). They go to the
//! Downloads folder, never into the books: the viewer stays read-only.

use std::path::{Path, PathBuf};

/// Where saved files go: `$XDG_DOWNLOAD_DIR`, else `~/Downloads`, else
/// the home directory.
pub fn downloads_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_DOWNLOAD_DIR").map(PathBuf::from)
        && d.is_dir()
    {
        return d;
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let d = home.join("Downloads");
    if d.is_dir() { d } else { home }
}

/// `name` made safe as a file name on every platform.
///
/// ```text
/// GOAL: a report title (company data) can never name a file outside the
///       downloads folder, or one a platform refuses.
/// SOURCE: docs/spec/sources/file-names.md (Microsoft's naming rules;
///         POSIX NAME_MAX).
/// INVARIANTS:
///   1. The result is `<base>.<ext>`, `base` holding no reserved
///      character (< > : " / \ | ? *) and no control character: each is
///      replaced ('-' or a space).
///   2. `base` is never empty, never "." or "..", never ends with a space
///      or a period, and is never a reserved device name (CON, NUL, COM1,
///      LPT¹, ... in any case, before its first period): such a base gets
///      a leading '_'.
///   3. The whole name is at most 255 bytes of UTF-8, cut at a character
///      boundary.
/// ```
pub fn file_name(name: &str, ext: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let mut base = clean.trim().trim_end_matches(['.', ' ']).to_string();
    if base.is_empty() {
        base = "report".to_string();
    }
    let stem = base
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2"
                        | "3"
                        | "4"
                        | "5"
                        | "6"
                        | "7"
                        | "8"
                        | "9"
                        | "\u{b9}"
                        | "\u{b2}"
                        | "\u{b3}"
                )
            })
        });
    if reserved {
        base.insert(0, '_');
    }
    let limit = 255usize.saturating_sub(ext.len() + 1);
    if base.len() > limit {
        let mut cut = limit;
        while !base.is_char_boundary(cut) {
            cut -= 1;
        }
        base.truncate(cut);
        // A cut can leave a trailing space or period: trim again.
        while base.ends_with(['.', ' ']) {
            base.pop();
        }
        if base.is_empty() {
            base = "report".to_string();
        }
    }
    format!("{base}.{ext}")
}

/// Write `bytes` to the downloads folder as `name.ext` (replacing a file
/// of that name: the same report renders the same bytes) and open it with
/// the system's viewer.
pub fn save_and_open(name: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = downloads_dir().join(file_name(name, ext));
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    open(&path);
    Ok(path)
}

/// Open a file with whatever the desktop uses for its type. Best effort:
/// the file is saved either way.
fn open(path: &Path) {
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = std::process::Command::new("xdg-open").arg(path).spawn();
    let _ = cmd;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(
            file_name("Acme LLC — Profit and loss — Jan 1/2026: draft", "pdf"),
            "Acme LLC — Profit and loss — Jan 1-2026- draft.pdf"
        );
    }
}
