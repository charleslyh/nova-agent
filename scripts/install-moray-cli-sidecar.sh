#!/usr/bin/env bash
# Build release moray-cli and install to Tauri externalBin (moray-cli-<host-triple>).
# See desktop/client/src-tauri/resources/binaries/README.md
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

host="$(rustc --print host-tuple)"
dest_dir="desktop/client/src-tauri/resources/binaries"
mkdir -p "$dest_dir"

if [[ "$host" == *windows* ]]; then
  src="target/release/moray-cli.exe"
  dest="$dest_dir/moray-cli-${host}.exe"
else
  src="target/release/moray-cli"
  dest="$dest_dir/moray-cli-${host}"
fi

cargo build -p moray-cli --release
cp -f "$src" "$dest"
echo "Installed moray-cli sidecar -> $dest"
