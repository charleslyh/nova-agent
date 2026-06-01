## Why
`chat` currently owns transcript loading, agent bootstrap, resume/invoke orchestration, and event append logic directly. This couples demo code with runtime lifecycle concerns and makes session behavior hard to reuse.

## What Changes
- Add a `session` module inside `moray-core` that owns session lifecycle orchestration for store + agent.
- Add a `SessionEvent` stream contract with dedicated lifecycle events and `AgentEvent { event: AgentInvokeEvent }` passthrough for invoke/resume events.
- Update `chat` requirements so session-related orchestration is delegated to `Session`.

## Impact
- Affected specs: `moray-core`, `moray-demos`
- Affected code: `core/src/lib.rs`, new `core/src/session.rs`, `demo/examples/chat.rs`, `demo/src/transcript.rs`
- Behavioral impact: no protocol rewrite of `AgentInvokeEvent`; session layer wraps and forwards existing semantics
