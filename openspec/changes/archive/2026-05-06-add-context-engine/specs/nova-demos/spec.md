## MODIFIED Requirements

### Requirement: chat example

The `demo` chat example SHALL use:

- a JSONL-backed `SessionStore` for persisted `SessionEvent` history,
- and a `ContextEngine` that assembles outbound request messages for `agent.run(...)` from persisted/derived context.

In the demo, a unified JSONL storage implementation MAY satisfy both traits (`SessionStore` and `ContextEngine`) to reuse raw transcript data, while preserving each trait's responsibilities.

The demo MAY keep lifecycle methods beyond `assemble` minimal in V1 (e.g. success without additional work).

#### Scenario: chat example uses store + engine split

- **WHEN** the demo `chat` example invokes or resumes an agent run
- **THEN** it MUST persist/replay history via JSONL `SessionStore`, and MUST obtain model request messages via `ContextEngine::assemble` rather than passing caller-built `messages` directly into `Agent::run` (regardless of whether both traits are implemented by one concrete storage type)
