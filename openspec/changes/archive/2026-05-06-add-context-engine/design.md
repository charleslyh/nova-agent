## Context

This change defines a strict boundary for context ownership:

- `Agent` executes completion and tool loops.
- `Session` orchestrates persistence, resumability, and run lifecycle.
- `SessionStore` persists `SessionEvent` history.
- `ContextEngine` assembles model request context from committed state.

## Design Decisions

### 1) `Agent::run` input boundary

- `Agent::new` does not accept `ContextEngine`.
- `Agent::run` accepts `Arc<dyn ContextEngine>` and `CancellationToken`.
- `Agent::run` does not accept caller-supplied `messages`.

### 2) Keep `SessionStore` and `ContextEngine` separate

- `SessionStore` responsibilities: `load_events`, `append_event`, `clear` semantics.
- `ContextEngine` responsibilities: `bootstrap`, `assemble`, `ingest`.
- `ContextEngine` is not a persistence replacement for `SessionStore`.
- In demo integration, harness construction binds the store once and context-engine creation reuses that bound dependency.

### 3) Consistency boundary

Before each model request, `ContextEngine::assemble` must observe required committed persisted history.

A revision/version handshake between `Session` and `ContextEngine` is acceptable as the concrete mechanism, but the required behavior is strict consistency for request assembly.

### 4) Resume-state boundary

`Session::resume` enforces resumability before starting a run. If the persisted state is not resumable, resume fails explicitly instead of running a no-op agent loop.

### 5) Streaming configuration boundary

Streaming is configured at `Agent::new` and is not part of `ContextEngine` lifecycle inputs.

### 6) Canonical runtime flow

1. `Session` persists input/lifecycle events through `SessionStore`.
2. `Session` validates resumability (resume path only).
3. `Session` calls `agent.run(context, cancellation)`.
4. `Agent` calls `bootstrap` once, then `assemble` before each completion.
5. `Session` persists committed agent outputs through `SessionStore`.
6. `Agent` calls `ingest` after each completed model+tool round.

## Interface Shape (V1)

### `SessionStore`

- `load_events` (or equivalent): load persisted `SessionEvent` sequence.
- `append_event` (or equivalent): persist session/agent events.
- `clear` (or equivalent): reset persisted conversation state.

### `ContextEngine`

- `bootstrap`: run initialization.
- `assemble`: produce outbound request messages from latest required context.
- `ingest`: advance context state from runtime outputs.

## Risks / Trade-offs

- Signature and wiring changes across `Agent`, `Session`, factory, and demo harness.
- Additional coordination boundary between store commits and engine assembly.
- Two abstractions are maintained intentionally to avoid role coupling.

## Migration Plan

1. Add `context_engine` module and trait.
2. Refactor `Agent::run` signature to remove caller `messages`.
3. Keep `SessionStore` in `Session`; wire `ContextEngine` in run path.
4. Move store binding from run-time `create_context_engine(store)` calls to harness construction-time ownership.
5. Add strict non-resumable handling in `Session::resume`.
6. Update demo JSONL flow (store persistence + engine assembly).
7. Update tests and run suite.
