## Context

Authorization currently lives in **`Agent`**: the agent batches `ToolCallRequest`s between the completion `Done` chunk and execution, emits **`AssistantToolCallAuthorizationRequired`** per call, owns a **`HashMap<call_id, oneshot::Sender<bool>>`**, and exposes **`Agent::reply_tool_auth`** so **`Session`** can forward user decisions. This couples UI/policy concerns to the ReAct loop and makes the agent's replay/recovery story more complex than it needs to be (`replay.rs` inspects auth-gate events to reconstruct `awaiting_tools`).

The refactor targets four outcomes: (1) remove auth state from the agent; (2) make the **`Toolbox`** the single integration point with the authorization policy — so the rest of `nova-core` (agent + session) is auth-trait-agnostic — *and* the sole owner of pending-authorization state; (3) preserve the existing ordering semantics of tool-call lifecycle (`ToolCallStarted` = authorization passed, tool execution actually started); and (4) eliminate the previously-required callback / mpsc fan-in plumbing inside the session.

## Goals / Non-Goals

- **Goals**
  - Agent code simplifies to: drive completion rounds; pump the toolbox lifecycle stream; emit only three variants — `ChatResponse { chunk }`, `ToolCall { event }` (the toolbox lifecycle event forwarded verbatim), and `Finished { kind }`.
  - The toolbox is the only place that knows the `ToolCallAuthPolicy` trait. It owns the policy at construction, gates tool calls through it, owns the pending-authorization `oneshot` map, and exposes `reply_toolcall_permission` for the session.
  - The policy has a **two-method surface**: `decide` returns the gating verdict (and may attach an opaque `data` payload via `AskUser`), and `reply` interprets the user's raw reply (`serde_json::Value`) into a final allow/deny so the policy can cache or persist the outcome. The pending `oneshot` map still lives inside the toolbox, not the policy.
  - The session is a pure event passthrough: it owns only `Arc<Toolbox>` (and the agent it built); it never imports `ToolCallAuthPolicy`.
  - `Session::new(session_id, store, factory, options)` replaces the prebuilt-agent path; the factory's only authorization-related method is `create_toolbox` (the policy is internal to whoever implements the factory).
  - Agent replay becomes trivial: rebuild messages only; no authorization state.
  - `ToolboxEvent::Started` retains the semantic "authorization passed, tool invocation is actually beginning". `ToolboxEvent::RequestingPermission` is emitted **only** when the policy returns `AskUser { data }` (i.e. user input is required) and forwards `data` verbatim. `ToolboxEvent::Requested` is the unconditional first event per call, announcing that the toolbox has accepted the call (before any policy consultation). `Allow` / `Deny` decisions never produce `RequestingPermission`, so the UI is never falsely prompted.

- **Non-Goals**
  - Forward compatibility with any existing API, persistence format, or transcript.
  - New authorization policies (auto-approve, batched prompts, timeouts). The refactor makes these easier but does not implement them.
  - Changing the `Tool` trait shape (`definition` + `call(arguments)`).

## Decisions

### Decision 1: `ToolCallAuthPolicy` decides *and* interprets the reply (toolbox owns the channels)

- **Trait shape:**
  ```rust
  #[derive(Clone, Debug, PartialEq)]
  pub enum AuthDecision {
      /// Execute the tool immediately; no `RequestingPermission` event.
      Allow,
      /// Refuse immediately; toolbox emits a single `Finished` with the denied marker.
      Deny,
      /// Stall execution; toolbox emits `RequestingPermission { call_id, data }` and awaits
      /// `reply_toolcall_permission`. `data` is an opaque payload the policy forwards to the UI
      /// (for example a prompt template, caller context, or a cached-decision hint); it is `None`
      /// when the policy has nothing extra to convey.
      AskUser { data: Option<serde_json::Value> },
  }

  #[async_trait]
  pub trait ToolCallAuthPolicy: Send + Sync {
      async fn decide(
          &self,
          call_id: &str,
          tool_name: &str,
          arguments: &str,
      ) -> AuthDecision;

      /// Interpret the UI's reply payload into a final allow (`true`) / deny (`false`) decision.
      /// The toolbox forwards the raw `Value` supplied to `Toolbox::reply_toolcall_permission`
      /// and then resolves the waiting `oneshot` with the returned boolean. Implementations MAY
      /// cache or persist the outcome (e.g. "always allow this tool for this session") before
      /// returning.
      async fn reply(
          &self,
          call_id: &str,
          data: serde_json::Value,
      ) -> bool;
  }
  ```
