## Why

`Agent::run` previously accepted caller-supplied `messages`, which spread request-context ownership across call sites and made consistency guarantees hard to enforce.

Persisted event history and model-facing context assembly have different responsibilities and should remain separate abstractions.

## What Changes

- Introduce `ContextEngine` for run-scoped context lifecycle and request assembly (`bootstrap`, `assemble`, `ingest`).
- Keep `SessionStore` dedicated to persisted `SessionEvent` history management.
- Change `Agent::run` to consume `ContextEngine` + cancellation only; remove caller-supplied transcript `messages`.
- Keep `Agent` session-agnostic: no session identifiers, no store handles.
- Keep streaming mode as `Agent` construction-time configuration.
- Keep resume-state strict at session boundary: non-resumable sessions return an explicit error.

## Scope

- `SessionStore` owns persistence of session/agent events, including streaming text-delta and tool-call related persisted events.
- `ContextEngine` owns model-facing context decisions and derived context state for assembly.
- `ContextEngine::assemble` MUST be based on required committed persisted history for that request.
- V1 requires full request semantics on `assemble` (including system/preamble injection).
- In demo wiring, the harness owns the store dependency and creates context engines without per-run store arguments.

## Out of Scope

- Introducing a new persistence schema.
- Changing toolbox authorization behavior.
- Shipping a bundled default `ContextEngine` implementation in `nova-core`.

## Impact

- **Affected specs**: `nova-core`, `nova-demos`
- **Affected code (planned)**:
  - `core/src/agent.rs`
  - `core/src/session.rs`
  - `core/src/session_factory.rs`
  - `core/src/lib.rs`
  - `core/src/context_engine.rs`
  - `demo/src/factory.rs`, `demo/src/transcript.rs`, `demo/tests/*`

## Naming

Use `ContextEngine` as the public trait name and `context_engine` as module path.
