## Context

Moray has Rust-side `moray-core` and `moray-sessions` abstractions for recoverable conversation flow, including streaming session events and tool authorization replies. We want to (1) validate that this runtime design is reasonable behind a real network boundary and (2) ship a cross-platform desktop chat surface for the same single-session model.

The simplest way to do both is to drive the runtime through an HTTP service and let the desktop app consume that service. The desktop side needs a native window, packaging, and OS-level controls, which Tauri handles well; the chat domain itself stays on the HTTP service so it can also be exercised independently.

We dedicate a new top-level `desktop/` directory to the cross-platform desktop application and put its sub-apps inside it. The `desktop/` namespace itself describes the application identity, and the sub-folders describe sub-app roles:

- `desktop/server` — `axum`-based chat HTTP service, compiled as a Rust library
- `desktop/client` — Tauri v2 desktop client / shell
- `desktop/web` — Vue 3 web app rendered inside the desktop window

Separately, we introduce `moray-builtin` at the repo root: a small Rust package of "batteries-included" building blocks that are too app-flavored to live in `moray-core` / `moray-sessions` but are obviously reusable across `demo/` and `desktop/server`. The first occupants are `JsonlTranscriptStore` (migrated from `demo/`) and a new `CalcTool`.

## Goals / Non-Goals

**Goals:**
- One in-process `axum` HTTP service at `desktop/server` that exposes chat behavior for one active session by composing `moray-core` + `moray-sessions` + `moray-builtin`.
- One Tauri v2 desktop client at `desktop/client` that opens the window, hosts the web app's assets, and manages `desktop/server`'s lifecycle as a Tokio task in the same process.
- One Vue 3 web app at `desktop/web`, written in plain JavaScript with Vite + pnpm, that uses a dedicated `ChatClient` abstraction for chat behavior with an HTTP / SSE implementation as the initial transport.
- Make it possible to later replace the HTTP `ChatClient` implementation with a Tauri-command implementation without changing UI components.
- Keep the web app's chat-state derived from server-emitted events rather than duplicating runtime execution logic in the UI.
- Introduce a new `moray-builtin` package providing `JsonlTranscriptStore` (migrated from `demo/`) and a new `CalcTool` (double-stack four-arithmetic calculator) wired with an `AskUser` authorization policy.
- Provide a local development path and build verification for all three desktop sub-apps and the new builtin package.

**Non-Goals:**
- Multi-session creation, switching, deletion, or session list UI.
- User accounts, cloud sync, or remote server deployment.
- Hardening `desktop/server` for public exposure (auth, TLS termination, rate limits).
- Redesigning `moray-core`, transcript storage semantics, or agent semantics.
- Writing the Tauri-command-backed `ChatClient` implementation in this change (only the abstraction must support it).
- Changing the demo's externally observable chat behavior (only its store import path changes).

## Decisions

- Decision: Use a dedicated `desktop/` top-level folder for the desktop application and its sub-apps.
  - Rationale: `desktop/` describes the application identity (cross-platform desktop app), so sub-folder names can stay short while still being unambiguous in context. A generic `apps/` bucket would mix unrelated future applications and make ownership unclear.
  - Alternatives considered:
    - Use a generic `apps/` directory: rejected because it scales poorly when more applications are added.
    - Place each sub-app at the repository root: rejected because it makes the "application boundary vs runtime crate" distinction less obvious.
    - Embed everything in an existing crate folder: rejected because reusable runtime crates should not host application packaging.

- Decision: Make `desktop/server` the chat boundary.
  - Rationale: A real HTTP boundary forces the `moray-core` + `moray-sessions` design through an external API and is what we want to validate; it also gives `desktop/web` a transport that does not depend on Tauri.
  - Alternatives considered:
    - Drive chat behavior directly from Tauri commands first: rejected because it would not validate the HTTP-shaped boundary the runtime is meant to support.
    - Embed the runtime entirely in the web app: rejected because the runtime is Rust and authority should not move into the UI.

