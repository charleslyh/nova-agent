use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use moray_core::{ToolCallAuthorizer, ToolCallResponder, ToolboxError};
use serde_json::Value;
use tokio::sync::oneshot;

pub struct AlwaysAsking {
    pending_auth: Mutex<HashMap<String, oneshot::Sender<bool>>>,
}

impl Default for AlwaysAsking {
    fn default() -> Self {
        Self::new()
    }
}

impl AlwaysAsking {
    pub fn new() -> Self {
        Self {
            pending_auth: Mutex::new(HashMap::new()),
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
impl ToolCallAuthorizer for AlwaysAsking {
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
            .expect("always-ask pending-auth mutex poisoned")
            .insert(call_id.to_string(), tx);
        responder.send_custom(None).await;
        rx.await.unwrap_or(false)
    }

    async fn reply(&self, call_id: &str, data: Value) -> Result<(), ToolboxError> {
        let tx = self
            .pending_auth
            .lock()
            .expect("always-ask pending-auth mutex poisoned")
            .remove(call_id)
            .ok_or_else(|| ToolboxError::NoPendingAuthorization {
                call_id: call_id.to_string(),
            })?;
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
