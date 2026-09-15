## ADDED Requirements

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
- **WHEN** auditing public APIs of `nova-desktop-server`
- **THEN** no public function MUST call `tokio::spawn`, `tokio::task::spawn`, `tokio::task::spawn_blocking`, or equivalent to drive `axum::serve` to completion on behalf of the caller
- **AND** no public type MUST expose a `shutdown` method that internally awaits a `JoinHandle`

#### Scenario: Desktop client owns the serve task
- **WHEN** `desktop/client/src-tauri` starts the embedded HTTP server
- **THEN** the client crate MUST be the one calling `tokio::spawn` on the `axum::serve` future
- **AND** the client crate MUST own the `CancellationToken` / `JoinHandle` pair that drives graceful shutdown on application exit

## MODIFIED Requirements

### Requirement: Server lives under desktop as a library
The repository SHALL provide the chat HTTP server as a sub-app under the top-level `desktop/` directory at `desktop/server`, compiled as a Rust library, separate from reusable runtime crates.

#### Scenario: Server is colocated with other desktop sub-apps
- **WHEN** reviewing the repository layout after implementation
- **THEN** the chat HTTP server's source and configuration MUST live at `desktop/server`
- **AND** reusable Nova runtime crates MUST NOT be converted into application-specific packages to host the server

#### Scenario: Server crate is consumed as a library
- **WHEN** inspecting `desktop/server`'s `Cargo.toml`
- **THEN** the crate MUST be configured as a library
- **AND** it MUST expose a public async **parameterless** `prepare` entry point that loads the default-path TOML internally and returns a `ServerComponents` value containing the bound `tokio::net::TcpListener`, its `local_addr: SocketAddr`, and the assembled `axum::Router`
- **AND** it MUST NOT expose a `start` function or a `ServerHandle` type that owns task or shutdown state
- **AND** it MUST expose a public `StartError` error type used as the failure variant of `prepare`'s `Result`

### Requirement: In-process server lifecycle with graceful shutdown
The chat HTTP server SHALL be runnable in the caller's process by composing the library-provided `ServerComponents` with the caller's own task spawning and graceful-shutdown wiring. The library SHALL NOT prescribe the runtime task topology or the shutdown signal source.

#### Scenario: Caller drives the serve loop on its own runtime
- **WHEN** the calling binary uses `nova_desktop_server::prepare()` to obtain `ServerComponents`
- **THEN** the binary MUST be the entity that calls `axum::serve(components.listener, components.app)` and awaits the resulting future on its chosen Tokio task
- **AND** the bound `local_addr` MUST be exposed to the binary via `ServerComponents::local_addr` synchronously before any `await` on the serve future

#### Scenario: Caller wires graceful shutdown
- **WHEN** the calling binary needs to stop the server cleanly
- **THEN** the binary MUST attach its own shutdown signal (e.g., `CancellationToken`, `oneshot::Receiver`) to `axum::serve(..).with_graceful_shutdown(..)`
- **AND** the server library MUST NOT provide a built-in cancellation token field bound to `axum::serve`'s graceful shutdown

#### Scenario: Library is reusable for an out-of-process binary
- **WHEN** a future executable crate wishes to run the server as a standalone process
- **THEN** that executable MUST be able to call the same `prepare()` API and drive `axum::serve` itself without any other public surface in `nova-desktop-server`
- **AND** no public API of `nova-desktop-server` MUST encode an assumption that the server runs inside a Tauri application

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

### Requirement: Local-only loopback binding with runtime-selected port
The library SHALL bind the listener that drives the server only to a local loopback address with a runtime-selected port and SHALL expose the chosen port to its caller via `ServerComponents::local_addr`.

#### Scenario: Library selects a free port at runtime
- **WHEN** `prepare()` completes configuration resolution successfully
- **THEN** the implementation MUST bind a `TcpListener` to `127.0.0.1:0`
- **AND** the resulting `ServerComponents::local_addr` MUST expose the actually bound port

#### Scenario: Server is not exposed beyond the local machine
- **WHEN** the listener is bound by `prepare()`
- **THEN** the implementation MUST NOT bind to a publicly reachable address by default
