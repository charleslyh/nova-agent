//! Tool registry, execution, authorization, and push-mode tool-call events.
//!
//! [`TypedTool`] is the typed implementation surface; [`Tool`] is the object-safe handle stored in
//! [`Toolbox`]. Lifecycle events flow through [`ToolCallEvent`] / [`ToolCallEventSink`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn, Instrument};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::types::{NovaError, ToolCallRequest, ToolCallResult, ToolCallStatus, ToolManifest};

// ---------------------------------------------------------------------------
// Constants & errors
// ---------------------------------------------------------------------------

/// Shown in the tool result when authorization denies execution.
pub const TOOL_CALL_DENIED_BY_USER: &str = "This tool call was denied by the user.";

/// Shown in the tool result when the turn or tool-call group is canceled.
pub const TOOL_CALL_CANCELED: &str = "This tool call was canceled.";

/// Errors from [`Toolbox`] and related flows.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolboxError {
    #[error("unknown tool {name}")]
    UnknownTool { name: String },

    #[error("unknown tool call group {group_id}")]
    UnknownGroup { group_id: u64 },

    /// [`ToolCallEventSink::emit`] rejected the event (agent stream closed or backpressure).
    #[error("tool call event sink closed")]
    EventSinkClosed,

    /// Local output path failed before reaching the sink (e.g. stdout flush in CLI).
    #[error("failed to deliver tool call output: {reason}")]
    DeliverFailed { reason: String },
}

impl From<ToolboxError> for NovaError {
    fn from(value: ToolboxError) -> Self {
        NovaError::Message(value.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tool-call events
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum ToolCallEventKind {
    Requested {
        name: String,
        arguments: Value,
    },

    Started,

    /// Only `Payload` events are forwarded into model reasoning;
    Payload {
        text: String,
    },

    /// Out-of-band metadata (authorization prompts, progress). Not model `tool` message content.
    Extra {
        data: Value,
    },

    Finished {
        status: ToolCallStatus,
    },
}

/// Tool-call lifecycle event: stable `call_id` plus a tagged [`ToolCallEventKind`] payload.
///
/// On the wire (`serde`), `kind` is flattened so JSON is
/// `{ "call_id", "type", ...payload }` rather than `{ "call_id", "kind": { ... } }`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ToolCallEvent {
    pub call_id: String,

    #[cfg_attr(feature = "serde", serde(flatten))]
    pub kind: ToolCallEventKind,
}

impl ToolCallEvent {
    pub fn requested(call_id: String, name: String, arguments: Value) -> Self {
        Self {
            call_id,
            kind: ToolCallEventKind::Requested { name, arguments },
        }
    }

    pub fn started(call_id: String) -> Self {
        Self {
            call_id,
            kind: ToolCallEventKind::Started,
        }
    }

    pub fn payload(call_id: String, text: String) -> Self {
        Self {
            call_id,
            kind: ToolCallEventKind::Payload { text },
        }
    }

    pub fn extra(call_id: String, data: Value) -> Self {
        Self {
            call_id,
            kind: ToolCallEventKind::Extra { data },
        }
    }

    pub fn finished(call_id: String, status: ToolCallStatus) -> Self {
        Self {
            call_id,
            kind: ToolCallEventKind::Finished { status },
        }
    }
}

// ---------------------------------------------------------------------------
// Tool traits
// ---------------------------------------------------------------------------

/// Object-safe tool handle stored in [`Toolbox`].
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;

    /// Execute the tool. Model-visible output goes through [`ToolCallResponder`]; lifecycle events
    /// (`Requested`, `Started`, `Finished`) are emitted only by [`Toolbox`]. The `cancellation`
    /// token lets the tool observe turn-level cancellation cooperatively: a tool that wants to
    /// emit a final message on cancel should `select!` on `cancellation.cancelled()` and
    /// `send_text` before returning. [`run_call`] does not race the tool future with the token,
    /// so the tool always observes the token and decides how to respond.
    async fn call(
        &self,
        call_id: &str,
        args: Value,
        responder: &dyn ToolCallResponder,
        cancellation: CancellationToken,
    ) -> Result<(), NovaError>;
}

/// Typed tool: per-tool [`Args`](Self::Args) + [`run`](Self::run). Metadata (description, JSON schema) comes from the app-layer tool catalog.
#[async_trait]
pub trait TypedTool: Send + Sync {
    type Args: DeserializeOwned + Send;
    const NAME: &'static str;

    /// Emit model-visible output via `responder` (supports streaming); return only on failure.
    /// The `cancellation` token lets the tool observe turn-level cancellation; a tool that
    /// wants to emit a final message on cancel should `select!` on `cancellation.cancelled()`.
    /// [`Toolbox::run_call`] calls this directly without an outer `select!`, so the tool is
    /// always responsible for observing the token — there is no hard-drop fallback inside
    /// `run_call`. (The group-level [`ToolCallGroup::join`] still aborts the spawned task
    /// if the whole group is cancelled and the tool has not returned yet.)
    async fn run(
        &self,
        args: Self::Args,
        responder: &dyn ToolCallResponder,
        cancellation: CancellationToken,
    ) -> Result<(), NovaError>;
}

#[async_trait]
impl<T> Tool for T
where
    T: TypedTool + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        T::NAME
    }

    async fn call(
        &self,
        _call_id: &str,
        args: Value,
        responder: &dyn ToolCallResponder,
        cancellation: CancellationToken,
    ) -> Result<(), NovaError> {
        let tool_name = T::NAME;
        tracing::info!(
            "[tool] {} args={}",
            tool_name,
            serde_json::to_string(&args).unwrap_or_default()
        );
        let args = serde_json::from_value(args)
            .map_err(|e| NovaError::Message(format!("{tool_name}: invalid JSON arguments: {e}")))?;
        let result = self
            .run(args, responder, cancellation)
            .await
            .map_err(|e| NovaError::Message(format!("{tool_name}: {e}")));
        tracing::info!("[tool] {} result={:?}", tool_name, result);
        result
    }
}