- **Why two methods, with the channel still in the toolbox:** `decide` answers *whether* a tool call needs user input; `reply` answers *how the user's raw payload maps onto a final allow/deny* (and lets the policy persist that mapping). The name is short because the type (`ToolCallAuthPolicy`) already implies the domain — `policy.reply(call_id, data)` reads unambiguously as "the policy's reply handler for this call". Pending-request state (the `oneshot` channel keyed by `call_id`) still belongs exclusively to the toolbox, which already knows the `call_id`, is the unique point where `RequestingPermission` is emitted, and is the waiter that needs to be unblocked. The policy never sees the `oneshot`; it only sees the decoded payload.
- **Why three `AuthDecision` variants instead of `bool`:** `Allow` / `Deny` are auto-decisions (no prompt); `AskUser { data }` explicitly requests user involvement and carries an optional payload forwarded to the UI. The toolbox can unambiguously decide whether to emit `RequestingPermission`.
- **Why `AuthDecision::AskUser` is `PartialEq`, not `Eq`:** `Value` is only `PartialEq`; the enum degrades accordingly. Callers comparing decisions in tests (none exist in `nova-core`) use `matches!` or `==` via `PartialEq`.
- **`nova-core` ships no concrete implementation.** Demo/tests/production each provide their own (e.g. `AlwaysAskPolicy` for interactive — interprets `Value::Bool` replies, `PredicatePolicy` for tests, a future `CachedPolicy`, etc.).

### Decision 2: `Toolbox` owns both the policy and the pending-authorization state

- **Construction:**
  ```rust
  impl Toolbox {
      pub fn new(
          tools: Vec<Arc<dyn Tool + Send + Sync>>,
          policy: Arc<dyn ToolCallAuthPolicy + Send + Sync>,
      ) -> Self;
  }
  ```
  `ToolboxBuilder::build(policy)` mirrors the same shape. Because the policy is bound at construction, callers above the toolbox never need to touch it.
- **Internal state:**
  ```rust
  pub struct Toolbox {
      tools: Vec<Arc<dyn Tool + Send + Sync>>,
      policy: Arc<dyn ToolCallAuthPolicy + Send + Sync>,
      pending_auth: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
  }
  ```
  The toolbox — **not** the policy — owns the `oneshot` map keyed by `call_id`.
- **Lifecycle stream:**
  ```rust
  pub enum ToolboxEvent {
      /// Unconditional, emitted before the policy is consulted. Carries the full
      /// `(call_id, name, arguments)` triple so UI / replay consumers can render the tool
      /// invocation (prompt, pending row, icon, etc.) directly from the lifecycle stream
      /// without having to correlate with the preceding chat-completion chunk.
      Requested { call_id: String, name: String, arguments: String },
      /// Emitted only when `decide` returned `AskUser { data }`; `data` is forwarded verbatim.
      /// Only carries `call_id` + the policy payload; tool name / arguments are recoverable
      /// from the preceding `Requested { call_id, name, arguments }` event for the same
      /// `call_id`. The policy also receives `(call_id, tool_name, arguments)` via `decide`.
      RequestingPermission { call_id: String, data: Option<serde_json::Value> },
      Started  { call_id: String },
      Finished { call_id: String, content: String },
  }

  impl Toolbox {
      pub fn call_tool(
          &self,
          call_id: &str,
          name: &str,
          arguments: &str,
      ) -> Pin<Box<dyn Stream<Item = ToolboxEvent> + Send + '_>>;

      pub async fn reply_toolcall_permission(
          &self,
          call_id: &str,
          data: serde_json::Value,
      ) -> Result<(), NovaError>;
  }
  ```
- **Semantics:**
  ```text
  yield Requested { call_id, name, arguments };          # always fires first, full signature
  let allowed = match self.policy.decide(call_id, name, args).await {
      AuthDecision::Allow  => true,
      AuthDecision::Deny   => false,
      AuthDecision::AskUser { data } => {
          let (tx, rx) = oneshot::channel();
          self.pending_auth.lock().insert(call_id.to_string(), tx);
          yield RequestingPermission { call_id, data };  # data forwarded verbatim
          rx.await.unwrap_or(false)
      }
  };
  if !allowed {
      yield Finished { call_id, content: TOOL_CALL_DENIED_BY_USER };
      return;
  }
  yield Started { call_id };
  yield Finished { call_id, content: <Tool::call output or error string> };
  ```
  `Toolbox::reply_toolcall_permission(call_id, data)` pops the sender from `pending_auth`, calls `self.policy.reply(call_id, data).await` to convert the payload into a boolean, and then forwards that boolean on the `oneshot`. Unknown `call_id`s return `NovaError::Message` without consulting the policy.
