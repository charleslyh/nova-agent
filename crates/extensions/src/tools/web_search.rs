use async_trait::async_trait;
use moray_core::{MorayError, ToolCallResponder, TypedTool};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Duration;

const API_URL: &str = "http://21.215.220.113:443/text_search";

pub struct WebSearchTool;

#[derive(Deserialize)]
pub struct WebSearchArgs {
    query: String,
}

#[async_trait]
impl TypedTool for WebSearchTool {
    type Args = WebSearchArgs;
    const NAME: &'static str = "web_search";

    async fn run(
        &self,
        args: WebSearchArgs,
        responder: &dyn ToolCallResponder,
    ) -> Result<(), MorayError> {
        let query = args.query.trim();
        if query.is_empty() {
            return Err(MorayError::Message("web_search: query is empty".into()));
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| MorayError::Message(format!("web_search: failed to build client: {e}")))?;
        let response = client
            .post(API_URL)
            .header("Content-Type", "application/json")
            .json(&json!({ "query": query }))
            .send()
            .await
            .map_err(|e| MorayError::Message(format!("web_search: request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(MorayError::Message(format!(
                "web_search: HTTP {}: {}",
                status.as_u16(),
                body
            )));
        }
        let text = response.text().await.map_err(|e| {
            MorayError::Message(format!("web_search: failed to read response body: {e}"))
        })?;
        responder.send_text(text).await;
        Ok(())
    }
}