// ---------------------------------------------------------------------------
// Responder, authorizer, event sink
// ---------------------------------------------------------------------------

/// Tool-facing output surface ([`Tool::call`], authorizers). Lifecycle events are not included.
#[async_trait]
pub trait ToolCallResponder: Send + Sync {
    /// Out-of-band metadata (authorization prompts, progress). Not model `tool` message content.
    async fn send_extra(&self, data: Value) -> Result<(), ToolboxError>;

    /// Model-visible incremental output.
    async fn send_text(&self, text: String) -> Result<(), ToolboxError>;
}

/// Generic pre-execution hook invoked inside `run_call` after the call is
/// registered with its tracker and before the `Requested` event is emitted.
///
/// Interceptors receive the full call context and decide their own behavior:
/// they MAY rewrite `request.arguments` in place (augment, modify or redirect
/// arguments; `call_id` and `name` changes are reverted by the toolbox), MAY
/// emit out-of-band events through `responder` (buffered by the toolbox and
/// flushed after `Requested` is emitted, so the wire contract of "Requested
/// first" always holds), and MAY block waiting for external input while
/// cooperatively responding to `cancellation`.
///
/// Returning `true` proceeds with the (possibly rewritten) request; returning
/// `false` skips execution entirely: the toolbox emits `Requested` with the
/// current request, flushes the interceptor's buffered output (appending a
/// default denial payload when no `Payload` was buffered), and finishes the
/// call with `Finished(Error)`. Interceptors that deny SHOULD describe the
/// denial themselves through the responder before returning `false`.
///
/// Multiple interceptors are chained in registration order: each observes the
/// rewrites of its predecessors, and the first `false` short-circuits the
/// chain. Implementations must not fail the call themselves; unexpected
/// internal errors should degrade to returning `true` with the request
/// untouched.
#[async_trait]
pub trait ToolCallInterceptor: Send + Sync {
    /// Intercepts a tool call before execution.
    async fn intercept(
        &self,
        request: &mut ToolCallRequest,
        manifest: Option<&ToolManifest>,
        responder: Arc<dyn ToolCallResponder>,
        cancellation: CancellationToken,
    ) -> bool;
}

/// [`ToolCallResponder`] that buffers events instead of forwarding them.
///
/// Used by `run_call` to hold an interceptor's output until the `Requested`
/// event has been emitted, then flushed in order.
#[derive(Default)]
struct BufferingToolCallResponder {
    events: std::sync::Mutex<Vec<ToolCallEventKind>>,
}

impl BufferingToolCallResponder {
    fn drain(&self) -> Vec<ToolCallEventKind> {
        std::mem::take(&mut self.events.lock().unwrap())
    }
}

#[async_trait]
impl ToolCallResponder for BufferingToolCallResponder {
    async fn send_extra(&self, data: Value) -> Result<(), ToolboxError> {
        self.events
            .lock()
            .unwrap()
            .push(ToolCallEventKind::Extra { data });
        Ok(())
    }

    async fn send_text(&self, text: String) -> Result<(), ToolboxError> {
        self.events
            .lock()
            .unwrap()
            .push(ToolCallEventKind::Payload { text });
        Ok(())
    }
}

/// Push destination for [`ToolCallEvent`] (agent run stream, tests, etc.).
#[async_trait]
pub trait ToolCallEventSink: Send + Sync {
    async fn emit(&self, ev: ToolCallEvent) -> bool;
}

// ---------------------------------------------------------------------------
// Per-call execution (tracker, group, run loop)
// ---------------------------------------------------------------------------

/// Per-call state for agent/toolbox runs: request, streaming content, result, and lifecycle events.
///
/// [`ToolCallGroup`] emits `Requested` / `Started` / `Finished` via [`Self::emit`] / [`Self::finish`].
/// Tools and authorizers interact through [`ToolCallResponder`].
struct ToolCallTracker {
    /// The request this call was (or will be) executed with. Interceptors may
    /// rewrite it before the `Requested` event is emitted, hence the lock.
    request: std::sync::Mutex<ToolCallRequest>,
    sink: Arc<dyn ToolCallEventSink>,
    content: Arc<Mutex<String>>,
    result: Arc<Mutex<Option<ToolCallResult>>>,
}

impl ToolCallTracker {
    fn new(request: ToolCallRequest, sink: Arc<dyn ToolCallEventSink>) -> Self {
        Self {
            request: std::sync::Mutex::new(request),
            sink,
            content: Arc::new(Mutex::new(String::new())),
            result: Arc::new(Mutex::new(None)),
        }
    }

    fn call_id(&self) -> String {
        self.request.lock().unwrap().call_id.clone()
    }

    fn request(&self) -> ToolCallRequest {
        self.request.lock().unwrap().clone()
    }

    /// Replaces the tracked request after interception, keeping the original
    /// `call_id` and `name`: only `arguments` may be rewritten, so that event
    /// correlation and transcript ingest stay aligned with the model's own
    /// tool call.
    fn set_request(&self, request: ToolCallRequest) {
        let mut guard = self.request.lock().unwrap();
        if request.call_id != guard.call_id || request.name != guard.name {
            warn!(
                original_call_id = %guard.call_id,
                original_name = %guard.name,
                "interceptor tried to rewrite call_id/name; reverting to original"
            );
        }
        let arguments = request.arguments;
        *guard = ToolCallRequest {
            call_id: guard.call_id.clone(),
            name: guard.name.clone(),
            arguments,
        };
    }

    async fn take_result(&self) -> ToolCallResult {
        self.result
            .lock()
            .await
            .take()
            .unwrap_or_else(|| ToolCallResult {
                call_id: self.call_id(),
                content: String::new(),
                status: ToolCallStatus::Error,
            })
    }

