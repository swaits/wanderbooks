//! The traceability gate (P14 unit X-J).
//!
//! A claim is worth what backs it. This gate reads every section of
//! `docs/spec/claims/*.md` whose claim is `yes` or `partly` and follows
//! each backticked name in its **Enforced** and **Checked** paragraphs to
//! something real:
//!
//! The scanner below mirrors the grammar it reads (a `use` line, a
//! `proptest!` block, a markdown field), so its nested `if let` blocks are
//! kept for the shape they name rather than collapsed into one condition.
#![allow(clippy::collapsible_if)]
//!
//! - a Rust item defined under `crates/*/src` (a `fn`, method, type,
//!   enum variant, field, `const`, `static`, module, trait or macro);
//! - a test function under `crates/*/tests` (or a `#[test]` / proptest
//!   in `crates/*/src`), a fuzz target, or a `#[kani::proof]` harness;
//! - a Lean theorem in bookspec's `Audit.lean`;
//! - a path that exists;
//! - a bookspec rule or a wanderbooks product rule that exists.
//!
//! Tokens that are plainly values (a quoted option word, a setting value,
//! a journal event, a tool name, an illustrative amount) are classified by
//! narrow, explicit rules below and counted, so the gate says how much it
//! actually resolved. Every `**bookspec:**` id must be a live record (a
//! retired one is written `**retired bookspec:**`); every `WB-` id is
//! unique.
//!
//! It also checks every `pub fn` in `crates/ledger-core/src/kernel/*.rs`:
//! a Kani harness calls it, a bridge property in `tests/kernel_bridge/`
//! calls it, and one of its harnesses carries a numbered law that is a
//! row of bookspec's `spec/laws.md`. The functions that are wrappers or
//! enum maps with no independent harness, bridge or law are named in
//! [`EXCEPTIONS`], with what each lacks and why; its length is pinned so
//! the list changes only on purpose.
//!
//! Finally, every law row of `spec/laws.md` that cites a rule wanderbooks
//! claims names Lean theorems that are all audited, or says `none —` with
//! a reason.

use crate::*;

/// The classes a backticked token can resolve to. Reported by count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    RustItem,
    Test,
    FuzzTarget,
    KaniHarness,
    LeanTheorem,
    Path,
    Crate,
    RuleId,
    ProductSurface,
    StringValue,
    OptionValue,
    NonName,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::RustItem => "rust item",
            Class::Test => "test",
            Class::FuzzTarget => "fuzz target",
            Class::KaniHarness => "kani harness",
            Class::LeanTheorem => "lean theorem",
            Class::Path => "path",
            Class::Crate => "crate",
            Class::RuleId => "rule id",
            Class::ProductSurface => "product surface",
            Class::StringValue => "string value",
            Class::OptionValue => "option/example value",
            Class::NonName => "plainly not a name",
        }
    }
}

const CLAIMS_FLOOR: usize = 120;
const TOKENS_FLOOR: usize = 1700;
const KERNEL_FLOOR: usize = 50;
const PROPTEST: [char; 9] = ['p', 'r', 'o', 'p', 't', 'e', 's', 't', '!'];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            walk(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// The source with comments and string/char literals blanked, so a word in
/// a comment or a string is never mistaken for a definition.
fn strip_code(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            let mut depth = 1usize;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if chars[i] == '"' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                } else if chars[i] == '"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
        } else if chars[i] == '\'' {
            // A char literal or a lifetime; only skip a literal with a
            // closing quote in the next few characters.
            let close = chars
                .iter()
                .enumerate()
                .skip(i + 1)
                .take(4)
                .find(|(_, c)| **c == '\'');
            if let Some((j, _)) = close {
                i = j + 1;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// The contents of every simple string literal.
fn literals(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() {
                if chars[i] == '\\' {
                    if let Some(c) = chars.get(i + 1) {
                        s.push(*c);
                    }
                    i += 2;
                } else if chars[i] == '"' {
                    i += 1;
                    break;
                } else {
                    s.push(chars[i]);
                    i += 1;
                }
            }
            out.push(s);
        } else {
            i += 1;
        }
    }
    out
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn word_at(chars: &[char], i: usize) -> Option<(String, usize)> {
    if i >= chars.len() || !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
        return None;
    }
    let mut j = i;
    while j < chars.len() && is_word_char(chars[j]) {
        j += 1;
    }
    Some((chars[i..j].iter().collect(), j))
}

fn skip_space(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

fn matching_brace(chars: &[char], open: usize, pair: (char, char)) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        if chars[i] == pair.0 {
            depth += 1;
        } else if chars[i] == pair.1 {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// `(start, end_inclusive, type name)` of every `impl` block.
fn impl_ranges(code: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
            i += 1;
            continue;
        }
        let Some((word, end)) = word_at(&chars, i) else {
            i += 1;
            continue;
        };
        if word != "impl" {
            i = end;
            continue;
        }
        // The type is the last identifier before the opening brace, and
        // after `for` when it is a trait impl.
        let mut j = end;
        while j < chars.len() && chars[j] != '{' {
            j += 1;
        }
        if j >= chars.len() {
            break;
        }
        let header: String = chars[end..j].iter().collect();
        let segment = match header.rsplit_once(" for ") {
            Some((_, after)) => after,
            None => &header,
        };
        let mut last = String::new();
        let mut k = 0;
        while k < segment.len() {
            let rest: Vec<char> = segment[k..].chars().collect();
            match word_at(&rest, 0) {
                Some((w, used)) => {
                    last = w;
                    k += segment[k..]
                        .chars()
                        .take(used)
                        .map(char::len_utf8)
                        .sum::<usize>();
                }
                None => k += segment[k..].chars().next().map(char::len_utf8).unwrap_or(1),
            }
        }
        if let Some(close) = matching_brace(&chars, j, ('{', '}')) {
            let start = chars[..i].iter().map(|c| c.len_utf8()).sum::<usize>();
            let end = chars[..=close].iter().map(|c| c.len_utf8()).sum::<usize>();
            out.push((start, end, last));
        }
        i = end;
    }
    out
}

