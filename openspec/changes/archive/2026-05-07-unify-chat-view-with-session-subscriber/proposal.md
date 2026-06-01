## Why

`demo/examples/chat.rs` currently renders chat output directly from the stream returned by `session.post`. This couples rendering to one operation path and creates multiple state sources when replay, reset, or other session operations also mutate state. We need a single state truth source so future UX (web navigation, resume rendering, consistent view recovery) can rely on the same delta stream.

## What Changes

- Refactor chat rendering flow so chat view no longer consumes the direct `session.post` stream for UI updates.
- Add subscriber support to transcript store after replay, and make chat view consume only subscriber-delivered session delta events.
- Start subscription from `last_event_seq + 1` (from `store.load`) so replayed history is not duplicated and only new deltas are rendered.
- Support multiple store subscribers concurrently.
- Auto-clean dropped or broken subscribers in `JsonlTranscriptStore` by removing channels whose send fails.
- Simplify runtime orchestration so session operations (`post`, `reset`, etc.) and state rendering/tracking are clearly separated while keeping control flow manageable.

## Capabilities

### New Capabilities
- `session-delta-subscription`: Defines store-backed multi-subscriber delta streaming with sequence-based subscription offsets for session state tracking.

### Modified Capabilities
- `moray-demos`: Update demo chat behavior to render from store subscriber deltas instead of directly binding rendering to `session.post` stream output.

## Impact

- Affected code: `demo/examples/chat.rs`, `sessions/src/session.rs`, `sessions/src/lib.rs`, transcript store implementation and related tests.
- Runtime behavior: chat UI consumes a unified delta feed; session operations become producers rather than direct render drivers.
- Concurrency model: introduces multi-subscriber fan-out and subscriber lifecycle cleanup in `JsonlTranscriptStore`.
- Product direction: enables future web-side state continuity and single-source-of-truth rendering behavior.
