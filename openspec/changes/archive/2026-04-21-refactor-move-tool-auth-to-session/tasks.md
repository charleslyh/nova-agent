## 1. Authorization policy abstraction

- [x] 1.1 Define `ToolCallAuthPolicy` trait with two methods: `async fn decide(...) -> AuthDecision` and `async fn reply(&self, call_id, data: serde_json::Value) -> bool`. Define `AuthDecision { Allow, Deny, AskUser { data: Option<serde_json::Value> } }` (no `Eq` derive — `serde_json::Value` is only `PartialEq`). The toolbox still owns the pending `oneshot` map; the policy only sees decoded payloads and MAY cache/persist the outcome.
- [x] 1.2 `moray-core` ships no concrete policy; exports only the trait + enum from `core/src/auth_policy.rs`. `serde_json` becomes a non-optional dependency so the `data` field compiles regardless of feature flags.

## 2. Toolbox lifecycle stream + policy + pending-auth ownership

- [x] 2.1 Define `ToolboxEvent { Requested { call_id, name, arguments }, RequestingPermission { call_id, data: Option<serde_json::Value> }, Started { call_id }, Finished { call_id, content } }` (no `Eq` derive — `Value` is only `PartialEq`). The `Requested` variant carries the full tool-invocation signature so UIs can render prompts / pending rows directly from the lifecycle stream; subsequent lifecycle variants (`RequestingPermission` / `Started` / `Finished`) carry only `call_id` and (where relevant) the policy's opaque `data` or the tool's `content`.
- [x] 2.2 `Toolbox::new(tools, policy)` and `ToolboxBuilder::build(policy)` accept `Arc<dyn ToolCallAuthPolicy + Send + Sync>` at construction. The `Toolbox` stores an `Arc` to its policy internally.
- [x] 2.3 `Toolbox` internally owns `Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>` for pending `AskUser` requests. The map is NOT part of the public surface.
- [x] 2.4 `Toolbox::call_tool(call_id, name, arguments) -> Stream<Item = ToolboxEvent>` runs:
  - emit `Requested { call_id, name, arguments }` unconditionally (before consulting the policy),
  - match `policy.decide(...)`:
    - `Allow`: emit `Started`, dispatch tool, emit `Finished { content }`,
    - `Deny`: emit `Finished { content = TOOL_CALL_DENIED_BY_USER }`,
    - `AskUser { data }`: allocate a fresh `oneshot::channel()`, insert the sender into the pending map keyed by `call_id`, emit `RequestingPermission { call_id, data }` (forwarding `data` verbatim), await the receiver, then proceed allowed/denied as above.
- [x] 2.5 Add `Toolbox::reply_toolcall_permission(call_id, data: serde_json::Value) -> Result<(), MorayError>` that pops the sender from the pending map, calls `policy.reply(call_id, data).await` to decode the payload into a boolean, then resolves the `oneshot` with that boolean. Unknown `call_id` returns an error without consulting the policy.
- [x] 2.6 `TOOL_CALL_DENIED_BY_USER` stays in toolbox module; re-export from `moray_core` unchanged.
- [x] 2.7 Unit-test the toolbox with in-file `AskUserPolicy` + `StaticPolicy` covering: `Allow` path (`Requested → Started → Finished`); `Deny` path (`Requested → Finished{denied}`); `AskUser` allowed (`Requested → RequestingPermission → Started → Finished`); `AskUser` denied (`Requested → RequestingPermission → Finished{denied}`); tool error path always ends with `Finished`; `reply_toolcall_permission` unknown `call_id` error; `reply_toolcall_permission` delegates the payload to the policy's `reply`.

## 3. Session harness factory

- [x] 3.1 `SessionHarnessFactory` exposes exactly two methods: `create_toolbox(&self) -> Arc<Toolbox>` and `create_completion(&self) -> Arc<dyn ChatCompletion + Send + Sync>`. No `create_policy`, no `(toolbox, policy)` tuple.
- [x] 3.2 `Session::new(session_id, store, factory, options)`: call `factory.create_toolbox()`, then `factory.create_completion()`, construct `Agent::new(completion, toolbox.clone())`. The `Session` struct stores only `Arc<Toolbox>` (no policy field, no `ToolCallAuthPolicy` import).
- [x] 3.3 Provide a demo `SessionHarnessFactory` (`DemoHarness`) whose `create_toolbox` builds an `AlwaysAskPolicy` (or user-supplied policy) and embeds it inside the toolbox.

## 4. Strip authorization from agent

- [x] 4.1 Remove `AgentRunResponseMessage::AssistantToolCallAuthorizationRequired`. Flatten `AgentRunResponseMessage` to exactly three variants — `ChatResponse { chunk }`, `ToolCall { event: ToolboxEvent }`, `Finished { kind }` — and forward the toolbox lifecycle stream verbatim inside the `ToolCall` variant (no dedicated `ToolCallAuthorizationRequested` / `ToolCallStarted` / `ToolCallFinished` variants). Drop `Eq` derive — the nested `Option<Value>` in `RequestingPermission` forces `PartialEq` only.
- [x] 4.2 Remove any remaining references to `Agent::reply_tool_auth`, `AuthState`, pending `oneshot` maps, and callback plumbing in `core/src/agent.rs`. Confirm no `use … ToolCallAuthPolicy`.
- [x] 4.3 Update the agent's call_tool drain loop to yield `AgentRunResponseMessage::ToolCall { event }` for every `ToolboxEvent` the toolbox emits (`Requested` / `RequestingPermission` / `Started` / `Finished`), without variant-specific rewriting.

