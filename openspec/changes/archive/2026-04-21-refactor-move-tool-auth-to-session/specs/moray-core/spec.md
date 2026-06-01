## ADDED Requirements

### Requirement: Tool call authorization policy abstraction

`moray-core` SHALL define a **`ToolCallAuthPolicy`** trait that gates tool execution independently of the agent loop. The trait SHALL expose **exactly two** methods:

- `async fn decide(&self, call_id: &str, tool_name: &str, arguments: &str) -> AuthDecision` — returns a pure decision describing whether the tool call is allowed, denied, or requires user confirmation (optionally attaching an opaque `data` payload forwarded to the UI).
- `async fn reply(&self, call_id: &str, data: serde_json::Value) -> bool` — interprets the UI's raw reply payload into a final allow (`true`) / deny (`false`) decision. The short name is unambiguous in context because the containing type (`ToolCallAuthPolicy`) already carries the "tool-call authorization" domain; implementations MAY cache or persist the outcome (for example "always allow this tool" or "remember this choice for the session") before returning.

`AuthDecision` SHALL have exactly three variants:

- `Allow` — execute the tool immediately; no `RequestingPermission` event.
- `Deny` — refuse immediately; the toolbox MUST emit a single `Finished` with `TOOL_CALL_DENIED_BY_USER`.
- `AskUser { data: Option<serde_json::Value> }` — stall execution; the toolbox MUST emit `RequestingPermission { call_id, data }` (forwarding `data` verbatim) and await resolution via `Toolbox::reply_toolcall_permission`.

Because `AskUser` carries `Option<serde_json::Value>` and `Value` implements only `PartialEq`, `AuthDecision` MUST derive `PartialEq` but MUST NOT derive `Eq`.

The trait MUST NOT expose any callback registration or channel factory. Pending-authorization state (the `oneshot` map keyed by `call_id`) lives inside `Toolbox`, not inside the policy. The policy only sees decoded payloads through `reply`; it never sees the `oneshot`. The trait MUST NOT produce tool-result payloads.

`moray-core` SHALL NOT ship a concrete `ToolCallAuthPolicy` implementation. Consumers (demo REPLs, production harnesses, persistence-backed policies, test doubles) MUST provide their own.

#### Scenario: Allow short-circuits without surfacing

- **WHEN** a policy returns `AuthDecision::Allow` for a `call_id`
- **THEN** the toolbox MUST NOT emit `ToolboxEvent::RequestingPermission`, the agent MUST NOT emit `AgentRunResponseMessage::ToolCall { event: RequestingPermission { .. } }` for that `call_id`, and the tool MUST be invoked immediately (after the unconditional `Requested` event)

#### Scenario: Deny short-circuits without invoking the tool

- **WHEN** a policy returns `AuthDecision::Deny` for a `call_id`
- **THEN** the toolbox MUST NOT emit `RequestingPermission`, MUST NOT emit `Started`, MUST NOT invoke the underlying `Tool::call`, and MUST emit `Requested` followed by exactly one `Finished { content = TOOL_CALL_DENIED_BY_USER }`

#### Scenario: AskUser surfaces exactly one permission event with forwarded data

- **WHEN** a policy returns `AuthDecision::AskUser { data }` for a fresh `call_id`
- **THEN** the toolbox lifecycle stream MUST emit exactly one `RequestingPermission { call_id, data }` item (with `data` byte-for-byte identical to the `data` inside `AuthDecision::AskUser`) and the outward `SessionEvent` stream MUST surface it verbatim as exactly one `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, data } } }`

#### Scenario: Policy interprets the reply payload

- **WHEN** the toolbox receives `Toolbox::reply_toolcall_permission(call_id, raw_value)` for a pending `call_id`
- **THEN** it MUST invoke `policy.reply(call_id, raw_value).await` and forward the returned boolean on the pending `oneshot`; the policy MAY cache or persist the decision before returning, and the toolbox MUST NOT inspect the payload's shape itself

### Requirement: Tool call lifecycle stream from the toolbox

`Toolbox` SHALL expose a `call_tool(call_id, name, arguments)` entry point whose return type is an async `Stream<Item = ToolboxEvent>`, where `ToolboxEvent` SHALL have exactly four variants:

- `Requested { call_id: String, name: String, arguments: String }` — emitted **unconditionally** as the first item of every `call_tool` invocation, **before** the policy is consulted. Announces that the toolbox has accepted the call and is about to gate it, and carries the full `(call_id, name, arguments)` triple so UI / replay consumers can render the tool invocation (prompt, pending row, icon, etc.) without having to correlate with the preceding `ChatCompletionResponseChunk::ToolCall` chunk.
- `RequestingPermission { call_id: String, data: Option<serde_json::Value> }` — emitted **only** when the policy returns `AuthDecision::AskUser { data }`; `data` is forwarded verbatim from the policy's decision. Emitted at most once per `call_tool` invocation and always after `Requested` and before `Started` / `Finished`. The event carries only `call_id` + `data`; consumers that need tool name / arguments (e.g. UI prompts) MUST correlate with the preceding `Requested { call_id, name, arguments }` event for the same `call_id`. The policy itself receives full context via `ToolCallAuthPolicy::decide(call_id, tool_name, arguments)`.
- `Started { call_id: String }` — emitted after the authorization decision (whether immediate or user-provided) resolves to allow, immediately before the underlying `Tool::call` is awaited. Denied calls never emit `Started`.
- `Finished { call_id: String, content: String }` — the terminal item, always emitted exactly once per invocation.

Because `RequestingPermission` nests `Option<serde_json::Value>` and `Value` implements only `PartialEq`, `ToolboxEvent` MUST derive `PartialEq` but MUST NOT derive `Eq`.

Ordering rules:

- **Allowed via `Allow`**: `Requested` → `Started` → `Finished { content = <tool output> }`. No `RequestingPermission`.
- **Allowed via `AskUser` + `reply_toolcall_permission(call_id, value)` where the policy decodes to `true`**: `Requested` → `RequestingPermission { data }` → `Started` → `Finished { content = <tool output> }`.
- **Denied via `Deny`**: `Requested` → `Finished { content = TOOL_CALL_DENIED_BY_USER }`. No `RequestingPermission`, no `Started`.
- **Denied via `AskUser` + `reply_toolcall_permission(call_id, value)` where the policy decodes to `false`**: `Requested` → `RequestingPermission { data }` → `Finished { content = TOOL_CALL_DENIED_BY_USER }`. No `Started`.
- **Tool error path**: the stream MUST still terminate with a `Finished { content }` item (error information mapped to a string payload) so downstream message-history construction remains deterministic.

The `TOOL_CALL_DENIED_BY_USER` constant SHALL live in the toolbox module. Agent code MUST treat the `content` returned by `Finished` as opaque.

#### Scenario: Lifecycle stream terminates with exactly one Finished

- **WHEN** `Toolbox::call_tool` is driven to completion (for any authorization outcome or tool outcome)
- **THEN** the produced stream MUST terminate with exactly one `Finished { content }` item as its final value

#### Scenario: Requested is always the first event and carries the full invocation signature

- **WHEN** `Toolbox::call_tool(call_id, name, arguments)` is invoked for any policy outcome (`Allow`, `Deny`, or `AskUser`)
- **THEN** the first item yielded on the returned stream MUST be `Requested { call_id, name, arguments }` with `name` / `arguments` byte-for-byte identical to the values passed into `call_tool`, and it MUST be emitted before the toolbox consults the policy

#### Scenario: Allow and Deny suppress RequestingPermission

- **WHEN** the policy returns `AuthDecision::Allow` or `AuthDecision::Deny` for a `call_id`
- **THEN** the stream MUST NOT contain a `RequestingPermission` item for that `call_id`

#### Scenario: Denial suppresses Started

- **WHEN** the effective authorization decision is denial (either `Deny` directly or `AskUser` followed by `reply_toolcall_permission` whose policy decode returned `false`)
- **THEN** the produced stream MUST NOT contain a `Started` item; its only terminal item MUST be `Finished { content = TOOL_CALL_DENIED_BY_USER }`

### Requirement: Toolbox owns the policy and pending-authorization state

`Toolbox` SHALL own both its `ToolCallAuthPolicy` and the pending-authorization state, and SHALL bind the policy at construction time:

```rust
impl Toolbox {
    pub fn new(
        tools: Vec<Arc<dyn Tool + Send + Sync>>,
        policy: Arc<dyn ToolCallAuthPolicy + Send + Sync>,
    ) -> Self;
}
```

(`ToolboxBuilder::build(policy)` mirrors the same shape.)

The toolbox SHALL internally maintain a pending-authorization map `HashMap<String /* call_id */, oneshot::Sender<bool>>`. On every `AuthDecision::AskUser { data }` it MUST:

1. allocate a new `oneshot::channel()`,
2. insert the sender into the pending map keyed by `call_id` **before** yielding `RequestingPermission { call_id, data }`,
3. await the receiver to obtain the eventual decision.

