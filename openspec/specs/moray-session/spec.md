# moray-session Specification

## Purpose

Defines **`moray-session`**: recoverable multi-turn **session domain** types and orchestration (event log, runtime, harness wiring). This crate defines **traits and models**, not concrete storage, model HTTP, or product TOML.

## Requirements

### Requirement: Session package layout

The repository SHALL provide `moray-session` at `crates/session` as a workspace member depending only on `moray-core` (plus minimal async/tokio).

#### Scenario: Session is buildable in isolation

- **WHEN** running `cargo build -p moray-session`
- **THEN** the crate MUST compile without `moray-extensions` or `moray-sonda`

### Requirement: Session domain traits

`moray-session` SHALL define session-scoped extension traits including at minimum:

- `Harness` — factory for per-turn `ChatCompletion`, `Toolbox`, and `Preambler` from `moray-core`
- `SessionEventSink` — append persisted `SessionEvent` rows
- `SessionEventSink` — in-process delivery of live events

#### Scenario: Harness is not defined in moray-core

- **WHEN** inspecting `moray-core`
- **THEN** `Harness` MUST NOT be defined there
- **AND** `Harness` MUST be defined in `moray-session`

### Requirement: Session runtime

`moray-session` SHALL provide `SessionRuntime` with `submit`, `reset`, and busy-session semantics, driven by a `Harness` and `SessionEventSink` supplied at construction.

#### Scenario: Submit persists and streams agent events

- **WHEN** `SessionRuntime::submit` completes a turn successfully
- **THEN** persisted events MUST include `TurnAccepted`, `AgentResponse` chunks, and `TurnFinish` via the configured writer
