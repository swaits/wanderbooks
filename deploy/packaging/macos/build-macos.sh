#!/usr/bin/env bash
# The macOS app and its .dmg (plan P15 unit C1; WB-pkg-1). Run on a Mac from
# the repository root (CI: the `macos` job of .github/workflows/release.yml):
#
#   build-macos.sh [VERSION]
#
# Puts Wanderbooks-VERSION-macos-universal.dmg into target/packages/VERSION/.
#
# One UNIVERSAL program (Apple silicon and Intel in one file, `lipo`ed from
# the two release builds): one download for everyone. The program is
# WB_PROGRAM if set (any program file), else the lipo of
# target/aarch64-apple-darwin/release/wanderbooks and
# target/x86_64-apple-darwin/release/wanderbooks.
#
# Signing and notarization need the owner's Apple Developer identity and run
# only when it is given, in the environment (the release workflow sets these
# from secrets; nothing here is a secret and nothing prints one):
#   WB_SIGN_IDENTITY   the signing identity, e.g.
#                      "Developer ID Application: Name (TEAMID)", already in
#                      the keychain (import-certificate.sh does that in CI)
#   WB_NOTARY_APPLE_ID, WB_NOTARY_TEAM_ID, WB_NOTARY_PASSWORD
#                      for `xcrun notarytool` (an app-specific password)
# Without WB_SIGN_IDENTITY the app is signed ad hoc ("-"): Apple silicon
# will not start an unsigned program at all, so it runs, but Gatekeeper
# still refuses a download of it (right-click > Open once, docs/install.md).
# Without the notarization variables the .dmg is signed but not notarized.
#
# WB_NO_ICNS=1 skips the icon (no `iconutil` off a Mac): the structure
# tests build the bundle that way on Linux; a release never does.
# WB_BUNDLE_ONLY=1 stops after the .app (the structure tests).
set -euo pipefail

here=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH='' cd -- "$here/../../.." && pwd)
cd "$root"
fail() { echo "build-macos: $*" >&2; exit 1; }
notice() { echo "::notice title=$1::$2"; echo "build-macos: $2"; }

own=$(sh deploy/packaging/version.sh)
version=${1:-$own}
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "'$version' is not an x.y.z version" ;;
esac
bundle_id=${WB_BUNDLE_ID:-ai.wanderbooks.app}
out="target/packages/$version"
stage="target/packages/.stage-macos-$version"
app="$stage/Wanderbooks.app"
dmg="Wanderbooks-$version-macos-universal.dmg"

for f in LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.txt deploy/packaging/icons/512x512.png \
         deploy/packaging/macos/Info.plist.in deploy/packaging/macos/entitlements.plist; do
  test -f "$f" || fail "$f is missing from the repository"
done

# 1. The program: universal, or the one given.
rm -rf "${stage:?}"
mkdir -p "$stage" "$out"
program=${WB_PROGRAM:-}
if [ -z "$program" ]; then
  arm=target/aarch64-apple-darwin/release/wanderbooks
  intel=target/x86_64-apple-darwin/release/wanderbooks
  test -f "$arm" || fail "$arm is missing: cargo build --release -p ledger-cli --target aarch64-apple-darwin"
  test -f "$intel" || fail "$intel is missing: cargo build --release -p ledger-cli --target x86_64-apple-darwin"
  lipo -create -output "$stage/wanderbooks-universal" "$arm" "$intel"
  archs=$(lipo -archs "$stage/wanderbooks-universal")
  case "$archs" in
    *arm64*x86_64*|*x86_64*arm64*) ;;
    *) fail "the universal program holds '$archs', not both arm64 and x86_64" ;;
  esac
  program="$stage/wanderbooks-universal"
fi
test -f "$program" || fail "the program is not at $program"

# 2. The bundle. The program's path inside it,
#    /Applications/Wanderbooks.app/Contents/MacOS/wanderbooks, is the one an
#    assistant is registered with: it does not change when the app is
#    replaced by a newer one dragged over it.
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$program" "$app/Contents/MacOS/wanderbooks"
chmod 755 "$app/Contents/MacOS/wanderbooks"
sed -e "s/@VERSION@/$version/g" -e "s/@BUNDLE_ID@/$bundle_id/g" \
  deploy/packaging/macos/Info.plist.in > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
cp LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.txt "$app/Contents/Resources/"
if [ "${WB_NO_ICNS:-}" = 1 ]; then
  notice "no icon" "WB_NO_ICNS=1: the bundle has no icon (a test build, never a release)"
else
  command -v iconutil >/dev/null || fail "iconutil is not here: build the icon on a Mac, or set WB_NO_ICNS=1 for a test bundle"
  iconset="$stage/wanderbooks.iconset"
  mkdir -p "$iconset"
  src=deploy/packaging/icons/512x512.png
  for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$src" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  done
  for size in 16 32 128 256; do
    double=$((size * 2))
    sips -z "$double" "$double" "$src" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
  done
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/wanderbooks.icns"
fi
if [ "${WB_BUNDLE_ONLY:-}" = 1 ]; then
  echo "build-macos: bundle at $app (WB_BUNDLE_ONLY=1)"
  exit 0
fi

# 3. Sign: hardened runtime, the entitlements, a secure timestamp.
identity=${WB_SIGN_IDENTITY:-}
if [ -z "$identity" ]; then
  notice "macOS signing skipped" "no WB_SIGN_IDENTITY: the app is signed ad hoc, so it runs here, but macOS Gatekeeper will refuse a download of it until it is signed with a Developer ID and notarized"
  identity=-
  timestamp=--timestamp=none
else
  timestamp=--timestamp
fi
codesign --force --options runtime "$timestamp" \
  --entitlements deploy/packaging/macos/entitlements.plist \
  --sign "$identity" "$app"
codesign --verify --deep --strict --verbose=2 "$app"

# 4. The .dmg: the app and a shortcut to Applications (drag to install).
mkdir -p "$stage/dmg"
cp -R "$app" "$stage/dmg/"
ln -s /Applications "$stage/dmg/Applications"
rm -f "${out:?}/${dmg:?}"
hdiutil create -volname "Wanderbooks" -srcfolder "$stage/dmg" -ov -format UDZO "$out/$dmg"
if [ "$identity" != "-" ]; then
  codesign --force --timestamp --sign "$identity" "$out/$dmg"
fi

# 5. Notarize and staple (the Developer ID signature and Apple's ticket).
if [ "$identity" != "-" ] && [ -n "${WB_NOTARY_APPLE_ID:-}" ] && [ -n "${WB_NOTARY_TEAM_ID:-}" ] && [ -n "${WB_NOTARY_PASSWORD:-}" ]; then
  xcrun notarytool submit "$out/$dmg" \
    --apple-id "$WB_NOTARY_APPLE_ID" --team-id "$WB_NOTARY_TEAM_ID" \
    --password "$WB_NOTARY_PASSWORD" --wait
  xcrun stapler staple "$out/$dmg"
  xcrun stapler validate "$out/$dmg"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$out/$dmg"
else
  notice "macOS notarization skipped" "no signing identity or no WB_NOTARY_APPLE_ID / WB_NOTARY_TEAM_ID / WB_NOTARY_PASSWORD: the .dmg is not notarized, and macOS will warn on the first open"
fi
rm -rf "${stage:?}"
ls -l "$out"
