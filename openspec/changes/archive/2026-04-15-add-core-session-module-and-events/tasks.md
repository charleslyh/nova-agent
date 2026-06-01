## 1. Specification
- [x] 1.1 Add `moray-core` deltas for `Session` and `SessionEvent` requirements.
- [x] 1.2 Modify `moray-demos` `chat` requirement to delegate session orchestration to `moray_core::Session`.

## 2. Core implementation
- [x] 2.1 Add `core/src/session.rs` with `Session`, `SessionStore`, and `SessionEvent` definitions.
- [x] 2.2 Wire `core/src/lib.rs` exports for the new session module.
- [x] 2.3 Keep `AgentInvokeEvent` as the single invoke/resume event schema and expose it only through `SessionEvent::AgentEvent`.

## 3. Demo extraction
- [x] 3.1 Adapt transcript store in `demo` to implement the new `SessionStore` interface.
- [x] 3.2 Refactor `demo/examples/chat.rs` to call `Session` for bootstrap, invoke, resume, auth replies, and reset.
- [x] 3.3 Preserve existing user-visible behavior and JSONL transcript compatibility.

## 4. Validation
- [x] 4.1 Run `openspec validate add-core-session-module-and-events --strict`.
- [x] 4.2 Run `cargo build`.
- [x] 4.3 Run targeted tests for `moray-core` and `demo` (`cargo test -p moray-core` and `cargo test -p demo`).