    /// Lifecycle events (`Requested`, `Started`, `Extra`). Not used for [`ToolCallEventKind::Finished`].
    async fn emit(&self, ev: ToolCallEvent) -> bool {
        debug_assert!(
            !matches!(ev.kind, ToolCallEventKind::Finished { .. }),
            "use ToolCallTracker::finish for Finished events"
        );
        self.sink.emit(ev).await
    }

    /// Ends the call: pushes [`ToolCallEventKind::Finished`] and records [`ToolCallResult`] for ingest.
    async fn finish(&self, status: ToolCallStatus) -> bool {
        // Move out the accumulated `send_text` buffer under the lock (no clone); mutex keeps an empty `String`.
        let content = std::mem::take(&mut *self.content.lock().await);

        *self.result.lock().await = Some(ToolCallResult {
            call_id: self.call_id(),
            content,
            status: status.clone(),
        });

        self.sink
            .emit(ToolCallEvent::finished(self.call_id(), status))
            .await
    }
}

#[async_trait]
impl ToolCallResponder for ToolCallTracker {
    async fn send_extra(&self, data: Value) -> Result<(), ToolboxError> {
        if self.emit(ToolCallEvent::extra(self.call_id(), data)).await {
            Ok(())
        } else {
            Err(ToolboxError::EventSinkClosed)
        }
    }

    async fn send_text(&self, text: String) -> Result<(), ToolboxError> {
        self.content.lock().await.push_str(&text);
        if self
            .emit(ToolCallEvent::payload(self.call_id(), text))
            .await
        {
            Ok(())
        } else {
            Err(ToolboxError::EventSinkClosed)
        }
    }
}

/// Concurrent batch of tool calls: shared event sink, per-call trackers, join-set execution.
struct ToolCallGroup {
    cancellation: CancellationToken,
    join_set: JoinSet<()>,
    sink: Arc<dyn ToolCallEventSink>,
    tool_calls: Vec<Arc<ToolCallTracker>>,
}

impl ToolCallGroup {
    fn new(sink: Arc<dyn ToolCallEventSink>, cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            join_set: JoinSet::new(),
            sink,
            tool_calls: Vec::new(),
        }
    }

    fn start_call(
        &mut self,
        tool: Arc<dyn Tool>,
        interceptors: Vec<Arc<dyn ToolCallInterceptor>>,
        manifest: Option<ToolManifest>,
        request: ToolCallRequest,
    ) {
        let tracker = Arc::new(ToolCallTracker::new(request, self.sink.clone()));
        self.tool_calls.push(tracker.clone());
        let cancellation = self.cancellation.clone();
        self.join_set
            .spawn(run_call(tool, interceptors, manifest, tracker, cancellation).in_current_span());
    }

    async fn join(mut self) -> (Vec<ToolCallRequest>, Vec<ToolCallResult>) {
        while !self.join_set.is_empty() {
            if self.cancellation.is_cancelled() {
                self.join_set.abort_all();
            }
            match self.join_set.join_next().await {
                Some(Ok(())) => {}
                Some(Err(_)) => {}
                None => break,
            }
        }

        if self.cancellation.is_cancelled() {
            for tracker in &self.tool_calls {
                if tracker.result.lock().await.is_none() {
                    finish_canceled(tracker).await;
                }
            }
        }

        let mut requests = Vec::with_capacity(self.tool_calls.len());
        let mut results = Vec::with_capacity(self.tool_calls.len());
        for tracker in self.tool_calls {
            requests.push(tracker.request());
            results.push(tracker.take_result().await);
        }

        (requests, results)
    }
}

async fn finish_canceled(tracker: &ToolCallTracker) {
    let _ = tracker.finish(ToolCallStatus::Canceled).await;
}

async fn run_call(
    tool: Arc<dyn Tool>,
    interceptors: Vec<Arc<dyn ToolCallInterceptor>>,
    manifest: Option<ToolManifest>,
    tracker: Arc<ToolCallTracker>,
    cancellation: CancellationToken,
) {
    if cancellation.is_cancelled() {
        finish_canceled(&tracker).await;
        return;
    }

    let call_id = tracker.call_id();
    let name = tracker.request().name.clone();

    // Interceptor chain, before `Requested` is emitted. Interceptors observe
    // and may rewrite the request in place; their responder output is buffered
    // so that `Requested` always reaches consumers first.
    let mut request = tracker.request();
    let mut buffered: Vec<ToolCallEventKind> = Vec::new();
    let mut allowed = true;
    if !interceptors.is_empty() {
        let buffering = Arc::new(BufferingToolCallResponder::default());
        for interceptor in &interceptors {
            let proceed = tokio::select! {
                _ = cancellation.cancelled() => {
                    finish_canceled(&tracker).await;
                    return;
                }
                proceed = interceptor.intercept(
                    &mut request,
                    manifest.as_ref(),
                    Arc::clone(&buffering) as Arc<dyn ToolCallResponder>,
                    cancellation.clone(),
                ) => proceed,
            };
            if !proceed {
                allowed = false;
                break;
            }
        }
        buffered = buffering.drain();
    }
    tracker.set_request(request);

    if cancellation.is_cancelled() {
        finish_canceled(&tracker).await;
        return;
    }

    let request = tracker.request();
    if !tracker
        .emit(ToolCallEvent::requested(
            call_id.clone(),
            name.clone(),
            request.arguments.clone(),
        ))
        .await
    {
        if cancellation.is_cancelled() {
            finish_canceled(&tracker).await;
        } else {
            let _ = tracker.finish(ToolCallStatus::Error).await;
        }
        return;
    }

    // Flush the interceptors' buffered output now that `Requested` is out.
    let mut buffered_payload = false;
    for kind in buffered {
        match kind {
            ToolCallEventKind::Payload { text } => {
                buffered_payload = true;
                let _ = tracker.send_text(text).await;
            }
            ToolCallEventKind::Extra { data } => {
                let _ = tracker.send_extra(data).await;
            }
            _ => {}
        }
    }

    if !allowed {
        warn!(call_id = %call_id, tool = %name, "tool call denied by interceptor");
        if !buffered_payload {
            let _ = tracker
                .send_text(TOOL_CALL_DENIED_BY_USER.to_string())
                .await;
        }
        let _ = tracker.finish(ToolCallStatus::Error).await;
        return;
    }

    if cancellation.is_cancelled() {
        finish_canceled(&tracker).await;
        return;
    }

    if !tracker.emit(ToolCallEvent::started(call_id.clone())).await {
        if cancellation.is_cancelled() {
            finish_canceled(&tracker).await;
        } else {
            let _ = tracker.finish(ToolCallStatus::Error).await;
        }
        return;
    }

    // Cooperative cancellation: call the tool directly with the token. The tool is
    // responsible for observing `cancellation.cancelled()` and emitting any final
    // model-visible message via `send_text` before returning `Ok(())`. We do NOT race
    // the tool future with the token here, so the tool's cancel-time output is never
    // dropped. After the tool returns, we check the token to decide the final status:
    // a tool that observed cancellation should report `Canceled` (not `Success`) so
    // downstream consumers (frontend, session log) mark it correctly.
    let cancel_check = cancellation.clone();
    let status = match tool
        .call(&call_id, request.arguments, tracker.as_ref(), cancellation)
        .await
    {
        Ok(()) => {
            if cancel_check.is_cancelled() {
                ToolCallStatus::Canceled
            } else {
                ToolCallStatus::Success
            }
        }
        Err(e) => {
            warn!(call_id = %call_id, tool = %name, error = %e, "tool call failed");
            let _ = tracker.send_text(format!("tool error: {e}")).await;
            ToolCallStatus::Error
        }
    };

    match status {
        ToolCallStatus::Success => {
            info!(call_id = %call_id, tool = %name, "tool call completed");
        }
        ToolCallStatus::Error | ToolCallStatus::Canceled => {}
    }

    let _ = tracker.finish(status).await;
}

