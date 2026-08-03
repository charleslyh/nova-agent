use async_trait::async_trait;
use regex::Regex;
use reqwest::header::CONTENT_TYPE;
use moray_core::{MorayError, ToolCallResponder, TypedTool};
use serde::Deserialize;
use tokio::time::Duration;

pub struct WebFetchTool;

#[derive(Deserialize)]
pub struct WebFetchArgs {
    url: String,
}

fn html_to_text(html: &str) -> String {
    // Keep this simple: remove scripts/styles first, then drop tags.
    let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").expect("valid regex");
    let style_re = Regex::new(r"(?is)<style[^>]*>.*?</style>").expect("valid regex");
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid regex");
    let no_script = script_re.replace_all(html, " ");
    let no_style = style_re.replace_all(&no_script, " ");
    tag_re.replace_all(&no_style, " ").to_string()
}

#[async_trait]
impl TypedTool for WebFetchTool {
    type Args = WebFetchArgs;
    const NAME: &'static str = "web_fetch";

    async fn run(
        &self,
        _call_id: &str,
        args: WebFetchArgs,
        responder: &dyn ToolCallResponder,
    ) -> Result<(), MorayError> {
        let url = reqwest::Url::parse(args.url.trim())
            .map_err(|e| MorayError::Message(format!("web_fetch: invalid URL: {e}")))?;
        match url.scheme() {
            "http" | "https" => {}
            _ => {
                return Err(MorayError::Message(
                    "web_fetch: only http/https URLs are supported".into(),
                ))
            }
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| MorayError::Message(format!("web_fetch: failed to build client: {e}")))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| MorayError::Message(format!("web_fetch: request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(MorayError::Message(format!(
                "web_fetch: HTTP {}",
                status.as_u16()
            )));
        }
        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let body = resp.text().await.map_err(|e| {
            MorayError::Message(format!("web_fetch: failed to read response body: {e}"))
        })?;

        let text = if content_type.contains("text/html") {
            html_to_text(&body)
        } else {
            body
        };
        responder.send_text(text).await?;
        Ok(())
    }
}