/// Every top-level item of a brace block, split on commas.
fn top_items(body: &str) -> Vec<String> {
    let chars: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for &c in &chars {
        match c {
            '(' | '[' | '{' | '<' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// The leading identifier of an item, after any `pub(...)`.
fn leading_ident(item: &str) -> Option<String> {
    let chars: Vec<char> = item.chars().collect();
    let mut i = 0;
    loop {
        i = skip_space(&chars, i);
        if chars.get(i) == Some(&'#') {
            let mut j = i + 1;
            if chars.get(j) == Some(&'!') {
                j += 1;
            }
            if chars.get(j) == Some(&'[') {
                i = matching_brace(&chars, j, ('[', ']'))? + 1;
                continue;
            }
            return None;
        }
        break;
    }
    let (first, end) = word_at(&chars, i)?;
    if first == "pub" {
        i = end;
        if chars.get(i) == Some(&'(') {
            i = matching_brace(&chars, i, ('(', ')'))? + 1;
        }
        i = skip_space(&chars, i);
    }
    word_at(&chars, i).map(|(w, _)| w)
}

fn find_named_item(code: &str, keyword: &str, name: &str) -> Option<usize> {
    let needle = format!("{keyword} {name}");
    let mut at = 0;
    while let Some(rel) = code[at..].find(&needle) {
        let pos = at + rel;
        let after = pos + needle.len();
        let boundary = code[after..]
            .chars()
            .next()
            .is_none_or(|c| !is_word_char(c));
        let before_ok = pos == 0 || !code[..pos].chars().next_back().is_some_and(is_word_char);
        if boundary && before_ok {
            return Some(pos);
        }
        at = pos + 1;
    }
    None
}

fn enum_variants(code: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = code.chars().collect();
    let Some(at) = find_named_item(code, "enum", name) else {
        return out;
    };
    let mut i = code[..at].chars().count();
    while i < chars.len() && chars[i] != '{' {
        i += 1;
    }
    let Some(close) = matching_brace(&chars, i, ('{', '}')) else {
        return out;
    };
    let body: String = chars[i + 1..close].iter().collect();
    for item in top_items(&body) {
        if let Some(v) = leading_ident(&item) {
            if v.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                out.push(v);
            }
        }
    }
    out
}

fn struct_fields(code: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = code.chars().collect();
    let Some(at) = find_named_item(code, "struct", name) else {
        return out;
    };
    let mut i = code[..at].chars().count();
    while i < chars.len() && chars[i] != '{' {
        i += 1;
    }
    let Some(close) = matching_brace(&chars, i, ('{', '}')) else {
        return out;
    };
    let body: String = chars[i + 1..close].iter().collect();
    for item in top_items(&body) {
        let Some(v) = leading_ident(&item) else {
            continue;
        };
        let after = item.find(&v).map(|p| &item[p + v.len()..]).unwrap_or("");
        if after.trim_start().starts_with(':') {
            out.push(v);
        }
    }
    out
}

/// Enum variants and struct fields in `code`, for definitions outside the
/// crate's `src` (an integration test's own model types).
fn collect_types(code: &str, items: &mut BTreeSet<String>, bare: &mut BTreeSet<String>) {
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
            i += 1;
            continue;
        }
        let Some((word, end)) = word_at(&chars, i) else {
            i += 1;
            continue;
        };
        if word == "enum" || word == "struct" {
            let j = skip_space(&chars, end);
            if let Some((name, _)) = word_at(&chars, j) {
                let found = if word == "enum" {
                    enum_variants(code, &name)
                } else {
                    struct_fields(code, &name)
                };
                for item in found {
                    items.insert(item.clone());
                    bare.insert(item);
                }
            }
        }
        i = end;
    }
}

/// The names a `use` statement imports.
fn collect_imports(code: &str, items: &mut BTreeSet<String>) {
    for line in code.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("use ") else {
            continue;
        };
        let decl = rest.split(';').next().unwrap_or(rest);
        let mut word = String::new();
        for c in decl.chars().chain(std::iter::once(' ')) {
            if c.is_ascii_alphanumeric() || c == '_' {
                word.push(c);
            } else {
                if !word.is_empty()
                    && !matches!(
                        word.as_str(),
                        "use" | "crate" | "self" | "super" | "as" | "pub"
                    )
                {
                    items.insert(word.clone());
                }
                word.clear();
            }
        }
    }
}

#[derive(Default)]
struct Definitions {
    /// Every Rust item name, bare or `Type::method`.
    items: BTreeSet<String>,
    bare: BTreeSet<String>,
    /// Test and property names under `crates/*/tests`, `#[test]` in src,
    /// and proptest functions.
    tests: BTreeSet<String>,
    /// `#[kani::proof]` functions under `src/proofs/`.
    harnesses: BTreeSet<String>,
    /// Fuzz targets under `fuzz/fuzz_targets/`.
    fuzz: BTreeSet<String>,
    /// Proof source file stems.
    proof_files: BTreeSet<String>,
    /// Every string literal in `crates/*` source.
    literals: BTreeSet<String>,
    /// All crate source text, for product-surface names.
    surface: String,
    /// Rules by id and whether the record is retired.
    rules: BTreeMap<String, bool>,
    /// Product rule ids (`WB-...`) and their count.
    product_rules: BTreeMap<String, usize>,
    /// Bookspec option names.
    options: BTreeSet<String>,
    /// Bookspec example file names.
    examples: Vec<String>,
    /// Bookspec Lean files, path under `lean/`.
    lean_files: BTreeSet<String>,
}

fn collect_function_bodies(code: &str, out: &mut Vec<(String, String)>) {
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
            i += 1;
            continue;
        }
        let Some((word, end)) = word_at(&chars, i) else {
            i += 1;
            continue;
        };
        if word != "fn" {
            i = end;
            continue;
        }
        let j = skip_space(&chars, end);
        let Some((name, name_end)) = word_at(&chars, j) else {
            i = end;
            continue;
        };
        let mut k = name_end;
        // Skip the parameter list, then find the body.
        if chars.get(k) == Some(&'<') {
            if let Some(gt) = matching_brace(&chars, k, ('<', '>')) {
                k = gt + 1;
            }
        }
        k = skip_space(&chars, k);
        if chars.get(k) == Some(&'(') {
            if let Some(close) = matching_brace(&chars, k, ('(', ')')) {
                k = close + 1;
            }
        }
        while k < chars.len() && chars[k] != '{' && chars[k] != ';' {
            k += 1;
        }
        if chars.get(k) == Some(&'{') {
            if let Some(close) = matching_brace(&chars, k, ('{', '}')) {
                let body: String = chars[k + 1..close].iter().collect();
                out.push((name, body));
            }
        }
        i = end;
    }
}

