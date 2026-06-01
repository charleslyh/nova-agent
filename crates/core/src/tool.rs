//! [`TypedTool`] for implementations; blanket [`Tool`] impl for `dyn` registration.

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::MorayError;

/// Deserialize tool arguments from a parsed JSON [`Value`].
pub fn parse_tool_args<T: DeserializeOwned>(tool_name: &str, value: Value) -> Result<T, MorayError> {
    serde_json::from_value(value).map_err(|e| {
        MorayError::Message(format!("{tool_name}: invalid JSON arguments: {e}"))
    })
}

/// Typed tool: per-tool [`Args`](Self::Args) + [`run`](Self::run). Metadata (description, JSON schema) comes from the app-layer tool catalog.
#[async_trait]
pub trait TypedTool: Send + Sync {
    type Args: DeserializeOwned + Send;
    const NAME: &'static str;

    async fn run(&self, args: Self::Args) -> Result<String, MorayError>;
}

/// Object-safe tool handle stored in [`crate::Toolbox`].
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;

    async fn call(&self, args: Value) -> Result<String, MorayError>;
}

#[async_trait]
impl<T> Tool for T
where
    T: TypedTool + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        T::NAME
    }

    async fn call(&self, args: Value) -> Result<String, MorayError> {
        let args = parse_tool_args::<T::Args>(T::NAME, args)?;
        self.run(args).await
    }
}