`Toolbox` SHALL expose a public `async fn reply_toolcall_permission(&self, call_id: &str, data: serde_json::Value) -> Result<(), MorayError>` method that resolves a pending `AskUser` request. It MUST:

1. pop the sender from the internal map using `call_id` (returning an error if not present, without consulting the policy),
2. invoke `policy.reply(call_id, data).await` to decode the payload into a boolean,
3. forward that boolean on the `oneshot`.

Unknown `call_id`s MUST return an error (not silently succeed) and MUST NOT invoke the policy. Out-of-order replies across distinct `call_id`s MUST be supported.

This is the only path through which the session resolves user authorization replies. The session MUST NOT import or hold a reference to `ToolCallAuthPolicy`.

#### Scenario: Session resolves authorization via the toolbox

- **WHEN** `Session::reply_toolcall_permission(call_id, data)` is invoked
- **THEN** the call MUST be implemented as a delegation to `Toolbox::reply_toolcall_permission(call_id, data)` on the session-held toolbox; the session MUST NOT directly invoke any `ToolCallAuthPolicy` method

#### Scenario: Session does not import the policy trait

- **WHEN** inspecting `core/src/session.rs`
- **THEN** it MUST NOT contain any `use … ToolCallAuthPolicy` and MUST NOT hold any field whose type mentions `ToolCallAuthPolicy`

#### Scenario: Unknown call_id reply returns an error without consulting the policy

- **WHEN** `Toolbox::reply_toolcall_permission("unknown", value)` is invoked with no pending request for that `call_id`
- **THEN** the call MUST return an error, MUST NOT invoke `policy.reply`, and MUST NOT silently ignore the reply

#### Scenario: Out-of-order replies resolve the correct pending requests

- **WHEN** two concurrent `call_tool` invocations (`call_id = A` and `call_id = B`) both yield `RequestingPermission`, and `Toolbox::reply_toolcall_permission("B", Value::Bool(true))` is invoked before `Toolbox::reply_toolcall_permission("A", Value::Bool(false))`
- **THEN** the two pending `oneshot` receivers MUST resolve with their matching decisions (as decoded by `policy.reply`) and no request MUST be lost or cross-mapped

### Requirement: Session harness factory for per-session construction

`moray-core` SHALL define a **`SessionHarnessFactory`** trait with exactly two methods:

- `create_toolbox(&self) -> Arc<Toolbox>` — produces the session-scoped toolbox (with its policy already bound),
- `create_completion(&self) -> Arc<dyn ChatCompletion + Send + Sync>`.

`Session::new` SHALL invoke the factory during construction in the order: `create_toolbox`, then `create_completion`. `Session::new` SHALL construct the `Agent` itself via `Agent::new(completion, toolbox)`. The session SHALL retain only an `Arc<Toolbox>` (no `Arc<dyn ToolCallAuthPolicy>`).

The factory MUST NOT expose a `create_policy` method or any `ToolCallAuthPolicy`-typed return value. Policy construction is an internal concern of the factory implementer; it is bound into the `Toolbox` returned by `create_toolbox`.

Whether `create_toolbox` returns fresh instances or clones of shared state is an implementation concern of the factory, not of `Session`.

#### Scenario: Factory produces a coherent toolbox

- **WHEN** a consumer constructs `Session` with a `SessionHarnessFactory` implementation
- **THEN** the session MUST be able to run a turn where the toolbox internally consults its own embedded policy, and `Session::reply_toolcall_permission` resolves through `Toolbox::reply_toolcall_permission`

#### Scenario: Factory does not expose the policy

- **WHEN** inspecting `SessionHarnessFactory`
- **THEN** it MUST NOT contain any method whose return type mentions `ToolCallAuthPolicy`; the only authorization-related coupling MUST be inside the `Toolbox` returned by `create_toolbox`

#### Scenario: Agent is assembled by Session, not the factory

- **WHEN** inspecting `SessionHarnessFactory`
- **THEN** it MUST NOT expose an `Agent`-creation method; agent construction MUST happen inside `Session::new` via `Agent::new(completion, toolbox)`

#### Scenario: Session::new does not take a prebuilt Agent

- **WHEN** inspecting `Session::new`
- **THEN** its signature MUST NOT require a prebuilt `Agent` argument

### Requirement: Session forwards tool-call authorization events inline

`Session::run` and `Session::resume` SHALL implement their outward `Stream<Item = SessionEvent>` as a pure pass-through over the agent stream: every `AgentRunResponseMessage` item produced by the agent SHALL be wrapped verbatim as `SessionEventKind::AgentEvent { event }`. There SHALL be no `mpsc::UnboundedSender`, no `tokio::select!` fan-in, no callback registration on the policy, and no variant-specific promotion on the session boundary. Because `AgentRunResponseMessage::ToolCall` nests `ToolboxEvent` (which in turn nests `Option<serde_json::Value>`), `SessionEventKind` MUST derive `PartialEq` but MUST NOT derive `Eq`.

