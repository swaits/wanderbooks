#!/usr/bin/env bash
# The Windows installer (plan P15 unit C1), from the release program already
# built at target/release/wanderbooks.exe (`cargo build --release -p
# ledger-cli`). Run from the repository root, in Git Bash on Windows (CI:
# the `windows` job of .github/workflows/release.yml) or anywhere makensis
# is:
#
#   build-windows.sh [VERSION]
#
# Puts Wanderbooks-VERSION-windows-x64-setup.exe into target/packages/VERSION/.
# VERSION defaults to the program's own (deploy/packaging/version.sh).
# WB_PROGRAM names another program file; WB_MAKENSIS another makensis.
#
# Unsigned: signing is a separate, optional step (sign-windows.ps1) that
# runs when the release workflow has the certificate's secrets.
set -euo pipefail

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$here/../../.." && pwd)
cd "$root"
own=$(sh deploy/packaging/version.sh)
version=${1:-$own}
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "build-windows: '$version' is not an x.y.z version" >&2; exit 1 ;;
esac
program=${WB_PROGRAM:-target/release/wanderbooks.exe}
out="target/packages/$version"
installer="Wanderbooks-$version-windows-x64-setup.exe"

fail() { echo "build-windows: $*" >&2; exit 1; }
test -f "$program" || fail "the release program is not at $program: run cargo build --release -p ledger-cli first"
for f in LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.txt deploy/packaging/icons/wanderbooks.ico deploy/packaging/windows/wanderbooks.nsi; do
  test -f "$f" || fail "$f is missing from the repository"
done

makensis=${WB_MAKENSIS:-}
if [ -z "$makensis" ]; then
  if command -v makensis >/dev/null 2>&1; then
    makensis=makensis
  elif [ -x "/c/Program Files (x86)/NSIS/makensis.exe" ]; then
    makensis="/c/Program Files (x86)/NSIS/makensis.exe"
  else
    fail "makensis (NSIS) is not installed: choco install nsis, or apt install nsis"
  fi
fi

# NSIS is a Windows program; Git Bash must neither rewrite its options
# (-D...) nor hand it a Unix path, so paths are made native here.
export MSYS2_ARG_CONV_EXCL='*'
native() { if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s\n' "$1"; fi; }

rm -rf "${out:?}"
mkdir -p "$out"
"$makensis" -V2 \
  "-DVERSION=$version" \
  "-DPROGRAM=$(native "$program")" \
  "-DICON=$(native deploy/packaging/icons/wanderbooks.ico)" \
  "-DLICENSES=$(native "$root")" \
  "-DOUTFILE=$(native "$out/$installer")" \
  "$(native deploy/packaging/windows/wanderbooks.nsi)"
test -s "$out/$installer" || fail "makensis finished but $out/$installer is missing or empty"
ls -l "$out"
