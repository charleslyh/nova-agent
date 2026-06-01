## Why

Moray currently exposes recoverable session and agent conversation behavior through Rust crates, but lacks both an HTTP-shaped boundary for exercising the `moray-core` + `moray-sessions` design and a cross-platform graphical surface for it. Adding a thin `axum`-based chat HTTP service alongside a Tauri + Vue desktop application lets us validate the runtime design through a real network boundary while shipping a usable single-session desktop app.

To support both this app and other reusable scenarios, the change also introduces a new top-level `moray-builtin` package that provides ready-to-use building blocks (an existing JSONL transcript store migrated out of `demo/`, plus a new `calc` tool).

## What Changes

- Add a new top-level `builtin/` directory containing a new `moray-builtin` Rust package with reusable, ready-to-use building blocks for Moray apps.
- Migrate `JsonlTranscriptStore` from `demo/` into `moray-builtin::stores`; update `demo` to consume it from the new package.
- Add a new `CalcTool` (double-stack four-arithmetic calculator) under `moray-builtin::tools`, gated by an `AskUser` authorization policy so it exercises the full tool-authorization flow.
- Add a new top-level `desktop/` directory that owns the cross-platform desktop application and all its sub-apps; keep `desktop/` as a self-contained Cargo workspace excluded from the root workspace via `exclude = ["desktop"]`.
- Add three colocated sub-apps under `desktop/`: a `server` (`axum`-based chat HTTP service), a `client` (Tauri v2 desktop client / shell), and a `web` (Vue 3 web app rendered inside the desktop window).
- Make `server` an in-process Rust library that drives the existing `moray-core` + `moray-sessions` runtime for a single active session, exposing chat HTTP endpoints (post message, SSE stream, tool authorization reply, reset).
- Make `client` host the `web` app's assets, spawn `server` as an in-process Tokio task on app start, expose its bound URL through a Tauri command, and shut it down gracefully on app exit.
- Restrict Tauri commands on `client` to local system / shell concerns (window, lifecycle, environment, server URL) and keep chat-domain operations on `server`.
- Have `web` consume chat behavior through a dedicated `ChatClient` abstraction so the underlying transport can later switch from HTTP to a Tauri-command implementation without rewriting UI code; ship `web` as Vue 3 + Vite + pnpm in plain JavaScript (no TypeScript).
- Keep the application scope to one active session, with no multi-session creation, switching, history browsing, or account-level features.

## Capabilities

Note: this section follows the OpenSpec schema. Each "capability" listed below is an OpenSpec spec namespace. Three of them (`server`, `client`, `web`) correspond one-to-one with the `desktop/` sub-apps introduced in *What Changes*; `moray-builtin` is a separate cross-cutting infra namespace, and `moray-demos` is touched because the JSONL store moves out of `demo/`.

### New Capabilities
- `server`: Provide an `axum`-based chat HTTP service that drives the `moray-core` + `moray-sessions` runtime for a single active session.
- `client`: Provide a Tauri v2 desktop client that opens the window, hosts `web`, and manages `server`'s lifecycle in-process.
- `web`: Provide a Vue 3 web app that renders single-session conversation state and accesses chat behavior through a transport-agnostic `ChatClient` abstraction.
- `moray-builtin`: Provide a Rust package of reusable, ready-to-use building blocks (initially: `JsonlTranscriptStore` migrated from `demo/`, and a new `CalcTool`).

### Modified Capabilities
- `moray-demos`: `demo` no longer owns `JsonlTranscriptStore`; the demo's chat example consumes the store from `moray-builtin::stores` instead. Demo's externally observable chat behavior is unchanged.

## Impact

- Affected code:
  - new `builtin/` Rust package (`moray-builtin`) at the workspace root;
  - new `desktop/` directory containing the `server`, `client`, and `web` sub-app packages;
  - `demo/`: drops its local `JsonlTranscriptStore` implementation and depends on `moray-builtin` for it.
- Affected APIs:
  - new HTTP API surface for chat (post message, SSE stream events, reply tool authorization, reset session);
  - small Tauri command surface limited to system / shell concerns (notably `get_server_url`).
- Affected dependencies:
  - `axum`, `tokio_util`, and supporting async server crates for `server`;
  - Tauri v2 / Vue 3 / Vite / pnpm toolchain for `client` and `web`.
- Affected systems:
  - root `Cargo.toml` adds `moray-builtin` as a workspace member and adds `exclude = ["desktop"]`;
  - `desktop/` becomes its own Cargo workspace;
  - local desktop development, packaging, and run flow.
- Existing `moray-core` and `moray-sessions` runtime crates are consumed but their behavioral contracts are not changed.
