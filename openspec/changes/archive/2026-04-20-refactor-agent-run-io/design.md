## Context

Today `AgentInvokeEvent` is both the **resume/run transcript line type** and the **`Agent::run`** I/O stream type. `replay` reconstructs `ChatCompletionRequestMessage` vectors from `AgentInvokeEvent` sequences. The goal is to make **inputs** identical to chat-completion **request** messages and **outputs** follow **completion chunk** semantics (`ChatCompletionResponseChunk`) where the model speaks, plus narrow variants for authorization and tool execution intervals.

## Goals / Non-Goals

- Goals: Reduce conceptual overhead; align agent I/O with `ChatCompletion` request/response types; simplify persistence that mirrors “what we send to the model” vs “what we streamed back.”
- Non-Goals: Vendor-specific **request** message shapes beyond what `ChatCompletionRequestMessage` already carries.

## Decisions

- **Decision: Output uses `ChatResponseChunk { chunk: ChatCompletionResponseChunk }` for model-emitted content.** Assistant text deltas, finalized tool calls from the model round, phase-boundary markers (see below), and the round `Done` chunk are carried inside the same enum used by `ChatCompletion`, instead of separate `AssistantTextBlock` / `AssistantToolCallRequest*` variants on the agent boundary.
- **Decision: Input is `&[ChatCompletionRequestMessage]`.** Resume and “new user turn” orchestration build this slice (including tail/tool state) using adapted replay logic; session remains responsible for persistence and calling `run` with the correct reconstructed history. The **last user text** for a new turn is taken from this slice (typically the trailing **`User`** message); the stream does not repeat it on **`InvokeStarted`**.
- **Decision: `InvokeStarted` is a lifecycle marker only** (no user payload). Callers rely on **`messages`** for input text.
- **Decision: `ToolCallAuthorization` remains an input path** via `reply_tool_auth` (not part of the outward `AgentRunResponseMessage` list), consistent with today’s split between stream observation and harness decisions.
- **Decision: Transcript / demo JSONL schema id** — no change required for this refactor; there is no shipped external version yet, so compatibility shims are out of scope for now.

### Text phase boundary vs tool-call listing (`AssistantTextDone`, `AssistantToolCallRequestsDone`)

Legacy **`AssistantTextDone`** marks **UX-relevant** “assistant text streaming has ended” (flush buffers before tool calls). Legacy **`AssistantToolCallRequestsDone`** is **not** reified as a separate chunk: **`ChatCompletionResponseChunk::Done`** already ends the completion round and, when the round includes **`ToolCall`** items, **MUST** be emitted only after **all** finalized **`ToolCall`** chunks for that round—so “all model tool calls are known” is implied at **`Done`** (then authorization / agent execution follow outside the completion stream).

**Decision — Option A (chosen): extend `ChatCompletionResponseChunk` with `TextDone` only:**

- Add **`TextDone`**: emitted after the last **`TextBlock`** for the round and **before** the first **`ToolCall`** when both text and tools exist. Rules for text-only rounds (e.g. **`TextDone`** immediately before **`Done`**, or omitted—implementation-defined) SHALL be documented in `nova-core`.

Adapters that do not receive an explicit wire **`TextDone`** **MUST** synthesize it at the correct position so the agent can forward it as **`ChatResponseChunk`** without a second parallel taxonomy.

## Risks / Trade-offs

- **Breaking API and in-repo transcripts** → Consumers replace `AgentInvokeEvent` matching with `AgentRunResponseMessage` / new chunks; no separate schema id migration until a public release policy requires it.
- **Chunk-level persistence size** vs old text-only events → Callers may coalesce or filter `ChatResponseChunk` for storage; spec allows implementation-level guidance in demo/transcript helpers.

## Migration Plan

1. Introduce `AgentRunResponseMessage` and switch `Agent::run` / `Session` streams.
2. Port `replay` / `resume` to derive state from message history + (where needed) tail metadata internal to core.
3. Update `demo` tests and transcript tooling; add compatibility shims or migration for saved JSONL if required by product.

## Open Questions

- Exact **serde** tagging for `AgentRunResponseMessage` and **`TextDone`** on **`ChatCompletionResponseChunk`**.
- Precise rules for when **`TextDone`** is emitted for rounds with **only** text (always immediately before **`Done`**, or omitted).
