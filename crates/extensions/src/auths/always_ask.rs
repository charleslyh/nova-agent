use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use std::sync::Arc;

use moray_channels::{ChannelError, ToolCallReplyRouter};
use moray_core::{ToolCallInterceptor, ToolCallRequest, ToolCallResponder, ToolManifest};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// Prompts the user before every tool call and waits for an approval
/// decision, except for tools in the auto-allow list.
///
/// The prompt is emitted through the interceptor's responder (the toolbox
/// buffers it and flushes after `Requested`), and decisions arrive out-of-band
/// through [`ToolCallReplyRouter::reply`] (IM channels or the desktop HTTP
/// route). Denials carry no custom payload, so the toolbox appends its default
/// denial text.
pub struct AlwaysAsking {
    pending_auth: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    auto_allow: HashSet<String>,
}

impl Default for AlwaysAsking {
    fn default() -> Self {
        Self::new()
    }
}

impl AlwaysAsking {
    pub fn new() -> Self {
        Self::with_auto_allow(std::iter::empty::<&str>())
    }

    pub fn with_auto_allow(names: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self {
            pending_auth: Mutex::new(HashMap::new()),
            auto_allow: names
                .into_iter()
                .map(|n| n.as_ref().to_string())
                .collect(),
        }
    }
}

fn decode_allow(data: &Value) -> bool {
    if let Some(b) = data.as_bool() {
        return b;
    }
    if let Some(b) = data.get("allow").and_then(|v| v.as_bool()) {
        return b;
    }
    matches!(
        data.get("decision").and_then(|v| v.as_str()),
        Some("allow_once" | "allow" | "approve")
    )
}

#[async_trait]
impl ToolCallInterceptor for AlwaysAsking {
    async fn intercept(
        &self,
        request: &mut ToolCallRequest,
        _manifest: Option<&ToolManifest>,
        responder: Arc<dyn ToolCallResponder>,
        _cancellation: CancellationToken,
    ) -> bool {
        if self.auto_allow.contains(request.name.as_str()) {
            return true;
        }

        let (tx, rx) = oneshot::channel();
        self.pending_auth
            .lock()
            .expect("always-ask pending-auth mutex poisoned")
            .insert(request.call_id.clone(), tx);
        if responder
            .send_extra(json!({
                "tool_name": request.name,
                "arguments": request.arguments,
            }))
            .await
            .is_err()
        {
            self.pending_auth
                .lock()
                .expect("always-ask pending-auth mutex poisoned")
                .remove(&request.call_id);
            return false;
        }
        rx.await.unwrap_or(false)
    }
}

#[async_trait]
impl ToolCallReplyRouter for AlwaysAsking {
    async fn reply(&self, call_id: &str, data: Value) -> Result<(), ChannelError> {
        let tx = self
            .pending_auth
            .lock()
            .expect("always-ask pending-auth mutex poisoned")
            .remove(call_id)
            .ok_or_else(|| ChannelError::ToolCallAuthNotFound(call_id.to_string()))?;
        let _ = tx.send(decode_allow(&data));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::decode_allow;
    use serde_json::json;

    #[test]
    fn decode_allow_accepts_im_channel_decision_payload() {
        assert!(decode_allow(&json!({"decision": "allow_once"})));
        assert!(!decode_allow(&json!({"decision": "deny"})));
        assert!(decode_allow(&json!(true)));
        assert!(decode_allow(&json!({"allow": true})));
    }
}
