#!/usr/bin/env bash
# A release folder on this machine, for trying `wanderbooks update` without
# a network or the owner's keys (P11a step 2; docs/packaging.md).
#
#   local-release.sh FILE VERSION OUTDIR
#
# Makes a throwaway signing key in OUTDIR/key (never a release key), signs
# FILE with it through cargo-packager, copies it beside a manifest.json in
# the shape `wanderbooks update` reads, and leaves the key's public half in
# OUTDIR/release.pub. Then:
#
#   wanderbooks update --feed file://OUTDIR/manifest.json \
#       --trusted-key OUTDIR/release.pub
set -euo pipefail
file=${1:?usage: local-release.sh FILE VERSION OUTDIR}
version=${2:?usage: local-release.sh FILE VERSION OUTDIR}
out=${3:?usage: local-release.sh FILE VERSION OUTDIR}
mkdir -p "$out"
if [ ! -f "$out/key" ]; then
    cargo packager signer generate --path "$out/key" --password "" --ci >/dev/null
    cp "$out/key.pub" "$out/release.pub"
fi
case "$(uname -m)" in arm64) arch=aarch64 ;; *) arch=$(uname -m) ;; esac
platform="$(uname -s | tr '[:upper:]' '[:lower:]')-$arch"
name="wanderbooks-$version-$platform"
cp "$file" "$out/$name"
cargo packager signer sign "$out/$name" --private-key "$out/key" --password "" >/dev/null
sha=$(sha256sum "$out/$name" | cut -d' ' -f1)
sig=$(tr -d '\n' <"$out/$name.sig")
printf '{"version":"%s","notes":"a local release","platforms":{"%s":{"url":"%s","sha256":"%s","signature":"%s"}}}\n' \
    "$version" "$platform" "$name" "$sha" "$sig" >"$out/manifest.json"
echo "feed: file://$(cd "$out" && pwd)/manifest.json   key: $out/release.pub"
