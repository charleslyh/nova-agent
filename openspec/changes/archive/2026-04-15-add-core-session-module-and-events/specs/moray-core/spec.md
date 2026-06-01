## ADDED Requirements

### Requirement: Session runtime orchestration in moray-core module

`moray-core` SHALL provide a `session` module that encapsulates session lifecycle orchestration over persisted event storage and `Agent` execution.

The `Session` abstraction SHALL own and coordinate:
- a store interface for loading, appending, and clearing persisted session events,
- a prebuilt `Agent` instance provided at construction time,
- invoke/resume/reset flows that keep persisted events and in-memory agent state consistent.

#### Scenario: Bootstrap session from persisted events

- **WHEN** a consumer constructs `Session` with a store containing prior `SessionEvent` records
- **THEN** the session MUST be able to continue using those persisted events and determine whether pending resumable work exists

#### Scenario: Reset session state

- **WHEN** a consumer requests a session reset
- **THEN** the session MUST clear persisted history through the store and reset session runtime counters/state used for subsequent turns

### Requirement: Session outward stream uses SessionEvent with agent passthrough

`moray-core` SHALL expose `SessionEvent` as the outward event stream contract for session operations.

`SessionEvent` MUST include:
- session lifecycle events for orchestration boundaries and failures,
- `AgentEvent { event: AgentInvokeEvent }` as the dedicated passthrough variant for agent invoke/resume events.

The session layer MUST NOT redefine a second set of invoke/resume semantics equivalent to `AgentInvokeEvent`.

#### Scenario: Invoke/resume events are forwarded without semantic duplication

- **WHEN** an underlying `Agent` emits `AgentInvokeEvent` items during `invoke` or `resume`
- **THEN** `Session` MUST publish those events via `SessionEvent::AgentEvent` while preserving the original event payload semantics

#### Scenario: Session lifecycle and agent events are distinguishable

- **WHEN** a consumer subscribes to `Session` output
- **THEN** it MUST be able to distinguish lifecycle transitions (such as started/resumed/reset/failed/finished) from forwarded `AgentInvokeEvent` items using `SessionEvent` variants