- Decision: Run `desktop/server` in-process as a Tokio task hosted by the Tauri main process.
  - Rationale: Tauri's main process is already a Rust + Tokio runtime; co-locating the server avoids cross-process IPC, sidecar packaging, port-discovery races, and orphan-process handling. We compile `desktop/server` as a `lib` crate that exports `start(opts) -> ServerHandle` plus a `shutdown()` method on the handle. `desktop/client` calls `start` from its Tauri `setup` hook and stores the handle in Tauri state. The handle exposes `local_addr` for URL injection and a `CancellationToken` for graceful shutdown via `axum::with_graceful_shutdown`.
  - Alternatives considered:
    - Spawn `desktop/server` as a child binary (sidecar): rejected because it adds packaging, IPC, port discovery, and orphan handling complexity for no benefit on the single-machine desktop case.
    - Run server out-of-process via an external launcher: rejected because the desktop UX should not require a separate process management story for users.
  - Note: a thin `bin/dev.rs` wrapper around `start(opts)` can be added later if a "no-Tauri standalone" debug binary is desired; the lib API does not block this.

- Decision: Bind `desktop/server` to `127.0.0.1:0` and surface the chosen port via the `ServerHandle`.
  - Rationale: Avoids static port collisions on user machines and limits exposure to the local loopback interface.
  - Alternatives considered:
    - Use a fixed port: rejected because of collision risk on user machines.

- Decision: Restrict `desktop/client` to system / shell responsibilities plus `desktop/server` lifecycle management.
  - Rationale: Keeps chat domain on the server side, makes the client's responsibilities focused (window, asset hosting, in-process server supervision, OS-level commands), and keeps the desktop shell replaceable.
  - Alternatives considered:
    - Have the client proxy chat commands to the server: rejected because the web app can speak HTTP directly and an extra hop adds complexity.

- Decision: Expose the server's bound URL to `desktop/web` via a single Tauri command `get_server_url`.
  - Rationale: Cleanly separates "system / shell command surface" (allowed) from "chat-domain command surface" (forbidden in this change). The web app issues a one-time `invoke('get_server_url')` at startup, then constructs an HTTP-backed `ChatClient` with the resolved base URL.
  - Alternatives considered:
    - Inject the URL through a window global / URL query: rejected because it leaks "magic" globals that fight the `ChatClient` boundary.

- Decision: Use SSE (`text/event-stream`) for the session event stream.
  - Rationale: SSE is one-way, server-pushed, has native browser support via `EventSource`, supports automatic reconnect with `Last-Event-ID`, and maps cleanly onto the `SessionEvent` flow.
  - Alternatives considered:
    - Chunked NDJSON: rejected because the client must hand-roll a reader and lose `Last-Event-ID` reconnect semantics.
    - WebSocket bi-directional channel: rejected as overkill for the single-direction event flow needed initially.
    - Polling for transcript snapshots: rejected because it loses ordering guarantees and inflates latency.

- Decision: Use a single `GET /events?from_seq=N` SSE endpoint that delivers replay history followed by live updates, distinguished by SSE event names.
  - Rationale: Keeps the wire surface small and avoids race conditions between a separate "history fetch" and a "live subscribe" call. Three SSE event names are used:
    - `event: replay` — historical events with `seq <= last_seq_at_subscribe`.
    - `event: live-start` — a marker (no payload) emitted exactly once between the replay batch and the live tail.
    - `event: live` — events produced after the subscription was opened.
  - Server implementation order to avoid races:
    1. Compute `last_seq = store.current_last_seq()`.
    2. Open `live_subscriber = store.subscribe(last_seq + 1)` to start buffering live events.
    3. Stream `[from_seq ..= last_seq]` from the store as `event: replay`.
    4. Emit `event: live-start`.
    5. Drain the live subscriber as `event: live`.
  - The `id:` field of each event is the session-event sequence; `Last-Event-ID` reconnect maps to `from_seq`.
  - Alternatives considered:
    - Two endpoints (`GET /transcript` + `GET /events`): rejected because clients must implement an explicit `LOADING_HISTORY → SUBSCRIBING → LIVE` state machine and ensure no event is dropped between the two calls.

- Decision: The web app does not respond to tool authorization requests during the replay phase.
  - Rationale: `Blocked` events that appear in replay either have already been resolved by a later persisted event or will be re-surfaced naturally on the live tail when `moray-sessions` resumes the active turn. Re-prompting users for historical authorization would be a confusing UX.
  - Implementation: the web app distinguishes phases by the SSE event name (`replay` vs `live`); only `live`-phase events trigger authorization UI.

