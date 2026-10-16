//! Files that carry full TINs never enter the books (rules.md B-26).
//!
//! ```text
//! GOAL: a file holding a full TIN (a W-9, a 1099 worksheet, a CPA
//!       package or a file from one) is never filed as a receipt or
//!       document, where books.get_receipt would hand its bytes to any
//!       agent.
//! SOURCE: rules.md B-26 (no tool output holds a full TIN); DECISION (P0
//!         review round 2, 2026-10-16): one registry, on disk, of every
//!         TIN-bearing file the server reads or writes, and a scan of
//!         every text file on its way in.
//! INVARIANTS:
//!   1. The registry holds the SHA-256 of every file the server read or
//!      wrote with full TINs: the W-9 books.set_w9 reads (recorded once
//!      the call has passed every check, before the vault keeps it: a
//!      refused call marks nothing), each books.get_w9 output, a
//!      books.vendor_1099 worksheet with TINs, and a CPA package with
//!      TINs together with each file inside it that holds one.
//!   2. It is one file beside the vault, so every session and every later
//!      server for these books shares it; hashes are not secret. A file
//!      is recorded before it is written out, and a failure to record it
//!      refuses the call.
//!   3. Every way bytes are filed into the books refuses a recorded hash,
//!      and refuses text (by its type, its name, or because it decodes as
//!      UTF-8 or UTF-16, with a byte-order mark or NUL-interleaved
//!      without one) holding a full TIN, read as either:
//!      a. a TIN the vault holds for a party of the books, anywhere in the
//!         text, written as nine digits in a row, as 3-2-4 digits joined
//!         by hyphens, spaces or dots, or as 2-7 digits joined by a hyphen
//!         or a space (compared on the server; the vault's TINs are never
//!         returned or logged). A vault that can't be opened leaves (b);
//!      b. a number printed the way an SSN or an EIN is, standing alone:
//!         3-2-4 digits joined by hyphens (123-45-6789) or 2-7 digits
//!         joined by a hyphen (12-3456789), with no digit, and no hyphen
//!         or dot followed by a digit, on either side.
//!      Full-width digits and Unicode hyphens and spaces read as their
//!      ASCII kin. Refused bytes are never stored.
//!   4. No probing (DECISION, P0 review, 2026-10-16): (a) answers for at
//!      most 2 candidates per filing. Before the vault is opened, text
//!      holding more than 2 distinct TIN-shaped numbers (any rendering
//!      (a) reads, the same number written twice counting once) that end
//!      in the last four digits of a TIN the books know (a party's TIN or
//!      W-9 facts, read from the books, not the vault) is refused with
//!      TIN_PROBE, whether or not one of them is a TIN the vault holds;
//!      the vault's own last fours, once it is opened, count too (a
//!      record the books' facts don't name). The last four is no secret
//!      (the tools show it), and the answer to such text never depends on
//!      the rest of the TIN, so finding one of the 100,000 TINs with a
//!      known last four takes on the order of 50,000 filings, each one
//!      refused and seen.
//!   5. Ordinary text passes: a bank export's routing and account numbers,
//!      long references, a date next to an amount (DECISION, owner,
//!      2026-10-16: bank exports are filed for reconciliation, so the
//!      text check is narrow; the broad has_full_tin check stays on what
//!      the server says).
//!   6. Not detected: a changed copy of a binary file (a re-saved or
//!      re-scanned W-9 PDF), and a TIN the vault doesn't hold written
//!      other than (b). plan/served.md P2 and P7 take the path tools and
//!      full TINs out of the agent's reach. Open until then: (a) still
//!      answers for up to 2 candidates a filing (4), so an agent that
//!      files 50,000 refused files can find a TIN; P2's access log
//!      records every one of those calls, and P7's human-only TIN channel
//!      takes full TINs, and so this check, off the agent's path
//!      (rules.md B-26).
//! ```

use std::path::Path;

/// The registry's file, in the vault's folder.
pub const FILE_NAME: &str = "tin-files.sha256";

/// The refusal for a file in the registry.
pub const TIN_FILE: &str = "that file is one the books server read or wrote with full TINs (a W-9, a 1099 worksheet, a CPA package or a file from one): it never goes into the books";

/// The refusal for text holding a TIN the vault holds (INVARIANT 3a).
pub const TIN_HELD: &str = "that file's text holds a full TIN the books keep in the TIN vault: a file with a full TIN never goes into the books";