fn build_definitions(root: &Path) -> Definitions {
    let mut defs = Definitions::default();

    let mut src_files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("crates") {
        let crate_dir = entry.expect("a crate").path();
        let src = crate_dir.join("src");
        if src.is_dir() {
            walk(&src, "rs", &mut src_files);
        }
    }
    src_files.sort();
    let mut enums = Vec::new();
    let mut structs = Vec::new();
    for path in &src_files {
        let text = read(path);
        let code = strip_code(&text);
        defs.surface.push_str(&text);
        defs.surface.push('\n');
        for literal in literals(&text) {
            defs.literals.insert(literal);
        }
        collect_imports(&code, &mut defs.items);
        // Keywords and names.
        let chars: Vec<char> = code.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
                i += 1;
                continue;
            }
            let Some((word, end)) = word_at(&chars, i) else {
                i += 1;
                continue;
            };
            match word.as_str() {
                "fn" | "struct" | "enum" | "union" | "trait" | "type" | "mod" | "const"
                | "static" => {
                    let j = skip_space(&chars, end);
                    if let Some((name, _)) = word_at(&chars, j) {
                        if word == "enum" {
                            enums.push((path.clone(), name.clone()));
                        }
                        if word == "struct" {
                            structs.push((path.clone(), name.clone()));
                        }
                        defs.items.insert(name.clone());
                        defs.bare.insert(name);
                    }
                }
                "macro_rules" => {
                    let mut j = skip_space(&chars, end);
                    if chars.get(j) == Some(&'!') {
                        j = skip_space(&chars, j + 1);
                        if let Some((name, _)) = word_at(&chars, j) {
                            defs.items.insert(name.clone());
                            defs.bare.insert(name);
                        }
                    }
                }
                _ => {}
            }
            i = end;
        }
        // Methods, qualified by their impl.
        for (start, end, ty) in impl_ranges(&code) {
            let block = &code[start..end];
            let mut fns = Vec::new();
            collect_pub_fns(block, &mut fns);
            for (name, _) in fns {
                let qualified = format!("{ty}::{name}");
                defs.items.insert(qualified);
                defs.items.insert(name.clone());
                defs.bare.insert(name);
            }
        }
    }
    for (path, name) in enums {
        let code = strip_code(&read(&path));
        for variant in enum_variants(&code, &name) {
            defs.items.insert(variant.clone());
            defs.bare.insert(variant);
        }
    }
    for (path, name) in structs {
        let code = strip_code(&read(&path));
        for field in struct_fields(&code, &name) {
            defs.items.insert(field.clone());
            defs.bare.insert(field);
        }
    }

    // Tests: every fn under `tests/`, and `#[test]`/proptest in src.
    let mut test_files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("crates") {
        let dir = entry.expect("a crate").path().join("tests");
        if dir.is_dir() {
            walk(&dir, "rs", &mut test_files);
        }
    }
    test_files.sort();
    for path in &test_files {
        let code = strip_code(&read(path));
        collect_types(&code, &mut defs.items, &mut defs.bare);
        collect_imports(&code, &mut defs.items);
        let mut fns = Vec::new();
        collect_function_bodies(&code, &mut fns);
        for (name, _) in fns {
            defs.tests.insert(name);
        }
    }
    for path in &src_files {
        let text = read(path);
        let code = strip_code(&text);
        let chars: Vec<char> = code.chars().collect();
        // `#[test]` functions.
        for (name, _) in collect_bodies_with_attribute(&code, "#[test]") {
            defs.tests.insert(name);
        }
        // Proptest blocks.
        let mut i = 0;
        while i < chars.len() {
            if chars[i..].starts_with(&PROPTEST) {
                if let Some(open) = code[i..].find('{') {
                    let at = i + open;
                    if let Some(close) = matching_brace(&chars, at, ('{', '}')) {
                        let block: String = chars[at + 1..close].iter().collect();
                        let mut fns = Vec::new();
                        collect_function_bodies(&block, &mut fns);
                        for (name, _) in fns {
                            defs.tests.insert(name);
                        }
                        i = close;
                        continue;
                    }
                }
            }
            i += 1;
        }
    }

    // Kani harnesses and proof files.
    let proofs = root.join("crates/ledger-core/src/proofs");
    let mut proof_files = Vec::new();
    walk(&proofs, "rs", &mut proof_files);
    proof_files.sort();
    for path in &proof_files {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            defs.proof_files.insert(stem.to_string());
        }
        let code = strip_code(&read(path));
        for (name, _) in collect_bodies_with_attribute(&code, "#[kani::proof]") {
            defs.harnesses.insert(name);
        }
    }

    // Fuzz targets.
    let fuzz = root.join("fuzz/fuzz_targets");
    let mut fuzz_files = Vec::new();
    walk(&fuzz, "rs", &mut fuzz_files);
    for path in &fuzz_files {
        let code = strip_code(&read(path));
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            defs.fuzz.insert(stem.to_string());
        }
        let mut fns = Vec::new();
        collect_function_bodies(&code, &mut fns);
        for (name, _) in fns {
            defs.fuzz.insert(name);
        }
    }

    // Bookspec rules, options, examples and Lean files.
    for record in bookspec_checker::records() {
        defs.rules
            .insert(record.id.to_string(), record.is_retired());
        for option in &record.options {
            defs.options.insert(option.name.clone());
        }
    }
    for (name, _) in bookspec_checker::examples() {
        defs.examples.push(name.replace(".toml", ""));
    }
    for (name, _) in bookspec_checker::lean_files() {
        defs.lean_files.insert(name.to_string());
    }

    defs
}

/// `pub fn` names in an impl block, with their offsets (unused offsets are
/// kept for a future stricter check).
fn collect_pub_fns(block: &str, out: &mut Vec<(String, usize)>) {
    let chars: Vec<char> = block.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
            i += 1;
            continue;
        }
        let Some((word, end)) = word_at(&chars, i) else {
            i += 1;
            continue;
        };
        if word != "pub" {
            i = end;
            continue;
        }
        let mut j = skip_space(&chars, end);
        if chars.get(j) == Some(&'(') {
            if let Some(close) = matching_brace(&chars, j, ('(', ')')) {
                j = skip_space(&chars, close + 1);
            }
        }
        if let Some((kw, kw_end)) = word_at(&chars, j) {
            if kw == "fn" {
                let k = skip_space(&chars, kw_end);
                if let Some((name, _)) = word_at(&chars, k) {
                    out.push((name, i));
                }
            }
        }
        i = end;
    }
}

