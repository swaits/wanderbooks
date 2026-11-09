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
//! actually resolved. A product surface is a registered tool name, or an
//! exact string literal the source declares. Every `**bookspec:**` id must
//! be a live record (a retired one is written `**retired bookspec:**`);
//! every `WB-` id is unique.
//!
//! It also checks every `pub fn` in `crates/ledger-core/src/kernel/*.rs`:
//! a Kani harness calls it, a bridge property in `tests/kernel_bridge/`
//! calls it, one of its harnesses carries a numbered law that is a row of
//! bookspec's `spec/laws.md`, and the source clause of its own spec block
//! cites a rule that law serves. The functions that are wrappers or enum
//! maps with no independent harness, bridge or law are named in
//! [`EXCEPTIONS`], with what each lacks and why; its length is pinned so
//! the list changes only on purpose. A function the gate cannot apply the
//! right-law check to (because it has no harness law, or cites no rule) is
//! recorded, and the set of those is pinned too: it must be exactly the
//! excepted wrappers [`SKIPPED`] names, never a hole ordinary functions
//! fall through.
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

/// Where a raw string starting at `i` ends (`r"…"`, `r#"…"#`, `br"…"`),
/// or `None` when `i` is not a raw string. Without this, a backslash in a
/// Windows path inside `r"…"` reads as an escape and blanks the rest of
/// the file.
fn raw_string_end(chars: &[char], i: usize) -> Option<usize> {
    if i > 0 && is_word_char(chars[i - 1]) {
        return None;
    }
    let mut j = i;
    if chars.get(j) == Some(&'b') {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0usize;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    j += 1;
    while j < chars.len() {
        if chars[j] == '"' {
            let mut k = j + 1;
            let mut seen = 0usize;
            while seen < hashes && chars.get(k) == Some(&'#') {
                seen += 1;
                k += 1;
            }
            if seen == hashes {
                return Some(k);
            }
        }
        j += 1;
    }
    Some(chars.len())
}

/// The source with comments and string/char literals blanked, so a word in
/// a comment or a string is never mistaken for a definition.
fn strip_code(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = raw_string_end(&chars, i) {
            i = end;
        } else if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
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
        // The type is the path's last segment after `for` when it is a
        // trait impl, with a leading generic parameter list skipped (an
        // `impl<T> Foo<T>` names `Foo`, not `T`).
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
        let last = impl_type_name(segment);
        if let Some(close) = matching_brace(&chars, j, ('{', '}')) {
            let start = chars[..i].iter().map(|c| c.len_utf8()).sum::<usize>();
            let end = chars[..=close].iter().map(|c| c.len_utf8()).sum::<usize>();
            out.push((start, end, last));
        }
        i = end;
    }
    out
}

/// The type an impl header after `impl` (and after `for`) names.
fn impl_type_name(segment: &str) -> String {
    let mut rest = segment.trim_start();
    if rest.starts_with('<') {
        let chars: Vec<char> = rest.chars().collect();
        if let Some(close) = matching_brace(&chars, 0, ('<', '>')) {
            let consumed: usize = chars[..=close].iter().map(|c| c.len_utf8()).sum();
            rest = rest[consumed..].trim_start();
        }
    }
    let head: String = rest
        .chars()
        .take_while(|c| !matches!(c, '<' | '{') && !c.is_whitespace())
        .collect();
    head.rsplit("::").next().unwrap_or(&head).to_string()
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
fn collect_types(
    code: &str,
    items: &mut BTreeSet<String>,
    qualified: &mut BTreeSet<String>,
    types: &mut BTreeSet<String>,
) {
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
                types.insert(name.clone());
                if word == "enum" {
                    for variant in enum_variants(code, &name) {
                        qualified.insert(format!("{name}::{variant}"));
                        items.insert(variant);
                    }
                } else {
                    for field in struct_fields(code, &name) {
                        qualified.insert(format!("{name}::{field}"));
                        items.insert(field);
                    }
                }
            }
        }
        i = end;
    }
}

/// The names a `use` statement imports. They are not definitions: only an
/// external type's associated item (`NaiveDate::MAX`) is excused by them.
fn collect_imports(code: &str, imports: &mut BTreeSet<String>) {
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
                    imports.insert(word.clone());
                }
                word.clear();
            }
        }
    }
}

