## Why

`Agent::run` currently takes and returns `AgentInvokeEvent`, mixing **chat-completion request shape** (what the model sees), **streaming response shape** (text deltas vs chunks), and **tool/runtime control** in one enum. Callers must mentally map between `AgentInvokeEvent` and `ChatCompletionRequestMessage` / `ChatCompletionResponseChunk`, which raises the cost of understanding, testing, and persisting transcripts.

## What Changes

- **BREAKING**: `Agent::run` input becomes an ordered `&[ChatCompletionRequestMessage]` (the same structure passed to `ChatCompletion::completion`), instead of `&[AgentInvokeEvent]`.
- **BREAKING**: `Agent::run` output stream item type becomes a new **`AgentRunResponseMessage`** enum whose variants include at minimum: `InvokeStarted` (lifecycle only — **no** user text; input comes from **`messages`**), `ChatResponseChunk { chunk: ChatCompletionResponseChunk }`, `AssistantToolCallAuthorizationRequired`, `ToolCallStarted`, `ToolCallFinished`, `InvokeFinished` — aligning outward **output** with completion streaming chunks plus explicit agent lifecycle edges.
- **BREAKING**: **`ChatCompletionResponseChunk`** gains **`TextDone`** (Option A) to replace **`AssistantTextDone`**-style “text streaming ended” signaling. **`AssistantToolCallRequestsDone`** is **not** duplicated: **`Done`** already closes the model round and, when **`ToolCall`** chunks are present, implies all tool calls for that round are finalized before **`Done`**; see `design.md`.
- **BREAKING**: `Session` / outward session streams that passthrough agent items update from `AgentInvokeEvent` to `AgentRunResponseMessage` (exact field names follow implementation).
- Replay, resume, demo integration tests, and JSONL transcript helpers are updated to the new split between **message history** (model input) and **response stream** (model-shaped output + tool edges). **No** demo transcript schema id change is required while there is no external release baseline.

## Impact

- Affected specs: `moray-core`, `moray-demos`
- Affected code (indicative): `core/src/agent.rs`, `core/src/replay.rs`, `core/src/session.rs`, `core/src/lib.rs`, `demo/` (examples, `chat_view`, transcript, tests)
