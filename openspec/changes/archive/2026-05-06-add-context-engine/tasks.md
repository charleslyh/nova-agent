## 1. Implementation

- [x] 1.1 Add `context_engine` module with `ContextEngine` trait (`bootstrap` / `assemble` / `ingest`).
- [x] 1.2 Refactor `Agent::run` to `(context, cancellation)` and remove caller-supplied transcript `messages`.
- [x] 1.3 Keep `Agent::new` stateless regarding context lifecycle (no constructor-bound context dependency).
- [x] 1.4 Keep streaming mode as construction-time configuration only.
- [x] 1.5 Keep `SessionStore` as the sole persistence owner for `SessionEvent` history (`load_events` / `append_event` / `clear`).
- [x] 1.6 Wire lifecycle calls in `Agent::run`: `bootstrap` once per run, `assemble` before each completion, `ingest` after completed model+tool rounds.
- [x] 1.7 Update `Session` flow to persist via `SessionStore`, run agent with `ContextEngine`, and keep committed outputs persisted through store.
- [x] 1.8 Enforce strict resume state at session boundary (`Session::resume` returns explicit error when state is not resumable).
- [x] 1.9 Update `SessionHarnessFactory`/demo harness wiring so store is bound at harness construction and `create_context_engine()` no longer takes per-run store input.
- [x] 1.10 In demo chat flow, keep JSONL-backed persistence + context assembly behavior and preserve system-message ordering.
- [x] 1.11 Verify with `cargo test -p nova-core` and `cargo test -p demo`.

## 2. Documentation

- [x] 2.1 Update docs/comments to clarify ownership boundary: `SessionStore` persists event log; `ContextEngine` assembles model context.

## 3. Spec

- [x] 3.1 Keep this change validated: `openspec validate add-context-engine --strict`
