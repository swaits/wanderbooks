#!/usr/bin/env python3
"""Rewrite wanderbooks' retired `B-n` rule references to bookspec ids.

Unit S-E of plan P14: bookspec (D-68) is now the definitive spec and
every rule's stable identifier is `BS-<area>-<n>`. This script rewrites
references in code comments, spec blocks' `SOURCE:` lines, docs and
tests from the old wanderbooks numbering (`docs/spec/rules.md` B-1..B-45)
to that identifier. It is reviewable and repeatable: running it on an
already-rewritten tree changes nothing.

Map: parsed, not hard-coded, from `docs/spec/rules.md`'s index table
(the `| B-n | BS-<area>-<n> |` rows); that file and `docs/spec/claims/`
are the map and are never rewritten, nor is `plan/` (its history keeps
its wording).

Patterns treated as rule references (n in 1..45, the mapped range):
  1. `docs/spec/rules.md` B-n, `rules.md` B-n -> `BS-<area>-<n>`. The `rules.md`
     prefix is dropped: the id names the rule wherever it lives.
     Sub-references survive untouched because only the `B-n` token is
     replaced: `rules.md` B-14 rule 11 -> `BS-statement-3 rule 11`,
     `B-37 item 7` -> `BS-statement-5 item 7`, `B-38 INVARIANT 2` ->
     `BS-arap-2 INVARIANT 2`.
  2. Any other bare `B-n` (`(B-19)`, `; B-35.`, `B-14 rule 3:`,
     `R-1/B-1`, `B-28's`, `the B-19 table`) -> `BS-<area>-<n>`. In this
     repo a word-boundary `B-<1..45>` outside the map files is always a
     rule citation; that is the judgement this rewrite encodes.

Patterns skipped (not rule references):
  * `WB-<area>-<n>`: already a wanderbooks product-rule id. The `B` is
    preceded by `W`, so no word boundary matches.
  * `B-` glued to a preceding word character, e.g. the asset-tag test
    datum `"ÄB-1"`: no word boundary, left alone.
  * Numbers outside 1..45 (there is no such rule; e.g. a future
    `B-1099`-style string would not be a citation of B-1..B-45 either --
    the full number must be in the map).
  * Anything under `plan/`, in `docs/spec/rules.md`, or in
    `docs/spec/claims/*.md` (the map itself).
  * Program-output string literals are not comments: the few that cited
    a rule (evidence-source strings, a tool description, the CPA
    package text, a GUI note) were reverted by hand after the run, so
    user-visible output and the golden files that record it are
    unchanged. The one exception is `DatedRule.catalog` in
    `crates/ledger-core/src/rules.rs`, metadata no output prints,
    which now holds the bookspec id.

WB dual-citation (task clause 1): where a sentence clearly concerns a
wanderbooks-only surface (MCP tools, the store, the viewer, the vault)
the product rule is a `WB-<area>-<n>` in that B section's claims file.
That judgement is not mechanical, so the script never guesses one; per
the task, the bookspec id alone is correct when unsure.

Usage: python3 scripts/rewrite-rule-refs.py [--dry-run] [root ...]
Default roots: crates docs AGENTS.md README.md deploy scripts .github
(a file root is rewritten in place; `scripts/rewrite-rule-refs.py`
itself is always skipped so the script's own examples stay literal).
Only text files are touched; binary files are skipped.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MAP_FILE = REPO / "docs" / "spec" / "rules.md"
SKIP_FILES = {MAP_FILE, Path(__file__).resolve()}
SKIP_DIRS = {REPO / "plan", REPO / "docs" / "spec" / "claims"}
DEFAULT_ROOTS = ["crates", "docs", "AGENTS.md", "README.md", "deploy", "scripts", ".github"]


def load_map() -> dict[int, str]:
    """B number -> bookspec id, from docs/spec/rules.md's index table."""
    mapping: dict[int, str] = {}
    row = re.compile(r"^\| B-(\d+) \| `(BS-[a-z0-9]+-\d+)` \|", re.M)
    for n, bs in row.findall(MAP_FILE.read_text(encoding="utf-8")):
        mapping[int(n)] = bs
    if sorted(mapping) != list(range(1, 46)):
        raise SystemExit(f"map in {MAP_FILE} is not B-1..B-45: {sorted(mapping)}")
    return mapping


def skipped(path: Path) -> bool:
    path = path.resolve()
    return path in SKIP_FILES or any(parent in SKIP_DIRS for parent in [path, *path.parents])


def rewrite(text: str, mapping: dict[int, str]) -> str:
    """Form 1 (`rules.md` B-n, prefix dropped), then form 2 (bare `B-n`)."""

    def sub(m: re.Match[str]) -> str:
        return mapping.get(int(m.group("n"))) or m.group(0)

    text = re.sub(r"`?(?:docs/spec/)?`?rules\.md`?\s+B-(?P<n>\d+)\b", sub, text)
    # Wrapped form: the filename ends one comment line, the number opens
    # the next (`rules.md\n    /// B-14 rule 3`): drop the filename too.
    text = re.sub(r"(?:docs/spec/)?`?rules\.md`?[ \t]*\n([ \t]*(?://[/!]?|#)?[ \t]*)\bB-(?P<n>\d+)\b", lambda m: m.group(1) + (mapping.get(int(m.group("n"))) or m.group(0)), text)
    return re.sub(r"\bB-(?P<n>\d+)\b", sub, text)


def main() -> int:
    dry = "--dry-run" in sys.argv
    roots = [a for a in sys.argv[1:] if not a.startswith("--")] or DEFAULT_ROOTS
    mapping = load_map()
    # Count by diffing tokens before/after rather than trusting counters.
    token = re.compile(r"\bB-(\d+)\b")
    total_refs = total_files = 0
    for root in roots:
        base = REPO / root
        files = [base] if base.is_file() else sorted(p for p in base.rglob("*") if p.is_file())
        for path in files:
            if skipped(path):
                continue
            try:
                before = path.read_text(encoding="utf-8")
            except (UnicodeDecodeError, ValueError):
                continue  # binary
            after = rewrite(before, mapping)
            if after == before:
                continue
            refs = sum(1 for n in token.findall(before) if int(n) in mapping)
            total_refs += refs
            total_files += 1
            print(f"{path.relative_to(REPO)}: {refs}")
            if not dry:
                path.write_text(after, encoding="utf-8")
    print(f"{'would rewrite' if dry else 'rewrote'} {total_refs} references in {total_files} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
