# nova-core Specification

## Purpose

Defines the **`nova-core`** crate: OpenAI-style **chat completion** trait, **toolbox** trait, ReAct **agent** with recoverable **domain events**, and **replay** helpers used by **`Agent::run`**.

## Requirements

### Requirement: Multi-crate workspace layout for nova agent core

The repository SHALL provide a Rust workspace whose members include **`nova-core`** and **`nova-extensions`**. **`nova-core` MUST NOT** depend on application or integration crates.

#### Scenario: Core builds standalone

- **WHEN** building the **`nova-core`** crate alone
- **THEN** the build MUST succeed without depending on any other workspace crate

### Requirement: Chat completion abstraction for the agent loop

`nova-core` SHALL define the **`ChatCompletion`** trait abstracting **OpenAI chat-completion-compatible** behavior used by the ReAct loop: streaming assistant output and tool invocations shaped so that a single mock can stand in for multiple vendors without changing the loop.

The trait SHALL expose capability information needed for recovery via **`ChatCompletionCapabilities`** (**`prefill_supported`**: assistant-message prefill or equivalent "continue final assistant message" semantics).

Streaming chunks SHALL use **`ChatCompletionResponseChunk`**: incremental **`TextBlock`** strings, finalized **`ToolCall`** items (**`ToolCallRequest`**), then exactly one **`Done`** with **`ChatCompletionFinishReason`**.

LLM interactions that are **not** chat completions (for example image generation protocols) are out of scope for this trait; integrating them MUST NOT expand this trait's surface—adapters SHOULD bridge such results into chat messages or tool outcomes instead.

#### Scenario: Capability is queryable without network

- **WHEN** a harness constructs a mock **`ChatCompletion`** implementation
- **THEN** the harness MUST be able to read **`capabilities().prefill_supported`** without performing HTTP calls

### Requirement: Toolbox abstraction

`nova-core` SHALL define a **`Toolbox`** type that supports listing invokable tools and executing a tool call. The toolbox SHALL hold an `Arc<dyn ToolCallAuthPolicy + Send + Sync>` bound at construction time and SHALL consult it on every `call_tool` invocation via `decide`.

The toolbox's execution entry point SHALL accept `(call_id, name, arguments)` and SHALL return an async `Stream<Item = ToolboxEvent>`. The stream MUST internally:

- emit `Requested { call_id, name, arguments }` first, unconditionally,
- await `policy.decide(call_id, name, arguments)`,
- on `AuthDecision::Allow`: emit `Started`, then dispatch to the matching `Tool`, then emit `Finished { content = <tool output> }`,
- on `AuthDecision::Deny`: emit a single `Finished { content = TOOL_CALL_DENIED_BY_USER }` without invoking the underlying `Tool`,
- on `AuthDecision::AskUser { data }`: allocate a fresh `oneshot::channel`, store the sender in the toolbox's pending-authorization map keyed by `call_id`, emit `RequestingPermission { call_id, data }` (forwarding `data` verbatim), await the receiver, then proceed with the same allowed/denied branching as above.

The toolbox SHALL expose `async fn reply_toolcall_permission(&self, call_id: &str, data: serde_json::Value) -> Result<(), NovaError>` that pops the pending sender for `call_id`, delegates payload decoding to `policy.reply(call_id, data).await`, and resolves the `oneshot` with the returned boolean.

#### Scenario: Denial produces a synthetic tool result without invoking the tool

- **WHEN** the effective authorization decision is denial for `call_id = X`
- **THEN** the toolbox lifecycle stream MUST yield `Requested { call_id = X, name, arguments }` followed by exactly one `Finished { content = TOOL_CALL_DENIED_BY_USER }` for `call_id = X` without invoking the underlying `Tool::call`

### Requirement: Dynamic polymorphism via trait objects (no static generics on Agent)

`nova-core` SHALL expose **`Agent::new`** taking **`Arc<dyn ChatCompletion + Send + Sync>`** and **`Arc<dyn Toolbox + Send + Sync>`**. Harnesses MUST be able to select **`ChatCompletion`** and **`Toolbox`** implementations at runtime using these trait objects. The **`Agent`** type SHALL NOT be parameterized by concrete model or toolbox types.

#### Scenario: Agent runs with trait-object model and toolbox

- **WHEN** a test constructs an **`Agent`** with **`Arc<dyn ChatCompletion + Send + Sync>`** and **`Arc<dyn Toolbox + Send + Sync>`** built from mock types
- **THEN** **`Agent::run`** MUST complete successfully and emit the expected **`AgentRunResponseMessage`** items

### Requirement: ReAct loop with explicit state machine

`nova-core` SHALL implement a ReAct-style agent loop as an explicit state machine driven by user input, chat completion stream chunks, and toolbox lifecycle events, emitting an ordered sequence of **`AgentRunResponseMessage`** on the **`Agent::run`** output stream. Authorization decisions are **not** handled by the agent; they are gated inside the toolbox via the toolbox-owned `ToolCallAuthPolicy`. The agent MUST NOT import `ToolCallAuthPolicy` and MUST NOT hold any pending-authorization state.

Each agent turn SHALL end with **`Finished { kind }`**, where **`kind`** is **`AgentInvokeFinishKind`**: **`succeeded`**, **`canceled`**, **`refused { reason?: … }`**, or **`failed { reason: … }`**.

For each tool call after the model round's **`Done`**, the loop SHALL drive the stream produced by `Toolbox::call_tool(call_id, name, arguments)` and forward its lifecycle events verbatim, wrapped in a single agent variant: every `ToolboxEvent` value (`Requested` / `RequestingPermission` / `Started` / `Finished`) becomes `AgentRunResponseMessage::ToolCall { event }` at the agent boundary.

