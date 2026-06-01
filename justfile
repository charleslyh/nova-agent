set shell := ["bash", "-cu"]

default:
  @just --list

# Run Tauri in dev mode (recommended for frontend hot-reload).
# User config: ~/.moray/server.toml — see desktop/README.md and desktop/server/*.toml.example.
dev:
  cd desktop/client && cargo tauri dev

# Run Tauri with --release (optimized Rust; still uses Vite dev server + hot-reload).
release:
  cd desktop/client && cargo tauri dev --release

web-dev:
  corepack pnpm --dir desktop/web dev

web-build:
  corepack pnpm --dir desktop/web build
