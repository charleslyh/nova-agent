## ADDED Requirements

### Requirement: Completion settings are loaded from a TOML file

The `desktop/server` library SHALL define a canonical configuration file path under the user's Moray data directory (`$HOME/.moray/` when `HOME` is set, using the same directory convention as other desktop artifacts such as the JSONL transcript), SHALL expose a public API to load from that default path, and SHALL expose a public API to load from a caller-provided filesystem path for tests and advanced use.

The on-disk TOML SHALL separate **credentials** from **capabilities** using distinct top-level tables: **`[credentials]`** (at least `api_key`, `base_url`, `model`) and **`[capabilities]`** (fields mapping to `moray_core::ChatCompletionCapabilities`, including **`prefill_supported`**). The **`[capabilities]`** table MAY be omitted; when omitted, capabilities MUST default such that **`prefill_supported` is false**.

The resolved in-memory value suitable for starting the HTTP server MUST contain non-empty `api_key`, `base_url`, and `model` strings after resolution, and MUST contain a **`ChatCompletionCapabilities`** value derived from `[capabilities]` or defaults.

### Requirement: Primary HTTP start loads default TOML internally

The `desktop/server` library SHALL expose a single public **parameterless** async **`start`** entry point that loads configuration from the canonical default configuration file path, resolves it to **`ResolvedServerConfig`**, and starts the HTTP server in one step, without requiring the caller to invoke **`load_config`** before **`start`**.

The library MAY expose **`load_config`** / **`load_config_from`** for parsing and validation only; those APIs SHALL NOT be required for the normal desktop startup path when using **`start()`**.

#### Scenario: Parameterless start loads default file before binding

- **WHEN** `start()` is invoked and the file at the canonical default path exists and contains valid TOML per the completion settings requirement
- **THEN** the implementation MUST read and resolve that file before accepting HTTP connections
- **AND** the server MUST use the resolved credentials and capabilities for the active session's completion adapter

#### Scenario: Default config path lives next to other ~/.moray artifacts

- **WHEN** the library resolves the default configuration file path on a system where `HOME` is set
- **THEN** the path MUST be under `$HOME/.moray/` with a fixed filename chosen by `desktop/server`
- **AND** that directory MUST be the same logical location used for other Moray desktop user data (including the hardcoded JSONL transcript path pattern)

#### Scenario: Literal api key in TOML

- **WHEN** the TOML file sets `[credentials].api_key` to a string that after trim does not begin with the prefix `env:`
- **THEN** the resolved `api_key` MUST equal that string after trim
- **AND** `base_url` and `model` MUST be taken from `[credentials]` string values after trim

#### Scenario: Environment-indirected api key in TOML

- **WHEN** the TOML file sets `[credentials].api_key` to a string whose trimmed value begins with `env:`
- **THEN** the implementation MUST take the substring after `env:`, trim it to obtain an environment variable name
- **AND** if that name is non-empty, the resolved `api_key` MUST be the value of `std::env::var` for that name, trimmed
- **AND** if that name is empty or the environment variable is unset or empty after trim, loading MUST fail with a deterministic error

#### Scenario: Capabilities default when omitted

- **WHEN** the TOML file contains `[credentials]` but omits the `[capabilities]` table
- **THEN** the resolved capabilities MUST be equivalent to `ChatCompletionCapabilities { prefill_supported: false }`

#### Scenario: prefill_supported can be set under capabilities

- **WHEN** the TOML file sets `[capabilities].prefill_supported` to `true`
- **THEN** the resolved `ChatCompletionCapabilities` MUST have `prefill_supported == true`

### Requirement: JSONL transcript path is not user-configurable

The server SHALL persist and resume the single active session using a JSONL transcript file at a path computed only by library code (not from TOML, not from `MORAY_DESKTOP_TRANSCRIPT_PATH`, and not from removed `ServerOptions` fields), using the same directory and filename rules as the previous default user-level path when `HOME` is set.