- Decision: Resume on startup.
  - Rationale: One of the points of validating `moray-core` + `moray-sessions` over HTTP is exercising the recovery flow. Starting fresh would short-circuit a major part of the design we want to test.
  - Implementation: server initializes its single `ChatSession` with the same JSONL transcript path used by demo (configurable via env), so app launches always continue from persisted history; the web app subscribes from `from_seq=0` on first load and from `Last-Event-ID + 1` on reconnect.

- Decision: HTTP error wire format is a small JSON envelope.
  - Rationale: A simple, predictable shape is sufficient for a local app.
  - Shape: `{ "code": "BUSY" | "NOT_FOUND" | "BAD_REQUEST" | "INTERNAL", "message": "..." }`.
  - Status mapping: `MorayError::Busy → 409 Conflict`; reply with unknown `call_id → 404`; malformed payload `→ 400`; everything else `→ 500`.
  - Alternatives considered:
    - RFC 7807 `application/problem+json`: rejected as overhead for a local-only API.

- Decision: `desktop/` is a self-contained Cargo workspace, excluded from the root workspace.
  - Rationale: Tauri projects have their own workspace and lockfile conventions, and we want desktop's iteration speed to be independent of the runtime crates' build graph.
  - Implementation:
    - The root `Cargo.toml` adds `exclude = ["desktop"]`.
    - `desktop/Cargo.toml` is its own workspace with members `server` and `client/src-tauri`.
    - `desktop/server` and `desktop/client/src-tauri` reference `moray-core`, `moray-sessions`, and `moray-builtin` via `path = "../../<crate>"`.

- Decision: Package names follow the existing `moray-` prefix convention.
  - Rationale: Consistency with `moray-core`, `moray-sessions`, `moray-builtin`.
  - Choices: Rust crates `moray-desktop-server`, `moray-desktop-client`; web app package `@moray/desktop-web` (pnpm-only, not published).

- Decision: Web stack is Vue 3 + Vite + pnpm in plain JavaScript (no TypeScript), with no UI framework.
  - Rationale: First app should validate the chat loop, not solve general application shell, settings, routing, or multi-page navigation. Plain JavaScript keeps the toolchain minimal; reactive `ref` is enough state management.
  - Alternatives considered:
    - TypeScript: rejected for this initial change to keep the surface small; can be revisited.
    - Adopt a UI framework (Naive UI / Element Plus / etc.): rejected as not needed for the minimal surface.

- Decision: Introduce a new `moray-builtin` Rust package at the workspace root for reusable building blocks.
  - Rationale: `JsonlTranscriptStore` is a `moray-sessions`-compatible store that both `demo/` and `desktop/server` need to share; keeping it inside `demo/` would force `desktop/server` to depend on `demo/`, which is wrong. A new "batteries-included" package is also a natural home for the new `CalcTool`.
  - Initial contents:
    - `moray_builtin::stores::JsonlTranscriptStore` — migrated from `demo/src/transcript.rs`; behavior unchanged; `demo` updates its imports.
    - `moray_builtin::tools::CalcTool` — new; double-stack arithmetic supporting `+ - * /` with operator precedence; gated by an `AskUser` authorization policy so the desktop app exercises the full authorization flow.
  - Naming alternatives considered:
    - `moray-prelude` / `moray-extras` / `moray-toolkit`: all viable; `moray-builtin` chosen because it most clearly conveys "official, ready-to-use building blocks".

- Decision: `CalcTool` JSON shapes.
  - Input: `{ "expression": "1+2*3" }` (string in any whitespace).
  - Output (success): `{ "value": 7 }`.
  - Output (error): `{ "error": "..." }` (e.g. division by zero, parse failure).
  - Authorization: every invocation surfaces `AuthDecision::AskUser` so `desktop/web` exercises the live tool authorization prompt.

- Decision: Graceful shutdown is driven by a `CancellationToken` owned by the `ServerHandle`.
  - Rationale: Tauri triggers shutdown on `RunEvent::ExitRequested` (or window close); the client cancels the token, `axum::with_graceful_shutdown` finalizes in-flight requests, and the awaited `JoinHandle` completes before the process exits. No orphaned state is possible because the server task lives in the same process.