/// Functions preceded (within the previous attribute block) by `attr`.
fn collect_bodies_with_attribute(code: &str, attr: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut search = 0;
    while let Some(at) = code[search..].find(attr) {
        let start = search + at;
        let after = start + attr.len();
        let tail = &code[after..];
        let Some(fn_at) = tail.find("fn ") else {
            break;
        };
        // Nothing but attributes/whitespace may sit between them.
        let between = &tail[..fn_at];
        if between.contains('\n') {
            let only_attrs = between.lines().all(|line| {
                let line = line.trim();
                line.is_empty()
                    || line.starts_with("#[")
                    || line.starts_with("//")
                    || line.starts_with(']')
            });
            if !only_attrs {
                search = after;
                continue;
            }
        }
        let at = after + fn_at + 3;
        let chars: Vec<char> = code.chars().collect();
        let prefix_len = code[..at].chars().count();
        let j = skip_space(&chars, prefix_len);
        let Some((name, _)) = word_at(&chars, j) else {
            search = after;
            continue;
        };
        let mut k = j;
        while k < chars.len() && chars[k] != '{' && chars[k] != ';' {
            k += 1;
        }
        if chars.get(k) == Some(&'{') {
            if let Some(close) = matching_brace(&chars, k, ('{', '}')) {
                let body: String = chars[k + 1..close].iter().collect();
                out.push((name, body));
            }
        }
        search = after;
    }
    out
}

/// Every `pub fn` in `crates/ledger-core/src/kernel/*.rs`, qualified by
/// its impl (`Type::method`).
fn kernel_functions(root: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    walk(
        &root.join("crates/ledger-core/src/kernel"),
        "rs",
        &mut files,
    );
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let code = strip_code(&read(&path));
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let impls = impl_ranges(&code);
        let chars: Vec<char> = code.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if !(chars[i].is_ascii_alphabetic() || chars[i] == '_') {
                i += 1;
                continue;
            }
            let Some((word, end)) = word_at(&chars, i) else {
                i += 1;
                continue;
            };
            if word != "pub" {
                i = end;
                continue;
            }
            let j = skip_space(&chars, end);
            if chars.get(j) == Some(&'(') {
                // `pub(crate) fn` is not a `pub fn`.
                i = end;
                continue;
            }
            if let Some((kw, kw_end)) = word_at(&chars, j) {
                if kw == "fn" {
                    let k = skip_space(&chars, kw_end);
                    if let Some((name, _)) = word_at(&chars, k) {
                        let offset = code[..i].chars().count();
                        let ty = impls
                            .iter()
                            .find(|(start, end, _)| offset >= *start && offset < *end)
                            .map(|(_, _, ty)| ty.clone());
                        let qualified = match ty {
                            Some(ty) => format!("{ty}::{name}"),
                            None => name,
                        };
                        out.push((qualified, file.clone()));
                    }
                }
            }
            i = end;
        }
    }
    out
}

/// True when `body` calls `name`, bare or qualified, including a method
/// call written `.name(`.
fn calls(body: &str, name: &str) -> bool {
    if contains_call(body, &format!("{name}(")) {
        return true;
    }
    let last = name.rsplit("::").next().unwrap_or(name);
    contains_call(body, &format!("{last}(")) || contains_word(body, last)
}

fn contains_word(body: &str, word: &str) -> bool {
    let mut at = 0;
    while let Some(rel) = body[at..].find(word) {
        let pos = at + rel;
        let before = body[..pos].chars().next_back();
        let after = body[pos + word.len()..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            return true;
        }
        at = pos + 1;
    }
    false
}

fn contains_call(body: &str, needle: &str) -> bool {
    let mut at = 0;
    while let Some(rel) = body[at..].find(needle) {
        let pos = at + rel;
        let before = body[..pos].chars().next_back();
        if !before.is_some_and(is_word_char) {
            return true;
        }
        at = pos + 1;
    }
    false
}

/// A backticked token's resolution.
fn resolve(
    token: &str,
    defs: &Definitions,
    bases: &[PathBuf],
    all_paths: &[String],
) -> Result<Class, String> {
    let token = token.trim();
    if token.is_empty() {
        return Ok(Class::NonName);
    }
    if token.starts_with("BS-") || token.starts_with("WB-") {
        if token.starts_with("BS-") && !defs.rules.contains_key(token) {
            return Err(format!("unknown bookspec rule {token}"));
        }
        if token.starts_with("WB-") && !defs.product_rules.contains_key(token) {
            return Err(format!("unknown product rule {token}"));
        }
        return Ok(Class::RuleId);
    }
    if token.starts_with("books.") || token.starts_with("books:") {
        if defs.surface.contains(token) || defs.literals.iter().any(|l| l.contains(token)) {
            return Ok(Class::ProductSurface);
        }
        return Err(format!("product surface {token} names nothing"));
    }
    if token.contains("://") {
        if defs.surface.contains(token) {
            return Ok(Class::ProductSurface);
        }
        return Ok(Class::NonName);
    }
    // `path::item`: the path exists and its item resolves.
    if let Some((head, item)) = token.split_once("::") {
        if is_path_like(head) {
            let exists = path_exists(head, bases, all_paths) || bookspec_path_exists(head);
            if !exists {
                return Err(format!("path {head} does not exist"));
            }
            return resolve(item, defs, bases, all_paths);
        }
    }
    if is_path_like(token) {
        if token.contains("...") {
            return Ok(Class::NonName);
        }
        if path_exists(token, bases, all_paths) {
            return Ok(Class::Path);
        }
        if bookspec_path_exists(token) {
            return Ok(Class::Path);
        }
        if token.contains('*') {
            return resolve_wildcard(token, defs);
        }
        if defs.literals.contains(token) {
            return Ok(Class::StringValue);
        }
        return Err(format!("path {token} does not exist"));
    }
    // `Type::{a, b}` expands to the items named.
    if let Some(open) = token.find("::{") {
        if let Some(close) = token[open + 3..].find('}') {
            let prefix = &token[..open + 2];
            let inner = &token[open + 3..open + 3 + close];
            let mut classes = Vec::new();
            for item in inner.split(',') {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                classes.push(resolve(&format!("{prefix}{item}"), defs, bases, all_paths)?);
            }
            if classes.is_empty() {
                return Err(format!("empty brace group {token}"));
            }
            return Ok(classes[0]);
        }
    }
    if token.contains('*') {
        return resolve_wildcard(token, defs);
    }
    // `base[_suffix]`: both names must be real Lean theorems.
    if let Some(open) = token.find('[') {
        if token.ends_with(']') {
            let base = &token[..open];
            let suffix = &token[open + 1..token.len() - 1];
            let full = format!("{base}{suffix}");
            if is_audited(base) && is_audited(&full) {
                return Ok(Class::LeanTheorem);
            }
            return Err(format!("Lean notation {token} names no audited theorems"));
        }
    }
    if is_audited(token) {
        return Ok(Class::LeanTheorem);
    }
    let last = token.rsplit("::").next().unwrap_or(token);
    if defs.items.contains(token) || defs.items.contains(last) || defs.bare.contains(last) {
        return Ok(Class::RustItem);
    }
    if defs.tests.contains(last) {
        return Ok(Class::Test);
    }
    if defs.harnesses.contains(last) {
        return Ok(Class::KaniHarness);
    }
    if defs.fuzz.contains(last) {
        return Ok(Class::FuzzTarget);
    }
    // A suffix fragment of a harness name, as in `_at_tenths`.
    if token.starts_with('_')
        && (defs.harnesses.iter().any(|h| h.ends_with(token))
            || defs.tests.iter().any(|t| t.ends_with(token)))
    {
        return Ok(Class::KaniHarness);
    }
    if is_crate_name(token, bases) {
        return Ok(Class::Crate);
    }
    if is_fixture_dir(token, all_paths) {
        return Ok(Class::Path);
    }
    if defs.options.contains(token) {
        return Ok(Class::OptionValue);
    }
    if defs.examples.iter().any(|e| e == token) {
        return Ok(Class::OptionValue);
    }
    if defs.literals.contains(token) {
        return Ok(Class::StringValue);
    }
    if is_nonn_name(token) {
        return Ok(Class::NonName);
    }
    // Lowercase dotted names that are settings, events or features, when
    // they appear as a string literal or source text.
    if token.contains('.') && defs.surface.contains(token) {
        return Ok(Class::StringValue);
    }
    if is_kebab(token) {
        return Err(format!(
            "option-like name {token} names no option or example"
        ));
    }
    Err(format!("{token} names no known item"))
}

