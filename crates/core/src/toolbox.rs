//! Tool registry, optional authorization gate, and lifecycle stream returned by [`Toolbox::call_tool`].

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use async_trait::async_trait;
use futures::Stream;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::tool::Tool;
use crate::types::MorayError;
use crate::types::{
    parse_tool_call_args, ToolCallRequest, ToolCallResult, ToolCallStatus, ToolManifest,
};

/// Restricted handle for emitting [`ToolCallEvent::Custom`] during
/// [`ToolCallAuthorizer::request`]. Toolbox and clients use this channel for opaque payloads (for
/// example authorization prompts or future tool progress); only [`ToolCallResponder::send_custom`]
/// is exposed so policies cannot emit unrelated lifecycle events.
#[derive(Clone)]
pub struct ToolCallResponder {
    call_id: String,
    tx: mpsc::Sender<ToolCallEvent>,
}

impl ToolCallResponder {
    pub(crate) fn new(call_id: String, tx: mpsc::Sender<ToolCallEvent>) -> Self {
        Self { call_id, tx }
    }

    /// Forwards a [`ToolCallEvent::Custom`] on the tool-call lifecycle stream. If the stream has
    /// been dropped, the send is ignored.
    pub async fn send_custom(&self, data: Option<Value>) {
        let _ = self
            .tx
            .send(ToolCallEvent::Custom {
                call_id: self.call_id.clone(),
                data,
            })
            .await;
    }
}

#[async_trait]
pub trait ToolCallAuthorizer: Send + Sync {
    /// Returns whether the tool call may proceed. `true` means execute the tool; `false` means
    /// emit a single [`ToolCallEvent::Completed`] with [`TOOL_CALL_DENIED_BY_USER`] and skip
    /// execution.
    async fn request(
        &self,
        call_id: &str,
        tool_name: &str,
        args: &Value,
        responder: ToolCallResponder,
    ) -> bool;
    async fn reply(&self, call_id: &str, data: Value) -> Result<(), ToolboxError>;
}

/// Errors from tool-call authorization flows surfaced by [`Toolbox`] and [`ToolCallAuthorizer::reply`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolboxError {
    /// No in-flight authorization is waiting for this `call_id` (expired, wrong id, or already completed).
    #[error("no pending tool authorization for call_id {call_id}")]
    NoPendingAuthorization { call_id: String },

    /// This authorizer does not accept out-of-band replies (for example a purely synchronous policy).
    #[error("no pending tool authorization for this policy")]
    ReplyNotSupported,
}

pub const TOOL_CALL_DENIED_BY_USER: &str = "This tool call was denied by the user.";

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum ToolCallEvent {
    Requested {
        content: ToolCallRequest,
    },
    Custom {
        call_id: String,
        data: Option<Value>,
    },
    Started {
        call_id: String,
    },
    Completed {
        content: ToolCallResult,
    },
}

