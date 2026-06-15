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

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::types::{
    MorayError, ToolCallRequest, ToolCallResult, ToolCallStatus,
    ToolManifest,
};

// ---------------------------------------------------------------------------
// Constants & errors
// ---------------------------------------------------------------------------

/// Shown in the tool result when authorization denies execution.
pub const TOOL_CALL_DENIED_BY_USER: &str = "This tool call was denied by the user.";

/// Shown in the tool result when the turn or tool-call group is canceled.
pub const TOOL_CALL_CANCELED: &str = "This tool call was canceled.";

/// Errors from [`Toolbox`], [`ToolCallAuthorizer::reply`], and related flows.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolboxError {
    #[error("unknown tool {name}")]
    UnknownTool { name: String },

    #[error("unknown tool call group {group_id}")]
    UnknownGroup { group_id: u64 },

    #[error("no pending tool authorization for call_id {call_id}")]
    NoPendingAuthorization { call_id: String },

    /// [`ToolCallEventSink::emit`] rejected the event (agent stream closed or backpressure).
    #[error("tool call event sink closed")]
    EventSinkClosed,

    /// Local output path failed before reaching the sink (e.g. stdout flush in CLI).
    #[error("failed to deliver tool call output: {reason}")]
    DeliverFailed { reason: String },
}

/// Authorization failures from [`ToolCallAuthorizer::reply`] (subset of [`ToolboxError`]).
pub type ToolCallAuthError = ToolboxError;

impl From<ToolboxError> for MorayError {
    fn from(value: ToolboxError) -> Self {
        MorayError::Message(value.to_string())
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
    Payload { text: String },

    /// Out-of-band metadata (authorization prompts, progress). Not model `tool` message content.
    Extra { data: Value },

    Finished { status: ToolCallStatus },
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
    /// (`Requested`, `Started`, `Finished`) are emitted only by [`Toolbox`].
    async fn call(&self, args: Value, responder: &dyn ToolCallResponder) -> Result<(), MorayError>;
}

/// Typed tool: per-tool [`Args`](Self::Args) + [`run`](Self::run). Metadata (description, JSON schema) comes from the app-layer tool catalog.
#[async_trait]
pub trait TypedTool: Send + Sync {
    type Args: DeserializeOwned + Send;
    const NAME: &'static str;

