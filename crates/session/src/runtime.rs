use std::mem;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::{Stream, StreamExt};
use tokio::spawn;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::harness::Harness;
use crate::{Result, SessionError};
use moray_core::{
    AgentRequestBuilder, AgentResponseEvent, ChatCompletionRequestMessage, ContextEngine,
    MorayError,
};

/// At most one in-flight turn: cancel token, [`oneshot`] completion, and cross-task access.
#[derive(Clone)]
struct ActiveTurn {
    inner: Arc<Mutex<Option<InflightTurn>>>,
}

struct InflightTurn {
    cancel: CancellationToken,
    done_tx: oneshot::Sender<()>,
    done_rx: oneshot::Receiver<()>,
}

impl ActiveTurn {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    fn begin(&self) -> CancellationToken {
        let (done_tx, done_rx) = oneshot::channel();
        let cancel = CancellationToken::new();
        let cancellation = cancel.clone();
        *self.inner.lock().expect("active_turn lock poisoned") = Some(InflightTurn {
            cancel,
            done_tx,
            done_rx,
        });
        cancellation
    }

    fn cancel_if_any(&self) {
        if let Some(turn) = self.inner.lock().expect("active_turn lock poisoned").as_ref() {
            turn.cancel.cancel();
        }
    }

    async fn wait_done(&self) {
        let done_rx = self.inner.lock().expect("active_turn lock poisoned").as_mut().map(
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

    /// Drain completion: take the registered turn and signal waiters. No-op if already cleared.
    fn finish(&self) {
        if let Some(turn) = self.inner.lock().expect("active_turn lock poisoned").take() {
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
    #[cfg_attr(feature = "serde", serde(alias = "content"))]
    pub text: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub resources: Vec<TurnResource>,
}

impl TurnInput {
    /// Format for [`ChatCompletionRequestMessage::User`] ingestion and replay.
    pub fn to_user_message_content(&self) -> String {
        if self.resources.is_empty() {
            return self.text.clone();
        }
        let mut parts = Vec::new();
        if !self.text.trim().is_empty() {
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

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum SessionEventKind {
    TurnAccepted { input: TurnInput },

    AgentResponse { agent: AgentResponseEvent },

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

pub struct SessionRuntime {
    session_id: String,
    stream: bool,
    event_sink: Arc<dyn SessionEventSink>,
    context: Arc<dyn ContextEngine>,
    harness: Arc<dyn Harness>,
    active: Arc<AtomicBool>,
    active_turn: ActiveTurn,
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

struct ActiveGuard {
    active: Arc<AtomicBool>,
}

impl ActiveGuard {
    fn new(active: Arc<AtomicBool>) -> Self {
        Self { active }
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
    }
}

impl SessionRuntime {
    pub fn new(
        session_id: impl Into<String>,
        event_sink: Arc<dyn SessionEventSink>,
        context: Arc<dyn ContextEngine>,
        harness: Arc<dyn Harness>,
        stream: bool,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            harness,
            stream,
            event_sink,
            context,
            active: Arc::new(AtomicBool::new(false)),
            active_turn: ActiveTurn::new(),
        }
    }

    pub async fn submit(&self, input: TurnInput) -> Result<()> {
        let guard = self.request_active_guard()?;
        let cancellation = self.active_turn.begin();

        let content = input.to_user_message_content();
        let event = event_of(self.session_id.as_str(), SessionEventKind::TurnAccepted { input });
        self.event_sink.append(&event)?;

        self.context
            .ingest(vec![ChatCompletionRequestMessage::User { content }])
            .await?;

        let completion = self.harness.create_completion(self.session_id.as_str())?;
        let toolbox = self.harness.create_toolbox(self.session_id.as_str())?;
        let agent_stream = AgentRequestBuilder::new()
            .completion(completion)
            .toolbox(toolbox)
            .context(self.context.clone())
            .stream(self.stream)
            .cancellation(cancellation)
            .run()?;

        spawn(Self::drain_agent_stream(
            self.session_id.clone(),
            agent_stream,
            self.event_sink.clone(),
            self.active_turn.clone(),
            guard,
        ));

        Ok(())
    }

    /// Cancel the in-flight agent run for the current turn, if any. Idempotent when idle.
    pub fn cancel(&self) -> Result<()> {
        self.active_turn.cancel_if_any();
        Ok(())
    }

    /// Reset the session working state and emit a [`SessionEventKind::Reset`] event.
    pub async fn reset(&self) -> Result<()> {
        if self.active.load(Ordering::SeqCst) {
            self.active_turn.cancel_if_any();
            self.active_turn.wait_done().await;
        }

        let _guard = self.request_active_guard()?;

        self.event_sink.append(&event_of(
            self.session_id.as_str(),
            SessionEventKind::Reset
        ))?;

        self.context.clear().await?;

        Ok(())
    }

    async fn drain_agent_stream(
        session_id: String,
        mut agent_stream: Pin<Box<dyn Stream<Item = AgentResponseEvent> + Send>>,
        store: Arc<dyn SessionEventSink>,
        active_turn: ActiveTurn,
        _guard: ActiveGuard,
    ) {
        while let Some(agent_ev) = agent_stream.next().await {
            let e = event_of(
                session_id.as_str(),
                SessionEventKind::AgentResponse { agent: agent_ev },
            );

            if store.append(&e).is_err() {
                break;
            }
        }

        let finish = event_of(session_id.as_str(), SessionEventKind::TurnFinish);
        let _ = store.append(&finish);

        active_turn.finish();
    }

    fn request_active_guard(&self) -> Result<ActiveGuard> {
        self.active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| SessionError::from(MorayError::Busy))?;

        Ok(ActiveGuard::new(self.active.clone()))
    }
}
