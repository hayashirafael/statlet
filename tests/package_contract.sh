#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/statlet-package-test.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)"
archive_name="Statlet-v${version}-macos-arm64.zip"

if "$repo_root/scripts/package-release.sh" / >/dev/null 2>&1; then
    echo "package-release.sh accepted the filesystem root as output" >&2
    exit 1
fi
if STATLET_TARGET=x86_64-apple-darwin "$repo_root/scripts/package-release.sh" "$test_root/invalid" >/dev/null 2>&1; then
    echo "package-release.sh accepted an unsupported architecture" >&2
    exit 1
fi

external_target="$test_root/external-target"
mkdir -p "$external_target"
echo "must remain untouched" >"$external_target/sentinel"
CARGO_TARGET_DIR="$external_target" "$repo_root/scripts/package-release.sh" "$test_root"
test "$(<"$external_target/sentinel")" = "must remain untouched"
"$repo_root/scripts/verify-bundle.sh" "$test_root/Statlet.app"
test -s "$test_root/Statlet.app/Contents/Resources/THIRD_PARTY_LICENSES.html"

fake_home="$test_root/home-with-mole-enabled"
mkdir -p "$fake_home/Library/Application Support/Statlet"
printf '%s\n' '{"version":1,"moleIntegrationEnabled":true,"warningThreshold":90}' \
    >"$fake_home/Library/Application Support/Statlet/preferences.json"
if HOME="$fake_home" "$repo_root/scripts/measure-soak.sh" "$test_root/Statlet.app" 1 1 "$test_root/invalid-soak" >/dev/null 2>&1; then
    echo "measure-soak.sh accepted a baseline with Mole integration enabled" >&2
    exit 1
fi

fake_v2_home="$test_root/home-with-nondefault-refresh"
mkdir -p "$fake_v2_home/Library/Application Support/Statlet"
printf '%s\n' '{"version":2,"moleIntegrationEnabled":false,"warningThreshold":90,"indicator":{"refreshInterval":1}}' \
    >"$fake_v2_home/Library/Application Support/Statlet/preferences.json"
if HOME="$fake_v2_home" "$repo_root/scripts/measure-soak.sh" "$test_root/Statlet.app" 1 1 "$test_root/invalid-v2-soak" >/dev/null 2>&1; then
    echo "measure-soak.sh accepted a v2 default baseline with a nondefault refresh interval" >&2
    exit 1
fi

archive="$test_root/$archive_name"
checksum="$archive.sha256"

test -f "$archive"
test -f "$checksum"

expected="$(awk '{print $1}' "$checksum")"
actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
test "$actual" = "$expected"
test "$(awk '{print $2}' "$checksum")" = "$archive_name"
(cd "$test_root" && shasum -a 256 -c "$(basename "$checksum")")

unpacked="$test_root/unpacked"
mkdir -p "$unpacked"
ditto -x -k "$archive" "$unpacked"
"$repo_root/scripts/verify-bundle.sh" "$unpacked/Statlet.app"

echo "Statlet package contract passed"
