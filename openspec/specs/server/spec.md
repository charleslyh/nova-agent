# server Specification

## Purpose

The desktop chat HTTP server (`desktop/server`): single-session loopback Axum API, TOML-backed OpenAI-compatible completion configuration under `~/.moray/`, JSONL transcript resume, and SSE event streaming.
## Requirements
### Requirement: Server lives under desktop as a library
The repository SHALL provide the chat HTTP server as a sub-app under the top-level `desktop/` directory at `desktop/server`, compiled as a Rust library, separate from reusable runtime crates.

#### Scenario: Server is colocated with other desktop sub-apps
- **WHEN** reviewing the repository layout after implementation
- **THEN** the chat HTTP server's source and configuration MUST live at `desktop/server`
- **AND** reusable Moray runtime crates MUST NOT be converted into application-specific packages to host the server

#### Scenario: Server crate is consumed as a library
- **WHEN** inspecting `desktop/server`'s `Cargo.toml`
- **THEN** the crate MUST be configured as a library
- **AND** it MUST expose a public async **parameterless** `prepare` entry point that loads the default-path TOML internally and returns a `ServerComponents` value containing the bound `tokio::net::TcpListener`, its `local_addr: SocketAddr`, and the assembled `axum::Router`
- **AND** it MUST expose `SondaGateway` (loopback bind, `axum::serve`, and coordinated `Sonda` shutdown) and `SondaGatewayError`
- **AND** it MUST expose a public `BootstrapError` error type used as the failure variant of `bootstrap`'s `Result`

### Requirement: In-process server lifecycle with graceful shutdown
The chat HTTP server SHALL be runnable in the caller's process by composing the library-provided `ServerComponents` with the caller's own task spawning and graceful-shutdown wiring. The library SHALL NOT prescribe the runtime task topology or the shutdown signal source.

#### Scenario: Caller drives the serve loop on its own runtime
- **WHEN** the calling binary uses `moray_desktop_server::prepare()` to obtain `ServerComponents`
- **THEN** the binary MUST be the entity that calls `axum::serve(components.listener, components.app)` and awaits the resulting future on its chosen Tokio task
- **AND** the bound `local_addr` MUST be exposed to the binary via `ServerComponents::local_addr` synchronously before any `await` on the serve future

#### Scenario: Caller wires graceful shutdown
- **WHEN** the calling binary needs to stop the server cleanly
- **THEN** the binary MUST attach its own shutdown signal (e.g., `CancellationToken`, `oneshot::Receiver`) to `axum::serve(..).with_graceful_shutdown(..)`
- **AND** the server library MUST NOT provide a built-in cancellation token field bound to `axum::serve`'s graceful shutdown

#### Scenario: Library is reusable for an out-of-process binary
- **WHEN** a future executable crate wishes to run the server as a standalone process
- **THEN** that executable MUST be able to call the same `prepare()` API and drive `axum::serve` itself without any other public surface in `moray-desktop-server`
- **AND** no public API of `moray-desktop-server` MUST encode an assumption that the server runs inside a Tauri application

### Requirement: Local-only loopback binding with runtime-selected port
The library SHALL bind the listener that drives the server only to a local loopback address with a runtime-selected port and SHALL expose the chosen port to its caller via `ServerComponents::local_addr`.

#### Scenario: Library selects a free port at runtime
- **WHEN** `prepare()` completes configuration resolution successfully
- **THEN** the implementation MUST bind a `TcpListener` to `127.0.0.1:0`
- **AND** the resulting `ServerComponents::local_addr` MUST expose the actually bound port

#### Scenario: Server is not exposed beyond the local machine
- **WHEN** the listener is bound by `prepare()`
- **THEN** the implementation MUST NOT bind to a publicly reachable address by default

### Requirement: Server composes runtime via moray-sonda completions module
The server SHALL build the active `SessionRuntime` by composing the `moray-core` agent with `moray_sonda::completions::OpenAIChatCompletion`, `moray_sonda::transcripts::SessionTranscripts`, and built-in tools from `moray-sonda`.