fn resolve_wildcard(token: &str, defs: &Definitions) -> Result<Class, String> {
    let (before, after) = match token.split_once('*') {
        Some((before, after)) => (before, after),
        None => (token, ""),
    };
    // An example path wildcard (`examples/1099-*.toml`) matches the
    // bookspec example names.
    let example_before = before
        .strip_prefix("bookspec/")
        .unwrap_or(before)
        .strip_prefix("examples/")
        .unwrap_or(before);
    let example_after = after.trim_end_matches(".toml");
    if defs
        .examples
        .iter()
        .any(|e| e.starts_with(example_before) && e.ends_with(example_after))
    {
        return Ok(Class::OptionValue);
    }
    if token.starts_with("proofs::") {
        let module = before.rsplit("::").next().unwrap_or("");
        if defs.proof_files.contains(module) || defs.harnesses.iter().any(|h| h.starts_with(module))
        {
            return Ok(Class::KaniHarness);
        }
    }
    if defs.tests.iter().any(|t| t.starts_with(before)) {
        return Ok(Class::Test);
    }
    if defs.harnesses.iter().any(|h| h.starts_with(before)) {
        return Ok(Class::KaniHarness);
    }
    if defs.items.iter().any(|i| i.starts_with(before)) {
        return Ok(Class::RustItem);
    }
    if defs.fuzz.iter().any(|f| f.starts_with(before)) {
        return Ok(Class::FuzzTarget);
    }
    if defs.examples.iter().any(|e| e.starts_with(before)) {
        return Ok(Class::OptionValue);
    }
    if defs.literals.iter().any(|l| l.starts_with(before)) {
        return Ok(Class::StringValue);
    }
    if defs.surface.contains(before) {
        return Ok(Class::StringValue);
    }
    Err(format!("wildcard {token} matches nothing"))
}

fn is_audited(name: &str) -> bool {
    let bare = name.strip_prefix("Bookspec.").unwrap_or(name);
    // Audit.lean is inside `namespace Bookspec`, so its names are relative;
    // a name may be written with any suffix of its namespace.
    let audited = bookspec_checker::audited_theorems(bookspec_checker::audit_lean());
    audited.contains(bare)
        || audited.iter().any(|a| {
            a == name || a.ends_with(&format!(".{name}")) || bare.ends_with(&format!(".{a}"))
        })
}

fn is_path_like(token: &str) -> bool {
    const EXTS: &[&str] = &[
        ".md", ".rs", ".toml", ".json", ".csv", ".lean", ".db", ".key", ".txt", ".pdf", ".yml",
        ".yaml", ".sh",
    ];
    EXTS.iter().any(|e| token.ends_with(e)) || token.contains('/') || token.contains('\\')
}

fn path_exists(token: &str, bases: &[PathBuf], all_paths: &[String]) -> bool {
    let token = token.trim_end_matches('/');
    if token.is_empty() {
        return false;
    }
    if bases.iter().any(|base| base.join(token).exists()) {
        return true;
    }
    all_paths
        .iter()
        .any(|p| p == token || p.ends_with(&format!("/{token}")))
}

fn bookspec_path_exists(token: &str) -> bool {
    let rest = token.strip_prefix("bookspec/").unwrap_or(token);
    if let Some(name) = rest.strip_prefix("examples/") {
        return bookspec_checker::examples()
            .iter()
            .any(|(file, _)| *file == name);
    }
    if let Some(name) = rest.strip_prefix("spec/") {
        return bookspec_checker::spec_files()
            .iter()
            .any(|(file, _)| *file == name);
    }
    if let Some(name) = rest.strip_prefix("lean/") {
        return bookspec_checker::lean_files()
            .iter()
            .any(|(file, _)| *file == name);
    }
    false
}

fn is_fixture_dir(token: &str, all_paths: &[String]) -> bool {
    all_paths
        .iter()
        .any(|path| path.contains("/fixtures/") && path.split('/').any(|part| part == token))
}

fn is_crate_name(token: &str, bases: &[PathBuf]) -> bool {
    let dashed = token.replace('_', "-");
    bases.iter().any(|base| {
        base.file_name()
            .is_some_and(|name| name == token || name == dashed.as_str())
    })
}

fn is_nonn_name(token: &str) -> bool {
    // A value: quoted text, a brace/field list, an assignment, a date
    // format, an invoice number, a percentage, an amount, punctuation.
    if token.chars().any(|c| {
        matches!(
            c,
            '"' | '\''
                | '{'
                | '}'
                | '['
                | ']'
                | '('
                | ')'
                | '<'
                | '>'
                | '='
                | '+'
                | '%'
                | '$'
                | '#'
                | '&'
                | '?'
                | '|'
                | ','
                | ';'
        ) || c.is_whitespace()
    }) {
        return true;
    }
    if token
        .chars()
        .all(|c| c.is_ascii_digit() || ".+-".contains(c))
    {
        return true;
    }
    if token.starts_with('.') || token.contains("...") {
        return true;
    }
    // INV- or INV-0001: an illustrative number.
    if token.starts_with("INV-")
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
    {
        return true;
    }
    false
}

