use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::agent::requests::multi::harness::AgentHarnessFactory;
use crate::agent::requests::multi::sink::{CollectingAgentEventSink, AgentRole, MultiAgentEventSink};
use crate::agent::requests::single::AgentRequestBuilder;
use crate::{
    ChatCompletionRequestMessage, ContextEngine, MorayError, Tool, ToolCallResponder,
    ToolManifest, Toolbox,
};

pub const RUN_SUB_AGENT_TOOL_NAME: &str = "run_sub_agent";

const RUN_SUB_AGENT_PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {
    "agent_id": { "type": "string", "description": "Sub-agent id to run" },
    "task": { "type": "string", "description": "Task description for the sub-agent" },
    "intent": {
      "type": "string",
      "description": "Extremely short label of why this sub-agent is being called, inferred by the model. Usually 1-3 keywords only (e.g. Chinese keywords like \"查天气\" or \"对比股价\"). Do not copy the full task."
    },
    "context": {
      "type": "string",
      "enum": ["isolated", "branch"],
      "description": "Optional override; omit to use the sub-agent's session default. isolated: delegate a standalone sub-task—the sub-agent sees only task, so put all required facts, constraints, and inputs into task. Prefer for independent or parallel work, specialist runs, or when prior chat is irrelevant/noisy. branch: delegate work that depends on prior conversation—the sub-agent inherits the leader transcript plus task. Prefer for follow-ups, references to earlier user messages, disambiguation, or continuing the same thread."
    }
  },
  "required": ["agent_id", "task", "intent"]
}"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SubAgentContextMode {
    #[default]
    Isolated,
    Branch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SubAgentSpec {
    pub agent_id: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub context_mode: SubAgentContextMode,
    /// Shown in the `run_sub_agent` tool manifest for this session binding.
    pub description: String,
    /// ReAct round cap for this sub-agent run. When omitted, [`AgentRequestBuilder`] uses its default.
    #[cfg_attr(feature = "serde", serde(default))]
    pub max_rounds: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RunSubAgentArgs {
    agent_id: String,
    task: String,
    /// Ultra-short caller intent (1–3 keywords); used for UI / logging only.
    #[serde(default)]
    intent: String,
    #[serde(default)]
    context: Option<String>,
}

pub(crate) struct RunSubAgentTool {
    factory: Arc<dyn AgentHarnessFactory>,
    stream: bool,
    sub_agents: Vec<SubAgentSpec>,
    leader_context: Arc<dyn ContextEngine>,
    events: Arc<dyn MultiAgentEventSink>,
    cancellation: CancellationToken,
}

impl RunSubAgentTool {
    pub(crate) fn new(
        factory: Arc<dyn AgentHarnessFactory>,
        stream: bool,
        sub_agents: Vec<SubAgentSpec>,
        leader_context: Arc<dyn ContextEngine>,
        events: Arc<dyn MultiAgentEventSink>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            factory,
            stream,
            sub_agents,
            leader_context,
            events,
            cancellation,
        }
    }
}

#[async_trait]
impl Tool for RunSubAgentTool {
    fn name(&self) -> &'static str {
        RUN_SUB_AGENT_TOOL_NAME
    }

    async fn call(
        &self,
        args: Value,
        responder: &dyn ToolCallResponder,
    ) -> std::result::Result<(), MorayError> {
        let args: RunSubAgentArgs = serde_json::from_value(args).map_err(|e| {
            MorayError::Message(format!("invalid run_sub_agent arguments: {e}"))
        })?;
        self.run_inner(args, responder).await
    }
}