// ---------------------------------------------------------------------------
// Toolbox registry & groups
// ---------------------------------------------------------------------------

/// Opaque handle for a batch of concurrent tool calls created by [`Toolbox::begin_group`].
pub type ToolCallGroupId = u64;

#[derive(Clone)]
pub struct Toolbox {
    /// Shared registry for concurrent tool-call tasks (`Arc` is applied in [`Toolbox::new`], not in [`ToolboxBuilder`]).
    tools: Arc<HashMap<String, Arc<dyn Tool>>>,
    manifests: Vec<ToolManifest>,
    interceptors: Vec<Arc<dyn ToolCallInterceptor>>,
    groups: Arc<Mutex<HashMap<ToolCallGroupId, ToolCallGroup>>>,
    next_group_id: Arc<AtomicU64>,
}

#[derive(Default)]
pub struct ToolboxBuilder {
    tools: HashMap<String, Arc<dyn Tool>>,
    manifests: Vec<ToolManifest>,
    interceptors: Vec<Arc<dyn ToolCallInterceptor>>,
}

impl ToolboxBuilder {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            manifests: Vec::new(),
            interceptors: Vec::new(),
        }
    }

    pub fn manifests(mut self, manifests: Vec<ToolManifest>) -> Self {
        self.manifests = manifests;
        self
    }

    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(tool.name().to_string(), tool);
        self
    }

    /// Appends an interceptor to the chain. Interceptors run in registration
    /// order before every tool call; the first one returning `false`
    /// short-circuits the chain and skips execution.
    pub fn interceptor(mut self, interceptor: Arc<dyn ToolCallInterceptor>) -> Self {
        self.interceptors.push(interceptor);
        self
    }

    pub fn build(self) -> Toolbox {
        Toolbox::new(self.tools, self.manifests, self.interceptors)
    }
}

