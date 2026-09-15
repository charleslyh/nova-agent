## Why

Tool-call authorization is currently entangled inside **`Agent`**: the agent owns the pending-auth map, emits **`AssistantToolCallAuthorizationRequired`**, and exposes **`Agent::reply_tool_auth`**. This mixes two concerns in the ReAct loop — model-turn correctness (completion rounds, message history, tool invocation) and harness-controlled policy (who grants permission, how the UI is prompted) — and forces **`Session`** to proxy auth replies into the agent despite owning the user-facing lifecycle.

Moving authorization into the **`Toolbox`** orbit (with the trait surface staying invisible to the rest of `nova-core`) produces the cleanest separation:

- the agent just drives completions and forwards the toolbox lifecycle stream;
- the toolbox is the single integration point with the authorization policy and is the sole producer of authorization-related lifecycle events. It also owns the pending-request state (the `oneshot` map keyed by `call_id`) that backs `AskUser` decisions;
- the authorization "policy" has a **two-method surface**: `decide(...) -> AuthDecision` returns `Allow | Deny | AskUser { data }`, and `reply(call_id, data) -> bool` interprets the user's eventual reply into a final allow / deny. The toolbox still owns the pending `oneshot` map; the policy only sees the reply payload so it can cache or persist the outcome (e.g. "always allow this tool for this session");
- the session is a thin orchestrator: it pumps the agent stream into `SessionEvent`s and routes user authorization replies to the toolbox; it never imports the policy trait.

This makes agent recovery trivial (no auth state to rebuild), unblocks future authorization policies (auto-approve, persisted-decision caching, out-of-band approvals) without touching the agent, and eliminates the now-unnecessary callback channel that the previous design needed to surface authorization requests through the session.

The project has no external API, persistence, or version compatibility to preserve, so this refactor is executed as a single breaking change.

## What Changes

- Remove all tool-authorization logic from **`nova_core::Agent`**:
  - Drop **`AgentRunResponseMessage::AssistantToolCallAuthorizationRequired`**.
  - Drop **`Agent::reply_tool_auth`**, **`AuthState`**, pending **`oneshot`** maps, and the auth sub-stream inside **`agent.rs`**.
  - Flatten `AgentRunResponseMessage` to exactly three variants — `ChatResponse { chunk }`, `ToolCall { event: ToolboxEvent }`, `Finished { kind }`. The agent forwards the toolbox lifecycle stream verbatim as the `ToolCall` variant; downstream consumers correlate with the preceding `ChatResponse(ToolCall { call_id, name, arguments })` chunk to recover tool name / arguments when needed (e.g. for rendering an authorization prompt).
- Introduce a **`ToolCallAuthPolicy`** trait in **`nova-core`** with **two** methods:
  - `async fn decide(&self, call_id, tool_name, arguments) -> AuthDecision`
  - `async fn reply(&self, call_id, data: serde_json::Value) -> bool`

  where `AuthDecision` is `Allow | Deny | AskUser { data: Option<serde_json::Value> }`. `decide` returns the gating verdict; when it returns `AskUser`, the toolbox carries `data` out to the UI inside `RequestingPermission`. When the UI replies, the toolbox forwards the raw reply payload (`Value`) to `reply`; the policy interprets it into a final allow / deny **and MAY cache / persist the outcome** before returning. `nova-core` does NOT ship a concrete implementation; consumers (demo, tests, production harnesses) provide their own.
- Rework **`Toolbox`** as the single owner of both the policy and the pending-authorization state:
  - `Toolbox::new(tools, Arc<dyn ToolCallAuthPolicy + Send + Sync>)` binds the policy at construction; `ToolboxBuilder::build(policy)` mirrors the same shape.
  - The toolbox internally owns the `oneshot` map keyed by `call_id` that backs `AskUser` flows.
  - `Toolbox::call_tool(call_id, name, arguments)` returns an async `Stream<Item = ToolboxEvent>` with four variants:
    - `Requested { call_id, name, arguments }` — **unconditional** first event, emitted before `decide` is consulted. Carries the full invocation signature so UIs can render the tool call (prompt, pending row, etc.) directly from the lifecycle stream without having to correlate with the preceding `ChatResponse(ToolCall)` chunk.
    - `RequestingPermission { call_id, data: Option<Value> }` — emitted **only** when `decide` returns `AskUser { data }`; immediately before yielding, the toolbox allocates a `oneshot` channel and stores the sender in its pending map; `data` is the policy's opaque payload forwarded verbatim,
    - `Started { call_id }` — emitted after the decision resolves to allow, immediately before `Tool::call`,
    - `Finished { call_id, content }` — terminal item; on denial carries `TOOL_CALL_DENIED_BY_USER`.
  - `Toolbox::reply_toolcall_permission(call_id, data: Value)` is the public surface that resolves a pending `AskUser` oneshot. It forwards `data` to `ToolCallAuthPolicy::reply`, then resolves the `oneshot` with the boolean the policy returns. `Session::reply_toolcall_permission` routes through this so the session never imports `ToolCallAuthPolicy`. Unknown `call_id`s return an error.
  - `TOOL_CALL_DENIED_BY_USER` stays in the toolbox module. The `Tool` trait is unchanged.
