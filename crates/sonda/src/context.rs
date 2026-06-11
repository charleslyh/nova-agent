//! Injected [`ContextEngine`] construction for live sessions.

use std::sync::Arc;

use moray_core::{ChatCompletionRequestMessage, ContextEngine};

use crate::error::Result;

pub type ContextBuilder = Arc<
    dyn Fn(&str, Vec<ChatCompletionRequestMessage>) -> Result<Arc<dyn ContextEngine>>
        + Send
        + Sync,
>;
