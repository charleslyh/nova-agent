//! WeCom channel construction and catalog secret merge/redact from `data`.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use moray_channels::ImChannel;

use super::channel::WeComChannel;
use crate::channels::secrets::{mask_secret, merge_preserved_secret};

#[derive(Debug, thiserror::Error)]
pub enum WeComConfigError {
    #[error("invalid wecom channel data: {0}")]
    InvalidData(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WeComChannelData {
    pub bot_id: String,
    pub bot_secret: String,
    #[serde(default = "default_wecom_heartbeat_interval")]
    pub heartbeat_interval_ms: u64,
    #[serde(default = "default_wecom_max_reconnect")]
    pub max_reconnect_attempts: u32,
    #[serde(default)]
    pub welcome_message: Option<String>,
}

fn default_wecom_heartbeat_interval() -> u64 {
    30_000
}

fn default_wecom_max_reconnect() -> u32 {
    10
}

fn parse_data(data: &Value) -> Result<WeComChannelData, WeComConfigError> {
    serde_json::from_value(data.clone()).map_err(|e| WeComConfigError::InvalidData(e.to_string()))
}

pub fn channel_from_config(
    data: &Value,
    workspace_dir: PathBuf,
) -> Result<Arc<dyn ImChannel>, WeComConfigError> {
    let parsed = parse_data(data)?;
    Ok(Arc::new(
        WeComChannel::new(
            parsed.bot_id,
            parsed.bot_secret,
            workspace_dir,
        )
        .with_heartbeat_interval(parsed.heartbeat_interval_ms)
        .with_max_reconnect_attempts(parsed.max_reconnect_attempts)
        .with_welcome_message(parsed.welcome_message),
    ))
}

pub fn merge_secrets(incoming: &Value, existing: &Value) -> Value {
    let Ok(mut inc) = serde_json::from_value::<WeComChannelData>(incoming.clone()) else {
        return incoming.clone();
    };
    let Ok(ex) = serde_json::from_value::<WeComChannelData>(existing.clone()) else {
        return incoming.clone();
    };
    inc.bot_secret = merge_preserved_secret(&inc.bot_secret, &ex.bot_secret);
    serde_json::to_value(inc).unwrap_or_else(|_| incoming.clone())
}

pub fn redact_secrets(data: &Value) -> Value {
    let Ok(mut parsed) = serde_json::from_value::<WeComChannelData>(data.clone()) else {
        return data.clone();
    };
    parsed.bot_secret = mask_secret(&parsed.bot_secret);
    serde_json::to_value(parsed).unwrap_or_else(|_| data.clone())
}