Outstanding authorization requests are represented by the already-nested shape:

```
SessionEventKind::AgentEvent {
    event: AgentRunResponseMessage::ToolCall {
        event: ToolboxEvent::RequestingPermission { call_id, data },
    },
}
```

The event carries only `call_id` + the policy's opaque `data`; UI consumers correlate with the preceding `AgentEvent(ToolCall { event: Requested { call_id, name, arguments } })` event (or, if they prefer, the earlier `AgentEvent(ChatResponse(ToolCall { call_id, name, arguments }))` chunk — both carry the same signature) to recover tool name / arguments for prompt rendering, and MAY additionally consume `data` (e.g. for cached-decision hints or prompt template selection). `SessionEventKind` SHALL NOT introduce a separate `ToolCallAuthorization` variant — that would be redundant with the nested shape and would force consumers to dispatch on two equivalent patterns.

For a single tool call `call_id = X` that requires external authorization, the outward order is:

```
AgentEvent(ChatResponse(ToolCall { call_id = X, ... }))
AgentEvent(ChatResponse(Done { ... }))
AgentEvent(ToolCall { event: Requested { call_id = X, name, arguments } })
AgentEvent(ToolCall { event: RequestingPermission { call_id = X, data } })
(user replies via Session::reply_toolcall_permission → Toolbox::reply_toolcall_permission
 → policy.reply → oneshot resolves inside toolbox)
AgentEvent(ToolCall { event: Started  { call_id = X } })
AgentEvent(ToolCall { event: Finished { call_id = X, content } })
```

When the policy short-circuits with `Allow`, `RequestingPermission` is absent; `Requested` / `Started` / `Finished` still appear in their causal order. When the policy short-circuits with `Deny`, `Requested` is followed directly by `Finished { TOOL_CALL_DENIED_BY_USER }` with neither `RequestingPermission` nor `Started`.

`Session::reply_toolcall_permission(call_id, data)` SHALL forward the decision to `Toolbox::reply_toolcall_permission`; the agent MUST NOT receive this call directly.

#### Scenario: Authorization surfaces via nested AgentEvent

- **WHEN** the model round yields a tool call and the toolbox's policy returns `AskUser`
- **THEN** the outward `SessionEvent` stream MUST produce exactly one `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, data } } }`, and `SessionEventKind` MUST NOT define any separate top-level `ToolCallAuthorization` variant

#### Scenario: Ordering around an externally-authorized tool call

- **WHEN** a turn executes a single tool call, the policy surfaces the request via `AskUser`, and the user replies with a payload the policy decodes to `true`
- **THEN** the outward `SessionEvent` order MUST be: `AgentEvent(ToolCall { event: Requested { call_id, name, arguments } })` → `AgentEvent(ToolCall { event: RequestingPermission { call_id, .. } })` → `AgentEvent(ToolCall { event: Started { call_id } })` → `AgentEvent(ToolCall { event: Finished { call_id } })`

#### Scenario: Denied call omits Started

- **WHEN** the user's reply to a surfaced `AgentEvent(ToolCall { event: RequestingPermission { call_id = X, .. } })` is decoded by the policy as `false`
- **THEN** the outward stream MUST NOT contain `AgentEvent(ToolCall { event: Started { call_id = X } })`; it MUST contain `AgentEvent(ToolCall { event: Finished { call_id = X, content = TOOL_CALL_DENIED_BY_USER } })`

#### Scenario: Short-circuited call omits RequestingPermission

- **WHEN** the policy returns `AuthDecision::Allow` for a tool call
- **THEN** the outward stream MUST NOT contain an `AgentEvent(ToolCall { event: RequestingPermission })` event for that `call_id`; it MUST still contain `AgentEvent(ToolCall { event: Requested { call_id, name, arguments } })`, `AgentEvent(ToolCall { event: Started })`, and `AgentEvent(ToolCall { event: Finished })` in causal order

#### Scenario: reply_toolcall_permission routes through the toolbox

- **WHEN** `Session::reply_toolcall_permission(call_id, data)` is invoked
- **THEN** the call MUST resolve via `Toolbox::reply_toolcall_permission` on the session-held toolbox; it MUST NOT call any agent-level API and MUST NOT directly import / dereference any `ToolCallAuthPolicy` value from the session module

## MODIFIED Requirements

