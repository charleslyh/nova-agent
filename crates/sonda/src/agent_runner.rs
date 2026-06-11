//! [`SondaAgentRunner`] and sub-agent tool execution for Sonda live chat sessions.

use std::sync::Arc;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use moray_core::{
    AgentFinishKind, AgentRequestBuilder, AgentResponseEvent, ChatCompletionResponseChunk,
    ContextEngine, MorayError, Tool, ToolCallResponder,
    ToolManifest, Toolbox, TypedTool,
};
use moray_extensions::context::CompositeContextEngineBuilder;
use moray_extensions::preambles::{SkillsSection, TemplatedPreamblerBuilder};
use moray_session::{
    AgentRole, AgentRunner, SessionAgentResponse, SessionError, SessionEvent, SessionEventKind,
    SessionEventSink, TurnInput,
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::completion_factory::SondaCompletionFactory;
use crate::error::{Result, SondaError};
use crate::session_catalog::{SessionSubAgentEntry, SubAgentContextMode};
use crate::skill_center::{SkillCenter, SkillFilterKind};
use crate::toolbox_factory::SondaToolboxFactory;
use crate::{SondaSessionCatalog, SondaSettingsStore};

/// Optional per-event hook for [`drain_agent_stream`]; sink emission is handled by the drain loop.
trait AgentStreamHandler {
    fn on_event(&mut self, _ev: &AgentResponseEvent) {}
}

struct LeaderStreamHandler;

impl AgentStreamHandler for LeaderStreamHandler {}

struct SubStreamHandler {
    pending_text: String,
    final_text: Option<String>,
    exit_error: Option<String>,
}

impl SubStreamHandler {
    fn new() -> Self {
        Self {
            pending_text: String::new(),
            final_text: None,
            exit_error: None,
        }
    }

    fn into_result(self) -> std::result::Result<String, String> {
        if let Some(err) = self.exit_error {
            return Err(err);
        }
        Ok(self.final_text.unwrap_or_default())
    }

    fn capture_pending_if_nonempty(&mut self) {
        let text = std::mem::take(&mut self.pending_text);
        if !text.trim().is_empty() {
            self.final_text = Some(text);
        }
    }
}

impl AgentStreamHandler for SubStreamHandler {
    fn on_event(&mut self, ev: &AgentResponseEvent) {
        match ev {
            AgentResponseEvent::Started => {
                self.pending_text.clear();
            }
            AgentResponseEvent::CompletionResponse { chunk } => match chunk {
                ChatCompletionResponseChunk::TextBlock(s) => {
                    self.pending_text.push_str(s);
                }
                ChatCompletionResponseChunk::ToolCall(_) => {
                    self.pending_text.clear();
                }
                ChatCompletionResponseChunk::Done { .. } => {
                    self.capture_pending_if_nonempty();
                }
                _ => {}
            },
            AgentResponseEvent::Finished { kind } => match kind {
                AgentFinishKind::Succeeded => {
                    self.capture_pending_if_nonempty();
                }
                AgentFinishKind::Canceled => {
                    self.exit_error = Some("sub agent run canceled".into());
                }
                AgentFinishKind::Refused { reason } => {
                    self.exit_error = Some(
                        reason
                            .clone()
                            .filter(|r| !r.trim().is_empty())
                            .unwrap_or_else(|| "sub agent refused".into()),
                    );
                }
                AgentFinishKind::Failed { reason } => {
                    self.exit_error = Some(format!("sub agent failed: {reason}"));
                }
            },
            AgentResponseEvent::ToolCall { .. } => {}
        }
    }
}

/// Drain an agent response stream into the session sink, with optional per-event handling.
async fn drain_agent_stream<S, H>(
    mut stream: S,
    cancellation: &CancellationToken,
    sink: &dyn SessionEventSink,
    session_id: &str,
    agent_id: &str,
    role: AgentRole,
    mut handler: H,
) -> std::result::Result<H, MorayError>
where
    S: Stream<Item = AgentResponseEvent> + Unpin,
    H: AgentStreamHandler,
{
    while let Some(ev) = stream.next().await {
        if cancellation.is_cancelled() {
            break;
        }

        handler.on_event(&ev);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let kind = SessionEventKind::AgentResponse(SessionAgentResponse {
            agent_id: agent_id.to_string(),
            role,
            event: ev,
        });

        sink.append(&SessionEvent {
            session_id: session_id.to_string(),
            ts,
            kind,
        })?;
    }

    Ok(handler)
}

pub const RUN_SUB_AGENT_TOOL_NAME: &str = "run_sub_agent";

const RUN_SUB_AGENT_PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {
    "agent_id": { "type": "string", "description": "Sub-agent id to run" },
    "task": { "type": "string", "description": "Task description for the sub-agent" },
    "context": {
      "type": "string",
      "enum": ["isolated", "branch"],
      "description": "Optional override; omit to use the sub-agent's session default. isolated: delegate a standalone sub-task—the sub-agent sees only task, so put all required facts, constraints, and inputs into task. Prefer for independent or parallel work, specialist runs, or when prior chat is irrelevant/noisy. branch: delegate work that depends on prior conversation—the sub-agent inherits the leader transcript plus task. Prefer for follow-ups, references to earlier user messages, disambiguation, or continuing the same thread."
    }
  },
  "required": ["agent_id", "task"]
}"#;

