#!/usr/bin/env bash
# The macOS bundle (P11a step 6), UNSIGNED, from the release program
# already built at target/release/wanderbooks (`cargo build --release -p
# ledger-cli`). Run on a Mac from the repository root:
#
#   build-macos.sh [VERSION]
#
# Puts Wanderbooks_VERSION_<arch>.dmg into target/packages/VERSION/. It
# is not signed or notarized (step 5): Gatekeeper refuses it on a Mac
# that downloaded it, and the owner opens it with right-click > Open.
# Portable shell: macOS has BSD sed and no GNU mktemp options.
set -euo pipefail
own=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/ledger-cli/Cargo.toml | head -n1)
version=${1:-$own}
out="target/packages/$version"
program=${WB_PROGRAM:-target/release/wanderbooks}
test -x "$program" || { echo "build the release program first: cargo build --release -p ledger-cli" >&2; exit 1; }
rm -rf "$out"; mkdir -p "$out"
stage="target/packages/.stage-$version"
rm -rf "$stage"; mkdir -p "$stage"
cp "$program" "$stage/wanderbooks"
config="$stage/packager.toml"
# cargo-packager reads paths relative to the config file, which is in the
# stage folder here, so they are made absolute.
sed -e "s/^version = \".*\"/version = \"$version\"/" \
    -e "s#^out-dir = .*#out-dir = \"$PWD/$out\"#" \
    -e "s#\"target/release\"#\"$PWD/$stage\"#" \
    -e "s#\"deploy/packaging/#\"$PWD/deploy/packaging/#" \
    -e "s#\"LICENSE-MIT\"#\"$PWD/LICENSE-MIT\"#" \
    deploy/packaging/packager-macos.toml >"$config"
cargo packager --config "$config" --formats dmg --release
rm -rf "$stage"
ls -l "$out"