#[derive(Default)]
struct Definitions {
    /// Every bare Rust item name (a `fn`, type, variant, field, `const`,
    /// module or macro) defined under `crates/*/src` or in a test file.
    items: BTreeSet<String>,
    /// The exact `Type::item` names: an impl's functions, an enum's
    /// variants, an impl's associated `const`s and types.
    qualified: BTreeSet<String>,
    /// Every real module's full path (`crate::a::b`): a file module
    /// (`src/a/b.rs`, `src/a/b/mod.rs`, the crate root) and every inline
    /// `mod b { … }`.
    modules: BTreeSet<String>,
    /// The items defined directly in each module, keyed by that full path.
    /// A glob re-export (`pub use entries::*;`) brings a module's items
    /// into the re-exporting one, as Rust does.
    module_items: BTreeMap<String, BTreeSet<String>>,
    /// Type names defined under `crates/*/src`.
    types: BTreeSet<String>,
    /// Names a `use` line imports (external types included).
    imports: BTreeSet<String>,
    /// `[package] name` of every crate.
    crates: BTreeSet<String>,
    /// The exact tool names `ledger-mcp` registers.
    product_surfaces: BTreeSet<String>,
    /// Test names: a `#[test]` fn or a `proptest!` body, in src or tests.
    tests: BTreeSet<String>,
    /// `#[kani::proof]` functions under `src/proofs/`.
    harnesses: BTreeSet<String>,
    /// Fuzz targets under `fuzz/fuzz_targets/`.
    fuzz: BTreeSet<String>,
    /// Proof source file stems.
    proof_files: BTreeSet<String>,
    /// Every string literal in `crates/*` source.
    literals: BTreeSet<String>,
    /// All crate source text, for wildcard prefixes.
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

/// The `[package] name` of a Cargo manifest.
fn package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = line.strip_prefix("name") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let rest = rest.trim();
                    if let Some(rest) = rest.strip_prefix('"') {
                        return rest.split('"').next().map(str::to_string);
                    }
                }
            }
        }
    }
    None
}

/// The module a source file is, as segments below its crate root: `lib.rs`
/// and `main.rs` are the root, `a.rs` and `a/mod.rs` are `a`, `a/b.rs` is
/// `a::b`.
fn module_path_of(path: &Path, src: &Path) -> Vec<String> {
    let Ok(rel) = path.strip_prefix(src) else {
        return Vec::new();
    };
    let mut parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let Some(last) = parts.pop() else {
        return Vec::new();
    };
    if last == "lib.rs" || last == "main.rs" {
        return Vec::new();
    }
    let Some(stem) = last.strip_suffix(".rs") else {
        return Vec::new();
    };
    if stem != "mod" {
        parts.push(stem.to_string());
    }
    parts
}

/// Record every item of `code`, the body of module `path` in `file`:
/// functions, types, variants' containers, `const`s, macros and submodule
/// names, each in its module. An inline `mod b { … }` is a module of its
/// own, recursed into with `path::b`; a `use …::*` is remembered so a glob
/// re-export can be applied once every module is known.
fn scan_module(
    code: &str,
    path: &str,
    file: &Path,
    defs: &mut Definitions,
    enums: &mut Vec<(PathBuf, String)>,
    structs: &mut Vec<(PathBuf, String)>,
    globs: &mut Vec<(String, String)>,
) {
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
        let mut after = end;
        match word.as_str() {
            "mod" => {
                let j = skip_space(&chars, end);
                if let Some((name, name_end)) = word_at(&chars, j) {
                    defs.items.insert(name.clone());
                    defs.module_items
                        .entry(path.to_string())
                        .or_default()
                        .insert(name.clone());
                    let mut k = skip_space(&chars, name_end);
                    while k < chars.len() && chars[k] != '{' && chars[k] != ';' {
                        k += 1;
                    }
                    if chars.get(k) == Some(&'{') {
                        if let Some(close) = matching_brace(&chars, k, ('{', '}')) {
                            let full = format!("{path}::{name}");
                            defs.modules.insert(full.clone());
                            let body: String = chars[k + 1..close].iter().collect();
                            scan_module(&body, &full, file, defs, enums, structs, globs);
                            after = close + 1;
                        }
                    }
                }
            }
            "use" => {
                let mut k = i;
                while k < chars.len() && chars[k] != ';' {
                    k += 1;
                }
                let decl: String = chars[i..k].iter().collect();
                if let Some(target) = glob_use_target(&decl) {
                    globs.push((path.to_string(), target));
                }
                after = k.min(chars.len());
            }
            "fn" | "struct" | "enum" | "union" | "trait" | "type" | "const" | "static" => {
                let j = skip_space(&chars, end);
                if let Some((name, _)) = word_at(&chars, j) {
                    if word == "enum" {
                        enums.push((file.to_path_buf(), name.clone()));
                    }
                    if word == "struct" {
                        structs.push((file.to_path_buf(), name.clone()));
                    }
                    if matches!(
                        word.as_str(),
                        "struct" | "enum" | "union" | "trait" | "type"
                    ) {
                        defs.types.insert(name.clone());
                    }
                    defs.items.insert(name.clone());
                    defs.module_items
                        .entry(path.to_string())
                        .or_default()
                        .insert(name);
                }
            }
            "macro_rules" => {
                let mut j = skip_space(&chars, end);
                if chars.get(j) == Some(&'!') {
                    j = skip_space(&chars, j + 1);
                    if let Some((name, _)) = word_at(&chars, j) {
                        defs.items.insert(name.clone());
                        defs.module_items
                            .entry(path.to_string())
                            .or_default()
                            .insert(name);
                    }
                }
            }
            _ => {}
        }
        i = after.max(end);
    }
}