For each session turn, the completion adapter and completion capabilities MUST be resolved from the agent currently bound to that session. The server SHALL expose a unified configuration access type that can resolve runtime configuration by `session_id` through the chain `session_id -> agent_id -> AgentConfig -> completion_id -> CompletionConfig`, regardless of whether the underlying data is stored in one TOML file, multiple TOML files, SQLite, or a future mixed backend. The agent MUST reference a configured completion record, and that completion record MUST provide `base_url`, `model`, and `ChatCompletionCapabilities` used to construct `OpenAIChatCompletion`. The `api_key` MUST be supplied to `OpenAIChatCompletion` as a resolved string at each turn construction time: literal keys are taken from stored configuration; omitted `api_key` and `env:` indirection MUST be resolved by reading the environment when constructing the adapter for that turn, not only once at configuration load time.

#### Scenario: Session uses builtin store and tools
- **WHEN** the server initializes its active session
- **THEN** the session MUST be backed by `moray_sonda::transcripts::SessionTranscripts`
- **AND** its `Toolbox` MUST include the default built-in desktop tools configured by `desktop/server`

#### Scenario: Completion credentials and capabilities come from the bound agent
- **WHEN** a session is bound to agent `abcdefgh` and that agent references completion `a1b2c3d4`
- **THEN** the server MUST pass `base_url` and `model` from completion `a1b2c3d4` into `OpenAIChatCompletion`
- **AND** it MUST pass `ChatCompletionCapabilities` from completion `a1b2c3d4` (including `prefill_supported`, defaulting to `false` when omitted)
- **AND** it MUST pass an `api_key` string resolved for that turn per the completion's literal, omitted, or `env:` rules

#### Scenario: Harness resolves completion by session id
- **WHEN** the server constructs runtime harness state for session `default`
- **THEN** the harness construction MUST receive or otherwise retain session id `default`
- **AND** completion construction MUST resolve the effective agent and completion through the server configuration layer using that session id

#### Scenario: Environment-indirected api key is resolved at turn construction time
- **WHEN** the selected completion's `api_key` value uses the `env:` indirection form
- **THEN** the server MUST read the named environment variable when constructing `OpenAIChatCompletion` for that turn
- **AND** the runtime completion adapter MUST receive the resolved key string for that turn, not the `env:` reference

### Requirement: Completion settings are loaded from a TOML file
The `desktop/server` library SHALL define a canonical configuration file path under the user's Moray data directory (`$HOME/.moray/server.toml` when `HOME` is set, using the same directory convention as other desktop artifacts such as the JSONL transcript), SHALL expose a public API to load from that default path, and SHALL expose a public API to load from a caller-provided filesystem path for tests and advanced use.

The on-disk TOML SHALL support the current multi-agent schema:

- `[[completions]]`: completion records with unique tinyid `id`, display `name`, OpenAI-compatible fields `base_url`, `model`, optional `api_key`, and optional nested `[completions.capabilities]`.
- `[[agents]]`: agent records with unique tinyid `id`, display `name`, `completion_id`, and reserved fields for future runtime configuration such as tools.

The loader MUST treat the previous single completion TOML shape with top-level `[credentials]` and optional `[capabilities]` as unsupported. This change is a breaking replacement of the not-yet-released desktop server configuration schema.

The resolved in-memory value suitable for starting the HTTP server MUST contain at least one valid completion and at least one valid agent. Session defaults and per-session agent ids MUST be loaded from the independent session configuration file, not from `server.toml`. Validation of environment-backed `api_key` values for completions that omit `api_key` or use the `env:` form MUST NOT require the environment variable to be set at configuration load time; missing or empty values MUST surface as deterministic errors when the server constructs the completion adapter for a turn that uses that completion.

#### Scenario: Default config path lives next to other ~/.moray artifacts
- **WHEN** the library resolves the default configuration file path on a system where `HOME` is set
- **THEN** the path MUST be `$HOME/.moray/server.toml`
- **AND** that directory MUST be the same logical location used for other Moray desktop user data

