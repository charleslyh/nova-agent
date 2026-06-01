//! QQ channel construction and catalog secret merge/redact from `data`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use moray_channels::ImChannel;

use super::channel::QQChannel;
use crate::channels::secrets::{mask_secret, merge_preserved_secret};

/// QQ API environment stored in channel catalog `data`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QQEnvironment {
    #[default]
    Production,
    Sandbox,
}

#[derive(Debug, thiserror::Error)]
pub enum QqConfigError {
    #[error("invalid qq channel data: {0}")]
    InvalidData(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QqChannelData {
    pub app_id: String,
    pub app_secret: String,
    #[serde(default)]
    pub allowed_users: Vec<String>,
    #[serde(default)]
    pub environment: QQEnvironment,
}

fn parse_data(data: &Value) -> Result<QqChannelData, QqConfigError> {
    serde_json::from_value(data.clone()).map_err(|e| QqConfigError::InvalidData(e.to_string()))
}

pub fn channel_from_config(data: &Value) -> Result<Arc<dyn ImChannel>, QqConfigError> {
    let parsed = parse_data(data)?;
    Ok(Arc::new(QQChannel::new_with_environment(
        parsed.app_id,
        parsed.app_secret,
        parsed.allowed_users,
        parsed.environment,
    )))
}

pub fn merge_secrets(incoming: &Value, existing: &Value) -> Value {
    let Ok(mut inc) = serde_json::from_value::<QqChannelData>(incoming.clone()) else {
        return incoming.clone();
    };
    let Ok(ex) = serde_json::from_value::<QqChannelData>(existing.clone()) else {
        return incoming.clone();
    };
    inc.app_secret = merge_preserved_secret(&inc.app_secret, &ex.app_secret);
    serde_json::to_value(inc).unwrap_or_else(|_| incoming.clone())
}

pub fn redact_secrets(data: &Value) -> Value {
    let Ok(mut parsed) = serde_json::from_value::<QqChannelData>(data.clone()) else {
        return data.clone();
    };
    parsed.app_secret = mask_secret(&parsed.app_secret);
    serde_json::to_value(parsed).unwrap_or_else(|_| data.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redact_masks_app_secret() {
        let data = json!({ "app_id": "id", "app_secret": "secret123" });
        let redacted = redact_secrets(&data);
        assert_eq!(redacted["app_secret"], "secr****");
    }
}