#### Scenario: Transcript location ignores TOML and removed env

- **WHEN** the server constructs `JsonlTranscriptStore`
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

## MODIFIED Requirements

### Requirement: Server crate is consumed as a library

The repository SHALL provide the chat HTTP server as a sub-app under the top-level `desktop/` directory at `desktop/server`, compiled as a Rust library, separate from reusable runtime crates.

#### Scenario: Server is colocated with other desktop sub-apps

- **WHEN** reviewing the repository layout after implementation
- **THEN** the chat HTTP server's source and configuration MUST live at `desktop/server`
- **AND** reusable Moray runtime crates MUST NOT be converted into application-specific packages to host the server

#### Scenario: Server crate is consumed as a library

- **WHEN** inspecting `desktop/server`'s `Cargo.toml`
- **THEN** the crate MUST be configured as a library
- **AND** it MUST expose a public async **parameterless** `start` entry point that returns `ServerHandle`, loads the default-path TOML internally, and does not take `ServerOptions`
- **AND** it MUST expose `ServerHandle::shutdown()` async method to its callers

### Requirement: Server composes runtime via moray-builtin completions module

The server SHALL build its single `ChatSession` by composing the `moray-core` agent with `moray_builtin::completions::OpenAIChatCompletion` and the `JsonlTranscriptStore` and `CalcTool` from `moray-builtin`.

#### Scenario: Session uses builtin store and tool

- **WHEN** the server initializes its single active session
- **THEN** the session MUST be backed by `moray_builtin::stores::JsonlTranscriptStore`
- **AND** its `Toolbox` MUST include `moray_builtin::tools::CalcTool`

#### Scenario: Completion credentials and capabilities come from resolved TOML-backed configuration

- **WHEN** the server starts via **`start()`** after loading and resolving the default-path TOML
- **THEN** it MUST pass `api_key`, `base_url`, and `model` from that resolved value into `OpenAIChatCompletion`
- **AND** it MUST pass `ChatCompletionCapabilities` from that value (including `prefill_supported` from `[capabilities]` or default **false** when omitted)
- **AND** for `api_key`, if the TOML value used the `env:` indirection form, the resolved value MUST have been read from the named environment variable at load time

### Requirement: Resume on startup

The server SHALL resume from the persisted JSONL transcript on startup so app launches continue from prior conversation history.

#### Scenario: Server boots with existing transcript

- **WHEN** the server starts and the JSONL transcript file at the library hardcoded path already contains session events
- **THEN** the server MUST initialize its active session against that transcript without truncating it
- **AND** the next subscribe operation MUST be able to replay those events to the client

### Requirement: In-process server lifecycle with graceful shutdown

The chat HTTP server SHALL run as a Tokio task in the calling process and SHALL support graceful shutdown driven by an external cancellation signal.

#### Scenario: Server runs in the caller's Tokio runtime

- **WHEN** `start()` is invoked from `desktop/client`
- **THEN** the server MUST spawn its `axum` task on the same Tokio runtime as the caller
- **AND** the returned `ServerHandle` MUST expose the bound `local_addr`

#### Scenario: Graceful shutdown drains in-flight requests

- **WHEN** the caller invokes `ServerHandle::shutdown()`
- **THEN** the server MUST stop accepting new requests
- **AND** the server MUST allow in-flight requests to finish before the awaited handle returns

### Requirement: Local-only loopback binding with runtime-selected port

The server SHALL bind only to a local loopback address with a runtime-selected port and SHALL expose the chosen port to its caller.

#### Scenario: Server selects a free port at runtime

- **WHEN** `start()` completes configuration resolution successfully
- **THEN** the server MUST bind to `127.0.0.1:0` (or an explicitly provided loopback address)
- **AND** the resulting `ServerHandle` MUST expose the actually bound port via `local_addr`

#### Scenario: Server is not exposed beyond the local machine

- **WHEN** the server is running
- **THEN** the server MUST NOT bind to a publicly reachable address by default
