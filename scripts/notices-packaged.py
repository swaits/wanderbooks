#!/usr/bin/env python3
"""The third part of THIRD-PARTY-NOTICES.txt (`scripts/notices.sh`).

cargo-about gives each crate the text of the licence it names. Where it
cannot match a crate's own licence file to that licence (a compound file:
two licences in one, or a text with a preamble), or the crate was packaged
with no licence file at all, it prints the licence's standard text, whose
copyright line is a placeholder ("Copyright (c) <year> <copyright
holders>"). MIT, BSD and ISC ask for the copyright notice itself, so a
placeholder does not carry what they ask for.

This script reads cargo-about's output (argument 1), finds every section
whose text holds such a placeholder, and writes to standard output:

  - every licence file those crates were packaged with, whole, each
    distinct text once, under the crates that carry it;
  - the crates packaged with no licence file, each with the repository its
    manifest names, where its notices are.

It reads the crates where cargo has them (`cargo metadata --locked
--offline`), so the output depends on Cargo.lock and nothing else.
"""

import json
import os
import re
import subprocess
import sys

RULE = "=" * 80
THIN = "-" * 80
PLACEHOLDER = re.compile(r"<(year|owner|copyright holders?|name of author)>", re.I)
LICENCE_FILE = re.compile(r"^(licen[sc]e|copying|copyright|notice|unlicense)", re.I)


def placeholder_crates(about):
    """(name, version) of every crate in a section with a placeholder."""
    lines = about.split("\n")
    # A section starts at a rule, a name, a rule.
    starts = [
        i
        for i in range(len(lines) - 2)
        if lines[i] == RULE and lines[i + 2] == RULE and lines[i + 1] != ""
    ]
    found = set()
    for n, start in enumerate(starts):
        end = starts[n + 1] if n + 1 < len(starts) else len(lines)
        body = lines[start + 3 : end]
        if "Used by:" not in body:
            continue
        at = body.index("Used by:") + 1
        crates = []
        while at < len(body) and body[at].startswith("  "):
            name, version = body[at].split()
            crates.append((name, version))
            at += 1
        if PLACEHOLDER.search("\n".join(body[at:])):
            found.update(crates)
    return sorted(found)


def packages():
    """(name, version) -> (directory, repository), from cargo."""
    out = subprocess.run(
        [
            "cargo", "metadata", "--format-version", "1", "--locked", "--offline",
            "--all-features", "--manifest-path", "crates/ledger-cli/Cargo.toml",
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return {
        (p["name"], p["version"]): (os.path.dirname(p["manifest_path"]), p.get("repository") or "")
        for p in json.loads(out)["packages"]
    }


def licence_files(directory):
    """The licence files at a crate's root, and in a LICENSES folder there."""
    files = []
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        if not LICENCE_FILE.match(name):
            continue
        if os.path.isfile(path):
            files.append((name, path))
        elif os.path.isdir(path):
            for inner in sorted(os.listdir(path)):
                if os.path.isfile(os.path.join(path, inner)):
                    files.append((f"{name}/{inner}", os.path.join(path, inner)))
    return files


def text_of(path):
    with open(path, "rb") as f:
        raw = f.read()
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        text = raw.decode("latin-1")
    lines = [line.rstrip() for line in text.replace("\r\n", "\n").split("\n")]
    return "\n".join(lines).strip("\n")


def main():
    with open(sys.argv[1], encoding="utf-8") as f:
        crates = placeholder_crates(f.read())
    if not crates:
        return
    known = packages()
    texts = {}  # the text's words -> (text, ["name version (file)"]), first seen first
    bare = []
    for name, version in crates:
        if (name, version) not in known:
            sys.exit(f"notices: cargo metadata does not list {name} {version}")
        directory, repository = known[(name, version)]
        files = licence_files(directory)
        if not files:
            if not repository:
                sys.exit(
                    f"notices: {name} {version} has no licence file and names no "
                    "repository: add its notice by hand in scripts/notices-packaged.py"
                )
            bare.append((f"{name} {version}", repository))
        for label, path in files:
            text = text_of(path)
            # The same text, wrapped or indented differently, is one text.
            key = " ".join(text.split())
            texts.setdefault(key, (text, []))[1].append(f"{name} {version} ({label})")

    print(RULE)
    print("LICENCE FILES AS PACKAGED")
    print(RULE)
    print()
    print("Some sections above give a licence's standard text, whose copyright line")
    print("is a placeholder: the crate's own licence file is a compound one (two")
    print("licences in one file, or a text with a preamble), or the crate was")
    print("packaged with no licence file. For every crate in such a section, the")
    print("licence files it was packaged with are given here, whole. A text that")
    print("several crates carry word for word is given once.")
    print()
    for text, carriers in texts.values():
        print(THIN)
        print("Packaged with:")
        for carrier in carriers:
            print(f"  {carrier}")
        print(THIN)
        print()
        print(text)
        print()
    if bare:
        print(THIN)
        print("Packaged with no licence file")
        print(THIN)
        print()
        print("The crates below name their licence in their manifest (it is the one")
        print("they are listed under above) and were packaged with no licence file.")
        print("Their copyright notices are in their source repositories:")
        print()
        width = max(len(crate) for crate, _ in bare)
        for crate, repository in bare:
            print(f"  {crate.ljust(width)}  {repository}")
        print()


main()