#[derive(Debug, Deserialize)]
pub(crate) struct RunSubAgentArgs {
    agent_id: String,
    task: String,
    #[serde(default)]
    context: Option<String>,
}

pub(crate) struct SondaSubAgentTrigger {
    toolbox_factory: Arc<SondaToolboxFactory>,
    stream: bool,
    session_id: String,
    sub_agents: Vec<SessionSubAgentEntry>,
    settings_store: Arc<SondaSettingsStore>,
    completion_factory: Arc<SondaCompletionFactory>,
    skill_center: SkillCenter,
    leader_context: Arc<dyn ContextEngine>,
    sink: Arc<dyn SessionEventSink>,
    cancellation: CancellationToken,
}

#[async_trait]
impl TypedTool for SondaSubAgentTrigger {
    type Args = RunSubAgentArgs;
    const NAME: &'static str = RUN_SUB_AGENT_TOOL_NAME;

    async fn run(
        &self,
        args: Self::Args,
        responder: &dyn ToolCallResponder,
    ) -> std::result::Result<(), MorayError> {
        let agent_id = args.agent_id.as_str();
        let entry = self
            .sub_agents
            .iter()
            .find(|e| e.agent_id == agent_id)
            .ok_or_else(|| {
                MorayError::Message(format!("unknown sub-agent id `{agent_id}` for this session"))
            })?;

        if !self.settings_store.has_agent(agent_id) {
            return Err(MorayError::Message(format!(
                "sub-agent `{agent_id}` is not defined in settings"
            )));
        }

        let mode = resolve_context_mode(args.context.as_deref(), entry.context_mode)?;

        let sub_context = build_sub_context(
            agent_id,
            self.settings_store.clone(),
            &self.skill_center,
            self.leader_context.clone(),
            mode,
            args.task,
        )?;

        let completion = self
            .completion_factory
            .create_completion(agent_id)
            .map_err(MorayError::from)?;

        let toolbox = Arc::new(
            self.toolbox_factory
                .create_toolbox(self.session_id.as_str(), agent_id)
                .map_err(MorayError::from)?,
        );

        let stream = AgentRequestBuilder::new()
            .completion(completion)
            .toolbox(toolbox)
            .context(sub_context)
            .stream(self.stream)
            .cancellation(self.cancellation.clone())
            .run()
            .map_err(MorayError::from)?;

        let handler = drain_agent_stream(
            stream,
            &self.cancellation,
            self.sink.as_ref(),
            self.session_id.as_str(),
            agent_id,
            AgentRole::Sub,
            SubStreamHandler::new(),
        )
        .await?;
        match handler.into_result() {
            Ok(text) => responder.send_text(text).await?,
            Err(err) => responder.send_text(err).await?,
        }

        Ok(())
    }
}

