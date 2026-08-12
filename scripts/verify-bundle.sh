#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
app="${1:?usage: verify-bundle.sh /path/to/Statlet.app}"
plist="$app/Contents/Info.plist"
privacy="$app/Contents/Resources/PrivacyInfo.xcprivacy"
executable="$app/Contents/MacOS/Statlet"
plist_buddy=/usr/libexec/PlistBuddy
cargo_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)"

test -d "$app"
test -x "$executable"
test -f "$app/Contents/Resources/AppIcon.icns"
test -f "$app/Contents/Resources/LICENSE"
test -f "$app/Contents/Resources/NOTICE"
test -f "$app/Contents/Resources/THIRD_PARTY_LICENSES.html"

plutil -lint "$plist" "$privacy" >/dev/null
test "$("$plist_buddy" -c 'Print :CFBundleIdentifier' "$plist")" = "io.github.hayashirafael.Statlet"
test "$("$plist_buddy" -c 'Print :CFBundleExecutable' "$plist")" = "Statlet"
test "$("$plist_buddy" -c 'Print :CFBundleIconFile' "$plist")" = "AppIcon"
test "$("$plist_buddy" -c 'Print :CFBundleShortVersionString' "$plist")" = "$cargo_version"
test "$("$plist_buddy" -c 'Print :CFBundleVersion' "$plist")" = "1"
test "$("$plist_buddy" -c 'Print :CFBundlePackageType' "$plist")" = "APPL"
test "$("$plist_buddy" -c 'Print :LSApplicationCategoryType' "$plist")" = "public.app-category.utilities"
test "$("$plist_buddy" -c 'Print :LSMinimumSystemVersion' "$plist")" = "14.0"
test "$("$plist_buddy" -c 'Print :LSMultipleInstancesProhibited' "$plist")" = "true"
test "$("$plist_buddy" -c 'Print :LSUIElement' "$plist")" = "true"

test "$("$plist_buddy" -c 'Print :NSPrivacyTracking' "$privacy")" = "false"
test "$(plutil -extract NSPrivacyCollectedDataTypes json -o - "$privacy")" = "[]"
test "$(plutil -extract NSPrivacyTrackingDomains json -o - "$privacy")" = "[]"
test "$("$plist_buddy" -c 'Print :NSPrivacyAccessedAPITypes:0:NSPrivacyAccessedAPIType' "$privacy")" = "NSPrivacyAccessedAPICategoryDiskSpace"
test "$("$plist_buddy" -c 'Print :NSPrivacyAccessedAPITypes:0:NSPrivacyAccessedAPITypeReasons:0' "$privacy")" = "85F4.1"

architectures="$(lipo -archs "$executable")"
test "$architectures" = "arm64"
minimum_macos="$(vtool -show-build "$executable" | awk '/^[[:space:]]+minos / { print $2; exit }')"
test "$minimum_macos" = "14.0"
codesign --verify --deep --strict "$app"
signature_info="$(codesign --display --verbose=4 "$app" 2>&1)"
grep -Fq "runtime" <<<"$signature_info"

grep -Fq "featherbar" "$app/Contents/Resources/NOTICE"
grep -Fq "Apache License" "$app/Contents/Resources/LICENSE"
grep -Fq "Statlet third-party licenses" "$app/Contents/Resources/THIRD_PARTY_LICENSES.html"

echo "Verified Statlet.app: version ${cargo_version}, macOS 14+, arm64, privacy manifest, notices, hardened-runtime signature"
