use std::path::PathBuf;

use async_trait::async_trait;
use moray_core::{MorayError, ToolCallResponder, TypedTool};
use serde::Deserialize;

use super::util::path::resolve_path;

pub struct FileWriteTool {
    cwd: PathBuf,
}

impl FileWriteTool {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[derive(Deserialize)]
pub struct FileWriteArgs {
    path: String,
    content: String,
}

#[async_trait]
impl TypedTool for FileWriteTool {
    type Args = FileWriteArgs;
    const NAME: &'static str = "file_write";

    async fn run(
        &self,
        args: FileWriteArgs,
        responder: &dyn ToolCallResponder,
    ) -> Result<(), MorayError> {
        let path = resolve_path(&self.cwd, &args.path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                MorayError::Message(format!(
                    "file_write: failed to create parent directory: {e}"
                ))
            })?;
        }
        tokio::fs::write(&path, args.content.as_bytes())
            .await
            .map_err(|e| MorayError::Message(format!("file_write: failed to write file: {e}")))?;
        responder
            .send_text(format!(
                "Written {} bytes to {}",
                args.content.len(),
                path.display()
            ))
            .await;
        Ok(())
    }
}