    /// Emit model-visible output via `responder` (supports streaming); return only on failure.
    async fn run(
        &self,
        args: Self::Args,
        responder: &dyn ToolCallResponder,
    ) -> Result<(), MorayError>;
}

#[async_trait]
impl<T> Tool for T
where
    T: TypedTool + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        T::NAME
    }

    async fn call(&self, args: Value, responder: &dyn ToolCallResponder) -> Result<(), MorayError> {
        let tool_name = T::NAME;
        let args = serde_json::from_value(args).map_err(|e| {
            MorayError::Message(format!("{tool_name}: invalid JSON arguments: {e}"))
        })?;
        self.run(args, responder)
            .await
            .map_err(|e| MorayError::Message(format!("{tool_name}: {e}")))
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

/// Optional gate before tool execution within a [`ToolCallGroup`].
#[async_trait]
pub trait ToolCallAuthorizer: Send + Sync {
    /// Returns whether the tool call may proceed. `true` means execute the tool; `false` means
    /// emit [`ToolCallEventKind::Finished`] with [`ToolCallStatus::Error`] and skip execution.
    async fn request(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &Value,
        responder: Arc<dyn ToolCallResponder>,
    ) -> bool;

    /// Resolves a user authorization reply for `call_id`. Override when the authorizer keeps pending state.
    async fn reply(&self, call_id: &str, _data: Value) -> Result<(), ToolCallAuthError> {
        Err(ToolboxError::NoPendingAuthorization {
            call_id: call_id.to_string(),
        })
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
    request: ToolCallRequest,
    sink: Arc<dyn ToolCallEventSink>,
    content: Arc<Mutex<String>>,
    result: Arc<Mutex<Option<ToolCallResult>>>,
}

impl ToolCallTracker {
    fn new(request: ToolCallRequest, sink: Arc<dyn ToolCallEventSink>) -> Self {
        Self {
            request,
            sink,
            content: Arc::new(Mutex::new(String::new())),
            result: Arc::new(Mutex::new(None)),
        }
    }

    fn request(&self) -> &ToolCallRequest {
        &self.request
    }

    async fn take_result(&self) -> ToolCallResult {
        self.result
            .lock()
            .await
            .take()
            .unwrap_or_else(|| ToolCallResult {
                call_id: self.request.call_id.clone(),
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
            call_id: self.request.call_id.clone(),
            content,
            status: status.clone(),
        });

        self.sink
            .emit(ToolCallEvent::finished(
                self.request.call_id.clone(),
                status,
            ))
            .await
    }
}

#[async_trait]
impl ToolCallResponder for ToolCallTracker {
    async fn send_extra(&self, data: Value) -> Result<(), ToolboxError> {
        if self
            .emit(ToolCallEvent::extra(
                self.request.call_id.clone(),
                data,
            ))
            .await
        {
            Ok(())
        } else {
            Err(ToolboxError::EventSinkClosed)
        }
    }

    async fn send_text(&self, text: String) -> Result<(), ToolboxError> {
        self.content.lock().await.push_str(&text);
        if self
            .emit(ToolCallEvent::payload(
                self.request.call_id.clone(),
                text,
            ))
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
        auth: Option<Arc<dyn ToolCallAuthorizer>>,
        request: ToolCallRequest,
    ) {
        let tracker = Arc::new(ToolCallTracker::new(request, self.sink.clone()));
        self.tool_calls.push(tracker.clone());
        let cancellation = self.cancellation.clone();
        self.join_set
            .spawn(run_call(tool, auth, tracker, cancellation));
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
            requests.push(tracker.request().clone());
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
    auth: Option<Arc<dyn ToolCallAuthorizer>>,
    tracker: Arc<ToolCallTracker>,
    cancellation: CancellationToken,
) {
    if cancellation.is_cancelled() {
        finish_canceled(&tracker).await;
        return;
    }

    let call_id = tracker.request.call_id.clone();
    let name = tracker.request.name.clone();
    let arguments = tracker.request.arguments.clone();

    if !tracker
        .emit(ToolCallEvent::requested(
            call_id.clone(),
            name.clone(),
            arguments.clone(),
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

    if cancellation.is_cancelled() {
        finish_canceled(&tracker).await;
        return;
    }

    let allowed = match &auth {
        Some(auth) => {
            tokio::select! {
                _ = cancellation.cancelled() => {
                    finish_canceled(&tracker).await;
                    return;
                }
                allowed = auth.request(
                    call_id.as_str(),
                    name.as_str(),
                    &arguments,
                    Arc::clone(&tracker) as Arc<dyn ToolCallResponder>,
                ) => allowed,
            }
        }
        None => true,
    };

    if !allowed {
        let _ = tracker
            .send_text(TOOL_CALL_DENIED_BY_USER.to_string())
            .await;
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

    let status = match tokio::select! {
        _ = cancellation.cancelled() => {
            finish_canceled(&tracker).await;
            return;
        }
        res = tool.call(arguments, tracker.as_ref()) => res,
    } {
        Ok(()) => ToolCallStatus::Success,
        Err(e) => {
            let _ = tracker
                .send_text(format!("tool error: {e}"))
                .await;
            ToolCallStatus::Error
        }
    };

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
    auth: Option<Arc<dyn ToolCallAuthorizer>>,
    groups: Arc<Mutex<HashMap<ToolCallGroupId, ToolCallGroup>>>,
    next_group_id: Arc<AtomicU64>,
}

#[derive(Default)]
pub struct ToolboxBuilder {
    tools: HashMap<String, Arc<dyn Tool>>,
    manifests: Vec<ToolManifest>,
    auth: Option<Arc<dyn ToolCallAuthorizer>>,
}

impl ToolboxBuilder {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            manifests: Vec::new(),
            auth: None,
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

    pub fn auth(mut self, auth: Arc<dyn ToolCallAuthorizer>) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn build(self) -> Toolbox {
        Toolbox::new(self.tools, self.manifests, self.auth)
    }
}

impl Toolbox {
    pub fn new(
        tools: HashMap<String, Arc<dyn Tool>>,
        manifests: Vec<ToolManifest>,
        auth: Option<Arc<dyn ToolCallAuthorizer>>,
    ) -> Self {
        Self {
            tools: Arc::new(tools),
            manifests,
            auth,
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
        let tool = self
            .tools
            .get(&request.name)
            .cloned()
            .ok_or_else(|| ToolboxError::UnknownTool {
                name: request.name.clone(),
            })?;

        let mut groups = self.groups.lock().await;

        let group = groups
            .get_mut(&group_id)
            .ok_or_else(|| ToolboxError::UnknownGroup {
                group_id,
            })?;

        group.start_call(tool, self.auth.clone(), request);

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
        assert_eq!(v, json!({
            "call_id": "c1",
            "type": "requested",
            "name": "echo",
            "arguments": "hi"
        }));
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
    use std::time::Duration;
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
    }

    #[async_trait]
    impl ToolCallAuthorizer for AskUserPolicy {
        async fn request(
            &self,
            call_id: &str,
            tool_name: &str,
            args: &Value,
            responder: Arc<dyn ToolCallResponder>,
        ) -> bool {
            let (tx, rx) = oneshot::channel();
            self.pending_auth
                .lock()
                .expect("ask-user pending-auth mutex poisoned")
                .insert(call_id.to_string(), tx);
            if responder
                .send_extra(json!({
                    "tool_name": tool_name,
                    "arguments": args,
                }))
                .await
                .is_err()
            {
                self.pending_auth
                    .lock()
                    .expect("ask-user pending-auth mutex poisoned")
                    .remove(call_id);
                return false;
            }
            rx.await.unwrap_or(false)
        }

        async fn reply(&self, call_id: &str, data: Value) -> Result<(), ToolCallAuthError> {
            let tx = self
                .pending_auth
                .lock()
                .expect("ask-user pending-auth mutex poisoned")
                .remove(call_id)
                .ok_or_else(|| ToolboxError::NoPendingAuthorization {
                    call_id: call_id.to_string(),
                })?;
            let allow = data.get("allow").and_then(|v| v.as_bool()).unwrap_or(false);
            let _ = tx.send(allow);
            Ok(())
        }
    }

    #[derive(Clone, Copy)]
    enum StaticDecision {
        Allow,
        Deny,
    }

    struct StaticPolicy(StaticDecision);

    #[async_trait]
    impl ToolCallAuthorizer for StaticPolicy {
        async fn request(&self, _: &str, _: &str, _: &Value, _: Arc<dyn ToolCallResponder>) -> bool {
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
        ) -> Result<(), MorayError> {
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
            responder: &dyn ToolCallResponder,
        ) -> Result<(), MorayError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            responder.send_text("slow".into()).await?;
            Ok(())
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
        ) -> Result<(), MorayError> {
            Err(MorayError::Message("boom".into()))
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
        auth: Arc<dyn ToolCallAuthorizer>,
        manifests: Vec<ToolManifest>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Toolbox {
        let mut builder = ToolboxBuilder::new().manifests(manifests).auth(auth);
        for tool in tools {
            builder = builder.tool(tool);
        }
        builder.build()
    }

    fn make_toolbox(auth: Arc<dyn ToolCallAuthorizer>) -> Toolbox {
        build_test_toolbox(
            auth,
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
    ) -> Vec<ToolCallEvent> {
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
        end_result.expect("end group");
        events
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
            let started = ev.call_id == call_id
                && matches!(ev.kind, ToolCallEventKind::Started);
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
        let policy_obj: Arc<dyn ToolCallAuthorizer> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let collect_fut = tokio::spawn({
            let tb = tb_arc.clone();
            async move {
                collect_call_events(tb, CancellationToken::new(), "c1", "echo", json!("hi")).await
            }
        });
        await_pending(policy.as_ref(), "c1").await;
        policy
            .reply("c1", json!({ "allow": true }))
            .await
            .expect("reply");
        let events = collect_fut.await.expect("join");
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
        let policy_obj: Arc<dyn ToolCallAuthorizer> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let collect_fut = tokio::spawn({
            let tb = tb_arc.clone();
            async move {
                collect_call_events(tb, CancellationToken::new(), "c1", "echo", json!({})).await
            }
        });
        await_pending(policy.as_ref(), "c1").await;
        policy
            .reply("c1", json!({ "allow": false }))
            .await
            .expect("deny");
        let events = collect_fut.await.expect("join");
        assert_eq!(
            events,
            vec![
                ToolCallEvent::requested("c1".into(), "echo".into(), json!({})),
                ToolCallEvent::extra(
                    "c1".into(),
                    json!({ "tool_name": "echo", "arguments": {} }),
                ),
                ToolCallEvent::payload("c1".into(), TOOL_CALL_DENIED_BY_USER.into()),
                ToolCallEvent::finished("c1".into(), ToolCallStatus::Error)
            ]
        );
    }

    #[tokio::test]
    async fn tool_error_path_still_emits_finished() {
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallAuthorizer> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let collect_fut = tokio::spawn({
            let tb = tb_arc.clone();
            async move {
                collect_call_events(tb, CancellationToken::new(), "c2", "bad", json!({})).await
            }
        });
        await_pending(policy.as_ref(), "c2").await;
        policy
            .reply("c2", json!({ "allow": true }))
            .await
            .expect("reply");
        let events = collect_fut.await.expect("join");
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
        let out =
            collect_call_events(tb.clone(), CancellationToken::new(), "c0", "echo", json!("z"))
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
        let out =
            collect_call_events(tb.clone(), CancellationToken::new(), "c3", "echo", json!("x"))
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
        let out =
            collect_call_events(tb.clone(), CancellationToken::new(), "c4", "echo", json!({})).await;
        assert_eq!(
            out,
            vec![
                ToolCallEvent::requested("c4".into(), "echo".into(), json!({})),
                ToolCallEvent::payload("c4".into(), TOOL_CALL_DENIED_BY_USER.into()),
                ToolCallEvent::finished("c4".into(), ToolCallStatus::Error)
            ]
        );
    }

    fn make_toolbox_with_slow(auth: Arc<dyn ToolCallAuthorizer>) -> Toolbox {
        build_test_toolbox(
            auth,
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
        let policy_obj: Arc<dyn ToolCallAuthorizer> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let (sink, rx) = MpscToolCallEventSink::pair(16);
        let group = tb_arc
            .begin_group(sink, turn.clone())
            .await;
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
        let (events, end_result) =
            tokio::join!(collect_fut, tb_arc.end_group(group));
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
        let group = tb_arc
            .begin_group(sink, turn.clone())
            .await;
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

    #[tokio::test]
    async fn reply_unknown_call_id_errors() {
        let policy = AskUserPolicy::new();
        let err = policy
            .reply("missing", json!({ "allow": true }))
            .await
            .expect_err("should error");
        assert_eq!(
            err,
            ToolboxError::NoPendingAuthorization {
                call_id: "missing".into()
            }
        );
    }
}
