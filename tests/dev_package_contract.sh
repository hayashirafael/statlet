#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/statlet-dev-package-test.XXXXXX")"
trap 'rm -rf "$test_root"' EXIT
canonical_plist_hash="$(shasum -a 256 "$repo_root/packaging/Info.plist" | awk '{print $1}')"

fake_bin="$test_root/fake-bin"
cargo_marker="$test_root/cargo-called"
mkdir -p "$fake_bin"
printf '%s\n' '#!/bin/bash' 'touch "$STATLET_TEST_CARGO_MARKER"' 'exit 99' >"$fake_bin/cargo"
chmod +x "$fake_bin/cargo"

assert_unsafe_output_rejected_before_build() {
    local output="$1"
    rm -f "$cargo_marker"
    if STATLET_TEST_CARGO_MARKER="$cargo_marker" PATH="$fake_bin:$PATH" \
        "$repo_root/scripts/package-dev.sh" --output "$output" --instance task-a --label "Task A" \
        >/dev/null 2>&1; then
        echo "package-dev.sh accepted unsafe output '$output'" >&2
        exit 1
    fi
    if [[ -e "$cargo_marker" ]]; then
        echo "package-dev.sh started cargo before rejecting unsafe output '$output'" >&2
        exit 1
    fi
}

assert_unsafe_output_rejected_before_build /
assert_unsafe_output_rejected_before_build /tmp/..
ln -s / "$test_root/root-link"
assert_unsafe_output_rejected_before_build "$test_root/root-link"
if "$repo_root/scripts/package-dev.sh" --output "$test_root/invalid" --instance ../escape --label "Task A" >/dev/null 2>&1; then
    echo "package-dev.sh accepted a path-like explicit instance seed" >&2
    exit 1
fi
if "$repo_root/scripts/package-dev.sh" --instance task-a --label $'Task\tA' --print-identity >/dev/null 2>&1; then
    echo "package-dev.sh accepted a control character in the display label" >&2
    exit 1
fi

identity_a="$($repo_root/scripts/package-dev.sh --instance task-a --label "Task A" --print-identity)"
identity_a_again="$($repo_root/scripts/package-dev.sh --instance task-a --label "Changed label" --print-identity)"
identity_b="$($repo_root/scripts/package-dev.sh --instance task-b --label "Task B" --print-identity)"
test "$identity_a" = "$identity_a_again"
test "$identity_a" != "$identity_b"

identity_env="$(STATLET_DEV_INSTANCE=env-instance "$repo_root/scripts/package-dev.sh" --task ignored-task --label "Task A" --print-identity)"
identity_env_alone="$(STATLET_DEV_INSTANCE=env-instance "$repo_root/scripts/package-dev.sh" --label "Task A" --print-identity)"
identity_cli_with_lower_precedence_sources="$(STATLET_DEV_INSTANCE=ignored-env "$repo_root/scripts/package-dev.sh" --instance task-a --task ignored-task --label "Task A" --print-identity)"
test "$identity_env" = "$identity_env_alone"
test "$identity_cli_with_lower_precedence_sources" = "$identity_a"

"$repo_root/scripts/package-dev.sh" --output "$test_root" --instance task-a --label "Task A"
"$repo_root/scripts/package-dev.sh" --output "$test_root" --instance task-b --label "Task B"

apps=("$test_root"/Statlet\ Dev\ *.app)
test "${#apps[@]}" -eq 2
test -d "${apps[0]}"
test -d "${apps[1]}"
"$repo_root/scripts/verify-dev-bundle.sh" "${apps[0]}"
"$repo_root/scripts/verify-dev-bundle.sh" "${apps[1]}"

plist_buddy=/usr/libexec/PlistBuddy
plist_a="${apps[0]}/Contents/Info.plist"
plist_b="${apps[1]}/Contents/Info.plist"
bundle_a="$($plist_buddy -c 'Print :CFBundleIdentifier' "$plist_a")"
bundle_b="$($plist_buddy -c 'Print :CFBundleIdentifier' "$plist_b")"
name_a="$($plist_buddy -c 'Print :CFBundleName' "$plist_a")"
name_b="$($plist_buddy -c 'Print :CFBundleName' "$plist_b")"
instance_a="$($plist_buddy -c 'Print :StatletDevInstanceID' "$plist_a")"
instance_b="$($plist_buddy -c 'Print :StatletDevInstanceID' "$plist_b")"
test "$bundle_a" != "$bundle_b"
test "$name_a" != "$name_b"
test "$instance_a" != "$instance_b"
test "$bundle_a" = "io.github.hayashirafael.Statlet.dev.$instance_a"
test "$bundle_b" = "io.github.hayashirafael.Statlet.dev.$instance_b"
test "$($plist_buddy -c 'Print :CFBundleExecutable' "$plist_a")" = "StatletDev"
test "$($plist_buddy -c 'Print :CFBundleExecutable' "$plist_b")" = "StatletDev"

test "$(shasum -a 256 "$repo_root/packaging/Info.plist" | awk '{print $1}')" = "$canonical_plist_hash"
test "$($plist_buddy -c 'Print :CFBundleIdentifier' "$repo_root/packaging/Info.plist")" = "io.github.hayashirafael.Statlet"
if "$plist_buddy" -c 'Print :StatletRuntimeProfile' "$repo_root/packaging/Info.plist" >/dev/null 2>&1; then
    echo "canonical production plist gained Dev metadata" >&2
    exit 1
fi

echo "Statlet Dev package contract passed"