- **Why this lives in toolbox, not agent or session:** the toolbox is the only layer that already knows the policy trait, the tool registry, the `call_id`, and the result shape. Co-locating the `oneshot` map with the conditional `RequestingPermission` emission removes cross-component coordination: there is no way to construct a channel without the toolbox also emitting the event, and there is no way to resolve it without the toolbox also running the policy's reply hook.

### Decision 3: Agent forwards lifecycle events verbatim, with **concurrent batch execution**

- `AgentRunResponseMessage` is collapsed to exactly three variants: `ChatResponse { chunk }`, `ToolCall { event: ToolboxEvent }`, and `Finished { kind }`. The agent forwards the toolbox lifecycle stream as `ToolCall { event }` without inspecting which inner variant it is — the four lifecycle sub-variants (`Requested`, `RequestingPermission`, `Started`, `Finished`) remain inside the `ToolboxEvent` type owned by the toolbox. Tool name / arguments needed for rendering an authorization prompt are recoverable from the preceding `ChatResponse(ToolCall { call_id, name, arguments })` in the same agent stream.
- The agent never inspects authorization state, never imports `ToolCallAuthPolicy`, and never holds pending decisions.
- **Batch concurrency (out-of-order authorization).** When one completion round yields N `ToolCallRequest`s, the agent drives all N `Toolbox::call_tool` lifecycle streams **concurrently** via `futures::stream::select_all`, rather than sequentially. This is required so the user can see every pending `RequestingPermission` at once and reply in any order: each call owns an independent `oneshot` inside the toolbox, and a reply to `call_id = X` unblocks exactly that call's execution without waiting for other still-pending calls to resolve.
  - **Guarantee preserved:** the agent still waits for all N `ToolboxEvent::Finished` events before invoking the next chat completion round — the batch-then-continue ReAct contract is unchanged.
  - **Guarantee preserved (per-call):** for a single `call_id`, the toolbox's `call_tool` stream yields `Requested → [RequestingPermission →] Started → Finished` in strict order (when applicable — `RequestingPermission` is skipped when the policy short-circuits to `Allow` / `Deny`, and `Started` is skipped on `Deny`).
  - **Guarantee relaxed (cross-call):** events for distinct `call_id`s may interleave freely; the stream reflects actual completion timing. Callers (agent consumers, tests, UIs) MUST NOT assume source-declaration order across different `call_id`s. The `Tool` messages appended to the runtime chat history follow completion order, which is permitted by OpenAI-compatible providers (tool results only need to match their `call_id` in the preceding assistant message).

### Decision 4: Session is a pure passthrough; no variant promotion

- **`SessionEventKind`** has NO dedicated authorization variant. Outstanding authorization requests surface through the already-nested shape `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, data } } }`. Adding a separate `ToolCallAuthorization` variant on top of this would be redundant — any consumer who needs to react to authorization requests destructures the nested pattern directly, and any consumer who does not (e.g. a raw event logger) gets uniform `AgentEvent` handling for free. The event carries `call_id` + the policy's opaque `data`; UI consumers correlate with the preceding `AgentEvent(ToolCall { event: Requested { call_id, name, arguments } })` event (or, if they prefer, the earlier persisted `AgentEvent(ChatResponse(ToolCall { call_id, name, arguments }))` chunk — both carry the same signature) to render the prompt and may additionally use `data` (e.g. for cached-decision hints). All agent events — including every `ToolboxEvent` sub-case — flow through unchanged as `SessionEventKind::AgentEvent { event }`.
- **`Session::wrap_stream` reduces to a single `while let Some(ev) = agent_stream.next().await` loop that wraps every item as `AgentEvent { event }`.** No `mpsc::UnboundedSender`, no `tokio::select!`, no callback registration, no callback cleanup on drop, no variant promotion.
- **`Session` does NOT hold or import `ToolCallAuthPolicy`.** It only stores `Arc<Toolbox>`. `reply_toolcall_permission(call_id, data)` is implemented as `self.toolbox.reply_toolcall_permission(call_id, data).await`.
- **Ordering contract** (per `call_id` `X`, with external authorization required):
  ```
  AgentEvent(ChatResponse(ToolCall { call_id: X, ... }))
  AgentEvent(ChatResponse(Done { ... }))
  AgentEvent(ToolCall { event: Requested { call_id: X, name, arguments } })    # toolbox-emitted; unconditional first lifecycle event, carries full signature
  AgentEvent(ToolCall { event: RequestingPermission { call_id: X, data } })     # toolbox-emitted only on AskUser
  (user replies via Session::reply_toolcall_permission → Toolbox::reply_toolcall_permission
   → policy.reply → oneshot resolves inside toolbox)
  AgentEvent(ToolCall { event: Started  { call_id: X } })                       # toolbox-emitted via lifecycle stream
  AgentEvent(ToolCall { event: Finished { call_id: X, content } })              # toolbox-emitted via lifecycle stream
  ```
  When the policy short-circuits with `Allow`: `Requested { call_id, name, arguments }` + `Started` + `Finished` (no `RequestingPermission`). With `Deny`: `Requested { call_id, name, arguments }` + `Finished { TOOL_CALL_DENIED_BY_USER }` (no `RequestingPermission`, no `Started`).

