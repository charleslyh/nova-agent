use std::path::PathBuf;

use async_trait::async_trait;
use moray_core::{MorayError, ToolCallResponder, TypedTool};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

use super::util::image_sanitize::sanitize_image_api_response;
use super::util::path::resolve_path;
use super::util::upload::upload_local_file;

const API_URL: &str = "http://21.215.220.113:443/image_edit";

pub struct ImageEditTool {
    cwd: PathBuf,
}

impl ImageEditTool {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[derive(Deserialize)]
pub struct ImageEditArgs {
    prompt: String,
    image_uri: String,
}

#[async_trait]
impl TypedTool for ImageEditTool {
    type Args = ImageEditArgs;
    const NAME: &'static str = "image_edit";

    async fn run(
        &self,
        args: ImageEditArgs,
        responder: &dyn ToolCallResponder,
        _cancellation: CancellationToken,
    ) -> Result<(), MorayError> {
        let prompt = args.prompt.trim();
        if prompt.is_empty() {
            return Err(MorayError::Message("image_edit: prompt is empty".into()));
        }

        let image_uri = args.image_uri.trim();
        if image_uri.is_empty() {
            return Err(MorayError::Message("image_edit: image_uri is empty".into()));
        }

        let final_image_url = if image_uri.starts_with("http://") || image_uri.starts_with("https://")
        {
            image_uri.to_string()
        } else {
            let path = resolve_path(&self.cwd, image_uri);
            upload_local_file(&path).await.map_err(|e| {
                MorayError::Message(format!("image_edit: failed to upload local image: {e}"))
            })?
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| MorayError::Message(format!("image_edit: failed to build client: {e}")))?;

        let response = client
            .post(API_URL)
            .header("Content-Type", "application/json")
            .json(&json!({
                "prompt": prompt,
                "image_url": final_image_url
            }))
            .send()
            .await
            .map_err(|e| MorayError::Message(format!("image_edit: request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(MorayError::Message(format!(
                "image_edit: HTTP {}: {}",
                status.as_u16(),
                body
            )));
        }

        let body = response.text().await.map_err(|e| {
            MorayError::Message(format!("image_edit: failed to read response body: {e}"))
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

    #[tokio::test]
    async fn rejects_empty_prompt() {
        let tool = ImageEditTool::new(std::env::temp_dir());
        let err = tool
            .run(
                ImageEditArgs {
                    prompt: "   ".into(),
                    image_uri: "https://example.com/img.png".into(),
                },
                &NoopResponder,
            )
            .await
            .expect_err("empty prompt");
        assert!(err.to_string().contains("prompt is empty"));
    }

    #[tokio::test]
    async fn rejects_empty_image_uri() {
        let tool = ImageEditTool::new(std::env::temp_dir());
        let err = tool
            .run(
                ImageEditArgs {
                    prompt: "make it blue".into(),
                    image_uri: "  ".into(),
                },
                &NoopResponder,
            )
            .await
            .expect_err("empty image_uri");
        assert!(err.to_string().contains("image_uri is empty"));
    }
}
