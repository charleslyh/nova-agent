## ADDED Requirements

### Requirement: Agent run input and output align with chat completion messages and chunks

`nova-core` SHALL expose **`Agent::run`** such that:

- **Input** is an ordered slice of **`ChatCompletionRequestMessage`** (the same structure used for **`ChatCompletion::completion`**), not **`AgentInvokeEvent`**.
- **Output** is a stream of **`AgentRunResponseMessage`**, an enum that SHALL include at minimum:
  - **`ChatResponseChunk { chunk: ChatCompletionResponseChunk }`** — forwards model streaming using the same chunk type as **`ChatCompletion`** (including **`TextBlock`**, **`TextDone`**, **`ToolCall`**, and **`Done`** in canonical order per **`ChatCompletionResponseChunk`** and `design.md`).
  - **`AssistantToolCallAuthorizationRequired { call_id }`**
  - **`ToolCallStarted { call_id }`**
  - **`ToolCallFinished { call_id, content }`** — JSON serialization tagged **`tool_call_finished`** at this boundary when serde is enabled.
  - **`InvokeFinished { kind: AgentInvokeFinishKind }`**

Harness input for allow/deny decisions remains **`reply_tool_auth`** and is **not** required to appear as an **`AgentRunResponseMessage`** variant.

#### Scenario: Model round streaming maps to completion chunks

- **WHEN** **`Agent::run`** executes a model round with streaming enabled
- **THEN** assistant text and tool calls from the model MUST be observable through **`ChatResponseChunk`** items that preserve **`ChatCompletionResponseChunk`** ordering rules, ending with **`Done`** for that round before tool execution proceeds

#### Scenario: Tool execution bracketing is unchanged semantically

- **WHEN** a tool is approved for execution after **`AssistantToolCallAuthorizationRequired`**
- **THEN** the implementation MUST emit **`ToolCallStarted`** immediately before awaiting **`Toolbox`** execution and **`ToolCallFinished`** afterward, matching prior ordering guarantees

### Requirement: `TextDone` chunk for assistant text phase end

**`ChatCompletionResponseChunk`** SHALL include **`TextDone`**, emitted after the last **`TextBlock`** and before the first **`ToolCall`** when a round contains both. Adapters **MUST** synthesize **`TextDone`** when the vendor stream does not provide an explicit equivalent, at the position required by **`nova-core`** documentation.

**`AssistantToolCallRequestsDone`** is **not** modeled as a separate chunk: **`Done`** terminates the completion round; when that round includes **`ToolCall`** chunks, **`Done`** MUST occur only after **all** such **`ToolCall`** chunks for the round, so consumers treat **`Done`** as “model tool calls for this completion are fully listed” before downstream authorization.

`nova-core` SHALL document ordering rules (including text-only rounds) so UI layers can flush streamed text without guessing from **`TextBlock`** / **`ToolCall`** heuristics alone.

#### Scenario: Consumers can detect end of text streaming before tools

- **WHEN** a model round emits assistant text and then tool calls
- **THEN** the stream MUST include **`TextDone`** after the last **`TextBlock`** and before the first **`ToolCall`**

#### Scenario: Done finalizes tool calls for the round

- **WHEN** a model round includes one or more **`ToolCall`** chunks
- **THEN** **`Done`** MUST be emitted only after every **`ToolCall`** for that completion round has been streamed, so consumers can align authorization with the prior **`AssistantToolCallRequestsDone`** “batch complete” moment without an extra chunk variant

## MODIFIED Requirements

### Requirement: Dynamic polymorphism via trait objects (no static generics on Agent)

`nova-core` SHALL expose **`Agent::new`** taking **`Arc<dyn ChatCompletion + Send + Sync>`** and **`Arc<dyn Toolbox + Send + Sync>`**. Harnesses MUST be able to select **`ChatCompletion`** and **`Toolbox`** implementations at runtime using these trait objects. The **`Agent`** type SHALL NOT be parameterized by concrete model or toolbox types (no **`Agent<M, T>`**-style static polymorphism for those boundaries).

Forwarding **`impl ChatCompletion for Box<dyn …>`** / **`Arc<dyn …>`** layers is **not** required; coercions from concrete types (for example **`Arc::new(EchoChat)`** into **`Arc<dyn ChatCompletion + Send + Sync>`**) are sufficient.

#### Scenario: Agent runs with trait-object model and toolbox

- **WHEN** a test constructs an **`Agent`** with **`Arc<dyn ChatCompletion + Send + Sync>`** and **`Arc<dyn Toolbox + Send + Sync>`** built from mock types
- **THEN** **`Agent::run`** MUST complete successfully and emit the expected **`AgentRunResponseMessage`** items

### Requirement: ReAct loop with explicit state machine

`nova-core` SHALL implement a ReAct-style agent loop as an explicit state machine driven by user input, chat completion stream chunks, toolbox results, and authorization decisions, emitting an ordered sequence of **`AgentRunResponseMessage`** on the **`Agent::run`** output stream.

