#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir=""
explicit_instance=""
explicit_instance_set=false
task_id=""
label=""
print_identity=false
target="${STATLET_TARGET:-aarch64-apple-darwin}"
export MACOSX_DEPLOYMENT_TARGET=14.0

usage() {
    echo "usage: package-dev.sh [--output DIR] [--instance SEED] [--task TASK] [--label NAME] [--print-identity]" >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --output)
            [[ $# -ge 2 ]] || usage
            output_dir="$2"
            shift 2
            ;;
        --instance)
            [[ $# -ge 2 ]] || usage
            explicit_instance="$2"
            explicit_instance_set=true
            shift 2
            ;;
        --task)
            [[ $# -ge 2 ]] || usage
            task_id="$2"
            shift 2
            ;;
        --label)
            [[ $# -ge 2 ]] || usage
            label="$2"
            shift 2
            ;;
        --print-identity)
            print_identity=true
            shift
            ;;
        *) usage ;;
    esac
done

if [[ "$target" != "aarch64-apple-darwin" ]]; then
    echo "Statlet Dev supports Apple Silicon only; got target '$target'" >&2
    exit 1
fi

valid_seed() {
    [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$ ]]
}

worktree="$(git -C "$repo_root" rev-parse --show-toplevel)"
if $explicit_instance_set; then
    valid_seed "$explicit_instance" || {
        echo "Invalid explicit instance seed" >&2
        exit 1
    }
    readable_seed="$explicit_instance"
    digest="$(printf '%s' "$explicit_instance" | shasum -a 256 | awk '{print substr($1, 1, 12)}')"
elif [[ -n "${STATLET_DEV_INSTANCE:-}" ]]; then
    valid_seed "$STATLET_DEV_INSTANCE" || {
        echo "Invalid STATLET_DEV_INSTANCE seed" >&2
        exit 1
    }
    readable_seed="$STATLET_DEV_INSTANCE"
    digest="$(printf '%s' "$STATLET_DEV_INSTANCE" | shasum -a 256 | awk '{print substr($1, 1, 12)}')"
elif [[ -n "$task_id" ]]; then
    valid_seed "$task_id" || {
        echo "Invalid task id" >&2
        exit 1
    }
    readable_seed="$task_id"
    digest="$(printf '%s\0%s' "$worktree" "$task_id" | shasum -a 256 | awk '{print substr($1, 1, 12)}')"
else
    readable_seed="$(basename "$worktree")"
    digest="$(printf '%s' "$worktree" | shasum -a 256 | awk '{print substr($1, 1, 12)}')"
fi

slug="$(printf '%s' "$readable_seed" | LC_ALL=C tr '[:upper:]' '[:lower:]' | LC_ALL=C tr -cs 'a-z0-9' '-' | sed 's/^-*//; s/-*$//' | cut -c1-24 | sed 's/-*$//')"
if [[ -z "$slug" ]]; then
    echo "Instance seed does not contain a usable ASCII slug" >&2
    exit 1
fi
instance_id="$slug-$digest"
short_marker="$(printf '%s' "${digest:0:4}" | tr '[:lower:]' '[:upper:]')"

if [[ -z "$label" ]]; then
    label="$(git -C "$repo_root" branch --show-current)"
    [[ -n "$label" ]] || label="$(basename "$worktree")"
fi
label_has_control=false
if printf '%s' "$label" | LC_ALL=C grep -q '[[:cntrl:]]'; then
    label_has_control=true
fi
if [[ -z "${label//[[:space:]]/}" || ${#label} -gt 80 ]] || $label_has_control; then
    echo "Invalid Dev display label" >&2
    exit 1
fi

if $print_identity; then
    printf '%s\n' "$instance_id"
    exit 0
fi
if [[ -z "$output_dir" ]]; then
    echo "Refusing unsafe Dev output directory: '$output_dir'" >&2
    exit 1
fi
mkdir -p "$output_dir"
resolved_output_dir="$(cd "$output_dir" && pwd -P)"
if [[ "$resolved_output_dir" == "/" ]]; then
    echo "Refusing unsafe Dev output directory: '$output_dir' resolves to '/'" >&2
    exit 1
fi
final_app="$resolved_output_dir/Statlet Dev $short_marker.app"
if [[ -e "$final_app" ]]; then
    echo "Refusing to overwrite existing Dev bundle: $final_app" >&2
    exit 1
fi

staging="$(mktemp -d "${TMPDIR:-/tmp}/statlet-dev.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
cargo_target="$staging/cargo-target"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -1)"
if [[ -z "$version" ]]; then
    echo "Could not read the Statlet version from Cargo.toml" >&2
    exit 1
fi

cargo build \
    --manifest-path "$repo_root/Cargo.toml" \
    --release \
    --locked \
    --target-dir "$cargo_target" \
    --target "$target"

app="$staging/Statlet Dev $short_marker.app"
contents="$app/Contents"
resources="$contents/Resources"
iconset="$staging/AppIcon.iconset"
plist="$contents/Info.plist"

mkdir -p "$contents/MacOS" "$resources" "$iconset"
install -m 0755 "$cargo_target/$target/release/statlet" "$contents/MacOS/StatletDev"
install -m 0644 "$repo_root/packaging/Info.plist" "$plist"
install -m 0644 "$repo_root/packaging/PrivacyInfo.xcprivacy" "$resources/PrivacyInfo.xcprivacy"
install -m 0644 "$repo_root/LICENSE" "$resources/LICENSE"
install -m 0644 "$repo_root/NOTICE" "$resources/NOTICE"
install -m 0644 "$repo_root/packaging/THIRD_PARTY_LICENSES.html" "$resources/THIRD_PARTY_LICENSES.html"

plutil -replace CFBundleExecutable -string StatletDev "$plist"
plutil -replace CFBundleIdentifier -string "io.github.hayashirafael.Statlet.dev.$instance_id" "$plist"
plutil -replace CFBundleName -string "Statlet Dev $short_marker" "$plist"
plutil -insert CFBundleDisplayName -string "Statlet Dev — $label" "$plist"
plutil -insert StatletRuntimeProfile -string development "$plist"
plutil -insert StatletDevInstanceID -string "$instance_id" "$plist"
plutil -insert StatletDevDisplayName -string "$label" "$plist"
plutil -insert StatletDevShortMarker -string "$short_marker" "$plist"
plutil -lint "$plist" >/dev/null

for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips --resampleHeightWidth "$size" "$size" "$repo_root/packaging/AppIcon.png" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips --resampleHeightWidth "$double" "$double" "$repo_root/packaging/AppIcon.png" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns "$iconset" --output "$resources/AppIcon.icns"

codesign --force --options runtime --sign - "$app"
"$repo_root/scripts/verify-dev-bundle.sh" "$app"

ditto "$app" "$final_app"
"$repo_root/scripts/verify-dev-bundle.sh" "$final_app"

echo "Dev bundle: $final_app"
echo "Dev instance: $instance_id"