### Requirement: Toolbox abstraction

`moray-core` SHALL define a **`Toolbox`** type that supports listing invokable tools and executing a tool call. The toolbox SHALL hold an `Arc<dyn ToolCallAuthPolicy + Send + Sync>` bound at construction time and SHALL consult it on every `call_tool` invocation via `decide`.

The toolbox's execution entry point SHALL accept `(call_id, name, arguments)` and SHALL return an async `Stream<Item = ToolboxEvent>` per the "Tool call lifecycle stream from the toolbox" requirement. The stream MUST internally:

- emit `Requested { call_id, name, arguments }` first, unconditionally,
- await `policy.decide(call_id, name, arguments)`,
- on `AuthDecision::Allow`: emit `Started`, then dispatch to the matching `Tool`, then emit `Finished { content = <tool output> }`,
- on `AuthDecision::Deny`: emit a single `Finished { content = TOOL_CALL_DENIED_BY_USER }` without invoking the underlying `Tool`,
- on `AuthDecision::AskUser { data }`: allocate a fresh `oneshot::channel`, store the sender in the toolbox's pending-authorization map keyed by `call_id`, emit `RequestingPermission { call_id, data }` (forwarding `data` verbatim), await the receiver, then proceed with the same allowed/denied branching as above.

The toolbox SHALL expose `async fn reply_toolcall_permission(&self, call_id: &str, data: serde_json::Value) -> Result<(), MorayError>` that pops the pending sender for `call_id`, delegates payload decoding to `policy.reply(call_id, data).await`, and resolves the `oneshot` with the returned boolean.

The `Tool` trait surface (`definition`, `call(arguments)`) SHALL remain unchanged.

Whether a `Toolbox` instance is constructed fresh per session or shared (with cloned `Arc`s) is an implementation concern of the `SessionHarnessFactory`, not a property of the `Toolbox` API itself.

#### Scenario: Mock toolbox lists and invokes without I/O

- **WHEN** tests register a mock toolbox with deterministic tool definitions and a mock policy that returns `AuthDecision::Allow`
- **THEN** the core loop MUST list those tools and observe `Requested` → `Started` → `Finished { content }` lifecycle events on invocation

#### Scenario: Denial produces a synthetic tool result without invoking the tool

- **WHEN** the effective authorization decision is denial (either `Deny` directly or `AskUser` followed by a `reply_toolcall_permission` whose policy decode returned `false`) for `call_id = X`
- **THEN** the toolbox lifecycle stream MUST yield `Requested { call_id = X, name, arguments }` followed by exactly one `Finished { content = TOOL_CALL_DENIED_BY_USER }` for `call_id = X` without invoking the underlying `Tool::call`

### Requirement: ReAct loop with explicit state machine

`moray-core` SHALL implement a ReAct-style agent loop as an explicit state machine driven by user input, chat completion stream chunks, and toolbox lifecycle events, emitting an ordered sequence of **`AgentRunResponseMessage`** on the **`Agent::run`** output stream. Authorization decisions are **not** handled by the agent; they are gated inside the toolbox via the toolbox-owned `ToolCallAuthPolicy`. The agent MUST NOT import `ToolCallAuthPolicy` and MUST NOT hold any pending-authorization state.

Each agent turn SHALL end with **`Finished { kind }`**, where **`kind`** is **`AgentInvokeFinishKind`**: **`succeeded`**, **`canceled`**, **`refused { reason?: … }`**, or **`failed { reason: … }`**.

Model output from the completion adapter SHALL be represented with **`ChatResponse`** carrying **`ChatCompletionResponseChunk`**.

For each tool call after the model round's **`Done`**, the loop SHALL drive the stream produced by `Toolbox::call_tool(call_id, name, arguments)` and forward its lifecycle events verbatim, wrapped in a single agent variant: every `ToolboxEvent` value (`Requested` / `RequestingPermission` / `Started` / `Finished`) becomes `AgentRunResponseMessage::ToolCall { event }` at the agent boundary without any variant-specific rewriting.

The agent MUST treat the `content` on `Finished` as opaque; when the toolbox emits no `Started` (denial case), the agent MUST NOT synthesize a `Started` lifecycle event. The agent SHALL NOT expose any `reply_tool_auth` / `reply_toolcall_permission` API.

