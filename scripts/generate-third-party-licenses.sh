#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cargo_about="${CARGO_ABOUT:-cargo-about}"

if ! command -v "$cargo_about" >/dev/null 2>&1; then
    echo "cargo-about is required; install it with: cargo install cargo-about --version 0.9.1 --locked --features cli" >&2
    exit 1
fi
if [[ "$("$cargo_about" --version)" != "cargo-about 0.9.1" ]]; then
    echo "cargo-about 0.9.1 is required for reproducible output" >&2
    exit 1
fi

"$cargo_about" generate \
    --manifest-path "$repo_root/Cargo.toml" \
    --config "$repo_root/about.toml" \
    --target aarch64-apple-darwin \
    --locked \
    --fail \
    --output-file "$repo_root/packaging/THIRD_PARTY_LICENSES.html" \
    "$repo_root/packaging/third-party-licenses.hbs"

echo "Generated packaging/THIRD_PARTY_LICENSES.html"
