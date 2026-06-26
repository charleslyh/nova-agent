use std::mem;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::spawn;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn, Instrument};

use crate::{AgentRunner, Result, SessionError};
use moray_core::{
    ChannelMultiAgentEventSink, ChatCompletionRequestMessage, ContextEngine, MorayError,
    MultiAgentResponseEvent,
};

/// Per-turn signals: cancel token and [`oneshot`] completion.
struct TurnControl {
    cancel: CancellationToken,
    done_tx: oneshot::Sender<()>,
    done_rx: oneshot::Receiver<()>,
}

impl TurnControl {
    fn new() -> (Self, CancellationToken) {
        let (done_tx, done_rx) = oneshot::channel();
        let cancel = CancellationToken::new();
        let cancellation = cancel.clone();
        (
            Self {
                cancel,
                done_tx,
                done_rx,
            },
            cancellation,
        )
    }
}

/// At most one exclusive session operation (agent turn or reset).
#[derive(Clone)]
struct InflightSlot {
    inner: Arc<Mutex<Option<TurnControl>>>,
}

/// Holds the inflight slot until dropped (e.g. during [`SessionRuntime::reset`]).
struct ExclusiveHold {
    slot: InflightSlot,
}

impl Drop for ExclusiveHold {
    fn drop(&mut self) {
        let _ = self.slot.inner.lock().expect("inflight lock poisoned").take();
    }
}

impl InflightSlot {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    fn try_start_turn(&self) -> Result<CancellationToken> {
        let mut slot = self.inner.lock().expect("inflight lock poisoned");
        if slot.is_some() {
            return Err(SessionError::from(MorayError::Busy));
        }

        let (control, cancellation) = TurnControl::new();
        *slot = Some(control);
        Ok(cancellation)
    }

    fn try_hold_exclusive(self) -> Result<ExclusiveHold> {
        {
            let mut slot = self.inner.lock().expect("inflight lock poisoned");
            if slot.is_some() {
                return Err(SessionError::from(MorayError::Busy));
            }

            let (control, _) = TurnControl::new();
            *slot = Some(control);
        }
        Ok(ExclusiveHold { slot: self })
    }

    fn cancel(&self) {
        if let Some(turn) = self.inner.lock().expect("inflight lock poisoned").as_ref() {
            turn.cancel.cancel();
        }
    }

    async fn wait_turn_done(&self) {
        let done_rx = self.inner.lock().expect("inflight lock poisoned").as_mut().map(
            |turn| {
                let (stub_tx, stub_rx) = oneshot::channel();
                let _ = stub_tx;
                mem::replace(&mut turn.done_rx, stub_rx)
            },
        );
        if let Some(done_rx) = done_rx {
            let _ = done_rx.await;
        }
    }

    /// Take the registered turn and signal waiters. No-op if already cleared.
    fn complete_turn(&self) {
        if let Some(turn) = self.inner.lock().expect("inflight lock poisoned").take() {
            let _ = turn.done_tx.send(());
        }
    }
}

/// A user-attached resource for one turn (images only for now).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum TurnResource {
    Image { path: String },
}

/// User-authored payload for one chat turn
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TurnInput {
    pub text: String,

    #[cfg_attr(feature = "serde", serde(default))]
    pub resources: Vec<TurnResource>,
}

impl TurnInput {
    /// Projects this turn into the core transcript user message.
    pub fn to_user_message(&self) -> ChatCompletionRequestMessage {
        ChatCompletionRequestMessage::User {
            content: self.user_message_content(),
        }
    }

    fn user_message_content(&self) -> String {
        if self.resources.is_empty() {
            return self.text.clone();
        }
        let mut parts = Vec::new();
        if !self.text.is_empty() {
            parts.push(self.text.clone());
        }
        for resource in &self.resources {
            match resource {
                TurnResource::Image { path } => parts.push(format!("[IMAGE:{path}]")),
            }
        }
        parts.join("\n")
    }
}

impl From<String> for TurnInput {
    fn from(text: String) -> Self {
        Self {
            text,
            resources: Vec::new(),
        }
    }
}

impl From<&str> for TurnInput {
    fn from(s: &str) -> Self {
        Self {
            text: s.to_owned(),
            resources: Vec::new(),
        }
    }
}

pub trait SessionEventSink: Send + Sync {
    fn append(&self, event: &SessionEvent) -> std::result::Result<(), MorayError>;
}