fn is_kebab(token: &str) -> bool {
    token.split('-').all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    }) && token.contains('-')
}

/// The `| Harness | Law |` rows of `text`, as (harness, law number).
fn law_rows(text: &str) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() < 4 {
            continue;
        }
        let first = cells[1];
        let Some(name) = first.strip_prefix('`').and_then(|s| s.strip_suffix('`')) else {
            continue;
        };
        let second = cells[2];
        if let Ok(law) = second.parse::<u32>() {
            out.push((name.to_string(), law));
        }
    }
    out
}

fn claimed_ids(root: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut live = BTreeSet::new();
    let mut retired = BTreeSet::new();
    for path in claims_files(root) {
        let text = read(&path);
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("**bookspec:**") {
                if let Some(id) = first_backtick(rest) {
                    live.insert(id);
                }
            }
            if let Some(rest) = line.strip_prefix("**retired bookspec:**") {
                if let Some(id) = first_backtick(rest) {
                    retired.insert(id);
                }
            }
        }
    }
    (live, retired)
}

fn first_backtick(text: &str) -> Option<String> {
    let start = text.find('`')? + 1;
    let end = text[start..].find('`')? + start;
    Some(text[start..end].to_string())
}

fn claims_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(&root.join("docs/spec/claims"), "md", &mut files);
    files.sort();
    files
}

#[derive(Default)]
struct Fields {
    bookspec: String,
    retired_bookspec: String,
    claims: String,
    enforced: String,
    checked: String,
}

fn fields(body: &str) -> Fields {
    let names = [
        ("Enforced", 0usize),
        ("Checked", 1),
        ("Claims", 2),
        ("bookspec", 3),
        ("retired bookspec", 4),
    ];
    let mut marks: Vec<(usize, usize, usize)> = Vec::new();
    for (name, id) in names {
        let mut at = 0;
        while let Some(rel) = body[at..].find(&format!("*{name}:*")) {
            let pos = at + rel;
            // The pattern matches inside `**name:**` too (at its second
            // star), which is the same marker: keep one, and consume the
            // second trailing star when it is the bold form.
            let mut end = pos + format!("*{name}:*").len();
            if body.as_bytes().get(end) == Some(&b'*') {
                end += 1;
            }
            if !marks.iter().any(|(p, _, _)| *p == pos) {
                marks.push((pos, end, id));
            }
            at = pos + 1;
        }
    }
    // Pick markers are boundaries only.
    let mut at = 0;
    while let Some(rel) = body[at..].find("*Pick") {
        let pos = at + rel;
        if !marks.iter().any(|(p, _, _)| *p == pos) {
            marks.push((pos, pos + 5, 9));
        }
        at = pos + 1;
    }
    marks.sort();
    let mut out = Fields::default();
    for (i, (_, end, id)) in marks.iter().enumerate() {
        let stop = marks.get(i + 1).map(|m| m.0).unwrap_or(body.len());
        let stop = stop.max(*end);
        let value = body[*end..stop].trim().to_string();
        match id {
            0 => out.enforced.push_str(&value),
            1 => out.checked.push_str(&value),
            2 => out.claims.push_str(&value),
            3 => out.bookspec.push_str(&value),
            4 => out.retired_bookspec.push_str(&value),
            _ => {}
        }
    }
    out
}

fn level(claims: &str) -> &str {
    claims.split_whitespace().next().unwrap_or("yes")
}

fn backticks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let token = &after[..end];
        if !token.contains('\n') && !token.is_empty() {
            out.push(token.to_string());
        }
        rest = &after[end + 1..];
    }
    out
}

