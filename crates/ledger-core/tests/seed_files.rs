//! Recorded failures stay replayed (AGENTS.md, "Verification protocol").
//!
//! When a property test fails, proptest writes the failing case's seed to a
//! file and replays every seed in that file before it tries new cases. It
//! finds the file from the test's own source path, and says nothing when
//! there is none. So a seed file whose test was moved or renamed is never
//! read again, and the failures it records stop being checked without any
//! test going red.
//!
//! That happened here: when the kernel bridge became a folder of modules
//! (`tests/kernel_bridge.rs` to `tests/kernel_bridge/main.rs`), its three
//! recorded failures stayed in `tests/kernel_bridge.proptest-regressions`,
//! which no test read any more (found 2026-11-09, by a marker line in each
//! candidate file and proptest's "unparsable line" warning naming the one
//! it opened).
//!
//! The two places proptest reads, as it resolves them:
//!
//! - `D/X.proptest-regressions` is read by the tests in `D/X.rs` (a test
//!   file with no `lib.rs` or `main.rs` above it).
//! - `D/proptest-regressions/R.txt` is read by the tests in `D/C/R.rs`,
//!   where `C` is a folder under `D` that holds a `lib.rs` or a `main.rs`
//!   (a crate's `src`, or a test that is a folder of modules).
//!
//! Every seed file under `crates/` must have that reader.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Every file under `dir`, skipping build output.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                files(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

/// The source files whose tests read `seeds` (more than one when two
/// tests that are folders hold a module of the same name: they share the
/// file), or why there is none.
fn readers(seeds: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = seeds.parent().expect("a file has a folder");
    if seeds
        .extension()
        .is_some_and(|e| e == "proptest-regressions")
    {
        let source = seeds.with_extension("rs");
        return if source.is_file() {
            Ok(vec![source])
        } else {
            Err(format!("no test file {}", source.display()))
        };
    }
    // A file inside a `proptest-regressions` folder: find the folder, then
    // every sibling of it that is a crate or test root holding the same path.
    let mut folder = dir;
    while folder
        .file_name()
        .is_some_and(|n| n != "proptest-regressions")
    {
        folder = folder
            .parent()
            .ok_or("not inside a proptest-regressions folder")?;
    }
    let rel = seeds
        .strip_prefix(folder)
        .expect("the folder is an ancestor")
        .with_extension("rs");
    let above = folder
        .parent()
        .ok_or("a proptest-regressions folder at the root")?;
    let siblings = std::fs::read_dir(above).map_err(|e| format!("{}: {e}", above.display()))?;
    let mut found = Vec::new();
    for sibling in siblings {
        let sibling = sibling.unwrap().path();
        let is_root = sibling.join("lib.rs").is_file() || sibling.join("main.rs").is_file();
        if is_root && sibling.join(&rel).is_file() {
            found.push(sibling.join(&rel));
        }
    }
    if found.is_empty() {
        return Err(format!(
            "no folder beside {} holds a lib.rs or main.rs and a {}",
            folder.display(),
            rel.display()
        ));
    }
    Ok(found)
}

#[test]
fn every_recorded_failure_has_a_test_that_replays_it() {
    let root = root();
    let mut all = Vec::new();
    files(&root.join("crates"), &mut all);
    let seeds: Vec<&PathBuf> = all
        .iter()
        .filter(|p| {
            p.extension().is_some_and(|e| e == "proptest-regressions")
                || p.components()
                    .any(|c| c.as_os_str() == "proptest-regressions")
        })
        .collect();
    // The floor: a walk that finds nothing would pass for the wrong reason.
    assert!(
        seeds.len() >= 20,
        "found only {} seed files under crates/; the walk is wrong",
        seeds.len()
    );
    let mut orphans = Vec::new();
    for path in &seeds {
        let shown = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        match readers(path) {
            Ok(sources) => {
                let tested = sources
                    .iter()
                    .any(|s| std::fs::read_to_string(s).unwrap().contains("proptest!"));
                if !tested {
                    let names: Vec<String> = sources
                        .iter()
                        .map(|s| s.strip_prefix(&root).unwrap_or(s).display().to_string())
                        .collect();
                    orphans.push(format!(
                        "{shown}: {} holds no property test",
                        names.join(", ")
                    ));
                }
            }
            Err(why) => orphans.push(format!("{shown}: {why}")),
        }
    }
    assert!(
        orphans.is_empty(),
        "seed files no test reads (move each beside the test that owns its cases):\n{}",
        orphans.join("\n")
    );
    println!("{} seed files, each read by a property test", seeds.len());
}