/// The target of a glob `use` (`use a::b::*;` names `a::b`), or `None` for
/// any other import.
fn glob_use_target(decl: &str) -> Option<String> {
    let decl = decl.trim_start().strip_prefix("use ")?.trim();
    let target = decl.strip_suffix("::*")?.trim();
    // `super` and `self` are relative to the inline test modules, whose
    // items no claim names; resolving them would need the parent chain.
    if target.is_empty() || target == "super" || target == "self" {
        return None;
    }
    Some(target.to_string())
}

/// Test names: a `#[test]` fn or a function inside a `proptest!` block.
fn collect_test_names(code: &str, tests: &mut BTreeSet<String>) {
    for attr in ["#[test]", "#[tokio::test"] {
        for (name, _) in collect_bodies_with_attribute(code, attr) {
            tests.insert(name);
        }
    }
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
                    for (name, _) in fns {
                        tests.insert(name);
                    }
                    i = close;
                    continue;
                }
            }
        }
        i += 1;
    }
}

/// Every function and associated `const`/type in an impl block, qualified
/// by the implementing type.
fn collect_impl_qualified(block: &str, ty: &str, qualified: &mut BTreeSet<String>) {
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
        if word == "fn" || word == "const" || word == "type" {
            let j = skip_space(&chars, end);
            if let Some((next, _)) = word_at(&chars, j) {
                // `const fn` is a function; the `fn` arm records it.
                if word != "const" || next != "fn" {
                    qualified.insert(format!("{ty}::{next}"));
                }
            }
        }
        i = end;
    }
}

/// The exact tool names `ledger-mcp` registers: every `name = "books.…"`
/// in its source.
fn collect_tool_names(root: &Path, out: &mut BTreeSet<String>) {
    let mut files = Vec::new();
    walk(&root.join("crates/ledger-mcp/src"), "rs", &mut files);
    for path in files {
        let text = read(&path);
        let needle = "name = \"";
        let mut at = 0;
        while let Some(rel) = text[at..].find(needle) {
            let start = at + rel + needle.len();
            let Some(end) = text[start..].find('"') else {
                break;
            };
            let name = &text[start..start + end];
            if name.starts_with("books.") {
                out.insert(name.to_string());
            }
            at = start + end + 1;
        }
    }
}