## 5. Session passthrough (no variant promotion)

- [x] 5.1 `SessionEventKind` has NO dedicated `ToolCallAuthorization` variant. Outstanding authorization requests surface through the already-nested shape `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, data } } }`. `Eq` is likewise dropped on `SessionEventKind`/`SessionEvent` — only `PartialEq`.
- [x] 5.2 Rewrite `Session::wrap_stream` as a single `while let Some(ev) = agent_stream.next().await` loop that wraps every item verbatim as `SessionEventKind::AgentEvent { event }`. Remove `mpsc::UnboundedSender`, `tokio::select!`, callback registration, any trailing-callback-drain block, and any variant-specific promotion. Keep `TurnDropCleanup` for `SessionFailed` persistence on early drop; it holds no policy/authorizer reference.
- [x] 5.3 `Session::reply_toolcall_permission(call_id, data: serde_json::Value)` calls `self.toolbox.reply_toolcall_permission(call_id, data).await`. No `policy` field on `Session`, no `use … ToolCallAuthPolicy` in `core/src/session.rs`.

## 6. Replay split

- [x] 6.1 Session-level replay: `replay_session_events` returns `{ messages, pending_authorizations, needs_fresh_completion }`. `pending_authorizations` are `ToolCallRequest`s whose `AgentEvent(ToolCall { event: RequestingPermission { call_id, .. } })` has not been paired with `AgentEvent(ToolCall { event: Finished { call_id } })` and not invalidated by a subsequent `UserMessage`. `Requested` events are treated as no-ops for pending tracking (they fire unconditionally and do not indicate user input is needed). Replay looks up the full `ToolCallRequest` from prior `pending_tools` or the tail assistant message's `tool_calls` (the permission event only carries `call_id` + opaque `data`).
- [x] 6.2 Agent-level replay: `replay_agent_events` returns `{ messages, needs_fresh_completion }` only. Treat `ToolCall { event: Requested }`, `ToolCall { event: RequestingPermission }`, and `ToolCall { event: Started }` as no-ops for message reconstruction; only `ToolCall { event: Finished }` appends a tool-role message. `SegmentedAgentReplay` drops `Eq` derive (its `resume_tail: Vec<AgentRunResponseMessage>` inherits `PartialEq`-only).
- [x] 6.3 `Session::prepare_turn` / `resume` use the session-level snapshot for `messages`. The toolbox + policy decide whether to re-surface `RequestingPermission` on resume (including any cached short-circuit from a prior `reply`).
- [x] 6.4 `session_resumable` derives pending work from the session-level snapshot.

## 7. Demo updates

- [x] 7.1 `demo/src/policies.rs` provides two concrete policies: `AlwaysAskPolicy` (returns `AskUser { data: None }`; `reply` decodes `Value::Bool`, defaulting to `false` for any other shape) and `PredicatePolicy<F>` (returns `Allow`/`Deny` from a synchronous predicate; `reply` defensively returns `false` because the predicate should never surface `AskUser`). `allow_all_policy()` convenience factory. No pending-queue, no `oneshot`, no callback field.
- [x] 7.2 Remove the legacy `ChannelAuthorizer` / `AutoAuthorizer` / `allow_all_authorizer` types; those responsibilities are now covered by `policies.rs` + `Toolbox`.
- [x] 7.3 `DemoHarness::create_toolbox` builds a fresh `Toolbox` per session, embedding the configured `Arc<dyn ToolCallAuthPolicy>` (default: `AlwaysAskPolicy`; `with_policy` for tests).
- [x] 7.4 `examples/chat.rs` drain loop matches the nested `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, .. } } }` pattern and forwards decisions via `Session::reply_toolcall_permission(&call_id, Value::Bool(allowed))`.
- [x] 7.5 Integration tests under `demo/tests/`: migrated from `AutoAuthorizer`/`allow_all_authorizer` to `PredicatePolicy`/`allow_all_policy`; `DemoHarness::with_authorizer` renamed to `with_policy`. Out-of-order authorization tests call `toolbox.reply_toolcall_permission(call_id, Value::Bool(allowed))`.

## 8. Specs and validation

- [x] 8.1 Apply `moray-core` spec deltas (this change set).
- [x] 8.2 Apply `moray-demos` spec deltas (mostly unchanged surface).
- [x] 8.3 Run `openspec validate refactor-move-tool-auth-to-session --strict`.
- [x] 8.4 Run `cargo build --workspace` and `cargo test --workspace` (offline); fix any breakage.
- [x] 8.5 Smoke-test `cargo run -p demo --example chat` against a stub to confirm the interactive authorization flow still works end-to-end.
