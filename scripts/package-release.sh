#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$repo_root/dist}"
target="${STATLET_TARGET:-aarch64-apple-darwin}"
version="1.0.0"
export MACOSX_DEPLOYMENT_TARGET=14.0

if [[ -z "$output_dir" || "$output_dir" == "/" ]]; then
    echo "Refusing unsafe release output directory: '$output_dir'" >&2
    exit 1
fi
if [[ "$target" != "aarch64-apple-darwin" ]]; then
    echo "Statlet v1 supports Apple Silicon only; got target '$target'" >&2
    exit 1
fi

staging="$(mktemp -d "${TMPDIR:-/tmp}/statlet-release.XXXXXX")"
trap 'rm -rf "$staging"' EXIT

cargo build \
    --manifest-path "$repo_root/Cargo.toml" \
    --release \
    --locked \
    --target "$target"

app="$staging/Statlet.app"
contents="$app/Contents"
resources="$contents/Resources"
iconset="$staging/AppIcon.iconset"

mkdir -p "$contents/MacOS" "$resources" "$iconset"
install -m 0755 "$repo_root/target/$target/release/statlet" "$contents/MacOS/Statlet"
install -m 0644 "$repo_root/packaging/Info.plist" "$contents/Info.plist"
install -m 0644 "$repo_root/packaging/PrivacyInfo.xcprivacy" "$resources/PrivacyInfo.xcprivacy"
install -m 0644 "$repo_root/LICENSE" "$resources/LICENSE"
install -m 0644 "$repo_root/NOTICE" "$resources/NOTICE"
install -m 0644 "$repo_root/packaging/THIRD_PARTY_LICENSES.html" "$resources/THIRD_PARTY_LICENSES.html"

for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips --resampleHeightWidth "$size" "$size" "$repo_root/packaging/AppIcon.png" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips --resampleHeightWidth "$double" "$double" "$repo_root/packaging/AppIcon.png" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns "$iconset" --output "$resources/AppIcon.icns"

if [[ -n "${STATLET_SIGNING_IDENTITY:-}" ]]; then
    codesign \
        --force \
        --options runtime \
        --timestamp \
        --sign "$STATLET_SIGNING_IDENTITY" \
        "$app"
else
    echo "No Developer ID configured; applying an ad-hoc hardened-runtime signature." >&2
    codesign --force --options runtime --sign - "$app"
fi

"$repo_root/scripts/verify-bundle.sh" "$app"

mkdir -p "$output_dir"
final_app="$output_dir/Statlet.app"
archive="$output_dir/Statlet-v${version}-macos-arm64.zip"
checksum="$archive.sha256"
rm -rf "$final_app"
rm -f "$archive" "$checksum"
ditto "$app" "$final_app"
ditto -c -k --sequesterRsrc --keepParent "$final_app" "$archive"
(
    cd "$output_dir"
    shasum -a 256 "$(basename "$archive")" >"$(basename "$checksum")"
)
"$repo_root/scripts/verify-bundle.sh" "$final_app"

echo "Bundle: $final_app"
echo "Archive: $archive"
echo "Checksum: $checksum"