pub type SessionAgentResponse = MultiAgentResponseEvent;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum SessionEventKind {
    TurnAccepted { input: TurnInput },

    AgentResponse(SessionAgentResponse),

    TurnFinish,

    Reset,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SessionEvent {
    pub session_id: String,
    pub ts: u64,
    pub kind: SessionEventKind,
}

const SESSION_EVENT_CHANNEL_CAPACITY: usize = 256;

pub struct SessionRuntime {
    session_id: String,
    event_sink: Arc<dyn SessionEventSink>,
    context: Arc<dyn ContextEngine>,
    agent_runner: Arc<dyn AgentRunner>,
    inflight: InflightSlot,
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn event_of(session_id: &str, kind: SessionEventKind) -> SessionEvent {
    SessionEvent {
        session_id: session_id.to_string(),
        ts: timestamp_ms(),
        kind,
    }
}

impl SessionRuntime {
    pub fn new(
        session_id: impl Into<String>,
        event_sink: Arc<dyn SessionEventSink>,
        context: Arc<dyn ContextEngine>,
        agent_runner: Arc<dyn AgentRunner>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_runner,
            event_sink,
            context,
            inflight: InflightSlot::new(),
        }
    }

    pub async fn submit(&self, input: TurnInput) -> Result<()> {
        let cancellation = match self.inflight.try_start_turn() {
            Ok(cancellation) => cancellation,
            Err(err) => {
                warn!(session_id = %self.session_id, "turn rejected: busy");
                return Err(err);
            }
        };

        let text_len = input.text.len();
        let resource_count = input.resources.len();
        info!(
            session_id = %self.session_id,
            text_len,
            resource_count,
            "turn accepted"
        );

        let user_message = input.to_user_message();

        let event = event_of(self.session_id.as_str(), SessionEventKind::TurnAccepted { input });
        self.event_sink.append(&event)?;

        if let Err(e) = self.context.ingest(vec![user_message]).await {
            warn!(
                session_id = %self.session_id,
                error = %e,
                "failed to ingest user message"
            );
            return Err(e.into());
        }

        let session_id = self.session_id.clone();
        let runner = self.agent_runner.clone();
        let context = self.context.clone();
        let sink = self.event_sink.clone();
        let inflight = self.inflight.clone();
        let parent_span = tracing::Span::current();

        spawn(
            async move {
                let (events_tx, mut events_rx) =
                    mpsc::channel::<MultiAgentResponseEvent>(SESSION_EVENT_CHANNEL_CAPACITY);
                let multi_sink = Arc::new(ChannelMultiAgentEventSink::new(events_tx));

                let consumer_session_id = session_id.clone();
                let consumer_sink = sink.clone();
                let consumer = spawn(
                    async move {
                        while let Some(frame) = events_rx.recv().await {
                            let event = SessionEvent {
                                session_id: consumer_session_id.clone(),
                                ts: timestamp_ms(),
                                kind: SessionEventKind::AgentResponse(frame),
                            };
                            if consumer_sink.append(&event).is_err() {
                                break;
                            }
                        }
                    }
                    .in_current_span(),
                );

                let run_result = runner.run(context, cancellation, multi_sink).await;
                let _ = consumer.await;
                match &run_result {
                    Ok(()) => info!(session_id = %session_id, "agent run completed"),
                    Err(e) => warn!(session_id = %session_id, error = %e, "agent run failed"),
                }

                let finish = event_of(session_id.as_str(), SessionEventKind::TurnFinish);
                if sink.append(&finish).is_err() {
                    warn!(session_id = %session_id, "failed to append TurnFinish event");
                }

                inflight.complete_turn();
            }
            .instrument(parent_span),
        );

        Ok(())
    }

    /// Cancel the in-flight agent run for the current turn, if any. Idempotent when idle.
    pub fn cancel(&self) -> Result<()> {
        info!(session_id = %self.session_id, "cancel requested");
        self.inflight.cancel();
        Ok(())
    }

    /// Reset the session working state and emit a [`SessionEventKind::Reset`] event.
    pub async fn reset(&self) -> Result<()> {
        info!(session_id = %self.session_id, "reset started");
        self.inflight.cancel();
        self.inflight.wait_turn_done().await;

        let _guard = self.inflight.clone().try_hold_exclusive()?;

        self.event_sink.append(&event_of(
            self.session_id.as_str(),
            SessionEventKind::Reset,
        ))?;

        self.context.clear().await?;

        info!(session_id = %self.session_id, "reset completed");
        Ok(())
    }
}