#### Scenario: Multi-agent TOML resolves completion records
- **WHEN** the TOML file contains `[[completions]]` entries
- **THEN** each completion MUST have non-empty `base_url` and `model` strings after trim at configuration load time
- **AND** each completion that sets `api_key` to a literal (trimmed value does not begin with the prefix `env:`) MUST have a non-empty `api_key` after trim at load time
- **AND** completions that omit `api_key` or use the `env:` indirection form MUST load successfully even when the referenced environment variable is unset at load time
- **AND** omitted completion capabilities MUST default to `ChatCompletionCapabilities { prefill_supported: false }`

#### Scenario: Legacy single completion TOML is rejected
- **WHEN** the TOML file contains top-level `[credentials]` and optional `[capabilities]` but no `[[completions]]` or `[[agents]]`
- **THEN** the loader MUST reject the file with a deterministic validation error
- **AND** it MUST NOT synthesize a default completion or default agent from the legacy fields

#### Scenario: Literal api key in TOML
- **WHEN** a completion sets `api_key` to a string that after trim does not begin with the prefix `env:`
- **THEN** the resolved `api_key` MUST equal that string after trim
- **AND** `base_url` and `model` MUST be taken from the completion string values after trim

#### Scenario: Environment-indirected api key in TOML
- **WHEN** a completion sets `api_key` to a string whose trimmed value begins with `env:`
- **THEN** the implementation MUST take the substring after `env:`, trim it to obtain an environment variable name
- **AND** when constructing `OpenAIChatCompletion` for a turn using this completion, if that name is non-empty, the runtime `api_key` MUST be the value of `std::env::var` for that name, trimmed
- **AND** if that name is empty or the environment variable is unset or empty after trim at that construction time, the server MUST fail that turn with a deterministic error

#### Scenario: Omitted api key defaults to MORAY_OPENAI_API_KEY
- **WHEN** a completion omits the `api_key` field
- **THEN** the implementation MUST behave as if that completion had set `api_key = "env:MORAY_OPENAI_API_KEY"` when resolving credentials for each turn construction
- **AND** when constructing `OpenAIChatCompletion` for a turn, if `MORAY_OPENAI_API_KEY` is unset or empty after trim, the server MUST fail that turn with a deterministic error

#### Scenario: Agent completion references are validated
- **WHEN** the server loads the multi-agent TOML
- **THEN** each agent's `completion_id` MUST reference an existing completion id
- **AND** loading MUST fail deterministically if an agent references an unknown completion id

### Requirement: Session-agent settings are loaded from an independent TOML file
The `desktop/server` library SHALL currently store session-agent configuration in an independent TOML file under the user's Moray data directory, separate from `server.toml` and separate from the JSONL transcript. Under `[sessions]`, the file SHALL contain exactly one **`default`** key whose value is the default agent tinyid, plus optional flat rows `session_id = agent_tinyid` for sessions that differ from that default. Keys `default_agent`, `default_agent_id`, and `bindings` MUST NOT appear under `[sessions]` in the supported on-disk shape.

The file split SHALL be hidden behind the server's unified configuration access type. HTTP handlers, runtime harness construction, and other server business logic MUST use that unified type rather than reading or composing `server.toml` and the session config file directly.

Writing either `server.toml` or the independent session configuration file SHALL serialize the current structured configuration and SHALL NOT preserve user comments.

#### Scenario: Session config path lives beside server config
- **WHEN** the library resolves the session-agent configuration path on a system where `HOME` is set
- **THEN** the path MUST be under `$HOME/.moray/`
- **AND** it MUST NOT be the same path as `$HOME/.moray/server.toml`
- **AND** it MUST NOT be the JSONL transcript path

#### Scenario: Session config validates default agent
- **WHEN** the server loads the independent session configuration file
- **THEN** its `[sessions].default` value MUST reference an agent loaded from `server.toml`
- **AND** loading MUST fail deterministically if no matching agent exists

