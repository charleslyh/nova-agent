## 1. Server Config Model

- [x] 1.1 Add a unified server config access type/store that owns loading, validation, querying, and writing for agent/completion/session configuration.
- [x] 1.2 Implement tinyid validation and generation for completion and agent ids, including uniqueness checks within each collection.
- [x] 1.3 Add file-backed storage under the unified config type: parse `server.toml` for `[[completions]]` and `[[agents]]`, and parse the independent session config file for `[sessions].default` plus flat per-session rows.
- [x] 1.4 Reject legacy top-level `[credentials]` and `[capabilities]` config with a deterministic validation error.
- [x] 1.5 Add unified config write methods that persist updates to the appropriate file-backed TOML schema and do not preserve comments.
- [x] 1.6 Add unified config APIs for listing agents/completions, resolving runtime config by session id, reading/switching session agent, and updating an agent's completion.
- [x] 1.7 Add unit tests for valid multi-agent config, unsupported top-level `[credentials]` rejection, invalid tinyids, duplicate ids, missing completion references, missing default agent, invalid session rows repaired to `[sessions].default` with persisted write-back, comment-discarding writes, literal `api_key`, `env:` and omitted `api_key` resolved at turn construction time (including failure when the variable is unset at construction time), and route/harness-facing access through the unified config type.

## 2. Session-Agent Binding Runtime

- [x] 2.1 Use the unified config type to resolve `session_id -> agent_id -> AgentConfig -> completion_id -> CompletionConfig`, assign the default agent when no binding exists, and persist explicit switches.
- [x] 2.2 Refactor `ServerHarness` construction to receive the session id and shared config resolver/store, and resolve completion credentials and capabilities from the session's currently bound agent.
- [x] 2.3 Ensure each post creates completion/toolbox from the current session binding via the session-aware harness, so an accepted switch affects the next post only.
- [x] 2.4 Ensure switching a session's agent does not clear the JSONL transcript, change SSE replay behavior, append an agent-switch transcript event, or mutate already-running turn instances.
- [x] 2.5 Add Rust tests proving config resolution by session id, post-message uses the selected agent, switching during an active turn is accepted for the next post, and switching preserves transcript state.

## 3. Server HTTP API

- [x] 3.1 Add response DTOs for agent cards and completion dropdown options that do not expose resolved API key secret values.
- [x] 3.2 Add routes for listing agent settings data and updating an agent's `completion_id`.
- [x] 3.3 Add routes for reading and switching a session's agent binding using an explicit `session_id` in the HTTP API shape (local web may use only `default` initially).
- [x] 3.4 Map unknown agent/completion ids to HTTP 404 `NOT_FOUND`; do not reject session-agent switches solely because a turn is active.
- [x] 3.5 Add HTTP tests for listing agents/completions, default session agent resolution, successful switch persistence, unknown id rejection, and active-turn switch persistence for the next post.

## 4. Web ChatClient Integration

- [x] 4.1 Extend `createChatClient` with methods for loading agent settings data, reading a session's agent, switching a session's agent (session id parameter; may default to `default` at call sites), and updating an agent's completion selection.
- [x] 4.2 Implement the new methods in the HTTP-backed `ChatClient` using the server routes.
- [x] 4.3 Update the chat session composable or a new focused composable to load agents, track current agent id, and coordinate switch/update calls.
- [x] 4.4 Keep Vue components free of direct `fetch`, `EventSource`, and Tauri command calls for agent operations.

## 5. Web Agents UI

- [x] 5.1 Add an agents settings entry (e.g. app chrome / `AppSidebar`) and wire it to open/close the settings surface (`SettingsDialog`).
- [x] 5.2 Implement a settings shell with sidebar nav and an Agents content view that renders one card per agent.
- [x] 5.3 Implement an agent edit dialog opened from an agent card with a completion dropdown populated from server-provided completion options (`AgentEditDialog`).
- [x] 5.4 Save agent `completion_id` changes through `ChatClient` and refresh visible card/detail state after successful save.
- [x] 5.5 Add an Agent selection menu to the composer's lower-left action area, initialized from the active session's server-bound agent.
- [x] 5.6 Allow composer Agent selection while the session status is running; accepted switches apply from the next post onward (optional extra UI copy).

## 6. Documentation and Verification

- [x] 6.1 Update desktop README or sample configuration documentation with the new `server.toml` multi-agent schema, optional `api_key` default behavior and **turn-time** `env:` / omitted-key resolution, invalid session row repair to `[sessions].default`, independent session config file, and comment-discarding write behavior.
- [x] 6.2 Run Rust formatting and compile/test commands for the affected Rust workspace/crates.
- [x] 6.3 Run the desktop web package build or equivalent verification command.
- [x] 6.4 Run `openspec validate add-agent-configs --strict` and fix any proposal/spec/task issues.