#[derive(Clone)]
pub struct Toolbox {
    tools: HashMap<String, Arc<dyn Tool>>,
    manifests: Vec<ToolManifest>,
    auth: Option<Arc<dyn ToolCallAuthorizer>>,
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
            tools,
            manifests,
            auth,
        }
    }

    pub async fn list_tools(&self) -> Vec<ToolManifest> {
        self.manifests.clone()
    }

    pub fn call_tool(
        self: Arc<Self>,
        call_id: &str,
        name: &str,
        arguments: &str,
    ) -> Pin<Box<dyn Stream<Item = ToolCallEvent> + Send>> {
        let (tx, rx) = mpsc::channel::<ToolCallEvent>(16);
        let call_id_owned = call_id.to_string();
        let name_owned = name.to_string();
        let arguments_owned = arguments.to_string();
        let args_value = parse_tool_call_args(&arguments_owned);
        let this = Arc::clone(&self);

        tokio::spawn(async move {
            async fn emit(tx: &mpsc::Sender<ToolCallEvent>, ev: ToolCallEvent) -> bool {
                tx.send(ev).await.is_ok()
            }

            if !emit(
                &tx,
                ToolCallEvent::Requested {
                    content: ToolCallRequest {
                        call_id: call_id_owned.clone(),
                        name: name_owned.clone(),
                        arguments: arguments_owned.clone(),
                    },
                },
            )
            .await
            {
                return;
            }

            let allowed = match &this.auth {
                Some(auth) => {
                    let responder = ToolCallResponder::new(call_id_owned.clone(), tx.clone());
                    auth.request(
                        call_id_owned.as_str(),
                        name_owned.as_str(),
                        &args_value,
                        responder,
                    )
                    .await
                }
                None => true,
            };

            if !allowed {
                let _ = emit(
                    &tx,
                    ToolCallEvent::Completed {
                        content: ToolCallResult {
                            call_id: call_id_owned.clone(),
                            content: TOOL_CALL_DENIED_BY_USER.to_string(),
                            status: ToolCallStatus::Error,
                        },
                    },
                )
                .await;
                return;
            }

            if !emit(
                &tx,
                ToolCallEvent::Started {
                    call_id: call_id_owned.clone(),
                },
            )
            .await
            {
                return;
            }

            let result = match this.dispatch(&name_owned, args_value).await {
                Ok(content) => ToolCallResult {
                    call_id: call_id_owned.clone(),
                    content,
                    status: ToolCallStatus::Success,
                },
                Err(e) => ToolCallResult {
                    call_id: call_id_owned,
                    content: format!("tool error: {e}"),
                    status: ToolCallStatus::Error,
                },
            };
            let _ = emit(&tx, ToolCallEvent::Completed { content: result }).await;
        });

        Box::pin(ReceiverStream::new(rx))
    }

    async fn dispatch(&self, name: &str, args: Value) -> Result<String, MorayError> {
        let Some(tool) = self.tools.get(name) else {
            return Err(MorayError::Message(format!("unknown tool {name}")));
        };
        tool.call(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::TypedTool;
    use futures::StreamExt;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokio::sync::oneshot;

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
            _tool_name: &str,
            _args: &Value,
            responder: ToolCallResponder,
        ) -> bool {
            let (tx, rx) = oneshot::channel();
            self.pending_auth
                .lock()
                .expect("ask-user pending-auth mutex poisoned")
                .insert(call_id.to_string(), tx);
            responder.send_custom(None).await;
            rx.await.unwrap_or(false)
        }

        async fn reply(&self, call_id: &str, data: Value) -> Result<(), ToolboxError> {
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
        async fn request(&self, _: &str, _: &str, _: &Value, _: ToolCallResponder) -> bool {
            match self.0 {
                StaticDecision::Allow => true,
                StaticDecision::Deny => false,
            }
        }

        async fn reply(&self, _: &str, _: Value) -> Result<(), ToolboxError> {
            Err(ToolboxError::ReplyNotSupported)
        }
    }

    struct EchoTool;

    #[async_trait]
    impl TypedTool for EchoTool {
        type Args = Value;
        const NAME: &'static str = "echo";

        async fn run(&self, args: Value) -> Result<String, MorayError> {
            Ok(format!(
                "echo:{}",
                serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
            ))
        }
    }

    struct FailTool;

    #[async_trait]
    impl TypedTool for FailTool {
        type Args = Value;
        const NAME: &'static str = "bad";

        async fn run(&self, _: Value) -> Result<String, MorayError> {
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

    fn make_toolbox(auth: Arc<dyn ToolCallAuthorizer>) -> Toolbox {
        ToolboxBuilder::new()
            .manifests(test_manifests())
            .tool(Arc::new(EchoTool) as Arc<dyn Tool>)
            .tool(Arc::new(FailTool) as Arc<dyn Tool>)
            .auth(auth)
            .build()
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

    #[tokio::test]
    async fn ask_user_allowed_emits_requested_then_permission_then_started_then_finished() {
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallAuthorizer> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let call_fut = {
            let tb = tb_arc.clone();
            tokio::spawn(async move {
                let mut s = tb.call_tool("c1", "echo", r#""hi""#);
                let mut out = Vec::new();
                while let Some(ev) = s.next().await {
                    out.push(ev);
                }
                out
            })
        };
        await_pending(policy.as_ref(), "c1").await;
        policy
            .reply("c1", json!({ "allow": true }))
            .await
            .expect("reply");
        let events = call_fut.await.expect("join");
        assert_eq!(
            events,
            vec![
                ToolCallEvent::Requested {
                    content: ToolCallRequest {
                        call_id: "c1".into(),
                        name: "echo".into(),
                        arguments: r#""hi""#.into(),
                    },
                },
                ToolCallEvent::Custom {
                    call_id: "c1".into(),
                    data: None,
                },
                ToolCallEvent::Started {
                    call_id: "c1".into()
                },
                ToolCallEvent::Completed {
                    content: ToolCallResult {
                        call_id: "c1".into(),
                        content: r#"echo:"hi""#.into(),
                        status: ToolCallStatus::Success,
                    }
                }
            ]
        );
    }

    #[tokio::test]
    async fn ask_user_denied_emits_permission_then_finished_with_denied_marker() {
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallAuthorizer> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let call_fut = {
            let tb = tb_arc.clone();
            tokio::spawn(async move {
                let mut s = tb.call_tool("c1", "echo", "{}");
                let mut out = Vec::new();
                while let Some(ev) = s.next().await {
                    out.push(ev);
                }
                out
            })
        };
        await_pending(policy.as_ref(), "c1").await;
        policy
            .reply("c1", json!({ "allow": false }))
            .await
            .expect("deny");
        let events = call_fut.await.expect("join");
        assert_eq!(
            events,
            vec![
                ToolCallEvent::Requested {
                    content: ToolCallRequest {
                        call_id: "c1".into(),
                        name: "echo".into(),
                        arguments: "{}".into(),
                    },
                },
                ToolCallEvent::Custom {
                    call_id: "c1".into(),
                    data: None,
                },
                ToolCallEvent::Completed {
                    content: ToolCallResult {
                        call_id: "c1".into(),
                        content: TOOL_CALL_DENIED_BY_USER.into(),
                        status: ToolCallStatus::Error,
                    }
                }
            ]
        );
    }

    #[tokio::test]
    async fn tool_error_path_still_emits_finished() {
        let policy = Arc::new(AskUserPolicy::new());
        let policy_obj: Arc<dyn ToolCallAuthorizer> = policy.clone();
        let tb_arc = Arc::new(make_toolbox(policy_obj));
        let call_fut = {
            let tb = tb_arc.clone();
            tokio::spawn(async move {
                let mut s = tb.call_tool("c2", "bad", "{}");
                let mut out = Vec::new();
                while let Some(ev) = s.next().await {
                    out.push(ev);
                }
                out
            })
        };
        await_pending(policy.as_ref(), "c2").await;
        policy
            .reply("c2", json!({ "allow": true }))
            .await
            .expect("reply");
        let events = call_fut.await.expect("join");
        assert_eq!(events.len(), 4);
        assert!(matches!(
            &events[0],
            ToolCallEvent::Requested { content }
                if content.call_id == "c2" && content.name == "bad" && content.arguments == "{}"
        ));
        assert!(matches!(
            &events[1],
            ToolCallEvent::Custom { call_id, .. } if call_id == "c2"
        ));
        assert!(matches!(
            events[2],
            ToolCallEvent::Started { ref call_id } if call_id == "c2"
        ));
        assert!(matches!(
            &events[3],
            ToolCallEvent::Completed { content }
                if content.call_id == "c2" && content.content.contains("boom") && content.status == ToolCallStatus::Error
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
        let mut s = tb.clone().call_tool("c0", "echo", r#""z""#);
        let mut out = Vec::new();
        while let Some(ev) = s.next().await {
            out.push(ev);
        }
        assert_eq!(
            out,
            vec![
                ToolCallEvent::Requested {
                    content: ToolCallRequest {
                        call_id: "c0".into(),
                        name: "echo".into(),
                        arguments: r#""z""#.into(),
                    },
                },
                ToolCallEvent::Started {
                    call_id: "c0".into()
                },
                ToolCallEvent::Completed {
                    content: ToolCallResult {
                        call_id: "c0".into(),
                        content: r#"echo:"z""#.into(),
                        status: ToolCallStatus::Success,
                    }
                }
            ]
        );
    }

    #[tokio::test]
    async fn allow_decision_emits_requested_then_started_then_finished() {
        let tb = Arc::new(make_toolbox(Arc::new(StaticPolicy(StaticDecision::Allow))));
        let mut s = tb.clone().call_tool("c3", "echo", r#""x""#);
        let mut out = Vec::new();
        while let Some(ev) = s.next().await {
            out.push(ev);
        }
        assert_eq!(
            out,
            vec![
                ToolCallEvent::Requested {
                    content: ToolCallRequest {
                        call_id: "c3".into(),
                        name: "echo".into(),
                        arguments: r#""x""#.into(),
                    },
                },
                ToolCallEvent::Started {
                    call_id: "c3".into()
                },
                ToolCallEvent::Completed {
                    content: ToolCallResult {
                        call_id: "c3".into(),
                        content: r#"echo:"x""#.into(),
                        status: ToolCallStatus::Success,
                    }
                }
            ]
        );
    }

    #[tokio::test]
    async fn deny_decision_emits_requested_then_finished_with_denied_marker() {
        let tb = Arc::new(make_toolbox(Arc::new(StaticPolicy(StaticDecision::Deny))));
        let mut s = tb.clone().call_tool("c4", "echo", "{}");
        let mut out = Vec::new();
        while let Some(ev) = s.next().await {
            out.push(ev);
        }
        assert_eq!(
            out,
            vec![
                ToolCallEvent::Requested {
                    content: ToolCallRequest {
                        call_id: "c4".into(),
                        name: "echo".into(),
                        arguments: "{}".into(),
                    },
                },
                ToolCallEvent::Completed {
                    content: ToolCallResult {
                        call_id: "c4".into(),
                        content: TOOL_CALL_DENIED_BY_USER.into(),
                        status: ToolCallStatus::Error,
                    }
                }
            ]
        );
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
