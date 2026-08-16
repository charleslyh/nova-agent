//! [`ChannelsManager`] — per-channel IM connector lifecycle.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use moray_session::{LiveSessions, SessionEvent, SessionEventKind, TurnInput};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::channel::{ApprovalDecision, ImChannel, InboundMessage};
use crate::error::{ChannelError, SessionLiveEventsError};

/// Routes an out-of-band tool-call approval reply to whichever component is
/// waiting for it (e.g. a `ToolCallInterceptor` holding a pending prompt).
///
/// moray-core deliberately knows nothing about reply routing: the toolbox
/// only exposes the interception hook, and applications decide how user
/// decisions reach a waiting interceptor. IM channels and HTTP routes share
/// this small trait to deliver those decisions.
#[async_trait]
pub trait ToolCallReplyRouter: Send + Sync {
    /// Delivers a user decision for `call_id`. Returns
    /// [`ChannelError::ToolCallAuthNotFound`] when no waiter is registered.
    async fn reply(&self, call_id: &str, data: serde_json::Value) -> Result<(), ChannelError>;
}

/// Full metadata required to run an IM channel connector.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelEntry {
    pub channel_id: String,
    pub session_id: String,
    #[serde(rename = "type")]
    pub channel_type: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

/// Subscribe to live session events for a single session (no persisted replay).
pub trait SessionLiveEvents: Send + Sync {
    fn subscribe_live(
        &self,
        session_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<SessionEvent>, SessionLiveEventsError>;
}

pub type ChannelFactoryFn =
    Arc<dyn Fn(&ChannelEntry) -> Result<Arc<dyn ImChannel>, ChannelError> + Send + Sync>;

struct RunningChannel {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

/// Default grace period for connector supervisor tasks to exit after cancellation.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
/// Max time to wait for the IM listener after cancellation before aborting it.
const LISTENER_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const TRANSCRIPT_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

async fn drain_listener(channel_id: &str, listener: JoinHandle<Result<(), ChannelError>>) {
    let mut listener = listener;
    match timeout(LISTENER_DRAIN_TIMEOUT, &mut listener).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => {
            warn!(channel_id = %channel_id, error = %e, "channel listener exited with error");
        }
        Ok(Err(e)) => {
            warn!(channel_id = %channel_id, error = %e, "channel listener task panicked");
        }
        Err(_) => {
            listener.abort();
            warn!(
                channel_id = %channel_id,
                timeout_secs = LISTENER_DRAIN_TIMEOUT.as_secs(),
                "channel listener drain timed out, aborted"
            );
        }
    }
}

async fn drain_transcript(channel_id: &str, transcript: JoinHandle<()>) {
    let mut transcript = transcript;
    match timeout(TRANSCRIPT_DRAIN_TIMEOUT, &mut transcript).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            warn!(channel_id = %channel_id, error = %e, "channel transcript relay panicked");
        }
        Err(_) => {
            transcript.abort();
            warn!(
                channel_id = %channel_id,
                timeout_secs = TRANSCRIPT_DRAIN_TIMEOUT.as_secs(),
                "channel transcript relay drain timed out, aborted"
            );
        }
    }
}

pub struct ChannelsManager {
    live_events: Arc<dyn SessionLiveEvents>,
    auth_replies: Arc<dyn ToolCallReplyRouter>,
    live_sessions: LiveSessions,
    factories: HashMap<String, ChannelFactoryFn>,
    running: DashMap<String, RunningChannel>,
}

impl ChannelsManager {
    pub fn new(
        live_events: Arc<dyn SessionLiveEvents>,
        auth_replies: Arc<dyn ToolCallReplyRouter>,
        live_sessions: LiveSessions,
        factories: HashMap<String, ChannelFactoryFn>,
    ) -> Self {
        Self {
            live_events,
            auth_replies,
            live_sessions,
            factories,
            running: DashMap::new(),
        }
    }

    pub fn start_all(&self, entries: impl IntoIterator<Item = ChannelEntry>) {
        for entry in entries {
            let channel_id = entry.channel_id.clone();
            if let Err(e) = self.start(entry) {
                warn!(
                    channel_id = %channel_id,
                    error = %e,
                    "failed to start channel"
                );
            }
        }
    }

