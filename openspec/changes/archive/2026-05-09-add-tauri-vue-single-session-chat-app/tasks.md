## 1. Builtin Package (nova-builtin)

- [x] 1.1 Create `builtin/` Rust crate at the workspace root and add it to the root workspace `members`.
- [x] 1.2 Move `JsonlTranscriptStore` (and its supporting types) from `demo/src/transcript.rs` to `nova-builtin::stores`; preserve public API and observable behavior.
- [x] 1.3 Update `demo/` to consume `JsonlTranscriptStore` from `nova-builtin::stores`; remove the old in-`demo` copy.
- [x] 1.4 Add `nova-builtin::tools::CalcTool` implementing a double-stack four-arithmetic calculator with operator precedence; input `{ "expression": String }`, output `{ "value": Number }` on success and `{ "error": String }` on failure.
- [x] 1.5 Wire `CalcTool` to surface `AuthDecision::AskUser` on every invocation so the desktop app exercises live authorization prompts.
- [x] 1.6 Add focused unit tests for `CalcTool` (precedence, division by zero, parse errors).

## 2. Desktop Workspace Scaffold

- [x] 2.1 Add `exclude = ["desktop"]` in the root `Cargo.toml`.
- [x] 2.2 Create the `desktop/` directory as its own Cargo workspace with members `server` and `client/src-tauri`.
- [x] 2.3 Wire `desktop/server` and `desktop/client/src-tauri` to reference `nova-core`, `nova-sessions`, and `nova-builtin` via `path` dependencies.
- [x] 2.4 Initialize `desktop/web` as a pnpm package with Vue 3 + Vite + JavaScript (no TypeScript), no UI framework, and minimal config.
- [x] 2.5 Wire `desktop/client/src-tauri`'s frontend dist path / dev server URL to `desktop/web`'s Vite output.
- [x] 2.6 Document local development and verification commands for each of the four packages.

## 3. Server (desktop/server)

- [x] 3.1 Add a `config` module that loads OpenAI completion credentials (`NOVA_OPENAI_API_KEY` / `NOVA_OPENAI_BASE_URL` / `NOVA_OPENAI_MODEL`) from env, with `ServerOptions` allowing programmatic override.
- [x] 3.2 Build a session harness composing `nova_builtin::completions::OpenAIChatCompletion`, the `JsonlTranscriptStore` from `nova-builtin`, and a `Toolbox` carrying `CalcTool` from `nova-builtin`.
- [x] 3.3 Compile `desktop/server` as a `lib` crate exporting `start(opts) -> ServerHandle` that binds `127.0.0.1:0`, builds the `axum` router, and spawns it on the current Tokio runtime.
- [x] 3.4 Implement `ServerHandle` carrying `local_addr`, a `CancellationToken`, and a `JoinHandle<()>`, plus an async `shutdown()` that cancels and awaits.
- [x] 3.5 Wire `axum::serve(...).with_graceful_shutdown(token.cancelled())` so shutdown drains in-flight requests.
- [x] 3.6 Implement `POST /messages` to call `ChatSession::post` with the user input; map `NovaError::Busy → 409`.
- [x] 3.7 Implement `POST /tool-authorizations/:call_id` to call `ChatSession::reply_toolcall_permission`; map unknown `call_id → 404`, no active turn → 409.
- [x] 3.8 Implement `POST /reset` to call `ChatSession::reset`; map `NovaError::Busy → 409`.
- [x] 3.9 Implement `GET /events?from_seq=N` (SSE):
  - read `last_seq = store.current_last_seq()`;
  - subscribe `live = store.subscribe(last_seq + 1)` first;
  - stream `[from_seq ..= last_seq]` from the store as `event: replay`;
  - emit `event: live-start`;
  - drain `live` as `event: live`; use the session-event sequence as the SSE `id:` so `Last-Event-ID` reconnect works.
- [x] 3.10 Standardize the JSON error envelope `{ "code", "message" }` and apply it across all endpoints.
- [x] 3.11 Add HTTP-level tests for endpoint behaviors (post → events, reply unknown id → 404, busy → 409, reset, replay vs live phase split).

## 4. Client (desktop/client)

- [x] 4.1 Configure Tauri v2 (desktop only; mobile target disabled) to open the desktop window and host `desktop/web` assets.
- [x] 4.2 In Tauri `setup`, call `server::start(opts)` and store the resulting `ServerHandle` in Tauri state.
- [x] 4.3 Expose `get_server_url` as the only chat-related Tauri command, returning `format!("http://{}", handle.local_addr)`.
- [x] 4.4 On `RunEvent::ExitRequested` (and window close request), call `handle.shutdown().await` before allowing the app to exit; ensure no orphan task or socket remains.
- [x] 4.5 Limit the Tauri command surface to system / shell concerns; do not add Tauri commands for posting messages, replying authorizations, resetting the session, or streaming events in this change.

## 5. Web (desktop/web)

- [x] 5.1 Define a transport-agnostic `ChatClient` interface (JavaScript module) covering `postMessage`, `subscribeEvents({ fromSeq, onReplay, onLiveStart, onLive })`, `replyToolAuth(callId, decision)`, and `reset`.
- [x] 5.2 Implement an HTTP-backed `ChatClient` that:
  - obtains the base URL via `invoke('get_server_url')` once at startup,
  - uses `fetch` for command endpoints,
  - uses `EventSource` for `GET /events?from_seq=N` and routes `replay` / `live-start` / `live` events to their listeners,
  - supports automatic reconnect via `Last-Event-ID`.
- [x] 5.3 Build the minimal single-session chat UI with transcript, input composer, turn status indicator, and reset control; state via Vue `ref`.
- [x] 5.4 Render assistant text, tool lifecycle, turn completion, and reset events from the `ChatClient` event stream.
- [x] 5.5 Render tool authorization prompts only for `event: live` items (not for replay), and forward approve / deny decisions through `ChatClient.replyToolAuth`.
- [x] 5.6 Ensure UI components access chat behavior only through `ChatClient`; do not call `fetch` / `EventSource` / Tauri APIs directly for chat behavior.
- [x] 5.7 Ensure no multi-session create / list / switch / delete controls are exposed.

## 6. Verification

- [x] 6.1 Add focused tests around the HTTP endpoints and the `ChatClient` interface contract where practical.
- [x] 6.2 Run `cargo build` / `cargo test` for the root workspace (now including `nova-builtin`) and for the independent `desktop/` workspace.
- [x] 6.3 Run the `desktop/web` build via pnpm to verify Vite output.
- [x] 6.4 Run the documented end-to-end app verification command and fix any failures.