#### Scenario: Session rows repair unknown agents
- **WHEN** the server loads session-agent rows from the independent session configuration file
- **AND** `[sessions].default` references an agent loaded from `server.toml`
- **THEN** for each per-session row whose agent id is not present in the loaded agents collection, the server MUST replace that row's agent id with the `[sessions].default` tinyid for the corresponding session id
- **AND** the server MUST persist the repaired session configuration file before serving chat requests (or use equivalent deterministic persistence)
- **AND** the server MUST NOT serve chat turns for a session that remains bound to an unknown agent id

#### Scenario: Config write does not preserve comments
- **WHEN** the server writes updated agent or session configuration to disk
- **THEN** it MUST write the structured TOML representation for the current schema
- **AND** it MUST NOT be required to preserve comments from the previous file content

#### Scenario: Server code uses unified config access
- **WHEN** HTTP routes or harness construction need agent, completion, default-agent, or per-session agent data
- **THEN** they MUST obtain that data through the unified server configuration access type
- **AND** they MUST NOT independently parse or join `server.toml` and the session configuration file

### Requirement: Server exposes agent configuration HTTP API
The HTTP server SHALL expose local-only endpoints for listing agents, listing completions needed by the agent settings UI, reading a session's current agent, switching a session's agent, and updating an agent's completion selection.

Session-scoped read and switch operations SHALL address the session by an explicit `session_id` in the HTTP request target (path segment, query key, or equivalent), so multiple sessions can be supported without a future breaking API change. The current desktop web client MAY use only the `default` session id.

These endpoints SHALL be served by the HTTP `ChatClient` transport; the Tauri command surface MUST NOT be expanded for agent or chat-domain operations.

#### Scenario: Client lists agents and completions
- **WHEN** the web app requests agent settings data
- **THEN** the server MUST return all configured agents with their ids, names, and selected `completion_id`
- **AND** it MUST return the configured completions with ids and display names sufficient to populate a dropdown
- **AND** it MUST NOT include resolved API key secret values in the response

#### Scenario: Client reads current session agent
- **WHEN** the web app requests the agent for a session id (for example `default`) using the session-scoped API shape
- **THEN** the server MUST return the agent id currently bound to that session
- **AND** if no per-session row exists for that session id, the server MUST return the agent id equal to `[sessions].default` (without requiring a redundant row when it would equal `default`)

#### Scenario: Client switches current session agent
- **WHEN** the web app requests that a given session id (for example `default`) switch to an existing agent id
- **THEN** the server MUST persist that session's agent row (or update `[sessions].default` when the session id is `default`)
- **AND** subsequent turns for that session id MUST use the newly selected agent

#### Scenario: Switching agent during active turn affects only future turns
- **WHEN** the web app requests a session-agent switch while the active session is running a turn
- **THEN** the server MUST persist the updated session configuration (row or `[sessions].default` as appropriate)
- **AND** the already-running turn MUST continue using the completion and toolbox instances created when that turn was posted
- **AND** the new mapping MUST be used by the next accepted post for that session

#### Scenario: Unknown agent is rejected
- **WHEN** the web app requests a session-agent switch or agent update using an unknown agent id or completion id
- **THEN** the server MUST respond with HTTP 404 and JSON error code `NOT_FOUND`
- **AND** persisted configuration MUST NOT be modified

### Requirement: Session turns use the persisted session-agent binding
The HTTP server SHALL use the persisted agent binding for the target session when accepting a new user message. A successful agent switch SHALL affect only future turns, including when the switch is accepted during an already-running turn, and SHALL NOT rewrite existing transcript events. The implementation SHALL keep session-agent lookup in the unified server configuration access type so runtime construction can resolve the effective agent from `session_id` without duplicating binding logic in UI code, transcript data, route handlers, or `moray-core`.