### Decision 5: `SessionHarnessFactory` exposes only `create_toolbox` and `create_completion`

- **Shape:**
  ```rust
  pub trait SessionHarnessFactory: Send + Sync {
      fn create_toolbox(&self) -> Arc<Toolbox>;
      fn create_completion(&self) -> Arc<dyn ChatCompletion + Send + Sync>;
  }
  ```
  `Session::new` calls `factory.create_toolbox()` and `factory.create_completion()`, then constructs `Agent::new(completion, toolbox.clone())`. The session retains only `Arc<Toolbox>`.
- **Why no `create_policy` method:** the policy is bound inside the toolbox at construction time. Splitting it into two factory methods would force every implementor to internally share one `Arc<dyn ToolCallAuthPolicy>` between the two methods (a footgun) — or to take the returned policy as a parameter to `create_toolbox` (another footgun, plus extra surface). A single `create_toolbox` makes the binding tamper-proof and lets the factory implementer decide internally whether the policy is per-session, shared, or derived from a cache.
- **Sharing policy across sessions:** an implementation choice of the factory. Per-session toolboxes (with per-session policies) are the common case for interactive harnesses; a factory may equally cache `Arc<Toolbox>` for stateless / always-allow setups.

### Decision 6: Split replay into session-level and agent-level

- **Session-level replay** consumes `Vec<SessionEvent>` and produces `{ messages, pending_authorizations, needs_fresh_completion }`. `pending_authorizations` are derived from `AgentEvent(ToolCall { event: RequestingPermission { call_id, .. } })` items not yet paired with an `AgentEvent(ToolCall { event: Finished { .. } })` for the same `call_id`, and invalidated by a subsequent `UserMessage`. `AgentEvent(ToolCall { event: Requested { .. } })` is a no-op for pending-authorization tracking (it fires unconditionally and does not indicate that user input is needed). Since `RequestingPermission` carries only `call_id` + opaque `data`, replay reconstructs each pending `ToolCallRequest` by looking up the `call_id` in the current turn's state (either `pending_tools` before a `Done` chunk, or the most recent assistant message's `tool_calls` after `Done`). The snapshot continues to expose full `ToolCallRequest` structs for caller convenience.
- **Agent-level replay** consumes only `Vec<AgentRunResponseMessage>` and produces `{ messages, needs_fresh_completion }`. It MUST NOT inspect or reconstruct authorization state. Inside the single `ToolCall { event }` variant, only `ToolboxEvent::Finished` contributes a tool-role message to history; `Requested` / `RequestingPermission` / `Started` are all no-ops for history reconstruction.
- **Resume flow:** `Session::resume` rebuilds `messages` from session-level replay and re-runs the agent. If a tool call still has no `Finished` lifecycle event, the agent re-invokes `Toolbox::call_tool`, which re-emits `Requested` and then re-invokes `policy.decide`. The policy decides whether to short-circuit (e.g. from a persisted decision cache populated via `reply`) or surface a fresh `AskUser`. The session never pre-emits authorization events from history.

### Decision 7: `AgentRunResponseMessage` variant set

```rust
pub enum AgentRunResponseMessage {
    ChatResponse { chunk: ChatCompletionResponseChunk },
    /// Toolbox lifecycle forwarded verbatim. The inner `ToolboxEvent` enum carries one of
    /// `Requested { call_id, name, arguments }` (unconditional first, full signature) /
    /// `RequestingPermission { call_id, data }` / `Started { call_id }` /
    /// `Finished { call_id, content }`. Downstream variants carry only `call_id`;
    /// correlate with the preceding `Requested` event (or the earlier `ChatResponse(ToolCall)`
    /// chunk) to recover tool name / arguments.
    ToolCall { event: ToolboxEvent },
    Finished { kind: AgentInvokeFinishKind },
}
```