Each agent turn (started via **`Agent::run`** or continued via **`Session::resume`**) SHALL end with **`InvokeFinished { kind }`**, where **`kind`** is **`AgentInvokeFinishKind`**: **`succeeded`**, **`canceled`**, **`refused { reason?: … }`**, or **`failed { reason: … }`** (payloads live on the enum variants, not as a separate sibling field).

Model output from the completion adapter SHALL be represented with **`ChatResponseChunk`** carrying **`ChatCompletionResponseChunk`** (including **`TextBlock`**, **`TextDone`**, **`ToolCall`**, **`Done`**) rather than separate assistant-text-only and assistant-tool-request event variants at this public boundary.

For each tool call, after a positive authorization decision the loop SHALL emit **`ToolCallStarted`** immediately before awaiting **`Toolbox`** execution, then **`ToolCallFinished`** with the tool-role payload (including synthetic text when the user denies). **`ToolCallFinished`** JSON serialization SHALL use **`tool_call_finished`** as the tagged `type` value.

When multiple tools from one model round require authorization, **`reply_tool_auth`** SHALL accept decisions in **any order** keyed by **`call_id`** (after the harness has emitted the corresponding **`AssistantToolCallAuthorizationRequired`** events).

#### Scenario: Tool call reaches authorization gate

- **WHEN** the completion stream yields a tool call that requires external authorization
- **THEN** the state machine MUST transition to an awaiting-decision state and emit an event that carries sufficient data for a harness to prompt and persist a decision

### Requirement: Run and resume share the same outward event stream shape

The public API for starting a new user turn (**`Session::run`** / **`Agent::run`**) and for resuming from persisted history (**`Session::resume`**) SHALL expose the same agent stream item type (**`AgentRunResponseMessage`**) and the same high-level streaming contract, so a single consumer can process both paths.

#### Scenario: Same stream consumer for run and resume

- **WHEN** the CLI handles an unfinished section and later handles a REPL turn
- **THEN** both paths MUST be able to reuse the same stream-processing logic (for example one **`run_*` helper** in **`demo`**) so **`run`** and **`resume`** are consumed consistently without separate incompatible match arms

### Requirement: Replay and partial assistant text without prefill

**`replay_events`** / **`resume_has_work`** SHALL encode recovery policy for **`!prefill_supported`**:

- **Mid-history new user while assistant text was still pending**: partial assistant output for the interrupted turn MUST NOT be committed into **`ChatCompletionRequestMessage`** history; **`ReplaySnapshot.needs_fresh_completion`** MUST be set so **`resume`** can run a fresh completion round from the prior stable context.
- **End of persisted history** with pending assistant output or tools: implementation flushes pending assistant into **`messages`** (see **`replay.rs`**), so a record that ends mid-stream may still materialize a partial assistant **`Assistant`** message—callers persisting streams SHOULD pair **`ChatResponseChunk`** streaming with completion **`Done`** and **`InvokeFinished`** (or equivalent commitment) if they need stricter “discard trailing partial” semantics on disk.

#### Scenario: Replay unit test covers aborted partial turn

- **WHEN** persisted history reflects one user turn with incomplete assistant streaming, then begins another user turn, without **`prefill_supported`**
- **THEN** **`replay_events`** MUST set **`needs_fresh_completion`** and MUST NOT retain the partial assistant text in **`messages`** for the aborted turn

### Requirement: Recovery from persisted events after external suspension

If execution stops while awaiting tool-call authorization (or another harness-controlled suspension encoded in persisted history), the agent MUST be able to rebuild deterministic state from persisted data and continue execution when resumed.

#### Scenario: Transcript ends at authorization gate

- **WHEN** the persisted history contains a tool-call round (including model **`ToolCall`** / **`Done`** chunks) and a subsequent authorization outcome is not yet persisted
- **THEN** after restart, **`resume`** MUST reproduce the same post-request state and, once the harness supplies the decision, continue without re-requesting chat completion for the same tool call unless policy requires it

### Requirement: Session outward stream uses SessionEvent with agent passthrough

`nova-core` SHALL expose `SessionEvent` as the outward event stream contract for session operations.

`SessionEvent` MUST include:
- session lifecycle events for orchestration boundaries and failures,
- `AgentEvent { event: AgentRunResponseMessage }` as the dedicated passthrough variant for agent invoke/resume stream items.

The session layer MUST NOT redefine a second set of invoke/resume semantics equivalent to `AgentRunResponseMessage`.

#### Scenario: Invoke/resume events are forwarded without semantic duplication

- **WHEN** an underlying `Agent` emits `AgentRunResponseMessage` items during `Session::run` or `Session::resume`
- **THEN** `Session` MUST publish those events via `SessionEvent::AgentEvent` while preserving the original event payload semantics

#### Scenario: Session lifecycle and agent events are distinguishable

- **WHEN** a consumer subscribes to `Session` output
- **THEN** it MUST be able to distinguish lifecycle transitions (such as started/resumed/reset/failed/finished) from forwarded `AgentRunResponseMessage` items using `SessionEvent` variants
