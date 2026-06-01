//! WeCom-specific outbound reply routing (`req_id` + chat recipient).

#[derive(Debug, Clone)]
pub(crate) struct WeComReplyRoute {
    pub recipient: String,
    pub req_id: Option<String>,
}

impl WeComReplyRoute {
    pub fn new(recipient: impl Into<String>, req_id: Option<String>) -> Self {
        Self {
            recipient: recipient.into(),
            req_id,
        }
    }
}
