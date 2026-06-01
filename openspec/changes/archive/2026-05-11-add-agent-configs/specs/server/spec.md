## MODIFIED Requirements

### Requirement: Server composes runtime via moray-builtin completions module
The server SHALL build the active `ChatSession` by composing the `moray-core` agent with `moray_builtin::completions::OpenAIChatCompletion` and the `JsonlTranscriptStore` and built-in tools from `moray-builtin`.

For each session turn, the completion adapter and completion capabilities MUST be resolved from the agent currently bound to that session. The server SHALL expose a unified configuration access type that can resolve runtime configuration by `session_id` through the chain `session_id -> agent_id -> AgentConfig -> completion_id -> CompletionConfig`, regardless of whether the underlying data is stored in one TOML file, multiple TOML files, SQLite, or a future mixed backend. The agent MUST reference a configured completion record, and that completion record MUST provide `base_url`, `model`, and `ChatCompletionCapabilities` used to construct `OpenAIChatCompletion`. The `api_key` MUST be supplied to `OpenAIChatCompletion` as a resolved string at each turn construction time: literal keys are taken from stored configuration; omitted `api_key` and `env:` indirection MUST be resolved by reading the environment when constructing the adapter for that turn, not only once at configuration load time.

#### Scenario: Session uses builtin store and tools
- **WHEN** the server initializes its active session
- **THEN** the session MUST be backed by `moray_builtin::stores::JsonlTranscriptStore`
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

## ADDED Requirements

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