## Risks / Trade-offs

- [HTTP and Tauri-command `ChatClient` implementations diverge in behavior] -> Mitigation: encode the contract in the `ChatClient` interface (and shared event/payload conventions) so both implementations satisfy the same shape; cover the abstraction with focused tests where practical.
- [In-process server panic crashes the desktop app] -> Mitigation: surround the axum task with a recovery layer for handler-level panics; non-handler panics intentionally bring the app down because the desktop client cannot meaningfully continue without the server.
- [Web app receives stale or duplicate events on reconnect] -> Mitigation: `Last-Event-ID` plus `from_seq` are the single source of truth; the server treats reconnects identically to fresh subscribes.
- [Replay phase prompts users for historical tool authorization] -> Mitigation: web only surfaces authorization prompts for `event: live` items; this rule is encoded both in design and in `web` spec scenarios.
- [Long-running chat work blocks the HTTP server or UI commands] -> Mitigation: drive session operations asynchronously on the server; `ChatSession::post` is fire-and-forget and progress flows through SSE.
- [Single-session assumptions leak into future multi-session work] -> Mitigation: name endpoints, app state, and DTOs around the current single active session and avoid premature multi-session abstractions.
- [Demo behavior regresses during `JsonlTranscriptStore` migration] -> Mitigation: keep public API and observable behavior identical; only the import path changes; demo integration tests must continue to pass.
- [Tauri toolchain complexity for new contributors] -> Mitigation: isolate Tauri / Vue dependencies under `desktop/` and document development / build commands.

## Migration Plan

1. Introduce `moray-builtin`:
   1. Create `builtin/` Rust crate at the workspace root; add it as a member of the root workspace.
   2. Move `JsonlTranscriptStore` from `demo/src/transcript.rs` to `moray-builtin::stores`; delete the old copy and re-export from `demo` if convenient, or update demo's imports directly.
   3. Add `moray-builtin::tools::CalcTool` with the JSON shapes above and an `AskUser` policy.
2. Set up the `desktop/` workspace:
   1. Add `exclude = ["desktop"]` in the root `Cargo.toml`.
   2. Create `desktop/Cargo.toml` as its own workspace with `server` and `client/src-tauri` as members.
   3. Reference `moray-core`, `moray-sessions`, `moray-builtin` via `path` deps.
3. Implement `desktop/server` as a lib:
   1. `start(opts) -> ServerHandle` builds the axum router around a single `ChatSession` (constructed from harness + `JsonlTranscriptStore` from `moray-builtin`).
   2. Implement endpoints: `POST /messages`, `POST /tool-authorizations/:call_id`, `POST /reset`, `GET /events?from_seq=N`.
   3. Wire SSE named events `replay` / `live-start` / `live` per the order in *Decisions*.
   4. Map `MorayError::Busy → 409`, unknown `call_id → 404`, etc., per error wire format.
4. Implement `desktop/client`:
   1. Spawn `desktop/server` from the Tauri `setup` hook; manage the handle in Tauri state.
   2. Expose `get_server_url` Tauri command.
   3. Hook `RunEvent::ExitRequested` to call `handle.shutdown().await` for graceful termination.
5. Implement `desktop/web`:
   1. Scaffold Vue 3 + Vite + pnpm in JavaScript.
   2. Implement the transport-agnostic `ChatClient` interface.
   3. Implement the HTTP-backed `ChatClient` using `fetch` (commands) + `EventSource` (SSE) against the URL from `get_server_url`.
   4. Build the minimal chat UI that subscribes via the `ChatClient`, renders replay vs live phases per the SSE event name, and only surfaces authorization prompts for `live`-phase events.
6. Verify that swapping the `ChatClient` implementation does not require changing any UI component (design-level review, no second implementation required).
7. Add Rust compilation, web build, and end-to-end app verification commands; fix issues.
8. Rollback plan: revert the new `desktop/` and `builtin/` directories, the root workspace's `exclude` and member additions, and the demo's import update; existing runtime crates are unaffected.

## Open Questions

- None blocking implementation. Future revisits include: TypeScript adoption for `desktop/web`, optional standalone `bin/dev.rs` for `desktop/server`, and (pending `add-sessions-manager`) refactoring `desktop/server` to delegate session lifecycle through `SessionsManager`.