fn build_definitions(root: &Path) -> Definitions {
    let mut defs = Definitions::default();

    // Crate names and directories: each crate under `crates/`, the
    // workspace root, and `fuzz`.
    let mut crate_dirs: Vec<(PathBuf, String)> = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("crates") {
        let dir = entry.expect("a crate").path();
        if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) {
            if let Some(name) = package_name(&text) {
                defs.crates.insert(name.clone());
                crate_dirs.push((dir, name));
            }
        }
    }
    for manifest in [root.join("Cargo.toml"), root.join("fuzz/Cargo.toml")] {
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if let Some(name) = package_name(&text) {
                defs.crates.insert(name);
            }
        }
    }
    collect_tool_names(root, &mut defs.product_surfaces);

    let proofs = root.join("crates/ledger-core/src/proofs");
    let mut src_files = Vec::new();
    let mut src_texts: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut enums = Vec::new();
    let mut structs = Vec::new();
    let mut globs = Vec::new();
    for (crate_dir, crate_name) in &crate_dirs {
        let src = crate_dir.join("src");
        if !src.is_dir() {
            continue;
        }
        // The crate root is a module even when nothing but `mod` lines
        // live there.
        defs.modules.insert(crate_name.clone());
        let mut files = Vec::new();
        walk(&src, "rs", &mut files);
        files.sort();
        for path in files {
            // Read each file once; later passes reuse this text.
            let text = read(&path);
            let code = strip_code(&text);
            defs.surface.push_str(&text);
            defs.surface.push('\n');
            for literal in literals(&text) {
                defs.literals.insert(literal);
            }
            // A proof harness is never a plain Rust item: it resolves only
            // as a `#[kani::proof]` fn, or deleting the attribute would go
            // unseen.
            if !path.starts_with(&proofs) {
                collect_imports(&code, &mut defs.imports);
                let segments = module_path_of(&path, &src);
                let module = if segments.is_empty() {
                    crate_name.clone()
                } else {
                    format!("{crate_name}::{}", segments.join("::"))
                };
                defs.modules.insert(module.clone());
                scan_module(
                    &code,
                    &module,
                    &path,
                    &mut defs,
                    &mut enums,
                    &mut structs,
                    &mut globs,
                );
                // Methods, associated items and variants, qualified by
                // their type.
                for (start, end, ty) in impl_ranges(&code) {
                    collect_impl_qualified(&code[start..end], &ty, &mut defs.qualified);
                }
            }
            src_texts.insert(path.clone(), text);
            src_files.push(path);
        }
    }
    // A glob re-export brings a module's items into the re-exporting one,
    // as Rust does: `pub use entries::*;` names a sibling, and
    // `crate::a::b` is absolute. Three passes reach a re-export chain.
    for _ in 0..3 {
        for (module, target) in &globs {
            let krate = module.split("::").next().unwrap_or("");
            let resolved = match target.strip_prefix("crate::") {
                Some(rest) => format!("{krate}::{rest}"),
                None if target.contains("::") => target.clone(),
                None => format!("{module}::{target}"),
            };
            if let Some(items) = defs.module_items.get(&resolved).cloned() {
                defs.module_items
                    .entry(module.clone())
                    .or_default()
                    .extend(items);
            }
        }
    }
    for (path, name) in &enums {
        if let Some(text) = src_texts.get(path) {
            let code = strip_code(text);
            for variant in enum_variants(&code, name) {
                defs.items.insert(variant.clone());
                defs.qualified.insert(format!("{name}::{variant}"));
            }
        }
    }
    for (path, name) in &structs {
        if let Some(text) = src_texts.get(path) {
            let code = strip_code(text);
            for field in struct_fields(&code, name) {
                defs.items.insert(field.clone());
                // A public field is the type's item too (`Iif::changes`).
                defs.qualified.insert(format!("{name}::{field}"));
            }
        }
    }

    // Tests: a `#[test]` fn or a `proptest!` body, in src or tests. A
    // plain helper under `tests/` is not a test.
    let mut test_files = Vec::new();
    for entry in std::fs::read_dir(root.join("crates")).expect("crates") {
        let dir = entry.expect("a crate").path().join("tests");
        if dir.is_dir() {
            walk(&dir, "rs", &mut test_files);
        }
    }
    test_files.sort();
    for path in &test_files {
        let text = read(path);
        let code = strip_code(&text);
        collect_types(&code, &mut defs.items, &mut defs.qualified, &mut defs.types);
        collect_imports(&code, &mut defs.imports);
        collect_test_names(&code, &mut defs.tests);
    }
    for path in &src_files {
        let code = strip_code(src_texts.get(path).expect("read above"));
        collect_test_names(&code, &mut defs.tests);
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

/// Functions preceded (within the previous attribute block) by `attr`.
fn collect_bodies_with_attribute(code: &str, attr: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut search = 0;
    while let Some(at) = code[search..].find(attr) {
        let start = search + at;
        // The attribute runs from its `[` to the matching `]`; an
        // attribute may carry arguments (`#[tokio::test(flavor = …)]`).
        let Some(open) = code[start..].find('[').map(|p| start + p) else {
            break;
        };
        let open = code[..open].chars().count();
        let Some(close) = matching_brace(&chars, open, ('[', ']')) else {
            break;
        };
        let mut i = skip_space(&chars, close + 1);
        // Further attributes may sit between this one and the item.
        while chars.get(i) == Some(&'#') {
            let j = if chars.get(i + 1) == Some(&'!') {
                i + 2
            } else {
                i + 1
            };
            if chars.get(j) != Some(&'[') {
                break;
            }
            let Some(c) = matching_brace(&chars, j, ('[', ']')) else {
                break;
            };
            i = skip_space(&chars, c + 1);
        }
        // Visibility and modifiers, then `fn`.
        while let Some((word, end)) = word_at(&chars, i) {
            match word.as_str() {
                "pub" => {
                    i = skip_space(&chars, end);
                    if chars.get(i) == Some(&'(') {
                        if let Some(c) = matching_brace(&chars, i, ('(', ')')) {
                            i = skip_space(&chars, c + 1);
                        }
                    }
                }
                "async" | "const" | "unsafe" | "extern" => {
                    i = skip_space(&chars, end);
                }
                "fn" => {
                    i = skip_space(&chars, end);
                    break;
                }
                _ => break,
            }
        }
        let Some((name, _)) = word_at(&chars, i) else {
            search = start + attr.len();
            continue;
        };
        let mut k = i;
        while k < chars.len() && chars[k] != '{' && chars[k] != ';' {
            k += 1;
        }
        if chars.get(k) == Some(&'{') {
            if let Some(end) = matching_brace(&chars, k, ('{', '}')) {
                let body: String = chars[k + 1..end].iter().collect();
                out.push((name, body));
            }
        }
        search = start + attr.len();
    }
    out
}

/// Every `pub fn` in `crates/ledger-core/src/kernel/*.rs`, qualified by
/// its impl (`Type::method`).
fn kernel_functions(root: &Path) -> Vec<(String, String, String)> {
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
                    if let Some((name, name_end)) = word_at(&chars, k) {
                        let offset = code[..i].chars().count();
                        let ty = impls
                            .iter()
                            .find(|(start, end, _)| offset >= *start && offset < *end)
                            .map(|(_, _, ty)| ty.clone());
                        let qualified = match ty {
                            Some(ty) => format!("{ty}::{name}"),
                            None => name,
                        };
                        // The exact signature, `pub` through the body's `{`:
                        // two items may share a name (`flip`), and the
                        // signature is what tells their doc blocks apart.
                        let mut body = name_end;
                        let mut after = skip_space(&chars, body);
                        if chars.get(after) == Some(&'<') {
                            if let Some(gt) = matching_brace(&chars, after, ('<', '>')) {
                                after = gt + 1;
                            }
                        }
                        after = skip_space(&chars, after);
                        if chars.get(after) == Some(&'(') {
                            if let Some(close) = matching_brace(&chars, after, ('(', ')')) {
                                body = close + 1;
                            }
                        }
                        let mut brace = body;
                        while brace < chars.len() && chars[brace] != '{' && chars[brace] != ';' {
                            brace += 1;
                        }
                        let signature: String = chars[i..brace].iter().collect();
                        out.push((qualified, file.clone(), signature));
                    }
                }
            }
            i = end;
        }
    }
    out
}

