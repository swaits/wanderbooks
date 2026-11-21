#!/usr/bin/env bash
# Put the owner's Developer ID certificate into a temporary keychain on a CI
# macOS runner, so `codesign` can use it (plan P15 unit C1). Run by the
# release workflow only when the secrets exist. Takes the certificate (a .p12,
# base64) and its password from the environment, never the command line:
#   APPLE_CERTIFICATE_P12_BASE64, APPLE_CERTIFICATE_PASSWORD
# Prints no secret. The keychain lives in RUNNER_TEMP and goes with the
# runner.
set -euo pipefail
: "${APPLE_CERTIFICATE_P12_BASE64:?the certificate secret is empty}"
: "${APPLE_CERTIFICATE_PASSWORD:?the certificate password secret is empty}"
: "${RUNNER_TEMP:?not on a GitHub runner}"
keychain="$RUNNER_TEMP/wanderbooks-signing.keychain-db"
password=$(uuidgen)
p12="$RUNNER_TEMP/wanderbooks-signing.p12"
printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$p12"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain" login.keychain
rm -f "$p12"
security find-identity -v -p codesigning "$keychain"
