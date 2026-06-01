set shell := ["bash", "-cu"]

default:
  @just --list

# Build release moray-cli and copy to Tauri externalBin (moray-cli-<host-triple>[.exe]).
# See desktop/client/src-tauri/resources/binaries/README.md
[unix]
install-moray-cli-sidecar:
  bash scripts/install-moray-cli-sidecar.sh

[windows]
install-moray-cli-sidecar:
  pwsh -NoProfile -File scripts/install-moray-cli-sidecar.ps1

# Run Tauri in dev mode (recommended for frontend hot-reload).
# Dev uses tauri.conf.json (com.moray.desktop.dev). Production: cargo tauri build --config src-tauri/tauri.pro.conf.json
# User config: ~/.moray/server.toml — see desktop/README.md and desktop/server/*.toml.example.
dev: install-moray-cli-sidecar
  cd desktop/client && cargo tauri dev

# Run Tauri with --release (optimized Rust; still uses Vite dev server + hot-reload).
release: install-moray-cli-sidecar
  cd desktop/client && cargo tauri dev --release

web-dev:
  corepack pnpm --dir desktop/web dev

web-build:
  corepack pnpm --dir desktop/web build