#### Scenario: Post message uses selected agent
- **WHEN** a session (for example `default`) is bound to agent `abcdefgh` and the client posts a new user message for that session
- **THEN** the server MUST construct or select the runtime harness using agent `abcdefgh`
- **AND** the completion used for that turn MUST be the completion referenced by agent `abcdefgh`

#### Scenario: Switching agent preserves transcript
- **WHEN** a user switches session `default` from one agent to another while no turn is running
- **THEN** the session transcript MUST NOT be cleared
- **AND** replay through the existing SSE endpoint MUST continue to include prior session events

### Requirement: Primary HTTP start loads default TOML internally
The `desktop/server` library SHALL expose a single public **parameterless** async **`prepare`** entry point that loads configuration from the canonical default configuration file path, resolves it to **`ResolvedServerConfig`**, binds the loopback listener, and assembles the `axum::Router` in one step, without requiring the caller to invoke **`load_config`** before **`prepare`**.

The library MAY expose **`load_config`** / **`load_config_from`** for parsing and validation only; those APIs SHALL NOT be required for the normal desktop startup path when using **`prepare()`**.

#### Scenario: Parameterless prepare loads default file before binding
- **WHEN** `prepare()` is invoked and the file at the canonical default path exists and contains valid TOML per the completion settings requirement
- **THEN** the implementation MUST read and resolve that file before binding the loopback listener
- **AND** the assembled router MUST use the resolved credentials and capabilities for the active session's completion adapter

#### Scenario: Prepare returns the bound listener for the caller
- **WHEN** `prepare()` completes successfully
- **THEN** the returned `ServerComponents` MUST own a `TcpListener` already bound to a loopback address with a runtime-selected port
- **AND** `ServerComponents::local_addr` MUST reflect that bound port without requiring an additional await

### Requirement: JSONL transcript path is not user-configurable

The server SHALL persist and resume the single active session using a JSONL transcript file at a path computed only by library code (not from TOML, not from `MORAY_DESKTOP_TRANSCRIPT_PATH`, and not from removed `ServerOptions` fields), using the same directory and filename rules as the previous default user-level path when `HOME` is set.

#### Scenario: Transcript location ignores TOML and removed env

- **WHEN** the server constructs `SessionTranscripts`
- **THEN** the transcript path MUST be derived solely from the hardcoded library function
- **AND** the path MUST NOT be read from the TOML configuration file
- **AND** the path MUST NOT be read from `MORAY_DESKTOP_TRANSCRIPT_PATH`

### Requirement: ServerHarness uses resolved configuration for completion

The server's `Harness` implementation type SHALL be constructed with the resolved configuration object (or an equivalent immutable snapshot of its fields) and SHALL supply `OpenAIChatCompletion` parameters from that object only, including **`ChatCompletionCapabilities`** derived from the `[capabilities]` table or defaults.

#### Scenario: Completion parameters do not fall back to MORAY_OPENAI_* for file-based startup

- **WHEN** the server builds `OpenAIChatCompletion` for the active session after **`start()`** has loaded and resolved the default-path TOML
- **THEN** `api_key`, `base_url`, and `model` MUST come from the resolved credentials produced from that load
- **AND** `ChatCompletionCapabilities` (including `prefill_supported`) MUST come from the resolved configuration
- **AND** the implementation MUST NOT use `MORAY_OPENAI_API_KEY`, `MORAY_OPENAI_BASE_URL`, or `MORAY_OPENAI_MODEL` as fallbacks on that startup path

### Requirement: Single active session HTTP boundary
The HTTP server SHALL manage exactly one active conversation session and route every chat-domain endpoint to that single session.

#### Scenario: Post message endpoint targets the single session
- **WHEN** a client calls the post-message endpoint with a user message
- **THEN** the server MUST forward that message to the single active session
- **AND** the server MUST NOT require the client to supply a session id to identify which session to use

#### Scenario: Busy session rejects concurrent posts
- **WHEN** a post is issued while the active session is already running a turn
- **THEN** the server MUST respond with HTTP 409 and the JSON error code `BUSY`
- **AND** the active session state MUST NOT be modified

