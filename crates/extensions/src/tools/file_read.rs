use std::path::PathBuf;

use async_trait::async_trait;
use moray_core::{MorayError, ToolCallResponder, TypedTool};
use serde::Deserialize;

use super::util::path::resolve_path;

pub struct FileReadTool {
    cwd: PathBuf,
}

impl FileReadTool {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[derive(Deserialize)]
pub struct FileReadArgs {
    path: String,
    offset: Option<isize>,
    limit: Option<usize>,
}

#[async_trait]
impl TypedTool for FileReadTool {
    type Args = FileReadArgs;
    const NAME: &'static str = "file_read";

    async fn run(
        &self,
        args: FileReadArgs,
        responder: &dyn ToolCallResponder,
    ) -> Result<(), MorayError> {
        let path = resolve_path(&self.cwd, &args.path);
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| MorayError::Message(format!("file_read: failed to read file: {e}")))?;

        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            responder.send_text("File is empty.".to_string()).await?;
            return Ok(());
        }

        let total = lines.len();
        let start = match args.offset {
            Some(v) if v > 0 => (v as usize).saturating_sub(1).min(total),
            Some(v) if v < 0 => total.saturating_sub(v.unsigned_abs()),
            _ => 0,
        };
        let end = match args.limit {
            Some(limit) => start.saturating_add(limit).min(total),
            None => total,
        };

        if start >= end {
            responder
                .send_text(format!("[No lines in range, file has {total} lines]"))
                .await?;
            return Ok(());
        }

        let output = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}|{}", start + i + 1, line))
            .collect::<Vec<_>>()
            .join("\n");
        responder.send_text(output).await?;
        Ok(())
    }
}
