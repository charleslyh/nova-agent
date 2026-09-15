## ADDED Requirements

### Requirement: Server lives under desktop as a library
The repository SHALL provide the chat HTTP server as a sub-app under the top-level `desktop/` directory at `desktop/server`, compiled as a Rust library, separate from reusable runtime crates.

#### Scenario: Server is colocated with other desktop sub-apps
- **WHEN** reviewing the repository layout after implementation
- **THEN** the chat HTTP server's source and configuration MUST live at `desktop/server`
- **AND** reusable Nova runtime crates MUST NOT be converted into application-specific packages to host the server

#### Scenario: Server crate is consumed as a library
- **WHEN** inspecting `desktop/server`'s `Cargo.toml`
- **THEN** the crate MUST be configured as a library
- **AND** it MUST expose a public `start(opts) -> ServerHandle` async API and a `ServerHandle::shutdown()` async method to its callers

### Requirement: In-process server lifecycle with graceful shutdown
The chat HTTP server SHALL run as a Tokio task in the calling process and SHALL support graceful shutdown driven by an external cancellation signal.

#### Scenario: Server runs in the caller's Tokio runtime
- **WHEN** `start(opts)` is invoked from `desktop/client`
- **THEN** the server MUST spawn its `axum` task on the same Tokio runtime as the caller
- **AND** the returned `ServerHandle` MUST expose the bound `local_addr`

#### Scenario: Graceful shutdown drains in-flight requests
- **WHEN** the caller invokes `ServerHandle::shutdown()`
- **THEN** the server MUST stop accepting new requests
- **AND** the server MUST allow in-flight requests to finish before the awaited handle returns

### Requirement: Local-only loopback binding with runtime-selected port
The server SHALL bind only to a local loopback address with a runtime-selected port and SHALL expose the chosen port to its caller.

#### Scenario: Server selects a free port at runtime
- **WHEN** `start(opts)` is invoked
- **THEN** the server MUST bind to `127.0.0.1:0` (or an explicitly provided loopback address)
- **AND** the resulting `ServerHandle` MUST expose the actually bound port via `local_addr`

#### Scenario: Server is not exposed beyond the local machine
- **WHEN** the server is running
- **THEN** the server MUST NOT bind to a publicly reachable address by default

### Requirement: Server composes runtime via nova-builtin completions module
The server SHALL build its single `ChatSession` by composing the `nova-core` agent with `nova_builtin::completions::OpenAIChatCompletion` and the `JsonlTranscriptStore` and `CalcTool` from `nova-builtin`.

#### Scenario: Session uses builtin store and tool
- **WHEN** the server initializes its single active session
- **THEN** the session MUST be backed by `nova_builtin::stores::JsonlTranscriptStore`
- **AND** its `Toolbox` MUST include `nova_builtin::tools::CalcTool`

#### Scenario: Completion credentials come from env or options
- **WHEN** the server starts
- **THEN** it MUST resolve OpenAI credentials from `ServerOptions` if provided, otherwise from `NOVA_OPENAI_API_KEY` / `NOVA_OPENAI_BASE_URL` / `NOVA_OPENAI_MODEL` environment variables

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
- **WHEN** the server starts and the configured transcript file already contains session events
- **THEN** the server MUST initialize its active session against that transcript without truncating it
- **AND** the next subscribe operation MUST be able to replay those events to the client

### Requirement: Single-endpoint SSE stream with replay and live phases
The server SHALL expose a single `GET /events?from_seq=N` endpoint that streams session events as Server-Sent Events, distinguishing replay history from live updates by SSE event names.

#### Scenario: Stream uses three named SSE event types
- **WHEN** a client opens the events endpoint
- **THEN** the server MUST emit historical events with `seq <= last_seq_at_subscribe` as `event: replay`
- **AND** it MUST emit a single `event: live-start` marker (with no payload data) after the last replay event
- **AND** it MUST emit subsequent events as `event: live`

#### Scenario: Server prevents replay-vs-live races
- **WHEN** the server begins serving an SSE subscription
- **THEN** it MUST start its live subscription against the store before draining the replay range
- **AND** it MUST NOT drop events that arrive between computing `last_seq_at_subscribe` and finishing replay

#### Scenario: SSE event id supports Last-Event-ID reconnect
- **WHEN** the server emits a session event
- **THEN** the SSE `id:` field MUST be the underlying session-event sequence number
- **AND** when a client reconnects with `Last-Event-ID: K`, the server MUST treat the request as `?from_seq=K+1`

#### Scenario: Replay phase preserves session-runtime emission order
- **WHEN** the server emits replay events for a given turn
- **THEN** their relative order MUST match the runtime's original emission order

#### Scenario: Live phase surfaces tool authorization requests
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
- **THEN** `NovaError::Busy` MUST map to HTTP 409 with `code: "BUSY"`
- **AND** unknown tool call ids MUST map to HTTP 404 with `code: "NOT_FOUND"`
- **AND** malformed payloads MUST map to HTTP 400 with `code: "BAD_REQUEST"`
- **AND** other runtime errors MUST map to HTTP 500 with `code: "INTERNAL"`