When one completion round yields **multiple** `ToolCallRequest`s, the agent SHALL drive every call's `Toolbox::call_tool` lifecycle stream **concurrently** (not sequentially) so that authorization prompts for all pending calls surface before any reply is required and so that an approval for `call_id = X` dispatches that tool immediately without being blocked by other calls still awaiting their own replies. The agent SHALL still wait for **every** call in the batch to emit `Finished` before driving the next completion round. Per-`call_id` lifecycle ordering (`Requested → [RequestingPermission →] Started → Finished`, with `RequestingPermission` present only on `AskUser` and `Started` absent on denial) SHALL be preserved by the toolbox; cross-`call_id` event ordering reflects actual completion order and MUST NOT be assumed to match source declaration order.

#### Scenario: Agent forwards toolbox lifecycle verbatim

- **WHEN** `Toolbox::call_tool` yields `Requested` then `RequestingPermission` then `Started` then `Finished { content }` for `call_id = X`
- **THEN** the agent MUST emit `ToolCall { event: Requested { call_id: X, name, arguments } }` then `ToolCall { event: RequestingPermission { call_id: X, .. } }` then `ToolCall { event: Started { call_id: X } }` then `ToolCall { event: Finished { call_id: X, content } }` in that order, and MUST NOT inject any other event in between

#### Scenario: Agent omits Started on denial

- **WHEN** `Toolbox::call_tool` yields `Requested` followed only by `Finished { content = TOOL_CALL_DENIED_BY_USER }` (denied-via-`Deny` path)
- **THEN** the agent MUST emit `ToolCall { event: Requested { call_id, name, arguments } }` then `ToolCall { event: Finished { call_id, content = TOOL_CALL_DENIED_BY_USER } }` and MUST NOT emit any `ToolCall { event: Started }`

#### Scenario: Agent does not import the policy trait

- **WHEN** inspecting `core/src/agent.rs`
- **THEN** it MUST NOT contain any `use … ToolCallAuthPolicy` and MUST NOT hold any field whose type mentions `ToolCallAuthPolicy`

#### Scenario: Batch tool calls authorize out of order

- **WHEN** a completion round emits two tool calls `c1` and `c2`, the policy returns `AskUser` for both, and the harness calls `Session::reply_toolcall_permission("c2", Value::Bool(true))` before replying to `c1`
- **THEN** the agent run stream MUST emit `ToolCall { event: Requested }` and `ToolCall { event: RequestingPermission }` for both `c1` and `c2` before either `ToolCall { event: Started }`, MUST emit `ToolCall { event: Started { c2 } }` and `ToolCall { event: Finished { c2, ... } }` before the reply for `c1` is received, MUST NOT emit any event for `c1` beyond its `Requested` / `RequestingPermission` until `reply_toolcall_permission("c1", ...)` is called, and MUST NOT drive a next completion round until both `c1` and `c2` have produced a `Finished` lifecycle event

### Requirement: Agent run input and output align with chat completion messages and chunks

`moray-core` SHALL expose **`Agent::run`** such that:

- **Input** is an ordered slice of **`ChatCompletionRequestMessage`**.
- **Output** is a stream of **`AgentRunResponseMessage`**, an enum that SHALL contain **exactly three** variants:
  - **`ChatResponse { chunk: ChatCompletionResponseChunk }`** — forwards model streaming using the same chunk type as **`ChatCompletion`**. JSON serde tag: `chat_response`.
  - **`ToolCall { event: ToolboxEvent }`** — forwards the full toolbox lifecycle stream verbatim. The inner `ToolboxEvent` value is one of `Requested { call_id, name, arguments }` (unconditional first event, carries the full invocation signature), `RequestingPermission { call_id, data }` (only emitted when the policy returned `AuthDecision::AskUser`), `Started { call_id }`, or `Finished { call_id, content }`. Permission / lifecycle variants carry only `call_id`; tool name / arguments for rendering UI prompts are recoverable from the preceding `Requested` event (or, equivalently, the earlier `ChatResponse(ToolCall)` chunk) in the same agent stream. JSON serde tag: `tool_call`.
  - **`Finished { kind: AgentInvokeFinishKind }`** — terminal marker for the current turn. JSON serde tag: `finished`.

**`AssistantToolCallAuthorizationRequired`** is removed from `AgentRunResponseMessage`. The previously-separate `ToolCallAuthorizationRequested` / `ToolCallStarted` / `ToolCallFinished` / `InvokeFinished` variants are also removed — their semantics are fully expressible via the three flattened variants above (with the four lifecycle sub-cases living inside `ToolboxEvent`). Because `ToolboxEvent::RequestingPermission` nests `Option<serde_json::Value>`, `AgentRunResponseMessage` MUST derive `PartialEq` but MUST NOT derive `Eq`. Agent-level APIs MUST NOT accept or expose authorization replies.

#### Scenario: Model round streaming maps to completion chunks