- Introduce a **`SessionHarnessFactory`** trait with exactly two methods:
  - `create_toolbox(&self) -> Arc<Toolbox>` (returned toolbox already has its policy wired in),
  - `create_completion(&self) -> Arc<dyn ChatCompletion + Send + Sync>`.

  `Session::new` constructs the `Agent` internally via `Agent::new(completion, toolbox)`. The factory does NOT expose policy construction directly — that is an internal concern of whoever implements the factory.
- Replace **`Session::new(session_id, store, agent, options)`** with **`Session::new(session_id, store, factory, options)`**. `Session` only retains an `Arc<Toolbox>`; it does NOT hold or import any `ToolCallAuthPolicy` reference.
- Outstanding authorization requests surface externally through the **already-flattened** agent shape: `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, data } } }`. `Session` does NOT introduce a separate `ToolCallAuthorization` variant — that would be redundant with the nested shape. `call_id` + the policy's opaque `data` payload are both carried; UI consumers correlate with the earlier persisted `AgentEvent(ChatResponse(ToolCall))` chunk to render the prompt and may additionally use `data` for richer rendering (e.g. cached-decision hints). The session inspects only variant tags — it never imports the policy trait.
- Route **`Session::reply_toolcall_permission(call_id, data: Value)`** to **`Toolbox::reply_toolcall_permission`**, which forwards the payload to the policy's `reply` and resolves the toolbox-owned `oneshot` with the returned boolean. The agent is no longer involved.
- The session outward stream becomes a pure passthrough: a single `while let Some(ev) = agent_stream.next().await` loop that wraps every item as `SessionEventKind::AgentEvent { event }`. No `mpsc`, no `tokio::select!`, no callback registration / cleanup, no variant promotion.
- Split **`replay`** into:
  - session-level replay, which reads `SessionEvent` history and produces `{ messages, pending_authorizations, needs_fresh_completion }`;
  - agent-level replay, which reads `AgentRunResponseMessage` history and produces `{ messages, needs_fresh_completion }` only.
- Update **`demo/examples/chat.rs`**, **`demo/src/chat_view.rs`**, **`demo/src/factory.rs`**, **`demo/src/policies.rs`**, and fixtures (`demo/src/mock.rs`, `demo/src/agent_test_fixtures.rs`) to the new surfaces. Prompt for authorization by matching `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, .. } } }`. Construct `Session` via the demo `SessionHarnessFactory` implementation that wires `AlwaysAskPolicy` (a trivial `AskUser { data: None }`-only policy that interprets `Value::Bool` replies) into a fresh `Toolbox` per session. Tests use `PredicatePolicy` (synchronous `Allow`/`Deny` predicate) via `DemoHarness::with_policy` to bypass prompting.

## Impact

- **Affected specs:** `nova-core`, `nova-demos`
- **Affected code:**
  - `core/src/agent.rs` (heavy simplification; no authorization state; lifecycle stream forwarded verbatim as the `ToolCall` variant)
  - `core/src/agent_run.rs` (drop `AssistantToolCallAuthorizationRequired`; flatten to `ChatResponse` / `ToolCall { event: ToolboxEvent }` / `Finished`; `Eq` dropped because `ToolboxEvent` nests `Option<Value>`)
  - `core/src/toolbox.rs` (lifecycle stream gains `Requested` + `RequestingPermission { data }`; embedded policy + pending `oneshot` map; `reply_toolcall_permission` public surface; `call_tool(call_id, ...)`)
  - `core/src/session.rs` (factory-based construction, pure passthrough wrap_stream, no policy reference, `reply_toolcall_permission(call_id, Value)` routes through toolbox)
  - `core/src/auth_policy.rs` (trait shape: `decide(...) -> AuthDecision::{Allow, Deny, AskUser { data: Option<Value> } }` plus `reply(call_id, Value) -> bool`; toolbox owns the `oneshot` map)
  - `core/src/session_factory.rs` (single-method `create_toolbox`)
  - `core/src/replay.rs` (split into session vs agent replay; remove auth-gate inspection)
  - `core/src/lib.rs` (re-exports)
  - `demo/examples/chat.rs`, `demo/src/chat_view.rs`, `demo/src/lib.rs`
  - `demo/src/policies.rs` (`AlwaysAskPolicy`, `PredicatePolicy`, `allow_all_policy` — decision-only)
  - `demo/src/factory.rs` (`DemoHarness::create_toolbox` builds the policy-embedded toolbox internally; `with_policy` factory variant for tests)
  - `demo/src/mock.rs` (removes legacy `AutoAuthorizer`/`allow_all_authorizer`; test doubles now stateless)
  - `demo/tests/*.rs`, `demo/src/agent_test_fixtures.rs`
