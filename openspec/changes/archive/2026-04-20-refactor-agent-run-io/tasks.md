## 1. Types and API

- [x] 1.1 Add `AgentRunResponseMessage` in `moray-core` with the agreed variants (`InvokeStarted` without user payload); re-export from `lib.rs` as needed.
- [x] 1.2 Change `Agent::run` to `(&[ChatCompletionRequestMessage], AgentRunOptions) -> Stream<Item = AgentRunResponseMessage>` (exact signatures per implementation).
- [x] 1.3 Implement **Option A**: extend **`ChatCompletionResponseChunk`** with **`TextDone`** only; ensure **`Done`** follows all **`ToolCall`** chunks for the round (no **`ToolCallRequestsDone`**); update mocks and adapters to synthesize **`TextDone`** as needed.
- [x] 1.4 Refactor internal `model_loop_stream` / tool paths to emit `AgentRunResponseMessage` (map `ChatCompletionResponseChunk` into `ChatResponseChunk`).

## 2. Replay and session

- [x] 2.1 Adapt `replay` helpers to reconstruct run input from persisted data using `AgentRunResponseMessage` / `SessionReplayItem` only (`AgentInvokeEvent` removed).
- [x] 2.2 Update `Session::run` / `Session::resume` to build message history, call `Agent::run`, and forward `SessionEvent::AgentEvent` with `AgentRunResponseMessage`.
- [x] 2.3 Preserve recovery guarantees (authorization suspension, `!prefill_supported` partial-turn policy) with tests updated to new types.

## 3. Demos and persistence

- [x] 3.1 Update `demo` (`chat_view`, examples, `transcript`) to consume `AgentRunResponseMessage`; centralize `run`/`resume` handling per `moray-demos` spec.
- [x] 3.2 Define JSONL / transcript migration or version field if on-disk format changes; document for callers.

## 4. Validation

- [x] 4.1 Update `demo/tests/*.rs` and `moray-core` unit tests for new streams.
- [x] 4.2 `cargo test -p moray-core` and `cargo test -p demo` (and `--features openai` if CI includes it).
- [x] 4.3 `cargo clippy` on touched crates with workspace standards.