fn resolve_context_mode(
    override_mode: Option<&str>,
    session_default: SubAgentContextMode,
) -> std::result::Result<SubAgentContextMode, MorayError> {
    if let Some(raw) = override_mode {
        return match raw.trim().to_ascii_lowercase().as_str() {
            "isolated" => Ok(SubAgentContextMode::Isolated),
            "branch" => Ok(SubAgentContextMode::Branch),
            // 当模型推理出错时，为避免执行中断，这里默认使用 isolated 模式
            _ => Ok(SubAgentContextMode::Isolated),
        };
    }
    Ok(session_default)
}

fn build_sub_context(
    agent_id: &str,
    settings_store: Arc<SondaSettingsStore>,
    skill_center: &SkillCenter,
    leader_context: Arc<dyn ContextEngine>,
    mode: SubAgentContextMode,
    task: String,
) -> std::result::Result<Arc<dyn ContextEngine>, MorayError> {
    // 和 leader agent 不一样，leader agent 的上下文生命周期和 session 是一致的，因此其 context engine 在创建 session 时
    // 就一次性创建好，并可以在所有后续 turn 中复用。而 sub agent 的上下文生命周期和 turn 是一致的，因此其
    // context engine 在每次 tool 调用时都需要重新创建。尤其注意需要理解 “和 turn 是一致的” 的具体含义。
    // 例如:
    // 1. turn 发起时携带上下文 C0
    // 2. turn 执行, 模型推理出 text block t1, 以及 tool call 1, 执行后返回结果 r1
    // 3. turn 继续，模型推理出要执行 sub agent A (by run_sub_agent tool), with 'task'
    // 那么，此时 sub agent A 的上下文应该是 C0 + t1 + r1 + task
    //
    // with_fn 是为了给 session 的 leader context 一次设置、多次动态决议用的。而这里的 build_sub_context 一定是
    // 当前 turn 的即时消费。因此没必要再考虑二次动态决议的问题。相对“静态”更高效

    let character = settings_store
        .agent_character(agent_id)
        .ok()
        .flatten()
        .unwrap_or_default();

    let preambler = TemplatedPreamblerBuilder::default()
        .template(settings_store.preamble_template())
        .with_string("character", character)
        .section(SkillsSection::new(skill_center.skills(SkillFilterKind::All)))
        .build();

    // 将模型推理出的 task 描述作为 "User Message" 指引 sub agent 完成特定任务
    let task_message = TurnInput::from(task).to_user_message();
    let messages = match mode {
        // 独立上下文模式：sub agent 仅看到 task 描述，通常用于完成一些简单、独立、从 task 描述就能完整拿到所需信息的工作
        SubAgentContextMode::Isolated => vec![task_message],

        // 分支上下文模式：sub agent 看到 leader 上下文和 task 描述，通常用于完成一些需要依赖 leader 上下文的复杂任务
        SubAgentContextMode::Branch => {
            let mut msgs = leader_context.snapshot().ok_or_else(|| {
                MorayError::Message("branch context requires leader context snapshot support".into())
            })?;
            msgs.push(task_message);
            msgs
        }
    };

    Ok(Arc::new(
        CompositeContextEngineBuilder::new()
            .messages(messages)
            .preamble(Arc::new(preambler))
            .build(),
    ))
}

/// Shared agent runner: completion/toolbox factories and agent stream assembly.
pub struct SondaAgentRunner {
    settings_store: Arc<SondaSettingsStore>,
    completion_factory: Arc<SondaCompletionFactory>,
    toolbox_factory: Arc<SondaToolboxFactory>,
    skill_center: SkillCenter,
    session_catalog: Arc<SondaSessionCatalog>,
    stream: bool,
}

impl SondaAgentRunner {
    pub fn new(
        settings_store: Arc<SondaSettingsStore>,
        completion_factory: Arc<SondaCompletionFactory>,
        toolbox_factory: Arc<SondaToolboxFactory>,
        skill_center: SkillCenter,
        session_catalog: Arc<SondaSessionCatalog>,
        stream: bool,
    ) -> Self {
        Self {
            settings_store,
            completion_factory,
            toolbox_factory,
            skill_center,
            session_catalog,
            stream,
        }
    }

    fn resolve_session_agent_id(&self, session_id: &str) -> Result<String> {
        Ok(self.session_catalog.get_session_agent_id(session_id)?)
    }

