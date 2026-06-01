//! QQ-specific outbound reply routing (passive `msg_id` + recipient).

#[derive(Debug, Clone)]
pub(crate) struct QqReplyRoute {
    pub recipient: String,
    pub msg_id: Option<String>,
}

impl QqReplyRoute {
    pub fn new(recipient: impl Into<String>, msg_id: Option<String>) -> Self {
        Self {
            recipient: recipient.into(),
            msg_id,
        }
    }
}