/// The refusal for text holding more than [`PROBE_BOUND`] TIN-shaped
/// numbers that end in a known TIN's last four (INVARIANT 4): the same
/// whether or not one of them is that TIN.
pub const TIN_PROBE: &str = "that file's text holds more than two numbers shaped like a TIN (nine digits, alone or grouped 3-2-4 or 2-7) that end in the last four digits of a TIN the books keep, so it isn't filed, whether or not one of them is that TIN: ask the human to save the original document as a PDF and file that";

/// How many TIN-shaped numbers ending in a known last four a filed text
/// may hold for (a) to compare against the vault (INVARIANT 4).
pub const PROBE_BOUND: usize = 2;

/// The refusal for text holding a number printed as an SSN or EIN is
/// (INVARIANT 3b).
pub const TIN_PRINTED: &str = "that file's text holds a number printed the way an SSN or EIN is (three, two and four digits joined by hyphens, or two and seven), which reads as a full TIN: a file with a full TIN never goes into the books. If it is no TIN (a reference number printed that way), ask the human to save the file as a PDF and file that";

/// Record that `bytes` carry full TINs (INVARIANT 2).
pub fn record(vault_dir: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let sha = ledger_store::journal::sha256_hex(bytes);
    if holds_sha(vault_dir, &sha)? {
        return Ok(());
    }
    let fail = |e: std::io::Error| {
        format!(
            "the books server couldn't record a file with full TINs: {}",
            e.kind()
        )
    };
    std::fs::create_dir_all(vault_dir).map_err(fail)?;
    let mut opts = std::fs::OpenOptions::new();
    opts.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(vault_dir.join(FILE_NAME)).map_err(fail)?;
    f.write_all(format!("{sha}\n").as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(fail)
}

/// Whether the registry holds `sha`.
fn holds_sha(vault_dir: &Path, sha: &str) -> Result<bool, String> {
    match std::fs::read_to_string(vault_dir.join(FILE_NAME)) {
        Ok(text) => Ok(text.lines().any(|l| l.trim() == sha)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        // Fail closed: a registry that can't be read refuses the file.
        Err(e) => Err(format!(
            "the books server couldn't check the file against the ones it wrote with full TINs: {}",
            e.kind()
        )),
    }
}

/// Refuse `bytes` on their way into the books if they carry a full TIN
/// (INVARIANT 3): a recorded file, or text holding one. `mime_type` and
/// `file_name` say whether to read the bytes as text even when they
/// aren't UTF-8. Asked for only when the bytes are text: `known_last4`
/// gives the last four digits of every TIN the books know of, from the
/// books (INVARIANT 4), and `held`, asked for only when the text holds
/// no more than [`PROBE_BOUND`] candidates, the vault's TINs (none when
/// the vault can't be opened).
pub fn refuse(
    vault_dir: &Path,
    bytes: &[u8],
    mime_type: &str,
    file_name: &str,
    known_last4: impl FnOnce() -> Vec<u32>,
    held: impl FnOnce() -> Vec<ledger_vault::Tin>,
) -> Result<(), String> {
    use zeroize::Zeroizing;
    if holds_sha(vault_dir, &ledger_store::journal::sha256_hex(bytes))? {
        return Err(TIN_FILE.to_string());
    }
    let Some(text) = text_of(bytes, mime_type, file_name) else {
        return Ok(());
    };
    let text: Vec<char> = text.chars().map(fold).collect();
    let mut last4 = known_last4();
    // Before the vault is opened (INVARIANT 4).
    if probes(&text, &last4) {
        return Err(TIN_PROBE.to_string());
    }
    let mut tins: Zeroizing<Vec<u32>> = Zeroizing::new(
        held()
            .iter()
            .filter_map(|t| t.digits().parse().ok())
            .collect(),
    );
    // The vault's own last fours count too.
    last4.extend(tins.iter().map(|t| t % 10_000));
    if probes(&text, &last4) {
        return Err(TIN_PROBE.to_string());
    }
    tins.sort_unstable();
    if holds_held_tin(&text, &tins) {
        return Err(TIN_HELD.to_string());
    }
    if holds_printed_tin(&text) {
        return Err(TIN_PRINTED.to_string());
    }
    Ok(())
}

/// A character as the scan reads it: full-width digits as ASCII ones,
/// Unicode hyphens and dashes as `-`, a full-width full stop as `.`,
/// no-break and other fixed spaces as ` ` (INVARIANT 3).
fn fold(c: char) -> char {
    match c {
        '\u{FF10}'..='\u{FF19}' => char::from_digit(u32::from(c) - 0xFF10, 10).expect("a digit"),
        '\u{2010}'..='\u{2013}' | '\u{2212}' | '\u{FE63}' | '\u{FF0D}' => '-',
        '\u{FF0E}' => '.',
        '\u{A0}' | '\u{2007}' | '\u{202F}' | '\u{3000}' => ' ',
        c => c,
    }
}

/// Read `groups` runs of ASCII digits from `text[at..]`, of the lengths
/// given, joined by one character from `joins`: the digits as a number
/// and the index after the last, or None.
fn digits_at(text: &[char], at: usize, groups: &[usize], joins: &[char]) -> Option<(u32, usize)> {
    let mut value = 0u32;
    let mut i = at;
    for (n, &len) in groups.iter().enumerate() {
        if n > 0 {
            if !joins.contains(text.get(i)?) {
                return None;
            }
            i += 1;
        }
        for _ in 0..len {
            // to_digit(10) reads ASCII digits only.
            value = value * 10 + text.get(i)?.to_digit(10)?;
            i += 1;
        }
    }
    Some((value, i))
}

/// Every TIN-shaped number in `text`, at every place, in every rendering
/// (a) reads: nine digits in a row, 3-2-4 joined by hyphens, spaces or
/// dots, 2-7 joined by a hyphen or a space (INVARIANT 3a).
fn tin_shaped(text: &[char]) -> impl Iterator<Item = u32> + '_ {
    const RENDERINGS: [(&[usize], &[char]); 3] = [
        (&[9], &[]),
        (&[3, 2, 4], &['-', ' ', '.']),
        (&[2, 7], &['-', ' ']),
    ];
    (0..text.len()).flat_map(move |at| {
        RENDERINGS
            .iter()
            .filter_map(move |(groups, joins)| digits_at(text, at, groups, joins).map(|(v, _)| v))
    })
}

/// Whether `text` holds more than [`PROBE_BOUND`] distinct TIN-shaped
/// numbers ending in one of `last4` (INVARIANT 4).
fn probes(text: &[char], last4: &[u32]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    tin_shaped(text)
        .filter(|v| last4.contains(&(v % 10_000)))
        .any(|v| {
            seen.insert(v);
            seen.len() > PROBE_BOUND
        })
}

/// Whether `text` holds one of `tins` (sorted), anywhere, in a common
/// rendering (INVARIANT 3a).
fn holds_held_tin(text: &[char], tins: &[u32]) -> bool {
    tin_shaped(text).any(|v| tins.binary_search(&v).is_ok())
}

/// Whether `text` holds a number printed as an SSN (3-2-4 digits joined
/// by hyphens) or an EIN (2-7 joined by a hyphen) is, standing alone: no
/// digit beside it, and no hyphen or dot that a digit follows on (INVARIANT 3b).
fn holds_printed_tin(text: &[char]) -> bool {
    let digit = |i: usize| text.get(i).is_some_and(|c| c.is_numeric());
    let joins_on = |join: usize, digit_at: usize| {
        text.get(join).is_some_and(|c| matches!(c, '-' | '.')) && digit(digit_at)
    };
    (0..text.len()).any(|at| {
        let alone_before = at == 0 || !(digit(at - 1) || at >= 2 && joins_on(at - 1, at - 2));
        alone_before
            && [&[3, 2, 4][..], &[2, 7][..]].iter().any(|groups| {
                digits_at(text, at, groups, &['-'])
                    .is_some_and(|(_, end)| !(digit(end) || joins_on(end, end + 1)))
            })
    })
}

/// Whether a file of this type or name is text: csv, tsv, txt, json, iif,
/// xml, html, md and their kin.
fn text_like(mime_type: &str, file_name: &str) -> bool {
    let mime = mime_type.trim().to_ascii_lowercase();
    let ext = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    mime.starts_with("text/")
        || [
            "json",
            "xml",
            "csv",
            "x-iif",
            "javascript",
            "x-yaml",
            "yaml",
        ]
        .iter()
        .any(|t| mime.ends_with(&format!("/{t}")) || mime.ends_with(&format!("+{t}")))
        || [
            "csv", "tsv", "txt", "text", "json", "iif", "xml", "html", "htm", "md", "markdown",
            "yaml", "yml", "tab", "prn", "log", "ofx", "qfx", "qif",
        ]
        .contains(&ext.as_str())
}

/// The bytes as text, if they are text (INVARIANT 3): UTF-16 with its
/// byte-order mark, UTF-8, or, for a text type or name, whatever they
/// decode to. Text with NUL bytes and no byte-order mark (UTF-16 written
/// without one is valid UTF-8, its digits split by NULs) is read every
/// way it can be (as it is, as UTF-16 both ways round, and with its NULs
/// taken out), each reading on a line of its own: so is bytes that aren't
/// UTF-8 but are NUL-interleaved as UTF-16 is.
fn text_of(bytes: &[u8], mime_type: &str, file_name: &str) -> Option<String> {
    let utf16 = |body: &[u8], le: bool| {
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if le {
                    u16::from_le_bytes(c)
                } else {
                    u16::from_be_bytes(c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, body @ ..] => Some(utf16(body, true)),
        [0xFE, 0xFF, body @ ..] => Some(utf16(body, false)),
        _ => {
            let plain = match std::str::from_utf8(bytes) {
                Ok(t) => Some(t.to_string()),
                Err(_) if text_like(mime_type, file_name) => {
                    Some(String::from_utf8_lossy(bytes).into_owned())
                }
                Err(_) => None,
            };
            if !bytes.contains(&0) || plain.is_none() && !nul_interleaved(bytes) {
                return plain;
            }
            let without_nuls: Vec<u8> = bytes.iter().copied().filter(|&b| b != 0).collect();
            let readings = [
                plain.unwrap_or_default(),
                utf16(bytes, true),
                utf16(bytes, false),
                String::from_utf8_lossy(&without_nuls).into_owned(),
            ];
            Some(readings.join("\n"))
        }
    }
}

/// Whether `bytes` look like UTF-16 without a byte-order mark: of their
/// byte pairs, at least half have a NUL on one side (the high byte of a
/// Latin character) and at most an eighth on the other.
fn nul_interleaved(bytes: &[u8]) -> bool {
    let pairs = bytes.as_chunks::<2>().0;
    let nuls = |side: usize| pairs.iter().filter(|p| p[side] == 0).count();
    let (first, second) = (nuls(0), nuls(1));
    let n = pairs.len();
    n > 0 && (2 * first >= n && 8 * second <= n || 2 * second >= n && 8 * first <= n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d =
            std::env::temp_dir().join(format!("ledger-mcp-tin-files-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// INVARIANTS 1-2: a recorded file is refused from then on, by this
    /// registry or any other reading the same folder (another session, a
    /// later server); recording twice keeps one line.
    #[test]
    fn a_recorded_tin_file_is_refused_by_every_reader_of_the_registry() {
        let vault = dir("registry");
        let w9 = b"\x89PNG binary W-9 scan".to_vec();
        assert!(refuse(&vault, &w9, "image/png", "w9.png", Vec::new, Vec::new).is_ok());
        record(&vault, &w9).unwrap();
        record(&vault, &w9).unwrap();
        assert_eq!(
            refuse(&vault, &w9, "image/png", "w9.png", Vec::new, Vec::new),
            Err(TIN_FILE.into())
        );
        let text = std::fs::read_to_string(vault.join(FILE_NAME)).unwrap();
        assert_eq!(text.lines().count(), 1);
        // Only the hash is kept.
        assert!(!text.contains("PNG"));
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// The registry's folder for a check (it never exists) and no vault.
    fn check(bytes: &[u8], mime: &str, name: &str) -> Result<(), String> {
        refuse(&dir("check"), bytes, mime, name, Vec::new, Vec::new)
    }

    /// The vault's TINs for the checks: an SSN and an EIN.
    fn vault() -> Vec<ledger_vault::Tin> {
        use ledger_vault::{Tin, TinType};
        vec![
            Tin::new("291122846", TinType::Ssn).unwrap(),
            Tin::new("20-5200615", TinType::Ein).unwrap(),
        ]
    }

    /// Their last fours, as the books know them.
    fn known() -> Vec<u32> {
        vec![2846, 615]
    }

    /// The same, with the vault holding an SSN and an EIN the books know
    /// the last fours of.
    fn check_held(bytes: &[u8], mime: &str, name: &str) -> Result<(), String> {
        refuse(&dir("check"), bytes, mime, name, known, vault)
    }

    /// `text` in each encoding and label a filed file can come as: UTF-8,
    /// UTF-16 either way round, and (for a text type or name) bytes that
    /// aren't UTF-8.
    fn encodings(text: &str) -> Vec<(Vec<u8>, &'static str, &'static str)> {
        let le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        let be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        let mut latin1 = b"Caf\xe9,".to_vec();
        latin1.extend_from_slice(text.as_bytes());
        // UTF-16 without a byte-order mark, either way round: valid UTF-8
        // when the text is ASCII, its digits split by NULs. With a Latin
        // letter (é is E9 00 or 00 E9), not UTF-8 at all.
        let bare = |text: &str, le: bool| -> Vec<u8> {
            text.encode_utf16()
                .flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() })
                .collect()
        };
        let accented = format!("Caf\u{e9},{text}");
        vec![
            (bare(text, true), "application/octet-stream", "export.bin"),
            (bare(text, false), "application/octet-stream", "export.bin"),
            (bare(&accented, true), "application/octet-stream", "x.bin"),
            (bare(&accented, false), "text/plain", "x.bin"),
            (text.as_bytes().to_vec(), "text/csv", "k1-members.csv"),
            (
                text.as_bytes().to_vec(),
                "application/octet-stream",
                "x.bin",
            ),
            (le, "application/octet-stream", "export.csv"),
            (be, "text/plain", "export.txt"),
            (latin1.clone(), "text/csv", "export.csv"),
            (latin1.clone(), "text/plain", "x.bin"),
            (latin1.clone(), "application/json", "x.bin"),
            (latin1.clone(), "application/ld+json", "x.bin"),
            (latin1, "application/octet-stream", "export.tsv"),
        ]
    }

    /// INVARIANT 3a: a TIN the vault holds is refused anywhere in the
    /// text, in every rendering, however the text is encoded or labeled;
    /// the vault is consulted for text only.
    #[test]
    fn text_holding_a_vault_tin_is_refused_in_every_rendering() {
        for tin in [
            "291122846",
            "291-12-2846",
            "291 12 2846",
            "291.12.2846",
            "291-12 2846",
            "29-1122846",
            "29 1122846",
            "205200615",
            "205-20-0615",
            "205 20 0615",
            "205.20.0615",
            "20-5200615",
            "20 5200615",
            // Inside a longer number, and in Unicode's forms.
            "ref 902911228461",
            "\u{FF12}\u{FF19}\u{FF11}\u{FF11}\u{FF12}\u{FF12}\u{FF18}\u{FF14}\u{FF16}",
            "291\u{2013}12\u{2011}2846",
            "291\u{FF0E}12\u{FF0E}2846",
            "291\u{A0}12\u{3000}2846",
            "20\u{2212}5200615",
        ] {
            let text = format!("vendor,tin\nAcme,{tin}\n");
            for (bytes, mime, name) in encodings(&text) {
                assert_eq!(
                    check_held(&bytes, mime, name),
                    Err(TIN_HELD.into()),
                    "{tin:?} as {name}"
                );
            }
            // Without the vault, only the printed shapes are refused.
            let printed = [
                "291-12-2846",
                "29-1122846",
                "205-20-0615",
                "20-5200615",
                "291\u{2013}12\u{2011}2846",
                "20\u{2212}5200615",
            ]
            .contains(&tin);
            assert_eq!(
                check(text.as_bytes(), "text/csv", "x.csv").is_err(),
                printed,
                "{tin:?} with the vault locked"
            );
        }
        // Binary bytes never open the vault.
        assert!(
            refuse(
                &dir("check"),
                b"%PDF-1.4\n\xff 291122846",
                "application/pdf",
                "scan.pdf",
                || panic!("the last fours were asked for a binary file"),
                || panic!("the vault was opened for a binary file")
            )
            .is_ok()
        );
    }

    /// INVARIANT 3b: a number printed as an SSN or EIN is, standing alone,
    /// is refused wherever it stands, however it is encoded, with no vault.
    #[test]
    fn a_printed_ssn_or_ein_is_refused() {
        for text in [
            "123-45-6789",
            "12-3456789",
            "name,ssn\nDana,187-05-1907\n",
            "2026-10-01,187-05-1907,12.50",
            "SSN:187-05-1907.",
            "EIN 98-7814321-",
            "x187-05-1907y",
            "-187-05-1907",
            "x-187-05-1907",
            "1234x-187-05-1907",
            ".12-3456789",
            "1-x12-3456789",
            "\u{FF11}87\u{2010}05\u{FF0D}1907",
        ] {
            for (bytes, mime, name) in encodings(text) {
                assert_eq!(
                    check(&bytes, mime, name),
                    Err(TIN_PRINTED.into()),
                    "{text:?} as {name}"
                );
            }
        }
    }

    /// INVARIANT 3b: an SSN or EIN shape inside a longer number (a digit,
    /// or a hyphen or dot and a digit, beside it) is no TIN.
    #[test]
    fn a_longer_number_holding_a_tin_shape_passes() {
        for text in [
            "1123-45-6789",
            "123-45-67890",
            "0123-45-67890",
            "1-123-45-6789",
            "1.123-45-6789",
            "123-45-6789-0",
            "123-45-6789.5",
            "112-3456789",
            "12-34567890",
            "2026-12-3456789",
            "12-3456789.01",
            "\u{0661}123-45-6789",
        ] {
            assert!(
                check(text.as_bytes(), "text/csv", "x.csv").is_ok(),
                "{text:?}"
            );
        }
    }

    /// INVARIANT 5: a realistic bank export passes, whether or not the
    /// vault holds TINs: routing and account numbers, long references, a
    /// date run into an amount, phone numbers, card numbers.
    #[test]
    fn an_ordinary_bank_export_passes() {
        let csv = "\
Date,Description,Amount,Running Bal.,Reference
10/01/2026,Beginning balance,,\"1,234,567.89\",
10/01/2026,ACH CREDIT APPLE INC 021000021 ID 9878143210123456,1499.00,\"1,236,066.89\",20261001000123456789
10/02/2026,WIRE IN FROM 026009593 ACCT 4417123456789,250000.00,1486066.89,FT26275987814321
10/03/2026,POS PURCHASE 2026-10-03 12345.67 MERCHANT 801-555-1234,-12.50,1486054.39,1002345678
10/04/2026 021000021,ZELLE TO 5551234567 CONF 1234-5678-9012,-40.00,1486014.39,000123456789012
10/05/2026,CARD 4111 1111 1111 1111 TRACE 0210000210012345,-9.99,1486004.40,2026100512.50
10/06/2026,CHECK 1042 SEQ 026 00 9593,-100.00,1485904.40,ROUTING 228620358
10/07/2026,INTEREST 2050 2026-01-31,0.42,1485904.82,84101-1234
";
        for (bytes, mime, name) in encodings(csv) {
            assert!(check(&bytes, mime, name).is_ok(), "{name}");
            assert!(check_held(&bytes, mime, name).is_ok(), "{name}, vault open");
        }
        assert!(check(b"date,amount\n2026-10-01,12.50\n", "text/csv", "bank.csv").is_ok());
        assert!(!dir("check").exists(), "a check never creates the registry");
    }

    /// INVARIANT 3, failing closed: a registry that is there but can't be
    /// read refuses every file, saying so.
    #[test]
    fn an_unreadable_registry_refuses_every_file() {
        let vault = dir("unreadable");
        std::fs::create_dir_all(vault.join(FILE_NAME)).unwrap();
        let why = refuse(
            &vault,
            b"%PDF a receipt",
            "application/pdf",
            "r.pdf",
            Vec::new,
            Vec::new,
        )
        .expect_err("a registry that can't be read let a file in");
        assert!(why.contains("couldn't check the file"), "{why}");
        assert!(record(&vault, b"bytes").is_err());
        let _ = std::fs::remove_dir_all(&vault);
    }

    /// INVARIANT 4: text holding more than two TIN-shaped numbers ending
    /// in a known last four is refused before the vault is opened, with
    /// the same words whether one of them is the vault's TIN or none is,
    /// whether the vault is open or locked, however the text is encoded.
    #[test]
    fn text_probing_for_a_tin_is_refused_alike_whether_or_not_it_holds_it() {
        let no_vault = || -> Vec<ledger_vault::Tin> { panic!("the vault was opened for a probe") };
        for (with_tin, without) in [
            // Bare, as an SSN, as an EIN, and in each grouping.
            (
                "291122846\n806812846\n832582846\n",
                "348072846\n806812846\n832582846\n",
            ),
            (
                "291-12-2846,806-81-2846,832-58-2846",
                "348-07-2846,806-81-2846,832-58-2846",
            ),
            (
                "20-5200615 10-0000615 38-9500615",
                "31-8640615 10-0000615 38-9500615",
            ),
            (
                "291 12 2846;80 6812846;832.58.2846",
                "348 07 2846;80 6812846;832.58.2846",
            ),
            // Across both last fours.
            (
                "205200615,806812846,100000615",
                "389500615,806812846,100000615",
            ),
            // Inside one long run of digits, and in Unicode's forms.
            (
                "ref 291122846832582846348072846",
                "ref 806812846832582846348072846",
            ),
            (
                "\u{FF12}\u{FF19}\u{FF11}\u{FF11}\u{FF12}\u{FF12}\u{FF18}\u{FF14}\u{FF16} 806\u{2013}81\u{2011}2846 83\u{2212}2582846",
                "348\u{A0}07\u{3000}2846 806\u{2013}81\u{2011}2846 83\u{2212}2582846",
            ),
        ] {
            for text in [with_tin, without] {
                for (bytes, mime, name) in encodings(text) {
                    let refusals = [
                        refuse(&dir("check"), &bytes, mime, name, known, vault),
                        refuse(&dir("check"), &bytes, mime, name, known, no_vault),
                        // The vault locked.
                        refuse(&dir("check"), &bytes, mime, name, known, Vec::new),
                    ];
                    for r in refusals {
                        assert_eq!(r, Err(TIN_PROBE.into()), "{text:?} as {name}");
                    }
                }
            }
            // Byte for byte the same answer.
            assert_eq!(
                check_held(with_tin.as_bytes(), "text/csv", "x.csv")
                    .unwrap_err()
                    .as_bytes(),
                check_held(without.as_bytes(), "text/csv", "x.csv")
                    .unwrap_err()
                    .as_bytes(),
            );
        }
        // A vault TIN whose last four the books don't know: once the vault
        // is opened, its last four counts too.
        for text in [
            "291122846 806812846 832582846",
            "348072846 806812846 832582846",
        ] {
            assert_eq!(
                refuse(
                    &dir("check"),
                    text.as_bytes(),
                    "text/csv",
                    "x.csv",
                    Vec::new,
                    vault
                ),
                Err(TIN_PROBE.into()),
                "{text:?}"
            );
        }
    }

    /// INVARIANT 4: up to two candidates are compared against the vault:
    /// none of them the TIN goes in, one that is is refused as a held TIN.
    /// The same number written many ways counts once, and numbers ending
    /// in another last four never count.
    #[test]
    fn one_or_two_candidates_are_compared_against_the_vault() {
        for (text, held) in [
            ("806812846", false),
            ("291122846", true),
            ("806812846,832582846", false),
            ("806812846,291-12-2846", true),
            ("205200615 806812846", true),
            ("100000615 806812846", false),
            // One number, written four ways (the printed shapes are (b)'s).
            ("806812846 806 81 2846 80 6812846 806.81.2846", false),
            (
                "291122846 291-12-2846 291 12 2846 29-1122846 291.12.2846",
                true,
            ),
            // Other last fours, many of them.
            (
                "100001111 100002222 100003333 100004444 100005555 806812846",
                false,
            ),
            (
                "100001111 100002222 100003333 100004444 100005555 291122846",
                true,
            ),
        ] {
            let want = if held { Err(TIN_HELD.into()) } else { Ok(()) };
            for (bytes, mime, name) in encodings(text) {
                assert_eq!(check_held(&bytes, mime, name), want, "{text:?} as {name}");
            }
            // The vault's last fours alone do the same.
            assert_eq!(
                refuse(
                    &dir("check"),
                    text.as_bytes(),
                    "text/csv",
                    "x.csv",
                    Vec::new,
                    vault
                ),
                want,
                "{text:?}, the books knowing no last four"
            );
        }
    }

    /// INVARIANT 3: binary bytes aren't text, so they aren't scanned.
    #[test]
    fn binary_bytes_are_not_scanned() {
        let mut bytes = b"name,tin\nCaf\xe9,291-12-2846\n".to_vec();
        bytes.push(0xFF);
        assert!(check_held(&bytes, "application/pdf", "scan.pdf").is_ok());
        assert_eq!(
            check_held(&bytes, "text/csv", "scan.csv"),
            Err(TIN_HELD.into())
        );
    }

    /// UTF-16 with a byte-order mark is read as UTF-16 whatever it holds,
    /// NULs or not: text of full-width digits and CJK has no NUL byte at
    /// all, either way round, and is still scanned.
    #[test]
    fn utf16_with_a_mark_is_read_whatever_it_holds() {
        let text = "\u{652F}\u{6255}\u{FF12}\u{FF19}\u{FF11}\u{2011}\u{FF11}\u{FF12}\u{2011}\u{FF12}\u{FF18}\u{FF14}\u{FF16}";
        for le in [true, false] {
            let mut bytes: Vec<u8> = if le {
                vec![0xFF, 0xFE]
            } else {
                vec![0xFE, 0xFF]
            };
            bytes.extend(
                text.encode_utf16()
                    .flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }),
            );
            assert!(!bytes.contains(&0), "no NUL to find it by");
            assert_eq!(
                check_held(&bytes, "application/octet-stream", "x.bin"),
                Err(TIN_HELD.into()),
                "le {le}"
            );
        }
    }

    /// NULs scattered through text (no UTF-16 at all) don't hide a TIN:
    /// the text is also read with them taken out.
    #[test]
    fn nuls_scattered_in_text_hide_no_tin() {
        let bytes = b"payee,tin\nDana,634\x00-28-1907\n";
        assert_eq!(check(bytes, "text/csv", "p.csv"), Err(TIN_PRINTED.into()));
        assert!(check(b"payee,note\nDana,\x00none\n", "text/csv", "p.csv").is_ok());
    }

    /// Binary bytes with NULs scattered through them (a PDF, an image) are
    /// not read as UTF-16, and neither are bytes that are mostly NULs on
    /// both sides of each pair; NUL-interleaved bytes are, either way.
    #[test]
    fn only_nul_interleaved_bytes_read_as_utf16_without_a_mark() {
        let mut pdf = b"%PDF-1.4\n\x00\x00\x00\x00 291-12-2846 stream\n".to_vec();
        pdf.push(0xFF);
        assert!(!nul_interleaved(&pdf));
        assert!(check(&pdf, "application/pdf", "scan.pdf").is_ok());
        let mut zeros = vec![0u8; 64];
        zeros.extend_from_slice(b"291-12-2846");
        zeros.push(0xFF);
        assert!(!nul_interleaved(&zeros));
        assert!(check(&zeros, "image/bmp", "scan.bmp").is_ok());
        assert!(!nul_interleaved(b""));
        assert!(!nul_interleaved(b"\x00"));
        let le: Vec<u8> = "x,291-12-2846"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let be: Vec<u8> = "x,291-12-2846"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        for bytes in [&le, &be] {
            assert!(nul_interleaved(bytes));
            assert_eq!(
                check(bytes, "application/octet-stream", "x.bin"),
                Err(TIN_PRINTED.into())
            );
        }
        // Exactly at the bounds: half the pairs NUL on one side, an eighth
        // on the other.
        let mut edge = Vec::new();
        for i in 0..16 {
            edge.push(b'a');
            edge.push(if i < 8 { 0 } else { b'b' });
        }
        assert!(nul_interleaved(&edge));
        edge[1] = b'b';
        assert!(!nul_interleaved(&edge), "fewer than half");
        let mut other = Vec::new();
        for i in 0..16 {
            other.push(if i < 2 { 0 } else { b'c' });
            other.push(0);
        }
        assert!(nul_interleaved(&other), "an eighth on the other side");
        other[4] = 0;
        assert!(!nul_interleaved(&other), "more than an eighth");
        let mut be_edge = Vec::new();
        for i in 0..16 {
            be_edge.push(if i < 8 { 0 } else { b'b' });
            be_edge.push(b'a');
        }
        assert!(nul_interleaved(&be_edge));
        // Half NUL on the first side, three of sixteen (over an eighth) on
        // the other: not UTF-16.
        let mut mixed = Vec::new();
        for i in 0..16 {
            mixed.push(if i < 8 { 0 } else { b'x' });
            mixed.push(if (8..11).contains(&i) { 0 } else { b'y' });
        }
        assert!(!nul_interleaved(&mixed));
        mixed[17] = b'y';
        mixed[19] = b'y';
        assert!(nul_interleaved(&mixed), "one of sixteen is under an eighth");
    }
}
