#!/usr/bin/env bash
# The Linux packages (P11a step 3), from the release program already built
# at target/release/wanderbooks (`cargo build --release -p ledger-cli`).
#
#   build-linux.sh [VERSION] [FORMATS]
#
# Puts into target/packages/VERSION/: the .deb, the AppImage and the
# pacman tarball with its PKGBUILD (cargo-packager), and the .rpm
# (cargo-generate-rpm). VERSION defaults to the program's own; a different
# one repackages the same program under that version, which is how the
# install-and-upgrade test gets its second release. FORMATS is
# cargo-packager's list (default deb,appimage,pacman); the .rpm is always
# built. Run from the repository root.
#
# The program is target/release/wanderbooks, or the file WB_PROGRAM names:
# a package must run on the oldest system it claims, and a program linked
# on a newer one (this machine's glibc) fails there with "GLIBC_2.43 not
# found". CI builds on ubuntu-24.04; `just test-linux-packages` builds in a
# Debian trixie container.
set -euo pipefail
own=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/ledger-cli/Cargo.toml | head -n1)
version=${1:-$own}
formats=${2:-deb,appimage,pacman}
out="target/packages/$version"
program=${WB_PROGRAM:-target/release/wanderbooks}
test -x "$program" || { echo "build the release program first: cargo build --release -p ledger-cli" >&2; exit 1; }
rm -rf "$out"; mkdir -p "$out"
stage="target/packages/.stage-$version"
rm -rf "$stage"; mkdir -p "$stage"
cp "$program" "$stage/wanderbooks"
config=$(mktemp --suffix=.toml)
trap 'rm -f "$config"' EXIT
# cargo-packager reads paths relative to the config file, which is in /tmp
# here, so they are made absolute.
sed "s/^version = \".*\"/version = \"$version\"/
     s#^out-dir = .*#out-dir = \"$PWD/$out\"#
     s#\"target/release\"#\"$PWD/$stage\"#
     s#\"deploy/packaging/#\"$PWD/deploy/packaging/#
     s#\"LICENSE-MIT\"#\"$PWD/LICENSE-MIT\"#" \
    deploy/packaging/packager.toml >"$config"
# The AppImage tools are themselves AppImages; run them without FUSE.
export APPIMAGE_EXTRACT_AND_RUN=1
cargo packager --config "$config" --formats "$formats" --release
assets='assets = [
  { source = "'$PWD/$stage'/wanderbooks", dest = "/usr/bin/wanderbooks", mode = "0755" },
  { source = "deploy/packaging/install-method-rpm", dest = "/usr/share/wanderbooks/install-method", mode = "0644" },
  { source = "deploy/packaging/wanderbooks-static.desktop", dest = "/usr/share/applications/wanderbooks.desktop", mode = "0644" },
]'
cargo generate-rpm -p crates/ledger-cli --set-metadata "version=\"$version\"" --set-metadata "$assets" -o "$out/"
rm -rf "$stage"
ls -l "$out"
