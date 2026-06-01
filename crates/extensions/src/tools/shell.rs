use std::path::PathBuf;

use async_trait::async_trait;
use moray_core::{MorayError, TypedTool};
use serde::Deserialize;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Runs shell commands in a session working directory with caller-supplied subprocess environment.
pub struct ShellTool {
    cwd: PathBuf,
    subprocess_env: Vec<(String, String)>,
}

impl ShellTool {
    pub fn new(cwd: PathBuf, subprocess_env: Vec<(String, String)>) -> Self {
        Self {
            cwd,
            subprocess_env,
        }
    }
}

#[derive(Deserialize)]
pub struct ShellArgs {
    command: String,
}

#[async_trait]
impl TypedTool for ShellTool {
    type Args = ShellArgs;
    const NAME: &'static str = "shell";

    async fn run(&self, args: ShellArgs) -> Result<String, MorayError> {
        let command = args.command.trim();
        if command.is_empty() {
            return Err(MorayError::Message("shell: command is empty".into()));
        }

        let mut cmd = if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-lc").arg(command);
            c
        };

        cmd.current_dir(&self.cwd);
        for (key, value) in &self.subprocess_env {
            cmd.env(key, value);
        }

        let output = timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS), cmd.output())
            .await
            .map_err(|_| MorayError::Message("shell: command timed out after 30 seconds".into()))?
            .map_err(|e| MorayError::Message(format!("shell: failed to execute command: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        serde_json::to_string(&serde_json::json!({
            "success": output.status.success(),
            "exit_code": output.status.code(),
            "stdout": stdout,
            "stderr": stderr
        }))
        .map_err(|e| MorayError::Message(format!("shell: serialization failed: {e}")))
    }
}