`AssistantToolCallAuthorizationRequired` is removed, as are the previously-separate `ToolCallAuthorizationRequested` / `ToolCallStarted` / `ToolCallFinished` / `InvokeFinished` variants — they are all expressible via the three flatter variants above. `Eq` is not derived on `AgentRunResponseMessage` because `ToolboxEvent::RequestingPermission` nests an `Option<serde_json::Value>` and `Value` is only `PartialEq`; `SessionEventKind` inherits the same limitation. Breaking in both Rust API and serde tags. Acceptable because there is no external version to preserve.

## Risks / Trade-offs

- **Authorization-event semantics depend on policy behavior.** A buggy policy that always returns `AskUser` even when it could short-circuit will surface unnecessary `RequestingPermission` events. This is correct (matches the contract) but may surprise UX. Mitigation: documented contract that `Allow` / `Deny` MUST be used for any decision the policy can make synchronously without user input.
- **`Toolbox::reply_toolcall_permission` is a public method on a concrete type.** Misuse (calling it with a `call_id` that is not pending) returns an error *without* consulting the policy; tests cover this. Valid calls additionally invoke `policy.reply`, so policies MUST tolerate being asked to decode payloads for calls they previously asked about — which holds trivially for stateless policies.
- **Stream-termination semantics for `call_tool`.** Every invocation MUST end with exactly one `Finished` (allowed, denied, or tool-error). Mitigation: enforced by toolbox implementation; unit-tested.
- **Per-session toolbox construction cost.** A factory that builds heavy tools per session pays a tax. Mitigation: tools can be shared as `Arc<dyn Tool>` across toolboxes; the toolbox itself is cheap (a `Vec` + `Arc<dyn Policy>` + a small `HashMap`).
- **Policy-is-implementation-detail expectation.** Some advanced consumers may want to share one policy across many toolboxes (e.g. a global policy). The current shape supports this trivially (the factory implementer creates one `Arc<dyn ToolCallAuthPolicy>` at startup and clones it into every `Toolbox::new`); it just isn't visible in the trait surface.
- **Pending `oneshot` on toolbox drop.** If the toolbox is dropped while a `call_id` is still pending, the sender drops, the `rx.await` resolves to `Err`, and the toolbox stream yields `Finished { ... TOOL_CALL_DENIED_BY_USER }` via `rx.await.unwrap_or(false)`. This matches the "denial" outcome and is the conservative choice.

## Migration Plan

Because there is no external compatibility to preserve, the migration is a single-PR rewrite:

1. Define `ToolCallAuthPolicy` (two methods: `decide` + `reply`) + `AuthDecision` (`Allow` / `Deny` / `AskUser { data }`) in `core/src/auth_policy.rs`; drop the previous `ToolCallAuthorizer` / `AuthorizationStart` / `AuthorizationCallback` / `set_callback` surface.
2. Rework `Toolbox`: `new`/`builder.build` take `Arc<dyn ToolCallAuthPolicy>`; `call_tool` returns the four-variant `ToolboxEvent` lifecycle stream (`Requested`, `RequestingPermission { data }`, `Started`, `Finished`) and owns the pending `oneshot` map; add `reply_toolcall_permission(call_id, data: Value)` which pops the sender and delegates payload decoding to `policy.reply`.
3. Rework `Agent` to drop all authorization state and forward every `ToolboxEvent` variant verbatim as `AgentRunResponseMessage::ToolCall { event }`.
4. Rewrite `Session::new` to take `SessionHarnessFactory` (now just `create_toolbox` + `create_completion`); collapse `Session::wrap_stream` to a pure passthrough that wraps every agent event as `AgentEvent { event }` (no variant promotion — authorization requests stay nested inside `AgentEvent(ToolCall { event: RequestingPermission { .. } })`). `Session::reply_toolcall_permission(call_id, data)` delegates to `self.toolbox.reply_toolcall_permission(...)`.
5. Split `replay.rs` into session-level and agent-level helpers; remove auth-gate inspection. Track pending authorizations by matching `ToolboxEvent::RequestingPermission` (not `Requested`).
6. Update demo (`demo/src/policies.rs` ships `AlwaysAskPolicy` + `PredicatePolicy` + `allow_all_policy` implementing both policy methods — `AlwaysAskPolicy::reply` decodes `Value::Bool`, the others defensively deny; `DemoHarness::create_toolbox` builds the policy-embedded toolbox internally; `chat.rs` and `chat_view.rs` prompt for authorization by destructuring `SessionEventKind::AgentEvent { event: AgentRunResponseMessage::ToolCall { event: ToolboxEvent::RequestingPermission { call_id, .. } } }` and reply with `Session::reply_toolcall_permission(&call_id, Value::Bool(allowed))`).
7. Update `nova-core` and `nova-demos` specs; run `openspec validate --strict` and `cargo test --workspace`.

**Rollback:** revert the PR. No downstream depends on the old API.
