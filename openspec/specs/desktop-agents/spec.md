# desktop-agents Specification

## Purpose
TBD - created by archiving change add-agent-configs. Update Purpose after archive.

## Requirements
### Requirement: Desktop agents are named runtime configuration records
The desktop application SHALL model an agent as a local persisted runtime configuration record with a stable `id`, a user-facing `name`, a `completion_id` reference, and reserved fields for future tool/context configuration.

Agent and completion ids MUST use tinyid format: exactly 8 ASCII lowercase letters or digits (`[a-z0-9]{8}`). Ids MUST be unique across the records of the same kind in the loaded server configuration.

#### Scenario: Agent record references an existing completion
- **WHEN** the server loads an agent record from persisted configuration
- **THEN** the agent's `completion_id` MUST refer to a configured completion record
- **AND** loading MUST fail deterministically if the referenced completion id does not exist

#### Scenario: Invalid id is rejected
- **WHEN** the server loads or accepts an agent or completion id that is not exactly 8 lowercase letters or digits
- **THEN** the configuration or request MUST be rejected with a deterministic validation error

#### Scenario: Duplicate ids are rejected within each collection
- **WHEN** the server loads multiple agents with the same id or multiple completions with the same id
- **THEN** the configuration MUST be rejected
- **AND** no ambiguous agent or completion selection MUST be allowed

### Requirement: Default agent determines unresolved sessions
The desktop configuration access layer SHALL expose one `[sessions].default` tinyid that identifies the agent used for any session id that has **no** explicit per-session row under `[sessions]`. Per-session rows MUST be omitted when the agent would equal `[sessions].default` (except that the session id `default` is represented by updating `[sessions].default`, not a separate row).

#### Scenario: New session uses default agent
- **WHEN** a session id has no per-session row and the server needs that session's agent
- **THEN** the effective agent id MUST equal `[sessions].default`
- **AND** subsequent reads of that session's agent MUST return the same agent id until a row is persisted for that session id or `[sessions].default` changes

#### Scenario: Default agent must exist
- **WHEN** the server loads configuration with `[sessions].default`
- **THEN** that id MUST reference an existing agent
- **AND** loading MUST fail deterministically if no matching agent exists

### Requirement: Session-agent binding is persisted locally
The desktop application SHALL persist the mapping from session id to agent id through the unified configuration access layer so that a session continues using the selected agent across web reloads and desktop app restarts. The current file-backed implementation stores this mapping in an independent local session configuration file, separate from both `server.toml` and the JSONL transcript.

#### Scenario: Session binding survives restart
- **WHEN** a user switches session `default` to agent `abcdefgh` and the desktop app restarts
- **THEN** the server MUST load session `default` as bound to agent `abcdefgh`
- **AND** subsequent messages in that session MUST use agent `abcdefgh` unless the user switches again

#### Scenario: Binding is not written to transcript
- **WHEN** a user switches session `default` from one agent to another
- **THEN** the server MUST persist the binding in the independent session configuration file
- **AND** it MUST NOT append an agent-switch event to the JSONL transcript

#### Scenario: Missing bound agent is repaired to default
- **WHEN** persisted per-session rows reference an agent id that is not present in the agents collection
- **AND** `[sessions].default` references an existing agent
- **THEN** the unified configuration access layer MUST replace each invalid row's agent id with the `[sessions].default` tinyid for the affected session id
- **AND** it MUST persist the repaired session configuration file (or equivalent deterministic persistence) before serving chat requests
- **AND** it MUST NOT run turns while a session remains bound to an unknown agent id
