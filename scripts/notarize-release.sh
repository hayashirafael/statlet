#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/dist}"
app="$output_dir/Statlet.app"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)"
archive="$output_dir/Statlet-v${version}-macos-arm64.zip"
checksum="$archive.sha256"

: "${STATLET_SIGNING_IDENTITY:?Set STATLET_SIGNING_IDENTITY to a Developer ID Application identity}"
: "${STATLET_NOTARY_PROFILE:?Set STATLET_NOTARY_PROFILE to a notarytool keychain profile}"

codesign --verify --deep --strict "$app"
xcrun notarytool submit "$archive" --keychain-profile "$STATLET_NOTARY_PROFILE" --wait
xcrun stapler staple "$app"
xcrun stapler validate "$app"
spctl --assess --type execute --verbose=4 "$app"

rm -f "$archive" "$checksum"
ditto -c -k --sequesterRsrc --keepParent "$app" "$archive"
(
    cd "$output_dir"
    shasum -a 256 "$(basename "$archive")" >"$(basename "$checksum")"
)
"$repo_root/scripts/verify-bundle.sh" "$app"

echo "Notarized release: $archive"