#### Scenario: No multi-session endpoints are exposed
- **WHEN** reviewing the server's HTTP surface
- **THEN** there MUST NOT be endpoints for creating, listing, switching, or deleting multiple sessions
- **AND** all chat operations MUST target the single active session

### Requirement: Resume on startup
The server SHALL resume from the persisted JSONL transcript on startup so app launches continue from prior conversation history.

#### Scenario: Server boots with existing transcript
- **WHEN** the server starts and the JSONL transcript file at the library hardcoded path already contains session events
- **THEN** the server MUST initialize its active session against that transcript without truncating it
- **AND** the next subscribe operation MUST be able to replay those events to the client

### Requirement: Single-endpoint SSE session event stream
The server SHALL expose a single `GET /sessions/{session_id}/events?from_seq=N` endpoint that streams session events as Server-Sent Events. The server MUST NOT expose a separate transcript snapshot HTTP endpoint.

#### Scenario: Stream uses one SSE event name
- **WHEN** a client opens the events endpoint
- **THEN** the server MUST emit every delivered session event as `event: session`
- **AND** persisted history with `from_seq <= seq < live_start_at_subscribe` and subsequent live events MUST use the same event name and payload shape

#### Scenario: Server prevents replay-vs-live races
- **WHEN** the server begins serving an SSE subscription via the transcript store
- **THEN** the store MUST register the live tail before replaying persisted rows for that subscription
- **AND** it MUST NOT drop or duplicate events across the persisted-to-live boundary

#### Scenario: SSE event id supports Last-Event-ID reconnect
- **WHEN** the server emits a session event
- **THEN** the SSE `id:` field MUST be the underlying session-event sequence number
- **AND** when a client reconnects with `Last-Event-ID: K`, the server MUST treat the request as `?from_seq=K+1`

#### Scenario: Persisted replay preserves session-runtime emission order
- **WHEN** the server emits persisted events for a given turn
- **THEN** their relative order MUST match the runtime's original emission order

#### Scenario: Stream surfaces tool authorization requests
- **WHEN** the active session emits a tool authorization request event during a live turn
- **THEN** the events stream MUST deliver an event payload that identifies the pending tool call so the client can prompt the user

### Requirement: Tool authorization reply endpoint
The HTTP server SHALL expose `POST /tool-authorizations/:call_id` that accepts an approve or deny decision for a pending tool call and forwards it to the active session.

#### Scenario: Authorization reply unblocks the session
- **WHEN** a client posts an approve-or-deny decision referencing a pending tool call id
- **THEN** the server MUST reply to the active session using the matching tool call id
- **AND** subsequent session stream events MUST continue to be delivered to subscribed clients

#### Scenario: Reply with unknown call id is rejected
- **WHEN** a client posts a reply for a tool call id that has no pending authorization
- **THEN** the server MUST respond with HTTP 404 and the JSON error code `NOT_FOUND`
- **AND** the active session state MUST NOT be modified

#### Scenario: Reply rejected when no active turn
- **WHEN** a client posts a reply but there is no active turn in progress
- **THEN** the server MUST respond with HTTP 409 and the JSON error code `BUSY`
- **AND** the active session state MUST NOT be modified

### Requirement: Active session reset endpoint
The HTTP server SHALL expose `POST /reset` that resets the active session conversation state.

#### Scenario: Reset clears active session state
- **WHEN** a client calls the reset endpoint while no turn is in progress
- **THEN** the server MUST reset the single active session state through the runtime's reset path
- **AND** subsequent posts MUST behave as starting a fresh conversation

#### Scenario: Reset rejected during an active turn
- **WHEN** a client calls the reset endpoint while a turn is in progress
- **THEN** the server MUST respond with HTTP 409 and the JSON error code `BUSY`
- **AND** the active session state MUST NOT be modified

### Requirement: JSON error envelope
The HTTP server SHALL return a uniform JSON error body for non-2xx responses.

