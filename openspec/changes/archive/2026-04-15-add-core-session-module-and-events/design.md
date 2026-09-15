## Context
`Agent` in `nova-core` already provides recoverable invoke/resume semantics via `AgentInvokeEvent`. However, orchestration concerns (loading history, constructing agents from history, appending events, reset flow, and interactive delegation) currently live in `demo/examples/chat.rs`.

The new design introduces a small session layer inside `nova-core` so orchestration can be reused while keeping `Agent` as the state-machine core.

## Goals / Non-Goals
- Goals:
  - Introduce a `Session` abstraction in `nova-core` that manages `store` and `agent` lifecycle.
  - Introduce `SessionEvent` with explicit session lifecycle events and `AgentEvent { event: AgentInvokeEvent }` passthrough.
  - Extract session-related logic from `chat` while preserving transcript behavior.
- Non-Goals:
  - No new `nova-session` crate in this change.
  - No redesign of `AgentInvokeEvent` schema.
  - No transcript format migration.

## Decisions
### Decision: Keep session inside `nova-core` as a module
- Rationale: smallest scope and lowest migration risk while making orchestration reusable.
- Alternative considered: a new `nova-session` crate.
  - Rejected for now to avoid package-level churn before API stabilizes.

### Decision: Keep `Session` constructor minimal (`store` + prebuilt `agent`)
- Rationale: current orchestration does not require a separate builder abstraction; passing a prebuilt `Agent` keeps API and implementation simple.
- Alternative considered: adding an `AgentBuilder` trait object.
  - Rejected because it is not necessary for the current session lifecycle behavior.

### Decision: `SessionEvent` is a thin wrapper for agent events
- Rationale: preserve `AgentInvokeEvent` as the single invoke/resume semantic source and avoid duplicated event taxonomies.
- Shape:
  - session lifecycle variants (started/resumed/reset/finished/failed)
  - `AgentEvent { event: AgentInvokeEvent }`

## Risks / Trade-offs
- Risk: session lifecycle events may overlap conceptually with existing invoke boundaries.
  - Mitigation: lifecycle events describe session orchestration only; invoke semantics stay in `AgentInvokeEvent`.
- Risk: demo refactor may accidentally alter interactive UX.
  - Mitigation: preserve one shared event-consumption path and keep transcript schema unchanged.

## Migration Plan
1. Add specs for `Session` and `SessionEvent`.
2. Implement `core/src/session.rs` and expose it from `core/src/lib.rs`.
3. Provide demo-side adapter from existing transcript store to `SessionStore`.
4. Move interactive orchestration to `Session` calls.
5. Validate with OpenSpec strict validation and Rust compile/tests.

## Open Questions
- Whether some lifecycle variants should carry metadata (session id, reason strings) on day one.
- Whether `SessionStore` should remain event-log-only or add snapshots in a later change.