fn b_sections(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(heading) = lines[i].strip_prefix("## ") {
            if heading.starts_with("B-") {
                let mut body = String::new();
                let mut j = i + 1;
                while j < lines.len() && !lines[j].starts_with("## ") {
                    body.push_str(lines[j]);
                    body.push('\n');
                    j += 1;
                }
                if let Some(at) = body.find("### Product rules") {
                    body.truncate(at);
                }
                out.push((heading.to_string(), body));
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Product-rule spans: (id, body).
fn wb_sections(text: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(rest) = lines[i].strip_prefix("**WB-") {
            let id_tail: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            let id = format!("WB-{id_tail}");
            let mut body = String::new();
            let mut j = i;
            while j < lines.len() {
                if j > i && (lines[j].starts_with("**WB-") || lines[j].starts_with("## ")) {
                    break;
                }
                body.push_str(lines[j]);
                body.push('\n');
                j += 1;
            }
            out.push((id, body));
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

fn index_paths(root: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut bases = vec![root.to_path_buf()];
    for entry in std::fs::read_dir(root.join("crates")).expect("crates") {
        let crate_dir = entry.expect("a crate").path();
        bases.push(crate_dir.clone());
        if crate_dir.join("tests").is_dir() {
            bases.push(crate_dir.join("tests"));
        }
    }
    bases.push(root.join("fuzz"));
    let mut all = Vec::new();
    collect_paths(root, root, &mut all);
    (bases, all)
}

fn collect_paths(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" || name == "node_modules" {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
        if path.is_dir() {
            collect_paths(root, &path, out);
        }
    }
}

fn main_index(root: &Path) -> (Definitions, Vec<PathBuf>, Vec<String>) {
    let mut defs = build_definitions(root);
    // Product rule ids and their counts, so duplicates are caught.
    for path in claims_files(root) {
        let text = read(&path);
        for (id, _) in wb_sections(&text) {
            *defs.product_rules.entry(id).or_insert(0) += 1;
        }
    }
    let (bases, paths) = index_paths(root);
    (defs, bases, paths)
}

#[test]
fn every_claim_is_backed() {
    let root = repo_root();
    let (defs, bases, all_paths) = main_index(&root);

    assert!(
        !defs.rules.is_empty(),
        "the pinned bookspec has rule records"
    );
    assert!(!defs.harnesses.is_empty(), "the proofs carry harnesses");

    let mut failures: Vec<String> = Vec::new();
    let mut counts: BTreeMap<Class, usize> = BTreeMap::new();
    let mut claims_checked = 0usize;
    let mut tokens_seen = 0usize;

    for path in claims_files(&root) {
        let text = read(&path);
        let file = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        for (heading, body) in b_sections(&text) {
            let f = fields(&body);
            // Live ids only.
            if !f.bookspec.is_empty() {
                for id in backticks(&f.bookspec) {
                    match defs.rules.get(&id) {
                        Some(false) => {}
                        Some(true) => failures.push(format!(
                            "{file}: {heading}: {id} is retired; write `**retired bookspec:**`"
                        )),
                        None => failures.push(format!(
                            "{file}: {heading}: bookspec id {id} does not exist"
                        )),
                    }
                }
            }
            if !f.retired_bookspec.is_empty() {
                for id in backticks(&f.retired_bookspec) {
                    match defs.rules.get(&id) {
                        Some(true) => {}
                        Some(false) => failures.push(format!(
                            "{file}: {heading}: {id} is live; write `**bookspec:**`"
                        )),
                        None => failures.push(format!(
                            "{file}: {heading}: retired bookspec id {id} does not exist"
                        )),
                    }
                }
            }
            let claim = level(&f.claims);
            if claim != "yes" && claim != "partly" {
                continue;
            }
            claims_checked += 1;
            for (name, value) in [("Enforced", &f.enforced), ("Checked", &f.checked)] {
                if value.trim().is_empty() {
                    failures.push(format!(
                        "{file}: {heading}: a `{claim}` claim has no **{name}:**"
                    ));
                    continue;
                }
                for token in backticks(value) {
                    tokens_seen += 1;
                    match resolve(&token, &defs, &bases, &all_paths) {
                        Ok(class) => *counts.entry(class).or_insert(0) += 1,
                        Err(error) => failures.push(format!(
                            "{file}: {heading}: {name}: backtick `{token}`: {error}"
                        )),
                    }
                }
            }
        }
        for (id, body) in wb_sections(&text) {
            let f = fields(&body);
            let claim = level(&f.claims);
            if claim != "yes" && claim != "partly" {
                continue;
            }
            claims_checked += 1;
            for (name, value) in [("Enforced", &f.enforced), ("Checked", &f.checked)] {
                if value.trim().is_empty() {
                    failures.push(format!(
                        "{file}: {id}: a `{claim}` product rule has no **{name}:**"
                    ));
                    continue;
                }
                for token in backticks(value) {
                    tokens_seen += 1;
                    match resolve(&token, &defs, &bases, &all_paths) {
                        Ok(class) => *counts.entry(class).or_insert(0) += 1,
                        Err(error) => failures
                            .push(format!("{file}: {id}: {name}: backtick `{token}`: {error}")),
                    }
                }
            }
        }
    }

    for (id, count) in &defs.product_rules {
        if *count > 1 {
            failures.push(format!("{id} is declared {count} times; WB ids are unique"));
        }
    }

    // The kernel: every pub fn has a harness, a bridge property and a law.
    let kernel = kernel_functions(&root);
    assert!(!kernel.is_empty(), "the kernel carries public functions");
    let mut harness_bodies: Vec<(String, String)> = Vec::new();
    let mut proof_files = Vec::new();
    walk(
        &root.join("crates/ledger-core/src/proofs"),
        "rs",
        &mut proof_files,
    );
    for path in &proof_files {
        let code = strip_code(&read(path));
        harness_bodies.extend(collect_bodies_with_attribute(&code, "#[kani::proof]"));
    }
    let mut bridge_bodies: Vec<(String, String)> = Vec::new();
    let mut bridge_files = Vec::new();
    walk(
        &root.join("crates/ledger-core/tests/kernel_bridge"),
        "rs",
        &mut bridge_files,
    );
    for path in &bridge_files {
        let text = read(path);
        let code = strip_code(&text);
        let chars: Vec<char> = code.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i..].starts_with(&PROPTEST) {
                if let Some(open) = code[i..].find('{') {
                    let at = i + open;
                    if let Some(close) = matching_brace(&chars, at, ('{', '}')) {
                        let block: String = chars[at + 1..close].iter().collect();
                        let mut fns = Vec::new();
                        collect_function_bodies(&block, &mut fns);
                        bridge_bodies.extend(fns);
                        i = close;
                        continue;
                    }
                }
            }
            i += 1;
        }
        for (name, body) in collect_bodies_with_attribute(&code, "#[test]") {
            bridge_bodies.push((name, body));
        }
    }

    let mut laws: BTreeMap<String, u32> = BTreeMap::new();
    for (name, law) in law_rows(&read(&root.join("docs/verification.md"))) {
        laws.insert(name, law);
    }
    let mut proof_notes = Vec::new();
    walk(&root.join("docs/proofs"), "md", &mut proof_notes);
    for path in &proof_notes {
        for (name, law) in law_rows(&read(path)) {
            laws.entry(name).or_insert(law);
        }
    }
    let laws_md = bookspec_checker::spec_files()
        .into_iter()
        .find(|(name, _)| *name == "laws.md")
        .map(|(_, text)| text)
        .expect("bookspec's spec/laws.md");
    let law_numbers: BTreeSet<u32> = laws_md
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.trim().split('|').map(str::trim).collect();
            if cells.len() < 5 {
                return None;
            }
            cells[1].parse::<u32>().ok()
        })
        .collect();
    assert!(!law_numbers.is_empty(), "bookspec numbers its laws");

    let mut kernel_checked = 0usize;
    let kernel_seen = kernel.len();
    let mut seen_exceptions = BTreeSet::new();
    for (name, file) in &kernel {
        let harnesses: Vec<&str> = harness_bodies
            .iter()
            .filter(|(_, body)| calls(body, name))
            .map(|(h, _)| h.as_str())
            .collect();
        let bridges: Vec<&str> = bridge_bodies
            .iter()
            .filter(|(_, body)| calls(body, name))
            .map(|(b, _)| b.as_str())
            .collect();
        let law = harnesses
            .iter()
            .filter_map(|h| laws.get(*h))
            .find(|n| law_numbers.contains(n));
        let mut lacks = Vec::new();
        if harnesses.is_empty() {
            lacks.push("a Kani harness");
        }
        if bridges.is_empty() {
            lacks.push("a bridge property");
        }
        if law.is_none() {
            lacks.push("a law");
        }
        if lacks.is_empty() {
            kernel_checked += 1;
            continue;
        }
        match EXCEPTIONS.iter().find(|(fn_name, _, _)| fn_name == name) {
            Some((_, what, why)) => {
                seen_exceptions.insert(name.clone());
                let stated = *what;
                for lack in &lacks {
                    let word = lack.trim_start_matches("a ");
                    assert!(
                        stated.contains(word) || stated.contains(lack),
                        "{name} is excepted for {stated:?}, but also lacks {lack} (in {file}); the list is stale"
                    );
                }
                assert!(!why.trim().is_empty(), "{name}'s exception says why");
            }
            None => failures.push(format!(
                "kernel `{name}` ({file}) lacks {}; add a harness, a bridge property or a law, or name it in EXCEPTIONS with why",
                lacks.join(", ")
            )),
        }
    }
    for (name, _, _) in EXCEPTIONS {
        assert!(
            kernel.iter().any(|(fn_name, _)| fn_name == name),
            "excepted kernel function {name} does not exist; the list is stale"
        );
        assert!(
            seen_exceptions.contains(*name),
            "excepted kernel function {name} is fully backed now; remove it from EXCEPTIONS"
        );
    }
    assert_eq!(
        EXCEPTIONS.len(),
        16,
        "the kernel exception list changes only on purpose"
    );

    // Every law row that cites a claimed rule names audited theorems.
    let (live_ids, _) = claimed_ids(&root);
    let audited = bookspec_checker::audited_theorems(bookspec_checker::audit_lean());
    assert!(!audited.is_empty(), "bookspec embeds its Audit.lean list");
    let mut law_rows_checked = 0usize;
    for line in laws_md.lines() {
        let cells: Vec<&str> = line.trim().split('|').map(str::trim).collect();
        if cells.len() < 5 {
            continue;
        }
        let Ok(number) = cells[1].parse::<u32>() else {
            continue;
        };
        let citations: BTreeSet<String> = cells[3]
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|w| w.starts_with("BS-"))
            .map(str::to_string)
            .collect();
        if !citations.iter().any(|id| live_ids.contains(id)) {
            continue;
        }
        law_rows_checked += 1;
        let theorems = cells[4];
        if theorems.contains("none —") {
            let reason = theorems.split("none —").nth(1).unwrap_or("").trim();
            assert!(
                !reason.is_empty(),
                "law {number} says `none` without a reason"
            );
            continue;
        }
        let tokens = backticks(theorems);
        assert!(
            !tokens.is_empty(),
            "law {number} cites a claimed rule but names no Lean theorem"
        );
        for token in tokens {
            let allowed_definition = token == "Bookspec.Accepted"
                && theorems.contains("`Bookspec.Accepted` (definition)");
            let bare = token.strip_prefix("Bookspec.").unwrap_or(&token);
            if !allowed_definition && !audited.contains(bare) {
                failures.push(format!(
                    "law {number}: Lean theorem `{token}` is not in Audit.lean"
                ));
            }
        }
    }

    println!("claims sections checked: {claims_checked}");
    println!("backticked tokens resolved: {tokens_seen}");
    for (class, count) in &counts {
        println!("  {}: {count}", class.name());
    }
    println!("kernel functions checked: {kernel_seen} ({kernel_checked} fully backed)");
    println!("kernel exceptions: {}", EXCEPTIONS.len());
    println!("law rows checked: {law_rows_checked}");

    assert!(
        failures.is_empty(),
        "the traceability gate found {} problem(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        claims_checked >= CLAIMS_FLOOR,
        "claims sections checked ({claims_checked}) fell below {CLAIMS_FLOOR}"
    );
    assert!(
        tokens_seen >= TOKENS_FLOOR,
        "backticked tokens checked ({tokens_seen}) fell below {TOKENS_FLOOR}"
    );
    assert!(
        kernel_seen >= KERNEL_FLOOR,
        "kernel functions checked ({kernel_seen}) fell below {KERNEL_FLOOR}"
    );
}

