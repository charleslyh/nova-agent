use async_trait::async_trait;
use moray_core::{MorayError, ToolCallResponder, TypedTool};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

use super::util::image_sanitize::sanitize_image_api_response;

const API_URL: &str = "http://21.215.220.113:443/image_create";

pub struct ImageCreateTool;

#[derive(Deserialize)]
pub struct ImageCreateArgs {
    query: String,
    #[serde(default = "default_aspect_ratio")]
    aspect_ratio: String,
}

fn default_aspect_ratio() -> String {
    "1:1".into()
}

#[async_trait]
impl TypedTool for ImageCreateTool {
    type Args = ImageCreateArgs;
    const NAME: &'static str = "image_create";

    async fn run(
        &self,
        args: ImageCreateArgs,
        responder: &dyn ToolCallResponder,
        _cancellation: CancellationToken,
    ) -> Result<(), MorayError> {
        let query = args.query.trim();
        if query.is_empty() {
            return Err(MorayError::Message("image_create: query is empty".into()));
        }

        let aspect_ratio = args.aspect_ratio.trim();
        let aspect_ratio = if aspect_ratio.is_empty() {
            "1:1"
        } else {
            aspect_ratio
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| MorayError::Message(format!("image_create: failed to build client: {e}")))?;
        let response = client
            .post(API_URL)
            .header("Content-Type", "application/json")
            .json(&json!({
                "query": query,
                "aspect_ratio": aspect_ratio
            }))
            .send()
            .await
            .map_err(|e| MorayError::Message(format!("image_create: request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(MorayError::Message(format!(
                "image_create: HTTP {}: {}",
                status.as_u16(),
                body
            )));
        }
        let body = response.text().await.map_err(|e| {
            MorayError::Message(format!("image_create: failed to read response body: {e}"))
        })?;
        responder
            .send_text(sanitize_image_api_response(&body))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use moray_core::{ToolCallResponder, ToolboxError};

    struct NoopResponder;

    #[async_trait]
    impl ToolCallResponder for NoopResponder {
        async fn send_extra(&self, _: serde_json::Value) -> Result<(), ToolboxError> {
            Ok(())
        }

        async fn send_text(&self, _: String) -> Result<(), ToolboxError> {
            Ok(())
        }
    }

    #[test]
    fn default_aspect_ratio_when_omitted() {
        let args: ImageCreateArgs =
            serde_json::from_value(json!({ "query": "a cat" })).expect("deserialize");
        assert_eq!(args.aspect_ratio, "1:1");
    }

    #[tokio::test]
    async fn rejects_empty_query() {
        let tool = ImageCreateTool;
        let err = tool
            .run(
                ImageCreateArgs {
                    query: "   ".into(),
                    aspect_ratio: "1:1".into(),
                },
                &NoopResponder,
            )
            .await
            .expect_err("empty query");
        assert!(err.to_string().contains("query is empty"));
    }
}
