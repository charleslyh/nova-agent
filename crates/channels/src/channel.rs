//! [`ImChannel`] — one configured IM instance (QQ, WeCom, …).

use std::sync::Arc;

use async_trait::async_trait;
use moray_session::SessionEvent;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::error::ChannelError;

/// Tool-auth decision normalized from QQ `/approve` or WeCom approval cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    Deny,
}

/// User-visible text from an IM platform.
#[derive(Debug, Clone)]
pub struct UserMessage {
    pub content: String,
}

/// Normalized tool-auth decision (QQ `/approve`, WeCom approval card, …).
#[derive(Debug, Clone)]
pub struct AuthReply {
    pub call_id: String,
    pub decision: ApprovalDecision,
}

/// Everything the channel manager receives from a channel listener.
#[derive(Debug, Clone)]
pub enum InboundMessage {
    User(UserMessage),
    Auth(AuthReply),
}

/// Long-lived listener plus a stream of [`InboundMessage`] for the channel manager.
pub struct ChannelRun {
    pub receiver: mpsc::Receiver<InboundMessage>,
    pub listener: JoinHandle<Result<(), ChannelError>>,
}

/// Platform-specific IM connector + session event rendering.
#[async_trait]
pub trait ImChannel: Send + Sync {
    /// Start the IM connection; returns inbound events and a join handle for the listener task.
    async fn run(
        self: Arc<Self>,
        cancel: CancellationToken,
    ) -> Result<ChannelRun, ChannelError>;

    /// Render agent session events to IM (including optional `Reset` handling).
    async fn on_session_event(self: Arc<Self>, event: &SessionEvent);
}
