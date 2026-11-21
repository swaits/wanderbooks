#!/bin/sh
# The program's version: the one place it is written is `version = "..."`
# in crates/ledger-cli/Cargo.toml (plan P15 C1, WB-pkg-6). Every package, the
# release workflow and the installer take it from here; the program itself
# reports the same file's version through `CARGO_PKG_VERSION`.
#
#   deploy/packaging/version.sh        prints 0.1.0
#
# A version that is not three plain numbers is refused: the Windows
# installer's file-version fields and the macOS bundle's both need them.
set -eu
here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$here/../../crates/ledger-cli/Cargo.toml" | head -n1)
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "version.sh: no x.y.z version in crates/ledger-cli/Cargo.toml (found: '$version')" >&2; exit 1 ;;
esac
case "$version" in
  *[!0-9.]*) echo "version.sh: '$version' is not three plain numbers" >&2; exit 1 ;;
esac
printf '%s\n' "$version"