impl RunSubAgentTool {
    async fn run_inner(
        &self,
        args: RunSubAgentArgs,
        responder: &dyn ToolCallResponder,
    ) -> std::result::Result<(), MorayError> {
        let agent_id = args.agent_id.as_str();
        let entry = self
            .sub_agents
            .iter()
            .find(|e| e.agent_id == agent_id)
            .ok_or_else(|| {
                let available = self
                    .sub_agents
                    .iter()
                    .map(|e| e.agent_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                MorayError::Message(format!(
                    "unknown sub-agent id `{agent_id}` for this session; available: [{available}]"
                ))
            })?;

        let mode = resolve_context_mode(args.context.as_deref(), entry.context_mode)?;
        info!(
            agent_id,
            intent = %args.intent,
            ?mode,
            max_rounds = ?entry.max_rounds,
            "run_sub_agent started"
        );

        let task_message = user_message_from_task(&args.task);
        let messages = match mode {
            SubAgentContextMode::Isolated => vec![task_message],
            SubAgentContextMode::Branch => {
                let mut msgs = self.leader_context.snapshot().ok_or_else(|| {
                    MorayError::Message(
                        "branch context requires leader context snapshot support".into(),
                    )
                })?;
                msgs.push(task_message);
                msgs
            }
        };

        let sub_context = self
            .factory
            .create_context(agent_id, messages)?;

        let completion = self.factory.create_completion(agent_id)?;

        let toolbox = Arc::new(self.factory.create_toolbox(agent_id)?);

        let collector = CollectingAgentEventSink::new(
            self.events.clone(),
            agent_id.to_string(),
            AgentRole::Sub,
        );

        let mut request = AgentRequestBuilder::new()
            .completion(completion)
            .toolbox(toolbox)
            .context(sub_context)
            .stream(self.stream);
        if let Some(max_rounds) = entry.max_rounds {
            request = request.max_rounds(max_rounds);
        }

        match request
            .cancellation(self.cancellation.clone())
            .run(collector.clone())?
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(join_err) => {
                return Err(MorayError::Message(format!(
                    "agent run task failed: {join_err}"
                )))
            }
        }

        match collector.finalize() {
            Ok(text) => responder.send_text(text).await?,
            Err(err) => responder.send_text(err).await?,
        }

        info!(agent_id, "run_sub_agent completed");
        Ok(())
    }
}

pub(crate) fn resolve_context_mode(
    override_mode: Option<&str>,
    default_mode: SubAgentContextMode,
) -> std::result::Result<SubAgentContextMode, MorayError> {
    if let Some(raw) = override_mode {
        return match raw.trim().to_ascii_lowercase().as_str() {
            "isolated" => Ok(SubAgentContextMode::Isolated),
            "branch" => Ok(SubAgentContextMode::Branch),
            _ => Ok(SubAgentContextMode::Isolated),
        };
    }
    Ok(default_mode)
}

pub(crate) fn sub_agent_manifest_description(sub_agents: &[SubAgentSpec]) -> String {
    let mut lines = vec![
        "Delegate a task to a sub-agent. Returns the sub-agent's final answer.".into(),
        "Always set `intent` to a 1-3 keyword summary of why you are calling the sub-agent.".into(),
        "".into(),
        "Available sub-agents:".into(),
    ];
    for entry in sub_agents {
        lines.push(format!("- {}: {}", entry.agent_id, entry.description));
    }
    lines.join("\n")
}

pub(crate) fn inject_sub_agents_trigger(
    toolbox: &mut Toolbox,
    sub_agents: &[SubAgentSpec],
    factory: Arc<dyn AgentHarnessFactory>,
    stream: bool,
    leader_context: Arc<dyn ContextEngine>,
    events: Arc<dyn MultiAgentEventSink>,
    cancellation: CancellationToken,
) {
    if sub_agents.is_empty() {
        return;
    }

    let manifest = ToolManifest {
        name: RUN_SUB_AGENT_TOOL_NAME.into(),
        description: sub_agent_manifest_description(sub_agents),
        parameters: RUN_SUB_AGENT_PARAMETERS.into(),
    };
    let tool: Arc<dyn Tool> = Arc::new(RunSubAgentTool::new(
        factory,
        stream,
        sub_agents.to_vec(),
        leader_context,
        events,
        cancellation,
    ));
    toolbox.extend(vec![manifest], vec![tool]);
}

fn user_message_from_task(task: &str) -> ChatCompletionRequestMessage {
    ChatCompletionRequestMessage::User {
        content: task.to_owned(),
    }
}
