//! The parity catalog's floor, machine-checked (docs/parity.md;
//! AGENTS.md, "Parity tracking").
//!
//! Every table with a `Status` column must hold only `done`, `partial` or
//! `todo`, and the number of `done` rows must equal the catalog's
//! `**Floor: N done**` line exactly. Equal, not at least: adding a done row
//! raises the floor in the same commit, so a later drop is always caught.
//! The history half (the floor never falls between commits) is
//! `scripts/parity-floor.sh`, run by CI before `just check`.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn catalog() -> String {
    let path = root().join("docs/parity.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The only statuses the catalog allows (docs/parity.md:
/// `done` = implemented + invariant-tested + golden-covered,
/// `partial` = implemented but missing tests or goldens, `todo` = not
/// started).
const STATUSES: [&str; 3] = ["done", "partial", "todo"];

#[derive(Debug)]
struct Row {
    area: String,
    status: String,
}

/// A markdown table row's cells, trimmed, without the outer pipes.
fn cells(line: &str) -> Vec<&str> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(str::trim)
        .collect()
}

/// The `**Floor: N done**` line, or `None` when the file has none.
fn parse_floor(text: &str) -> Option<u32> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("**Floor: ") else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let (Some(number), Some(marker)) = (words.next(), words.next()) else {
            continue;
        };
        if marker != "done**" {
            continue;
        }
        return number.parse().ok();
    }
    None
}

/// Every data row of every table whose header has a `Status` column.
/// Errors name the row's first cell and the bad status.
fn parse_tables(text: &str) -> Result<Vec<Row>, Vec<String>> {
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let mut status_col: Option<usize> = None;

    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if !line.trim_start().starts_with('|') {
            // A table ends at the first non-table line.
            status_col = None;
            continue;
        }
        let parts = cells(line);
        let separator = parts
            .iter()
            .all(|cell| !cell.is_empty() && cell.chars().all(|c| c == '-' || c == ':'));
        if separator {
            continue;
        }
        let Some(col) = status_col else {
            if let Some(col) = parts.iter().position(|cell| *cell == "Status") {
                status_col = Some(col);
            }
            continue;
        };
        let area = parts.first().copied().unwrap_or_default().to_string();
        match parts.get(col).copied() {
            None => errors.push(format!(
                "docs/parity.md:{line_no}: row `{area}` has no status cell"
            )),
            Some("") => errors.push(format!(
                "docs/parity.md:{line_no}: row `{area}` has an empty status"
            )),
            Some(status) if !STATUSES.contains(&status) => errors.push(format!(
                "docs/parity.md:{line_no}: row `{area}` has status `{status}`, \
                 not one of done, partial, todo"
            )),
            Some(status) => rows.push(Row {
                area,
                status: status.to_string(),
            }),
        }
    }
    if errors.is_empty() {
        Ok(rows)
    } else {
        Err(errors)
    }
}

#[test]
fn every_row_has_a_known_status() {
    let text = catalog();
    if let Err(errors) = parse_tables(&text) {
        panic!(
            "docs/parity.md's statuses must be exactly done, partial or todo:\n{}",
            errors.join("\n")
        );
    }
}

#[test]
fn the_done_count_is_the_floor() {
    let text = catalog();
    let rows = parse_tables(&text).unwrap_or_else(|errors| {
        panic!(
            "docs/parity.md's statuses must be exactly done, partial or todo:\n{}",
            errors.join("\n")
        )
    });
    let done: Vec<&str> = rows
        .iter()
        .filter(|row| row.status == "done")
        .map(|row| row.area.as_str())
        .collect();
    let floor = parse_floor(&text).unwrap_or_else(|| {
        panic!(
            "docs/parity.md has no `**Floor: N done**` line; restore it \
             (docs/parity.md's intro and AGENTS.md, \"Parity tracking\")"
        )
    });
    if done.len() > floor as usize {
        panic!(
            "docs/parity.md has {} `done` rows but its floor line says {floor}. \
             Raise the line to `**Floor: {} done**` in this commit: adding a \
             done row raises the floor in the same commit, so a later drop is \
             always caught.\nDone rows:\n  {}",
            done.len(),
            done.len(),
            done.join("\n  ")
        );
    }
    if done.len() < floor as usize {
        panic!(
            "docs/parity.md has {} `done` rows but its floor line says {floor}. \
             Restore the missing row: the floor never decreases, and lowering \
             it needs the owner's decision.\nDone rows:\n  {}",
            done.len(),
            done.join("\n  ")
        );
    }
}

#[test]
fn a_bad_status_names_its_row() {
    let text = "| Area | Status | Notes |\n|---|---|---|\n| Ledger | finished | — |\n";
    let errors = parse_tables(text).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("Ledger"), "{errors:?}");
    assert!(errors[0].contains("finished"), "{errors:?}");
}

#[test]
fn a_missing_status_cell_is_an_error() {
    let text = "| Area | Status | Notes |\n|---|---|---|\n| Ledger | | — |\n";
    let errors = parse_tables(text).unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("Ledger"), "{errors:?}");
}

#[test]
fn a_missing_floor_line_is_none() {
    assert_eq!(parse_floor("# Parity catalog\n\nNo floor here.\n"), None);
    assert_eq!(
        parse_floor("**Floor: 7 done** (2026-01-01; the sections below record each rise)"),
        Some(7)
    );
}