- **WHEN** **`Agent::run`** executes a model round with streaming enabled
- **THEN** assistant text and tool calls from the model MUST be observable through **`ChatResponse`** items that preserve **`ChatCompletionResponseChunk`** ordering rules, ending with **`Done`** for that round before tool execution proceeds

#### Scenario: AgentRunResponseMessage enum has the expected variants

- **WHEN** inspecting the `AgentRunResponseMessage` type (or its serde tag set)
- **THEN** it MUST contain exactly: `ChatResponse`, `ToolCall`, `Finished`. It MUST NOT contain `AssistantToolCallAuthorizationRequired`, `ChatResponseChunk`, `ToolCallAuthorizationRequested`, `ToolCallStarted`, `ToolCallFinished`, or `InvokeFinished`.

### Requirement: Recovery from persisted events after external suspension

If execution stops while awaiting tool-call authorization (or another harness-controlled suspension encoded in persisted history), the session MUST be able to rebuild deterministic state from persisted data and continue execution when resumed.

On resume, the agent simply attempts to invoke every tool call whose result is not yet recorded. The toolbox, as always, consults the policy for each such call. Whether the policy short-circuits (e.g. from a persisted-decision cache) or surfaces a fresh `AskUser` is a policy-implementation decision — the session-level replay MUST NOT take that decision itself.

The session MUST NOT pre-emit pending authorization events from persisted history; those events are re-generated naturally when the resumed toolbox invocation calls the policy.

#### Scenario: Transcript ends at authorization gate (policy surfaces again)

- **WHEN** the persisted history contains a tool-call round (including model `ToolCall` / `Done` chunks) whose corresponding `ToolCall { event: Finished { .. } }` is not present, and the policy returns `AskUser` on resume
- **THEN** after restart, `Session::resume` MUST cause the toolbox to emit `Requested` and `RequestingPermission` for that `call_id`, which the session MUST surface as fresh `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::Requested { .. } / RequestingPermission { .. } } }` events; once the harness supplies the decision via `Session::reply_toolcall_permission`, execution MUST continue without re-requesting chat completion for the same tool call unless policy requires it

#### Scenario: Transcript ends at authorization gate (policy short-circuits)

- **WHEN** the persisted history contains an incomplete tool call and the policy returns `Allow` or `Deny` on resume (for example because `reply` previously cached a decision)
- **THEN** after restart, `Session::resume` MUST emit `AgentEvent(ToolCall { event: Requested })` for that `call_id`, and then proceed directly to `AgentEvent(ToolCall { event: Started })` / `AgentEvent(ToolCall { event: Finished })` (allow path) or directly to `AgentEvent(ToolCall { event: Finished { denied marker } })` (deny path) without surfacing an `AgentEvent(ToolCall { event: RequestingPermission })` event

### Requirement: Replay and partial assistant text without prefill

`moray-core` SHALL split replay responsibilities:

- **Session-level replay** consumes `SessionEvent` history and produces a snapshot including `messages` (the `ChatCompletionRequestMessage` slice to feed the agent), `pending_authorizations` (outstanding `ToolCallRequest`s whose corresponding `AgentEvent(ToolCall { event: RequestingPermission { call_id, .. } })` has not been paired with `AgentEvent(ToolCall { event: Finished { call_id } })` and which have not been invalidated by a subsequent `UserMessage`), and `needs_fresh_completion`. `AgentEvent(ToolCall { event: Requested { .. } })` is a no-op for pending-authorization tracking (it fires unconditionally and does not indicate user input is required). Pending authorization tracking lives **only** in session-level replay.
- **Agent-level replay** consumes only `AgentRunResponseMessage` history and derives `messages` and `needs_fresh_completion`. It MUST NOT inspect or reconstruct authorization state. Inside the single `ToolCall { event }` variant, only `ToolboxEvent::Finished` contributes a tool-role message to history; `Requested`, `RequestingPermission`, and `Started` are folded as no-ops.

Recovery policy for **`!prefill_supported`** remains:

- **Mid-history new user while assistant text was still pending**: partial assistant output for the interrupted turn MUST NOT be committed into **`ChatCompletionRequestMessage`** history; `needs_fresh_completion` MUST be set so `resume` can run a fresh completion round from the prior stable context.
- **End of persisted history** with pending assistant output or tools: implementation flushes pending assistant into **`messages`**, so a record that ends mid-stream may still materialize a partial assistant `Assistant` message.

#### Scenario: Replay unit test covers aborted partial turn