/// True when `body` calls `name`: a qualified call (`Type::name(`), or a
/// bare `name(` that is not a method call (`.name(`). A word mention is
/// not a call.
fn calls(body: &str, name: &str) -> bool {
    if name.contains("::") {
        return contains_qualified_call(body, name);
    }
    contains_bare_call(body, name)
}

fn contains_qualified_call(body: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    let mut at = 0;
    while let Some(rel) = body[at..].find(&needle) {
        let pos = at + rel;
        let before = body[..pos].chars().next_back();
        if !before.is_some_and(is_word_char) {
            return true;
        }
        at = pos + 1;
    }
    false
}

/// A bare `name(` that is neither a method call (`.name(`) nor the tail of
/// a longer identifier. A qualified call counts only as `kernel::name(`:
/// another module's `take(` is not the kernel's.
fn contains_bare_call(body: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    let mut at = 0;
    while let Some(rel) = body[at..].find(&needle) {
        let pos = at + rel;
        let before = &body[..pos];
        let counts = match before.chars().next_back() {
            None => true,
            Some('.') => false,
            Some(c) if is_word_char(c) => false,
            Some(':') => {
                let qualifier = before
                    .trim_end_matches(':')
                    .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .next()
                    .unwrap_or("");
                qualifier == "kernel"
            }
            Some(_) => true,
        };
        if counts {
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
    if token.starts_with("books.") {
        // The exact tool registrations, or an exact name the books declare
        // (a setting key such as `books.cash_accounts`).
        if defs.product_surfaces.contains(token) || defs.literals.contains(token) {
            return Ok(Class::ProductSurface);
        }
        return Err(format!("product surface {token} names no tool or setting"));
    }
    if token.starts_with("books:") {
        // A scope (`books:post`) or a resource URI (`books://digest`), each
        // an exact name in the source.
        if defs.literals.contains(token) {
            return Ok(Class::ProductSurface);
        }
        return Err(format!(
            "product surface {token} names no scope or resource"
        ));
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
            return resolve(item, &defs, &bases, &all_paths);
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
                classes.push(resolve(&format!("{prefix}{item}"), &defs, &bases, &all_paths)?);
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
    if token.contains("::") {
        return resolve_qualified(token, &defs, &bases, &all_paths);
    }
    // A harness before an item: a proof function is never an ordinary item,
    // so deleting its `#[kani::proof]` attribute leaves the claim unbacked.
    if defs.harnesses.contains(token) {
        return Ok(Class::KaniHarness);
    }
    if defs.items.contains(token) {
        return Ok(Class::RustItem);
    }
    // An imported type (`Decimal`): a real item, though defined elsewhere.
    if defs.imports.contains(token) && token.chars().next().is_some_and(|c| c.is_ascii_uppercase())
    {
        return Ok(Class::RustItem);
    }
    if defs.tests.contains(token) {
        return Ok(Class::Test);
    }
    if defs.fuzz.contains(token) {
        return Ok(Class::FuzzTarget);
    }
    // A suffix fragment of a harness name, as in `_at_tenths`.
    if token.starts_with('_')
        && (defs.harnesses.iter().any(|h| h.ends_with(token))
            || defs.tests.iter().any(|t| t.ends_with(token)))
    {
        return Ok(Class::KaniHarness);
    }
    if is_crate_name(token, defs) {
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
    if is_kebab(token) {
        return Err(format!(
            "option-like name {token} names no option or example"
        ));
    }
    Err(format!("{token} names no known item"))
}

/// The associated items of types the tree does not define, which the
/// claims cite today. Everything else is checked against a type's own
/// items; a new name here is a deliberate addition, not a way past the
/// check.
const EXTERNAL_ITEMS: &[&str] = &["NaiveDate::MAX"];

/// The package name a token names (a citation writes `ledger_core` for the
/// `ledger-core` package).
fn canonical_crate<'a>(token: &str, defs: &'a Definitions) -> Option<&'a str> {
    let dashed = token.replace('_', "-");
    defs.crates
        .iter()
        .find(|name| **name == token || **name == dashed)
        .map(String::as_str)
}

/// True when `token` names a type the source defines, or an uppercase
/// imported type (whose items are then the explicit [`EXTERNAL_ITEMS`]).
fn is_typeish(token: &str, defs: &Definitions) -> bool {
    defs.types.contains(token)
        || (defs.imports.contains(token)
            && token.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
}

/// True when `parts` names a real module path of `krate`.
fn module_exists(krate: &str, parts: &[&str], defs: &Definitions) -> bool {
    let full = std::iter::once(krate)
        .chain(parts.iter().copied())
        .collect::<Vec<_>>()
        .join("::");
    defs.modules.contains(&full)
}

/// A `::` path, walked whole: an optional crate name, then a module path
/// whose every segment is a real module, then the type or item defined
/// there.
fn resolve_qualified(
    token: &str,
    defs: &Definitions,
    bases: &[PathBuf],
    all_paths: &[String],
) -> Result<Class, String> {
    let mut segments: Vec<&str> = token.split("::").collect();
    if matches!(segments.first(), Some(&"crate") | Some(&"self")) {
        segments.remove(0);
    }
    let mut krate: Option<&str> = None;
    if let Some(first) = segments.first().copied() {
        if let Some(found) = canonical_crate(first, defs) {
            krate = Some(found);
            segments.remove(0);
        }
    }
    let Some(item) = segments.last().copied() else {
        return Err(format!("{token} names no item"));
    };
    if segments.len() == 1 {
        return resolve(item, &defs, &bases, &all_paths);
    }
    let container = segments[segments.len() - 2];
    let candidates: Vec<&str> = match krate {
        Some(name) => vec![name],
        None => defs.crates.iter().map(String::as_str).collect(),
    };
    if is_typeish(container, defs) {
        // The segments before the type must still be a real module path:
        // `bogus::Ledger::post_entry` is no path to `Ledger`.
        let modules = &segments[..segments.len() - 2];
        if !modules.is_empty() && !candidates.iter().any(|k| module_exists(k, modules, defs)) {
            return Err(format!(
                "{token}: `{}` is no module path",
                modules.join("::")
            ));
        }
        let qualified = format!("{container}::{item}");
        if defs.qualified.contains(&qualified) {
            return Ok(Class::RustItem);
        }
        if !defs.types.contains(container) && EXTERNAL_ITEMS.contains(&qualified.as_str()) {
            return Ok(Class::RustItem);
        }
        return Err(format!("{token} names no `{qualified}`"));
    }
    // Every segment but the item is a module of the crate, in order.
    let modules = &segments[..segments.len() - 1];
    for krate in &candidates {
        if !module_exists(krate, modules, defs) {
            continue;
        }
        let full = std::iter::once(*krate)
            .chain(modules.iter().copied())
            .collect::<Vec<_>>()
            .join("::");
        if defs
            .module_items
            .get(&full)
            .is_some_and(|items| items.contains(item))
        {
            return Ok(Class::RustItem);
        }
    }
    Err(format!(
        "{token}: `{}` is no module path or the item is not there",
        modules.join("::")
    ))
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

/// The audited theorem list, parsed once.
fn audited_theorems() -> &'static BTreeSet<String> {
    static AUDITED: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    AUDITED.get_or_init(|| bookspec_checker::audited_theorems(bookspec_checker::audit_lean()))
}

fn is_audited(name: &str) -> bool {
    let bare = name.strip_prefix("Bookspec.").unwrap_or(name);
    // Audit.lean lists bare names (inside `namespace Bookspec`); a citation
    // may write either form, but never a namespace suffix.
    audited_theorems().contains(bare) || audited_theorems().contains(name)
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

/// A `[package] name` of one of the crates (dashed or underscored).
fn is_crate_name(token: &str, defs: &Definitions) -> bool {
    let dashed = token.replace('_', "-");
    defs.crates.contains(token) || defs.crates.contains(&dashed)
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

/// Record a harness's law row; a harness listed twice with different laws
/// is a conflict, not a precedence.
fn record_law(
    laws: &mut BTreeMap<String, u32>,
    failures: &mut Vec<String>,
    name: String,
    law: u32,
    source: &str,
) {
    match laws.get(&name) {
        Some(&existing) if existing != law => failures.push(format!(
            "harness `{name}` is listed with law {existing} and law {law} ({source}); one row says the law"
        )),
        Some(_) => {}
        None => {
            laws.insert(name, law);
        }
    }
}

/// The `BS-` ids on the source-citation line of the spec block directly
/// above a kernel function's signature (attributes may sit between). Only
/// that line is read, never every `BS-` mention in the block: prose may
/// cite a rule the function does not serve (`fx_booked` mentions
/// `BS-ledger-14` as the CHOICE it follows). The signature identifies the
/// item, so the free `flip` and `Direction::flip` read their own blocks.
fn source_ids(text: &str, signature: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let Some(pos) = text.find(signature) else {
        return ids;
    };
    let line_start = text[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    for line in text[..line_start].lines().rev() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("///") {
            if let Some(at) = rest.find("SOURCE:") {
                for word in rest[at..].split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
                    if word.starts_with("BS-") {
                        ids.insert(word.to_string());
                    }
                }
            }
        } else if !trimmed.starts_with("#[") {
            break;
        }
    }
    ids
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
        record_law(&mut laws, &mut failures, name, law, "docs/verification.md");
    }
    let mut proof_notes = Vec::new();
    walk(&root.join("docs/proofs"), "md", &mut proof_notes);
    for path in &proof_notes {
        let source = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        for (name, law) in law_rows(&read(path)) {
            record_law(&mut laws, &mut failures, name, law, &source);
        }
    }
    let laws_md = bookspec_checker::spec_files()
        .into_iter()
        .find(|(name, _)| *name == "laws.md")
        .map(|(_, text)| text)
        .expect("bookspec's spec/laws.md");
    let mut law_rules: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    for line in laws_md.lines() {
        let cells: Vec<&str> = line.trim().split('|').map(str::trim).collect();
        if cells.len() < 5 {
            continue;
        }
        let Ok(number) = cells[1].parse::<u32>() else {
            continue;
        };
        let rules: BTreeSet<String> = cells[3]
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|w| w.starts_with("BS-"))
            .map(str::to_string)
            .collect();
        law_rules.insert(number, rules);
    }
    let law_numbers: BTreeSet<u32> = law_rules.keys().copied().collect();
    assert!(!law_numbers.is_empty(), "bookspec numbers its laws");

    // The kernel sources, by file name, for the spec block above each one.
    let mut kernel_texts: BTreeMap<String, String> = BTreeMap::new();
    let mut kernel_paths = Vec::new();
    walk(
        &root.join("crates/ledger-core/src/kernel"),
        "rs",
        &mut kernel_paths,
    );
    for path in &kernel_paths {
        if let Some(file) = path.file_name().and_then(|n| n.to_str()) {
            kernel_texts.insert(file.to_string(), read(path));
        }
    }

    let mut kernel_checked = 0usize;
    let kernel_seen = kernel.len();
    let mut seen_exceptions = BTreeSet::new();
    let mut right_law_applied = 0usize;
    let mut right_law_skipped = 0usize;
    let mut right_law_skipped_names = BTreeSet::new();
    for (name, file, signature) in &kernel {
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
        // The harness's law row must serve the rule the function's own spec
        // block cites: a `fx_booked` harness mapped to law 4 would fail.
        let ids = kernel_texts
            .get(file)
            .map(|text| source_ids(text, signature))
            .unwrap_or_default();
        let fn_laws: BTreeSet<u32> = harnesses
            .iter()
            .filter_map(|h| laws.get(*h).copied())
            .filter(|n| law_numbers.contains(n))
            .collect();
        let applicable: Vec<u32> = fn_laws
            .iter()
            .copied()
            .filter(|n| law_rules.get(n).is_some_and(|rules| !rules.is_empty()))
            .collect();
        if !ids.is_empty() && !applicable.is_empty() {
            right_law_applied += 1;
            let shared = applicable.iter().any(|n| {
                law_rules
                    .get(n)
                    .is_some_and(|rules| rules.iter().any(|id| ids.contains(id)))
            });
            if !shared {
                failures.push(format!(
                    "kernel `{name}` ({file}) cites {}; its harness law(s) {applicable:?} cite no shared rule",
                    ids.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
        } else {
            right_law_skipped += 1;
            right_law_skipped_names.insert(name.clone());
        }
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
            kernel.iter().any(|(fn_name, _, _)| fn_name == name),
            "excepted kernel function {name} does not exist; the list is stale"
        );
        assert!(
            seen_exceptions.contains(*name),
            "excepted kernel function {name} is fully backed now; remove it from EXCEPTIONS"
        );
    }
    assert_eq!(
        EXCEPTIONS.len(),
        14,
        "the kernel exception list changes only on purpose"
    );
    // No ordinary function may fall through the right-law check: the
    // skipped set is exactly these wrappers, each excepted because its
    // law is its inner function's (or it has no law at all).
    let expected_skips: BTreeSet<String> = SKIPPED.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        right_law_skipped_names, expected_skips,
        "the set of kernel functions the right-law check cannot apply to changed"
    );
    for name in SKIPPED {
        assert!(
            EXCEPTIONS.iter().any(|(excepted, _, _)| excepted == name),
            "skipped kernel function {name} is not excepted; it must cite its rule"
        );
    }

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
    println!(
        "harness law matches the function's rule: {right_law_applied} applied, {right_law_skipped} skipped"
    );
    println!(
        "  skipped (excepted, no law or no rule): {}",
        right_law_skipped_names
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
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

/// The kernel functions the right-law check cannot apply to, pinned
/// exactly: each is excepted above, and lacks either a harness law or a
/// cited rule for one. `movements` and `reversal` allocate, so every
/// harness proves their fixed-array helper; `round_places`, `round_minor`
/// and `fx_booked_minor` are Decimal wrappers whose law is the inner
/// function's; `Direction::key` is a wire name and `MonthPart::is_valid` a
/// range predicate, with no law at all. A new name here is a deliberate
/// widening, never a silent hole.
const SKIPPED: &[&str] = &[
    "movements",
    "reversal",
    "Direction::key",
    "MonthPart::is_valid",
    "round_places",
    "round_minor",
    "fx_booked_minor",
];

#[test]
fn made_up_names_do_not_resolve() {
    let root = repo_root();
    let (defs, bases, all_paths) = main_index(&root);
    for token in [
        "no_such_fn_xyz",
        "NoSuch.theorem",
        "books.no_such_tool_xyz",
        // An associated item the type does not have.
        "Ledger::no_such_method",
        // A real item under the wrong type (`target` is the free fn in
        // `shares`, not a `Ledger` method).
        "Ledger::target",
        // A test helper, not a `#[test]` or `proptest!` function.
        "arb_cents",
        // A product name that is not a registered tool.
        "books.add",
        // A Lean name that is not audited, with or without the prefix.
        "Bookspec.NoSuch.theorem",
    ] {
        assert!(
            resolve(token, &defs, &bases, &all_paths).is_err(),
            "made-up name `{token}` must not resolve"
        );
    }
    // A directory is not a crate (`tests` resolves only as `mod tests`).
    assert!(
        !is_crate_name("tests", &defs),
        "a directory name must not pass as a crate"
    );
}

/// `calls` finds a call, never a mention: a method call, a bare mention, or
/// another module's function of the same name is not the kernel's call.
#[test]
fn calls_finds_a_call_not_a_mention() {
    assert!(
        !calls("x.take(5)", "take"),
        "a method call is not the kernel's"
    );
    assert!(!calls("take", "take"), "a bare mention is not a call");
    assert!(
        !calls("other::take(5)", "take"),
        "another module's take is not the kernel's"
    );
    assert!(calls("kernel::take(5)", "take"), "a kernel call counts");
    assert!(calls("take(5)", "take"), "an imported bare call counts");
    assert!(calls("kernel::Direction::flip(d)", "Direction::flip"));
    assert!(
        !calls("kernel::Direction::flip(d)", "flip"),
        "a type's flip is not the free flip"
    );
}

/// A field belongs to its own type, and a method to its own type: a field
/// of `Iif` never backs `TaxSummary::…`, nor the reverse.
#[test]
fn a_field_belongs_to_its_own_type() {
    let root = repo_root();
    let (defs, bases, all_paths) = main_index(&root);
    for token in [
        "Iif::changes",
        "Iif::receivable_payable",
        "Iif::warnings",
        "TaxSummary::form_4797",
    ] {
        assert_eq!(
            resolve(token, &defs, &bases, &all_paths),
            Ok(Class::RustItem),
            "`{token}` is a real item of its type"
        );
    }
    for token in ["Iif::form_4797", "TaxSummary::changes"] {
        assert!(
            resolve(token, &defs, &bases, &all_paths).is_err(),
            "`{token}` is a field of the other type and must not resolve"
        );
    }
}

#[test]
fn a_harness_listed_twice_must_agree() {
    let mut laws = BTreeMap::new();
    let mut failures = Vec::new();
    record_law(&mut laws, &mut failures, "harness".into(), 4, "a.md");
    record_law(&mut laws, &mut failures, "harness".into(), 4, "b.md");
    assert!(failures.is_empty(), "the same law twice is not a conflict");
    record_law(&mut laws, &mut failures, "harness".into(), 5, "c.md");
    assert_eq!(failures.len(), 1, "two laws for one harness is a conflict");
}

#[test]
fn harnesses_resolve_only_as_proofs() {
    let root = repo_root();
    let (defs, bases, all_paths) = main_index(&root);
    let harness = defs
        .harnesses
        .iter()
        .next()
        .expect("the proofs carry harnesses");
    assert_eq!(
        resolve(harness, &defs, &bases, &all_paths),
        Ok(Class::KaniHarness),
        "a harness classifies as a Kani harness"
    );
    // `any_proration` is a plain helper in `src/proofs/`, not a proof.
    assert!(
        !defs.harnesses.contains("any_proration"),
        "the helper is not a harness"
    );
    assert_ne!(
        resolve("any_proration", &defs, &bases, &all_paths).ok(),
        Some(Class::KaniHarness),
        "a proof helper without `#[kani::proof]` is not a harness"
    );
}