impl Toolbox {
    pub fn new(
        tools: HashMap<String, Arc<dyn Tool>>,
        manifests: Vec<ToolManifest>,
        interceptors: Vec<Arc<dyn ToolCallInterceptor>>,
    ) -> Self {
        Self {
            tools: Arc::new(tools),
            manifests,
            interceptors,
            groups: Arc::new(Mutex::new(HashMap::new())),
            next_group_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Merges `manifests` and `tools` into this toolbox in place.
    /// It is used to support SubAgents mode, that `sub agent trigger` tool can be injected into leader agent's toolbox.
    pub fn extend(
        &mut self,
        manifests: impl IntoIterator<Item = ToolManifest>,
        tools: impl IntoIterator<Item = Arc<dyn Tool>>,
    ) {
        let tool_map = Arc::make_mut(&mut self.tools);
        for tool in tools {
            tool_map.insert(tool.name().to_string(), tool);
        }
        self.manifests.extend(manifests);
    }

    pub async fn list_tools(&self) -> Vec<ToolManifest> {
        self.manifests.clone()
    }

    pub async fn begin_group(
        &self,
        sink: Arc<dyn ToolCallEventSink>,
        turn_cancellation: CancellationToken,
    ) -> ToolCallGroupId {
        let id = self.next_group_id.fetch_add(1, Ordering::Relaxed);
        let group_cancellation = turn_cancellation.child_token();
        self.groups
            .lock()
            .await
            .insert(id, ToolCallGroup::new(sink, group_cancellation));
        id
    }

    /// Starts one tool call in `group_id`. Events are emitted as they occur (no batching at the group layer).
    pub async fn call_tool(
        &self,
        group_id: ToolCallGroupId,
        request: ToolCallRequest,
    ) -> Result<(), ToolboxError> {
        let tool =
            self.tools
                .get(&request.name)
                .cloned()
                .ok_or_else(|| ToolboxError::UnknownTool {
                    name: request.name.clone(),
                })?;

        let manifest = self
            .manifests
            .iter()
            .find(|m| m.name == request.name)
            .cloned();

        let mut groups = self.groups.lock().await;

        let group = groups
            .get_mut(&group_id)
            .ok_or(ToolboxError::UnknownGroup { group_id })?;

        group.start_call(tool, self.interceptors.clone(), manifest, request);

        Ok(())
    }

    /// Waits until every call in `group` has finished.
    ///
    /// Returns `(requests, results)` with the same length; indices align with [`Self::call_tool`] order.
    pub async fn end_group(
        &self,
        group_id: ToolCallGroupId,
    ) -> Result<(Vec<ToolCallRequest>, Vec<ToolCallResult>), ToolboxError> {
        let group = self
            .groups
            .lock()
            .await
            .remove(&group_id)
            .ok_or(ToolboxError::UnknownGroup { group_id })?;
        Ok(group.join().await)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flatten_serializes_kind_fields_at_top_level() {
        let ev = ToolCallEvent::requested("c1".into(), "echo".into(), json!("hi"));
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            json!({
                "call_id": "c1",
                "type": "requested",
                "name": "echo",
                "arguments": "hi"
            })
        );
        assert!(v.get("kind").is_none());
    }

    #[test]
    fn flatten_deserializes_roundtrip() {
        let raw = json!({
            "call_id": "c2",
            "type": "payload",
            "text": "chunk"
        });
        let ev: ToolCallEvent = serde_json::from_value(raw).unwrap();
        assert_eq!(ev.call_id, "c2");
        assert_eq!(
            ev.kind,
            ToolCallEventKind::Payload {
                text: "chunk".into()
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::sync::{mpsc, oneshot};
    use tokio_util::sync::CancellationToken;

    struct MpscToolCallEventSink(mpsc::Sender<ToolCallEvent>);

    impl MpscToolCallEventSink {
        fn pair(capacity: usize) -> (Arc<Self>, mpsc::Receiver<ToolCallEvent>) {
            let (tx, rx) = mpsc::channel(capacity);
            (Arc::new(Self(tx)), rx)
        }
    }

    #[async_trait]
    impl ToolCallEventSink for MpscToolCallEventSink {
        async fn emit(&self, ev: ToolCallEvent) -> bool {
            self.0.send(ev).await.is_ok()
        }
    }

    struct AskUserPolicy {
        pending_auth: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    }

    impl AskUserPolicy {
        fn new() -> Self {
            Self {
                pending_auth: Mutex::new(HashMap::new()),
            }
        }

        async fn reply(&self, call_id: &str, data: Value) {
            let tx = self
                .pending_auth
                .lock()
                .expect("ask-user pending-auth mutex poisoned")
                .remove(call_id)
                .expect("pending auth entry");
            let allow = data.get("allow").and_then(|v| v.as_bool()).unwrap_or(false);
            let _ = tx.send(allow);
        }
    }

    #[async_trait]
    impl ToolCallInterceptor for AskUserPolicy {
        async fn intercept(
            &self,
            request: &mut ToolCallRequest,
            _manifest: Option<&ToolManifest>,
            responder: Arc<dyn ToolCallResponder>,
            _cancellation: CancellationToken,
        ) -> bool {
            let (tx, rx) = oneshot::channel();
            self.pending_auth
                .lock()
                .expect("ask-user pending-auth mutex poisoned")
                .insert(request.call_id.clone(), tx);
            if responder
                .send_extra(json!({
                    "tool_name": request.name,
                    "arguments": request.arguments,
                }))
                .await
                .is_err()
            {
                self.pending_auth
                    .lock()
                    .expect("ask-user pending-auth mutex poisoned")
                    .remove(&request.call_id);
                return false;
            }
            rx.await.unwrap_or(false)
        }
    }

    #[derive(Clone, Copy)]
    enum StaticDecision {
        Allow,
        Deny,
    }

    struct StaticPolicy(StaticDecision);

    #[async_trait]
    impl ToolCallInterceptor for StaticPolicy {
        async fn intercept(
            &self,
            _request: &mut ToolCallRequest,
            _manifest: Option<&ToolManifest>,
            _responder: Arc<dyn ToolCallResponder>,
            _cancellation: CancellationToken,
        ) -> bool {
            match self.0 {
                StaticDecision::Allow => true,
                StaticDecision::Deny => false,
            }
        }
    }

    struct EchoTool;

    #[async_trait]
    impl TypedTool for EchoTool {
        type Args = Value;
        const NAME: &'static str = "echo";

        async fn run(
            &self,
            args: Value,
            responder: &dyn ToolCallResponder,
            _cancellation: CancellationToken,
        ) -> Result<(), NovaError> {
            responder
                .send_text(format!(
                    "echo:{}",
                    serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
                ))
                .await?;
            Ok(())
        }
    }

    struct SlowTool;

    #[async_trait]
    impl TypedTool for SlowTool {
        type Args = Value;
        const NAME: &'static str = "slow";

        async fn run(
            &self,
            _: Value,
            _responder: &dyn ToolCallResponder,
            _cancellation: CancellationToken,
        ) -> Result<(), NovaError> {
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    struct FailTool;

    #[async_trait]
    impl TypedTool for FailTool {
        type Args = Value;
        const NAME: &'static str = "bad";

        async fn run(
            &self,
            _: Value,
            _responder: &dyn ToolCallResponder,
            _cancellation: CancellationToken,
        ) -> Result<(), NovaError> {
            Err(NovaError::Message("boom".into()))
        }
    }

    fn test_manifests() -> Vec<ToolManifest> {
        vec![
            ToolManifest {
                name: "echo".into(),
                description: "".into(),
                parameters: r#"{"type":"object"}"#.into(),
            },
            ToolManifest {
                name: "bad".into(),
                description: "".into(),
                parameters: r#"{"type":"object"}"#.into(),
            },
        ]
    }

    fn build_test_toolbox(
        interceptors: Vec<Arc<dyn ToolCallInterceptor>>,
        manifests: Vec<ToolManifest>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Toolbox {
        let mut builder = ToolboxBuilder::new().manifests(manifests);
        for interceptor in interceptors {
            builder = builder.interceptor(interceptor);
        }
        for tool in tools {
            builder = builder.tool(tool);
        }
        builder.build()
    }

    fn make_toolbox(interceptor: Arc<dyn ToolCallInterceptor>) -> Toolbox {
        build_test_toolbox(
            vec![interceptor],
            test_manifests(),
            vec![
                Arc::new(EchoTool) as Arc<dyn Tool>,
                Arc::new(FailTool) as Arc<dyn Tool>,
            ],
        )
    }

    async fn collect_call_events(
        tb: Arc<Toolbox>,
        turn: CancellationToken,
        call_id: &str,
        name: &str,
        arguments: Value,
    ) -> (
        Vec<ToolCallEvent>,
        Vec<ToolCallRequest>,
        Vec<ToolCallResult>,
    ) {
        let (sink, mut rx) = MpscToolCallEventSink::pair(16);
        let group = tb.begin_group(sink, turn).await;
        tb.call_tool(
            group,
            ToolCallRequest {
                call_id: call_id.to_string(),
                name: name.to_string(),
                arguments,
            },
        )
        .await
        .expect("start call");
        let mut out = Vec::new();
        let recv_fut = async {
            while let Some(ev) = rx.recv().await {
                out.push(ev);
            }
            out
        };
        let (events, end_result) = tokio::join!(recv_fut, tb.end_group(group));
        let (requests, results) = end_result.expect("end group");
        (events, requests, results)
    }

    async fn await_pending(policy: &AskUserPolicy, call_id: &str) {
        for _ in 0..2000 {
            if policy.pending_auth.lock().unwrap().contains_key(call_id) {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("toolbox never registered pending request {call_id}");
    }

    async fn recv_until_started(
        rx: &mut mpsc::Receiver<ToolCallEvent>,
        call_id: &str,
        buffer: &mut Vec<ToolCallEvent>,
    ) {
        while let Some(ev) = rx.recv().await {
            let started = ev.call_id == call_id && matches!(ev.kind, ToolCallEventKind::Started);
            buffer.push(ev);
            if started {
                return;
            }
        }
        panic!("tool call stream closed before Started for {call_id}");
    }

    async fn drain_receiver(mut rx: mpsc::Receiver<ToolCallEvent>) -> Vec<ToolCallEvent> {
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        out
    }

    async fn drain_into(rx: &mut mpsc::Receiver<ToolCallEvent>, buffer: &mut Vec<ToolCallEvent>) {
        while let Some(ev) = rx.recv().await {
            buffer.push(ev);
        }
    }

    #[tokio::test]
    async fn ask_user_allowed_emits_requested_then_permission_then_started_then_finished() {
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallInterceptor> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let collect_fut = tokio::spawn({
            let tb = tb_arc.clone();
            async move {
                collect_call_events(tb, CancellationToken::new(), "c1", "echo", json!("hi")).await
            }
        });
        await_pending(policy.as_ref(), "c1").await;
        policy.reply("c1", json!({ "allow": true })).await;
        let (events, ..) = collect_fut.await.expect("join");
        assert_eq!(
            events,
            vec![
                ToolCallEvent::requested("c1".into(), "echo".into(), json!("hi")),
                ToolCallEvent::extra(
                    "c1".into(),
                    json!({ "tool_name": "echo", "arguments": "hi" }),
                ),
                ToolCallEvent::started("c1".into()),
                ToolCallEvent::payload("c1".into(), r#"echo:"hi""#.into()),
                ToolCallEvent::finished("c1".into(), ToolCallStatus::Success)
            ]
        );
    }

    #[tokio::test]
    async fn ask_user_denied_emits_permission_then_finished_with_denied_marker() {
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallInterceptor> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let collect_fut = tokio::spawn({
            let tb = tb_arc.clone();
            async move {
                collect_call_events(tb, CancellationToken::new(), "c1", "echo", json!({})).await
            }
        });
        await_pending(policy.as_ref(), "c1").await;
        policy.reply("c1", json!({ "allow": false })).await;
        let (events, ..) = collect_fut.await.expect("join");
        assert_eq!(
            events,
            vec![
                ToolCallEvent::requested("c1".into(), "echo".into(), json!({})),
                ToolCallEvent::extra("c1".into(), json!({ "tool_name": "echo", "arguments": {} }),),
                ToolCallEvent::payload("c1".into(), TOOL_CALL_DENIED_BY_USER.into()),
                ToolCallEvent::finished("c1".into(), ToolCallStatus::Error)
            ]
        );
    }

    #[tokio::test]
    async fn tool_error_path_still_emits_finished() {
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallInterceptor> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let collect_fut = tokio::spawn({
            let tb = tb_arc.clone();
            async move {
                collect_call_events(tb, CancellationToken::new(), "c2", "bad", json!({})).await
            }
        });
        await_pending(policy.as_ref(), "c2").await;
        policy.reply("c2", json!({ "allow": true })).await;
        let (events, ..) = collect_fut.await.expect("join");
        assert_eq!(events.len(), 5);
        assert!(matches!(
            &events[0],
            ToolCallEvent {
                call_id,
                kind: ToolCallEventKind::Requested { name, arguments }
            } if call_id == "c2" && name == "bad" && arguments == &json!({})
        ));
        assert!(matches!(
            &events[1],
            ToolCallEvent { call_id, kind: ToolCallEventKind::Extra { .. } }
                if call_id == "c2"
        ));
        assert!(matches!(
            events[2],
            ToolCallEvent { ref call_id, kind: ToolCallEventKind::Started }
                if call_id == "c2"
        ));
        assert!(matches!(
            &events[3],
            ToolCallEvent {
                call_id,
                kind: ToolCallEventKind::Payload { text }
            } if call_id == "c2" && text.contains("boom")
        ));
        assert!(matches!(
            &events[4],
            ToolCallEvent {
                call_id,
                kind: ToolCallEventKind::Finished { status }
            } if call_id == "c2" && *status == ToolCallStatus::Error
        ));
    }

    #[tokio::test]
    async fn none_auth_skips_gate_and_runs_tool() {
        let tb = Arc::new(
            ToolboxBuilder::new()
                .manifests(test_manifests())
                .tool(Arc::new(EchoTool) as Arc<dyn Tool>)
                .build(),
        );
        let (out, ..) = collect_call_events(
            tb.clone(),
            CancellationToken::new(),
            "c0",
            "echo",
            json!("z"),
        )
        .await;
        assert_eq!(
            out,
            vec![
                ToolCallEvent::requested("c0".into(), "echo".into(), json!("z")),
                ToolCallEvent::started("c0".into()),
                ToolCallEvent::payload("c0".into(), r#"echo:"z""#.into()),
                ToolCallEvent::finished("c0".into(), ToolCallStatus::Success)
            ]
        );
    }

    #[tokio::test]
    async fn allow_decision_emits_requested_then_started_then_finished() {
        let tb = Arc::new(make_toolbox(Arc::new(StaticPolicy(StaticDecision::Allow))));
        let (out, ..) = collect_call_events(
            tb.clone(),
            CancellationToken::new(),
            "c3",
            "echo",
            json!("x"),
        )
        .await;
        assert_eq!(
            out,
            vec![
                ToolCallEvent::requested("c3".into(), "echo".into(), json!("x")),
                ToolCallEvent::started("c3".into()),
                ToolCallEvent::payload("c3".into(), r#"echo:"x""#.into()),
                ToolCallEvent::finished("c3".into(), ToolCallStatus::Success)
            ]
        );
    }

    #[tokio::test]
    async fn deny_decision_emits_requested_then_finished_with_denied_marker() {
        let tb = Arc::new(make_toolbox(Arc::new(StaticPolicy(StaticDecision::Deny))));
        let (out, ..) = collect_call_events(
            tb.clone(),
            CancellationToken::new(),
            "c4",
            "echo",
            json!({}),
        )
        .await;
        assert_eq!(
            out,
            vec![
                ToolCallEvent::requested("c4".into(), "echo".into(), json!({})),
                ToolCallEvent::payload("c4".into(), TOOL_CALL_DENIED_BY_USER.into()),
                ToolCallEvent::finished("c4".into(), ToolCallStatus::Error)
            ]
        );
    }

    /// Interceptor that records the arguments it observed, then replaces them.
    struct RewritePolicy {
        replacement: Value,
        seen: Arc<Mutex<Vec<Value>>>,
    }

    #[async_trait]
    impl ToolCallInterceptor for RewritePolicy {
        async fn intercept(
            &self,
            request: &mut ToolCallRequest,
            _manifest: Option<&ToolManifest>,
            _responder: Arc<dyn ToolCallResponder>,
            _cancellation: CancellationToken,
        ) -> bool {
            self.seen.lock().unwrap().push(request.arguments.clone());
            request.arguments = self.replacement.clone();
            true
        }
    }

    /// Interceptor that only records the arguments it observed.
    struct RecordPolicy {
        seen: Arc<Mutex<Vec<Value>>>,
    }

    #[async_trait]
    impl ToolCallInterceptor for RecordPolicy {
        async fn intercept(
            &self,
            request: &mut ToolCallRequest,
            _manifest: Option<&ToolManifest>,
            _responder: Arc<dyn ToolCallResponder>,
            _cancellation: CancellationToken,
        ) -> bool {
            self.seen.lock().unwrap().push(request.arguments.clone());
            true
        }
    }

    #[tokio::test]
    async fn interceptor_rewrite_flows_to_requested_transcript_and_tool() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let tb = Arc::new(make_toolbox(Arc::new(RewritePolicy {
            replacement: json!({ "msg": "rewritten" }),
            seen: seen.clone(),
        })));
        let (events, requests, results) = collect_call_events(
            tb,
            CancellationToken::new(),
            "c5",
            "echo",
            json!({ "msg": "original" }),
        )
        .await;
        assert_eq!(
            events,
            vec![
                ToolCallEvent::requested("c5".into(), "echo".into(), json!({ "msg": "rewritten" })),
                ToolCallEvent::started("c5".into()),
                ToolCallEvent::payload("c5".into(), r#"echo:{"msg":"rewritten"}"#.into()),
                ToolCallEvent::finished("c5".into(), ToolCallStatus::Success)
            ]
        );
        assert_eq!(requests[0].arguments, json!({ "msg": "rewritten" }));
        assert_eq!(results[0].status, ToolCallStatus::Success);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[json!({ "msg": "original" })]
        );
    }

    #[tokio::test]
    async fn interceptors_run_in_order_and_observe_rewrites() {
        let second_seen = Arc::new(Mutex::new(Vec::new()));
        let tb = Arc::new(build_test_toolbox(
            vec![
                Arc::new(RewritePolicy {
                    replacement: json!({ "step": 1 }),
                    seen: Arc::new(Mutex::new(Vec::new())),
                }),
                Arc::new(RecordPolicy {
                    seen: second_seen.clone(),
                }),
            ],
            test_manifests(),
            vec![Arc::new(EchoTool) as Arc<dyn Tool>],
        ));
        let (events, ..) = collect_call_events(
            tb,
            CancellationToken::new(),
            "c6",
            "echo",
            json!({ "step": 0 }),
        )
        .await;
        assert_eq!(
            second_seen.lock().unwrap().as_slice(),
            &[json!({ "step": 1 })]
        );
        assert!(matches!(
            &events[0],
            ToolCallEvent {
                kind: ToolCallEventKind::Requested { arguments, .. },
                ..
            } if arguments == &json!({ "step": 1 })
        ));
    }

    #[tokio::test]
    async fn interceptor_denial_short_circuits_chain() {
        let second_seen = Arc::new(Mutex::new(Vec::new()));
        let tb = Arc::new(build_test_toolbox(
            vec![
                Arc::new(StaticPolicy(StaticDecision::Deny)),
                Arc::new(RecordPolicy {
                    seen: second_seen.clone(),
                }),
            ],
            test_manifests(),
            vec![Arc::new(EchoTool) as Arc<dyn Tool>],
        ));
        let (events, ..) =
            collect_call_events(tb, CancellationToken::new(), "c7", "echo", json!({})).await;
        assert!(second_seen.lock().unwrap().is_empty());
        assert_eq!(
            events,
            vec![
                ToolCallEvent::requested("c7".into(), "echo".into(), json!({})),
                ToolCallEvent::payload("c7".into(), TOOL_CALL_DENIED_BY_USER.into()),
                ToolCallEvent::finished("c7".into(), ToolCallStatus::Error)
            ]
        );
    }

    #[tokio::test]
    async fn interceptor_denial_with_own_message_skips_fallback() {
        struct VerboseDeny;

        #[async_trait]
        impl ToolCallInterceptor for VerboseDeny {
            async fn intercept(
                &self,
                _request: &mut ToolCallRequest,
                _manifest: Option<&ToolManifest>,
                responder: Arc<dyn ToolCallResponder>,
                _cancellation: CancellationToken,
            ) -> bool {
                let _ = responder.send_text("not today".to_string()).await;
                false
            }
        }

        let tb = Arc::new(make_toolbox(Arc::new(VerboseDeny)));
        let (events, ..) =
            collect_call_events(tb, CancellationToken::new(), "c9", "echo", json!({})).await;
        assert_eq!(
            events,
            vec![
                ToolCallEvent::requested("c9".into(), "echo".into(), json!({})),
                ToolCallEvent::payload("c9".into(), "not today".into()),
                ToolCallEvent::finished("c9".into(), ToolCallStatus::Error)
            ]
        );
    }

    #[tokio::test]
    async fn interceptor_cannot_rewrite_call_id_or_name() {
        struct IdentityMangler;

        #[async_trait]
        impl ToolCallInterceptor for IdentityMangler {
            async fn intercept(
                &self,
                request: &mut ToolCallRequest,
                _manifest: Option<&ToolManifest>,
                _responder: Arc<dyn ToolCallResponder>,
                _cancellation: CancellationToken,
            ) -> bool {
                request.call_id = "mangled".into();
                request.name = "mangled".into();
                request.arguments = json!({ "ok": true });
                true
            }
        }

        let tb = Arc::new(make_toolbox(Arc::new(IdentityMangler)));
        let (events, requests, ..) =
            collect_call_events(tb, CancellationToken::new(), "c8", "echo", json!({})).await;
        assert_eq!(requests[0].call_id, "c8");
        assert_eq!(requests[0].name, "echo");
        assert_eq!(requests[0].arguments, json!({ "ok": true }));
        assert!(matches!(
            &events[0],
            ToolCallEvent {
                call_id,
                kind: ToolCallEventKind::Requested { name, arguments }
            } if call_id == "c8" && name == "echo" && arguments == &json!({ "ok": true })
        ));
    }

    fn make_toolbox_with_slow(interceptor: Arc<dyn ToolCallInterceptor>) -> Toolbox {
        build_test_toolbox(
            vec![interceptor],
            vec![ToolManifest {
                name: "slow".into(),
                description: "".into(),
                parameters: r#"{"type":"object"}"#.into(),
            }],
            vec![Arc::new(SlowTool) as Arc<dyn Tool>],
        )
    }

    #[tokio::test]
    async fn cancel_during_auth_finishes_canceled() {
        let turn = CancellationToken::new();
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallInterceptor> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let (sink, rx) = MpscToolCallEventSink::pair(16);
        let group = tb_arc.begin_group(sink, turn.clone()).await;
        tb_arc
            .call_tool(
                group,
                ToolCallRequest {
                    call_id: "cx".to_string(),
                    name: "echo".to_string(),
                    arguments: json!({}),
                },
            )
            .await
            .expect("start call");
        let collect_fut = tokio::spawn(drain_receiver(rx));
        await_pending(policy.as_ref(), "cx").await;
        turn.cancel();
        let (events, end_result) = tokio::join!(collect_fut, tb_arc.end_group(group));
        end_result.expect("end group");
        let events = events.expect("join");
        assert!(matches!(
            events.last(),
            Some(ToolCallEvent {
                call_id,
                kind: ToolCallEventKind::Finished { status }
            }) if call_id == "cx" && *status == ToolCallStatus::Canceled
        ));
        assert!(!events.iter().any(|ev| matches!(
            ev,
            ToolCallEvent {
                kind: ToolCallEventKind::Started,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn cancel_after_started_finishes_canceled() {
        let turn = CancellationToken::new();
        let tb_arc = Arc::new(make_toolbox_with_slow(Arc::new(StaticPolicy(
            StaticDecision::Allow,
        ))));
        let (sink, mut rx) = MpscToolCallEventSink::pair(16);
        let group = tb_arc.begin_group(sink, turn.clone()).await;
        tb_arc
            .call_tool(
                group,
                ToolCallRequest {
                    call_id: "cy".to_string(),
                    name: "slow".to_string(),
                    arguments: json!({}),
                },
            )
            .await
            .expect("start call");
        let mut events = Vec::new();
        recv_until_started(&mut rx, "cy", &mut events).await;
        turn.cancel();
        let drain = drain_into(&mut rx, &mut events);
        let (_, end_result) = tokio::join!(drain, tb_arc.end_group(group));
        end_result.expect("end group");
        assert!(events.iter().any(|ev| matches!(
            ev,
            ToolCallEvent {
                kind: ToolCallEventKind::Started,
                ..
            }
        )));
        assert!(matches!(
            events.last(),
            Some(ToolCallEvent {
                call_id,
                kind: ToolCallEventKind::Finished { status }
            }) if call_id == "cy" && *status == ToolCallStatus::Canceled
        ));
    }
}