    fn sub_agent_manifest_description(&self, sub_agents: &[SessionSubAgentEntry]) -> String {
        let mut lines = vec![
            "Delegate a task to a sub-agent. Returns the sub-agent's final answer.".into(),
            "".into(),
            "Available sub-agents:".into(),
        ];
        for entry in sub_agents {
            let desc = self
                .settings_store
                .agent_desc(&entry.agent_id)
                .unwrap_or_default();
            lines.push(format!("- {}: {}", entry.agent_id, desc));
        }
        lines.join("\n")
    }

    fn create_leader_toolbox(
        &self,
        session_id: &str,
        leader_agent_id: &str,
        leader_context: Arc<dyn ContextEngine>,
        sink: Arc<dyn SessionEventSink>,
        cancellation: CancellationToken,
    ) -> Result<Arc<Toolbox>> {
        let mut toolbox = self
            .toolbox_factory
            .create_toolbox(session_id, leader_agent_id)?;

        let sub_agents = self.session_catalog.get_session_sub_agents(session_id)?;
        if !sub_agents.is_empty() {
            let manifest = ToolManifest {
                name: RUN_SUB_AGENT_TOOL_NAME.into(),
                description: self.sub_agent_manifest_description(sub_agents.as_slice()),
                parameters: RUN_SUB_AGENT_PARAMETERS.into(),
            };
            let tool: Arc<dyn Tool> = Arc::new(SondaSubAgentTrigger {
                toolbox_factory: self.toolbox_factory.clone(),
                stream: self.stream,
                session_id: session_id.to_string(),
                sub_agents,
                settings_store: self.settings_store.clone(),
                completion_factory: self.completion_factory.clone(),
                skill_center: self.skill_center.clone(),
                leader_context,
                sink,
                cancellation,
            });
            toolbox.extend(vec![manifest], vec![tool]);
        }

        Ok(Arc::new(toolbox))
    }
}

#[async_trait]
impl AgentRunner for SondaAgentRunner {
    async fn run_turn(
        &self,
        session_id: &str,
        context: Arc<dyn ContextEngine + Send + Sync>,
        cancellation: CancellationToken,
        sink: Arc<dyn SessionEventSink + Send + Sync>,
    ) -> std::result::Result<(), SessionError> {
        let leader_agent_id = self
            .resolve_session_agent_id(session_id)
            .map_err(|e: SondaError| SessionError::from(MorayError::from(e)))?;

        let toolbox = self
            .create_leader_toolbox(
                session_id,
                leader_agent_id.as_str(),
                context.clone(),
                sink.clone(),
                cancellation.clone(),
            )
            .map_err(|e: SondaError| SessionError::from(MorayError::from(e)))?;

        let completion = self
            .completion_factory
            .create_completion(leader_agent_id.as_str())
            .map_err(|e: SondaError| SessionError::from(MorayError::from(e)))?;

        let stream = AgentRequestBuilder::new()
            .completion(completion)
            .toolbox(toolbox)
            .context(context)
            .stream(self.stream)
            .cancellation(cancellation.clone())
            .run()
            .map_err(|e: MorayError| SessionError::from(e))?;

        drain_agent_stream(
            stream,
            &cancellation,
            sink.as_ref(),
            session_id,
            leader_agent_id.as_str(),
            AgentRole::Leader,
            LeaderStreamHandler,
        )
        .await
        .map(|_| ())
        .map_err(SessionError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moray_core::{ChatCompletionFinishReason, ToolCallRequest};

    fn text_block(s: &str) -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::TextBlock(s.into()),
        }
    }

    fn completion_done() -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::Done {
                reason: ChatCompletionFinishReason::Stop,
            },
        }
    }

    fn tool_call_chunk() -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::ToolCall(ToolCallRequest {
                call_id: "c1".into(),
                name: "image_create".into(),
                arguments: "{}".into(),
            }),
        }
    }

    #[test]
    fn sub_handler_keeps_last_nonempty_round_after_tool_call() {
        let mut handler = SubStreamHandler::new();
        handler.on_event(&text_block("partial "));
        handler.on_event(&tool_call_chunk());
        handler.on_event(&completion_done());
        handler.on_event(&text_block("final summary"));
        handler.on_event(&completion_done());
        assert_eq!(handler.into_result(), Ok("final summary".into()));
    }
}