    pub fn start(&self, entry: ChannelEntry) -> Result<(), String> {
        let channel_id = entry.channel_id.clone();
        if self.running.contains_key(&channel_id) {
            return Ok(());
        }

        let factory = self
            .factories
            .get(&entry.channel_type)
            .ok_or_else(|| format!("unknown channel type `{}`", entry.channel_type))?;
        let channel = factory(&entry).map_err(|e| e.to_string())?;
        let session_id = entry.session_id.clone();
        let channel_id_log = channel_id.clone();

        let cancel = CancellationToken::new();
        let live_events = self.live_events.clone();
        let auth_replies = self.auth_replies.clone();
        let live_sessions = self.live_sessions.clone();
        let run_cancel = cancel.clone();

        let task = tokio::spawn(async move {
            let transcript_cancel = run_cancel.clone();
            let channel_events = Arc::clone(&channel);
            let session_for_transcript = session_id.clone();
            let transcript_task = tokio::spawn(async move {
                let rx = match live_events.subscribe_live(&session_for_transcript) {
                    Ok(rx) => rx,
                    Err(e) => {
                        warn!(
                            session_id = %session_for_transcript,
                            error = %e,
                            "live session events subscribe failed"
                        );
                        return;
                    }
                };
                let mut rx = rx;
                loop {
                    tokio::select! {
                        _ = transcript_cancel.cancelled() => break,
                        msg = rx.recv() => {
                            match msg {
                                Some(event) => {
                                    let forward = matches!(
                                        event.kind,
                                        SessionEventKind::AgentResponse(_)
                                            | SessionEventKind::TurnFinish
                                            | SessionEventKind::Reset
                                    );
                                    if forward {
                                        channel_events
                                            .clone()
                                            .on_session_event(&event)
                                            .await;
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }
            });

            let run = match channel.run(run_cancel.clone()).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "channel run failed");
                    run_cancel.cancel();
                    let _ = transcript_task.await;
                    return;
                }
            };

            let mut receiver = run.receiver;
            'ingress: loop {
                tokio::select! {
                    _ = run_cancel.cancelled() => break 'ingress,
                    msg = receiver.recv() => {
                        match msg {
                            Some(inbound) => {
                                let dispatch = dispatch_inbound(
                                    &live_sessions,
                                    &auth_replies,
                                    &session_id,
                                    inbound,
                                );
                                tokio::select! {
                                    _ = run_cancel.cancelled() => break 'ingress,
                                    result = dispatch => {
                                        if let Err(e) = result {
                                            warn!(
                                                session_id = %session_id,
                                                error = %e,
                                                "inbound dispatch failed"
                                            );
                                        }
                                    }
                                }
                            }
                            None => break 'ingress,
                        }
                    }
                }
            }

            run_cancel.cancel();
            drain_listener(&channel_id_log, run.listener).await;
            drain_transcript(&channel_id_log, transcript_task).await;
        });

        self.running.insert(
            channel_id,
            RunningChannel {
                cancel,
                task,
            },
        );
        Ok(())
    }

    pub fn stop(&self, channel_id: &str) {
        let Some((_, running)) = self.running.remove(channel_id) else {
            return;
        };
        running.cancel.cancel();
        let channel_id = channel_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = running.task.await {
                warn!(channel_id = %channel_id, error = %e, "channel stop: task join error");
            }
        });
    }

    /// Cancel every running connector and wait for tasks to finish (bounded by [`SHUTDOWN_TIMEOUT`]).
    pub async fn shutdown_all(&self) {
        let channel_ids: Vec<String> = self.running.iter().map(|e| e.key().clone()).collect();
        let count = channel_ids.len();
        if count == 0 {
            return;
        }

        let shutdown_started = Instant::now();

        for channel_id in channel_ids {
            let Some((_, entry)) = self.running.remove(&channel_id) else {
                continue;
            };
            entry.cancel.cancel();
            let mut task = entry.task;
            match timeout(SHUTDOWN_TIMEOUT, &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    warn!(
                        channel_id = %channel_id,
                        error = %e,
                        "channels shutdown: task join error"
                    );
                }
                Err(_) => {
                    task.abort();
                    warn!(
                        channel_id = %channel_id,
                        timeout_secs = SHUTDOWN_TIMEOUT.as_secs(),
                        "channels shutdown: timed out, aborted supervisor task"
                    );
                }
            }
        }

        info!(
            count,
            elapsed_ms = shutdown_started.elapsed().as_millis(),
            "channels shutdown complete"
        );
    }

    pub fn restart(&self, entry: ChannelEntry) -> Result<(), String> {
        self.stop(&entry.channel_id);
        self.start(entry)
    }
}

/// Inbound IM slash commands handled by the channels manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelSlashCommand {
    /// Clear session transcript (`/new`).
    New,
}

fn parse_channel_slash_command(msg: &str) -> Option<ChannelSlashCommand> {
    match msg.trim() {
        "/new" => Some(ChannelSlashCommand::New),
        _ => None,
    }
}

async fn dispatch_inbound(
    live_sessions: &LiveSessions,
    auth_replies: &Arc<dyn ToolCallReplyRouter>,
    session_id: &str,
    msg: InboundMessage,
) -> Result<(), String> {
    match msg {
        InboundMessage::User(user) => {
            if matches!(
                parse_channel_slash_command(&user.content),
                Some(ChannelSlashCommand::New)
            ) {
                live_sessions
                    .reset(session_id)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
            live_sessions
                .submit(session_id, TurnInput::from(user.content.as_str()))
                .await
                .map_err(|e| e.to_string())
        }
        InboundMessage::Auth(auth) => {
            reply_tool_auth(auth_replies, &auth.call_id, auth.decision).await
        }
    }
}

async fn reply_tool_auth(
    auth_replies: &Arc<dyn ToolCallReplyRouter>,
    call_id: &str,
    decision: ApprovalDecision,
) -> Result<(), String> {
    let payload = serde_json::json!({
        "decision": match decision {
            ApprovalDecision::AllowOnce => "allow_once",
            ApprovalDecision::Deny => "deny",
        }
    });
    auth_replies
        .reply(call_id, payload)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_channel_slash_command, ChannelSlashCommand};

    #[test]
    fn parse_channel_slash_command_new() {
        assert_eq!(
            parse_channel_slash_command("/new"),
            Some(ChannelSlashCommand::New)
        );
        assert_eq!(
            parse_channel_slash_command("  /new  "),
            Some(ChannelSlashCommand::New)
        );
        assert_eq!(parse_channel_slash_command("/new extra"), None);
        assert_eq!(parse_channel_slash_command("hello"), None);
    }
}
