#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/statlet-package-test.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT

if "$repo_root/scripts/package-release.sh" / >/dev/null 2>&1; then
    echo "package-release.sh accepted the filesystem root as output" >&2
    exit 1
fi
if STATLET_TARGET=x86_64-apple-darwin "$repo_root/scripts/package-release.sh" "$test_root/invalid" >/dev/null 2>&1; then
    echo "package-release.sh accepted an unsupported architecture" >&2
    exit 1
fi

"$repo_root/scripts/package-release.sh" "$test_root"
"$repo_root/scripts/verify-bundle.sh" "$test_root/Statlet.app"
test -s "$test_root/Statlet.app/Contents/Resources/THIRD_PARTY_LICENSES.html"

archive="$test_root/Statlet-v1.0.0-macos-arm64.zip"
checksum="$archive.sha256"

test -f "$archive"
test -f "$checksum"

expected="$(awk '{print $1}' "$checksum")"
actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
test "$actual" = "$expected"
test "$(awk '{print $2}' "$checksum")" = "Statlet-v1.0.0-macos-arm64.zip"
(cd "$test_root" && shasum -a 256 -c "$(basename "$checksum")")

unpacked="$test_root/unpacked"
mkdir -p "$unpacked"
ditto -x -k "$archive" "$unpacked"
"$repo_root/scripts/verify-bundle.sh" "$unpacked/Statlet.app"

echo "Statlet package contract passed"
