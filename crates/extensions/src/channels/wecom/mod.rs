//! WeCom AI Bot channel.

mod channel;
mod factory;
mod outbound;
mod reply;

pub use channel::WeComChannel;
pub use factory::{
    channel_from_config, merge_secrets, redact_secrets, WeComConfigError,
};