#### Scenario: Agent forwards toolbox lifecycle verbatim

- **WHEN** `Toolbox::call_tool` yields `Requested` then `RequestingPermission` then `Started` then `Finished { content }` for `call_id = X`
- **THEN** the agent MUST emit `ToolCall { event: Requested }`, `ToolCall { event: RequestingPermission }`, `ToolCall { event: Started }`, then `ToolCall { event: Finished }` in that order

### Requirement: Agent run input and output align with chat completion messages and chunks

`nova-core` SHALL expose **`Agent::run`** such that:

- **Input** is an ordered slice of **`ChatCompletionRequestMessage`**.
- **Output** is a stream of **`AgentRunResponseMessage`**, an enum with exactly three variants:
  - **`ChatResponse { chunk: ChatCompletionResponseChunk }`** — forwards model streaming.
  - **`ToolCall { event: ToolboxEvent }`** — forwards the full toolbox lifecycle stream verbatim.
  - **`Finished { kind: AgentInvokeFinishKind }`** — terminal marker for the current turn.

#### Scenario: AgentRunResponseMessage enum has the expected variants

- **WHEN** inspecting the `AgentRunResponseMessage` type
- **THEN** it MUST contain exactly: `ChatResponse`, `ToolCall`, `Finished`

### Requirement: `TextDone` chunk for assistant text phase end

**`ChatCompletionResponseChunk`** SHALL include **`TextDone`**, emitted after the last **`TextBlock`** and before the first **`ToolCall`** when a round contains both. Adapters **MUST** synthesize **`TextDone`** when the vendor stream does not provide an explicit equivalent.

#### Scenario: Consumers can detect end of text streaming before tools

- **WHEN** a model round emits assistant text and then tool calls
- **THEN** the stream MUST include **`TextDone`** after the last **`TextBlock`** and before the first **`ToolCall`**

### Requirement: Tool call authorization policy abstraction

`nova-core` SHALL define a **`ToolCallAuthPolicy`** trait that gates tool execution independently of the agent loop. The trait SHALL expose exactly two methods:

- `async fn decide(&self, call_id: &str, tool_name: &str, arguments: &str) -> AuthDecision` — returns a pure decision (allow / deny / ask-user with optional `data`).
- `async fn reply(&self, call_id: &str, data: serde_json::Value) -> bool` — interprets the UI's raw reply payload into a final allow / deny decision.

`AuthDecision` SHALL have exactly three variants: `Allow`, `Deny`, `AskUser { data: Option<serde_json::Value> }`.

`nova-core` SHALL NOT ship a concrete `ToolCallAuthPolicy` implementation. Consumers MUST provide their own.

#### Scenario: AskUser surfaces exactly one permission event with forwarded data

- **WHEN** a policy returns `AuthDecision::AskUser { data }` for a fresh `call_id`
- **THEN** the toolbox lifecycle stream MUST emit exactly one `RequestingPermission { call_id, data }` item with `data` byte-for-byte identical to the `data` inside `AuthDecision::AskUser`

### Requirement: Tool call lifecycle stream from the toolbox

`Toolbox` SHALL expose a `call_tool(call_id, name, arguments)` entry point returning an async `Stream<Item = ToolboxEvent>`, where `ToolboxEvent` SHALL have exactly four variants:

- `Requested { call_id, name, arguments }` — emitted unconditionally as the first item of every invocation, before the policy is consulted.
- `RequestingPermission { call_id, data }` — emitted only when the policy returns `AskUser`.
- `Started { call_id }` — emitted after authorization resolves to allow, immediately before `Tool::call`.
- `Finished { call_id, content }` — the terminal item, always emitted exactly once per invocation.

Ordering rules:

- **Allowed via `Allow`**: `Requested` → `Started` → `Finished { content }`.
- **Allowed via `AskUser` + reply `true`**: `Requested` → `RequestingPermission` → `Started` → `Finished`.
- **Denied via `Deny`**: `Requested` → `Finished { TOOL_CALL_DENIED_BY_USER }`.
- **Denied via `AskUser` + reply `false`**: `Requested` → `RequestingPermission` → `Finished { TOOL_CALL_DENIED_BY_USER }`.
- **Tool error path**: the stream MUST still terminate with a `Finished { content }` item.

#### Scenario: Lifecycle stream terminates with exactly one Finished

- **WHEN** `Toolbox::call_tool` is driven to completion
- **THEN** the produced stream MUST terminate with exactly one `Finished { content }` item as its final value

### Requirement: Toolbox owns the policy and pending-authorization state

`Toolbox` SHALL own both its `ToolCallAuthPolicy` and the pending-authorization state, and SHALL bind the policy at construction time. The toolbox SHALL internally maintain a pending-authorization map `HashMap<String, oneshot::Sender<bool>>`.

`Toolbox` SHALL expose `async fn reply_toolcall_permission(&self, call_id: &str, data: serde_json::Value) -> Result<(), NovaError>` that pops the sender for `call_id` (returning an error if not present), invokes `policy.reply(call_id, data).await`, and forwards the boolean on the `oneshot`.

#### Scenario: Unknown call_id reply returns an error without consulting the policy

- **WHEN** `Toolbox::reply_toolcall_permission("unknown", value)` is invoked with no pending request for that `call_id`
- **THEN** the call MUST return an error, MUST NOT invoke `policy.reply`, and MUST NOT silently ignore the reply
