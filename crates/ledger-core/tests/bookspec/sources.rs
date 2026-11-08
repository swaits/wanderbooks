use crate::*;

/// Whitespace and typographic quotes made plain, so a pin copied from a
/// wrapped or curly-quoted source still matches.
pub(crate) fn squash(s: &str) -> String {
    let plain = s
        .replace(['\u{2019}', '\u{2018}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"");
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every vendored citation in the specification names a file in
/// `docs/spec/sources`, and each quoted pin is verbatim in it. (The
/// specification crate checks the citations' form; the vendored texts live
/// here.)
#[test]
pub(crate) fn every_vendored_pin_is_verbatim() {
    let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/sources");
    let mut vendored = 0;
    let mut bad = Vec::new();
    for (name, text) in bookspec_checker::spec_files() {
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("  - vendored `") else {
                continue;
            };
            let (file, rest) = rest
                .split_once("`:")
                .unwrap_or_else(|| panic!("{name}: {line}"));
            let path = sources.join(file.strip_prefix("sources/").unwrap_or(file));
            let hay = squash(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|_| panic!("{name}: {file} is not vendored")),
            );
            for q in rest.split('"').skip(1).step_by(2) {
                if !hay.contains(&squash(q)) {
                    bad.push(format!("{name}: {q:?} is not in {file}"));
                }
            }
            vendored += 1;
        }
    }
    assert!(bad.is_empty(), "pins not found:\n{}", bad.join("\n"));
    assert!(vendored >= 30, "vendored citations: {vendored}");
}