/// Kernel public functions with no independent harness, bridge or law,
/// and why. The length is pinned above.
const EXCEPTIONS: &[(&str, &str, &str)] = &[
    (
        "movements",
        "a Kani harness and a law",
        "it allocates a Vec; every harness calls the fixed-array `movements_into` (proofs/mod.rs any_len says why), and its law 3 is movements_into's",
    ),
    (
        "reversal",
        "a Kani harness and a law",
        "it allocates a Vec; every reversal harness calls `reverse_line` directly, and its law 5 is reverse_line's",
    ),
    (
        "flip",
        "a bridge property",
        "it maps a two-variant enum with no amount to bridge; Kani `flip_negates` proves it",
    ),
    (
        "signed",
        "a bridge property",
        "it is called by the bridged `net` and `totals` properties, never as a bridge's own subject",
    ),
    (
        "movements_into",
        "a bridge property",
        "it is the fixed-array helper; bridge `sums_agree` bridges its public wrapper `movements`, and every harness calls it",
    ),
    (
        "reverse_line",
        "a bridge property",
        "bridge `reversal_agrees` covers it through `reversal`",
    ),
    (
        "payment_bucket",
        "a bridge property",
        "it maps two enums to a bucket with no amount to bridge; Kani `payment_bucket_is_the_table` proves the table",
    ),
    (
        "OnBehalf::mirror",
        "a bridge property",
        "it maps a two-variant enum to itself with no amount to bridge; Kani `constructive_mirrors` proves it",
    ),
    (
        "Direction::flip",
        "a bridge property",
        "it maps a three-variant enum with no amount to bridge; engine tests check it",
    ),
    (
        "Direction::of",
        "a Kani harness, a bridge property and a law",
        "it is a three-way sign classification, exercised through `party_target`; no harness or law states it directly",
    ),
    (
        "Direction::key",
        "a Kani harness, a bridge property and a law",
        "it is a wire-name lookup with no arithmetic; engine and store tests check the names",
    ),
    (
        "MonthPart::is_valid",
        "a Kani harness, a bridge property and a law",
        "it is a range predicate with no arithmetic; `part_weight` and the proration harnesses exercise it",
    ),
    (
        "part_weight",
        "a Kani harness, a bridge property and a law",
        "it is generic over `Units`, not `Amount`; laws 26-28 are stated through `prorate_by_days`, which calls it",
    ),
    (
        "fx_booked_minor",
        "a Kani harness and a law",
        "it is the Decimal/currency wrapper around `fx_booked`; the harnesses and law 41 are fx_booked's, and the bridge carries them to Decimal",
    ),
    (
        "round_places",
        "a Kani harness and a law",
        "it is the Decimal wrapper around `round_step`; the harnesses and law 40 are round_step's, and the bridge carries them to Decimal",
    ),
    (
        "round_minor",
        "a Kani harness and a law",
        "it is the Decimal/currency wrapper around `round_step`; the harnesses and law 40 are round_step's, and the bridge carries them to Decimal",
    ),
];

#[test]
fn made_up_names_do_not_resolve() {
    let root = repo_root();
    let (defs, bases, all_paths) = main_index(&root);
    for token in ["no_such_fn_xyz", "NoSuch.theorem", "books.no_such_tool_xyz"] {
        assert!(
            resolve(token, &defs, &bases, &all_paths).is_err(),
            "made-up name `{token}` must not resolve"
        );
    }
}