- **WHEN** persisted history reflects one user turn with incomplete assistant streaming, then begins another user turn, without **`prefill_supported`**
- **THEN** the session-level replay MUST set `needs_fresh_completion` and MUST NOT retain the partial assistant text in `messages` for the aborted turn

#### Scenario: Agent replay does not track authorization

- **WHEN** invoking agent-level replay helpers on an `AgentRunResponseMessage` slice
- **THEN** the returned snapshot MUST NOT include any `awaiting_tools` / `pending_authorizations` field; authorization tracking is reachable only via session-level replay

### Requirement: Session runtime orchestration in moray-core module

`moray-core` SHALL provide a `session` module that encapsulates session lifecycle orchestration over persisted event storage, `Agent` execution, and per-session tool authorization (delegated to the toolbox).

The `Session` abstraction SHALL own and coordinate:
- a store interface for loading, appending, and clearing persisted session events,
- a `SessionHarnessFactory` invoked during construction that yields the session-scoped `Toolbox` (with embedded policy) and the `ChatCompletion`,
- construction of the `Agent` from those harness components via `Agent::new(completion, toolbox)` inside `Session::new`,
- an `Arc<Toolbox>` reference for runtime event plumbing and for `reply_toolcall_permission` delegation,
- run/resume/reset flows that keep persisted events and in-memory agent state consistent.

`Session::new` SHALL take `(session_id, store, factory, options)` and MUST NOT accept a prebuilt `Agent`. The session MUST NOT hold any `Arc<dyn ToolCallAuthPolicy>` reference.

#### Scenario: Bootstrap session from persisted events

- **WHEN** a consumer constructs `Session` with a store containing prior `SessionEvent` records and a `SessionHarnessFactory`
- **THEN** the session MUST be able to continue using those persisted events and determine whether pending resumable work exists

#### Scenario: Reset session state

- **WHEN** a consumer requests a session reset
- **THEN** the session MUST clear persisted history through the store and reset session runtime counters/state used for subsequent turns

#### Scenario: Session owns the toolbox, not the policy

- **WHEN** a turn is in progress and the user calls `Session::reply_toolcall_permission(call_id, data)`
- **THEN** the call MUST be routed via `Toolbox::reply_toolcall_permission` on the session-held toolbox; the session MUST NOT hold or import a `ToolCallAuthPolicy` reference

### Requirement: Session outward stream uses SessionEvent with agent passthrough

`moray-core` SHALL expose `SessionEvent` as the outward event stream contract for session operations.

`SessionEvent` MUST include:
- session lifecycle events for orchestration boundaries and failures,
- `AgentEvent { event: AgentRunResponseMessage }` as the single passthrough variant for every agent stream item (with no authorization-specific carveout).

The session SHALL wrap every incoming `AgentRunResponseMessage` verbatim as `SessionEventKind::AgentEvent { event }`. Outstanding authorization requests are distinguished by the nested shape `AgentEvent { event: ToolCall { event: RequestingPermission { .. } } }`, not by a separate top-level variant. `SessionEventKind` SHALL NOT declare a `ToolCallAuthorization` variant — it would be redundant with the already-flattened `AgentRunResponseMessage::ToolCall { event }` form.

The session layer MUST NOT redefine a second set of invoke/resume semantics equivalent to `AgentRunResponseMessage`.

#### Scenario: All agent events are forwarded as AgentEvent without semantic duplication

- **WHEN** an underlying `Agent` emits any `AgentRunResponseMessage` item during `Session::run` or `Session::resume`
- **THEN** `Session` MUST publish that item via `SessionEvent::AgentEvent { event }` while preserving the original event payload semantics, including for every `ToolCall { event: Requested / RequestingPermission / Started / Finished }` variant

#### Scenario: Session lifecycle and agent events are distinguishable

- **WHEN** a consumer subscribes to `Session` output
- **THEN** it MUST be able to distinguish lifecycle transitions from forwarded `AgentRunResponseMessage` items using `SessionEvent` variants; authorization requests are recognized by destructuring the nested `AgentEvent { event: ToolCall { event: RequestingPermission { .. } } }` pattern

#### Scenario: Authorization events precede Started for the same call

- **WHEN** the toolbox surfaces a `RequestingPermission` for `call_id = X`
- **THEN** the outward `SessionEvent` stream MUST emit `AgentEvent(ToolCall { event: Requested { call_id: X, name, arguments } })` before `AgentEvent(ToolCall { event: RequestingPermission { call_id: X, .. } })`, which in turn MUST precede `AgentEvent(ToolCall { event: Started { call_id: X } })`, and all three MUST precede `AgentEvent(ToolCall { event: Finished { call_id: X, ... } })`
