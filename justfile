set shell := ["bash", "-cu"]

default:
  @just --list

# Build all workspace crates.
build:
  cargo build --workspace

# Run all workspace tests.
test:
  cargo test --workspace

# Lint with clippy (warnings as errors).
lint:
  cargo clippy --workspace -- -D warnings