#### Scenario: Error responses share the same shape
- **WHEN** any endpoint returns a non-2xx response
- **THEN** the response body MUST be JSON of shape `{ "code": String, "message": String }`
- **AND** the `code` MUST be one of `BUSY`, `NOT_FOUND`, `BAD_REQUEST`, or `INTERNAL`

#### Scenario: Error status codes map deterministically
- **WHEN** mapping runtime errors to HTTP responses
- **THEN** `MorayError::Busy` MUST map to HTTP 409 with `code: "BUSY"`
- **AND** unknown tool call ids MUST map to HTTP 404 with `code: "NOT_FOUND"`
- **AND** malformed payloads MUST map to HTTP 400 with `code: "BAD_REQUEST"`
- **AND** other runtime errors MUST map to HTTP 500 with `code: "INTERNAL"`

### Requirement: Library lib.rs contains no implementation
The `desktop/server` crate's root `src/lib.rs` SHALL contain only module declarations and re-exports, with no function bodies, type definitions, trait implementations, constants beyond crate-level attributes, or tests.

#### Scenario: Root lib.rs has only mod and pub use
- **WHEN** inspecting `desktop/server/src/lib.rs` after the change
- **THEN** the file MUST contain only the `#![forbid(unsafe_code)]` crate attribute, `mod <name>;` declarations, and `pub use` re-exports
- **AND** the file MUST NOT contain `fn`, `struct`, `enum`, `impl`, `trait`, `const`, or `static` items
- **AND** the file MUST NOT contain `#[cfg(test)] mod tests`

#### Scenario: Implementation is split across dedicated submodules
- **WHEN** reviewing the `desktop/server/src/` directory after the change
- **THEN** the directory MUST contain dedicated submodules for `config`, `state`, `harness`, `error`, `routes`, `sse`, and `bootstrap` concerns
- **AND** each submodule MUST own exactly one of: configuration parsing, application state, harness composition, HTTP error encoding, request handlers plus router assembly, SSE encoding, or startup assembly

### Requirement: Server library trusts the intra-process client for session id
The `desktop/server` crate SHALL NOT validate the `session_id` path segment against an allow-list. Because the server only binds to a loopback address and is consumed by an in-process desktop client, the request payload SHALL be treated as trusted; defensive routing for unknown session ids is out of scope for the single-session phase.

#### Scenario: Handlers do not branch on the session_id path value
- **WHEN** auditing the request handlers in `desktop/server/src/routes.rs`
- **THEN** no handler MUST contain a branch that compares the `session_id` extracted from the URL path to a hard-coded constant
- **AND** no handler MUST return `404 NOT_FOUND` solely because the supplied `session_id` does not match an expected value

#### Scenario: Single-session guard helper is removed
- **WHEN** searching the `desktop/server` crate after the change
- **THEN** no `SINGLE_SESSION_ID` constant MUST exist
- **AND** no `reject_unknown_session` (or equivalent) helper MUST exist

### Requirement: Server library does not own task or shutdown lifecycle
The `desktop/server` crate SHALL NOT spawn Tokio tasks for serving HTTP requests, and SHALL NOT define or return a handle that owns a `JoinHandle` or `CancellationToken` for the HTTP serve loop. Lifecycle management of the serve task SHALL belong to the calling binary.

#### Scenario: Library does not spawn the axum serve task
- **WHEN** auditing public APIs of `moray-desktop-server`
- **THEN** no public function MUST call `tokio::spawn`, `tokio::task::spawn`, `tokio::task::spawn_blocking`, or equivalent to drive `axum::serve` to completion on behalf of the caller
- **AND** no public type MUST expose a `shutdown` method that internally awaits a `JoinHandle`

#### Scenario: Desktop client owns the serve task
- **WHEN** `desktop/client/src-tauri` starts the embedded HTTP server
- **THEN** the client crate MUST be the one calling `tokio::spawn` on the `axum::serve` future
- **AND** the client crate MUST own the `CancellationToken` / `JoinHandle` pair that drives graceful shutdown on application exit

