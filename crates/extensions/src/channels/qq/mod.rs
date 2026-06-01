//! QQ Official Bot channel.

mod approval;
mod channel;
mod factory;
mod outbound;
mod reply;

pub use channel::QQChannel;
pub use factory::{
    channel_from_config, merge_secrets, redact_secrets, QQEnvironment, QqConfigError,
};
