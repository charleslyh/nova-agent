# `moray-cli` sidecar binaries

Tauri bundles executables listed in `tauri.conf.json` → `bundle.externalBin`. The desktop **client does not build** `moray-cli`; maintainers install binaries here manually.

**Policy:** files in this directory must be **release** builds (`cargo build -p moray-cli --release`). Do not copy `target/debug/moray-cli` here — debug binaries are much larger and not intended for bundling.

## Install steps

From the repo root, `just install-moray-cli-sidecar` (or `bash scripts/install-moray-cli-sidecar.sh` / `pwsh -File scripts/install-moray-cli-sidecar.ps1` on Windows) builds release `moray-cli` and copies it with the correct host-triple name. Manual steps:

1. Build the CLI (from the repo root):

   ```bash
   cargo build -p moray-cli --release
   ```

2. Find your host triple:

   ```bash
   rustc --print host-tuple
   ```

3. Copy the **release** binary into this directory with the required name:

   | Platform | Example filename |
   |----------|------------------|
   | macOS Apple Silicon | `moray-cli-aarch64-apple-darwin` |
   | macOS Intel | `moray-cli-x86_64-apple-darwin` |
   | Linux | `moray-cli-x86_64-unknown-linux-gnu` |
   | Windows | `moray-cli-x86_64-pc-windows-msvc.exe` |

   Example (macOS ARM):

   ```bash
   cd desktop/client/src-tauri/resources/binaries
   cp ../../../../../target/release/moray-cli \
     moray-cli-$(rustc --print host-tuple)
   ```

4. Rebuild the desktop client (`cargo build` in `src-tauri`, or `cargo tauri dev`). `tauri-build` copies the sidecar next to the app binary as `moray-cli` (basename only). During **client** dev that path is often `target/debug/moray-cli` — that refers to the desktop app’s debug output dir, not the CLI profile you installed in step 1.

Binaries with target triple suffixes are gitignored; only this README is tracked.
