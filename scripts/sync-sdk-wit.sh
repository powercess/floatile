#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_wit="$repo_root/wit/floatile-widget.wit"
snapshot_dir="$repo_root/crates/floatile-sdk/wit"

mkdir -p "$snapshot_dir"
cp "$source_wit" "$snapshot_dir/floatile-widget.wit"
