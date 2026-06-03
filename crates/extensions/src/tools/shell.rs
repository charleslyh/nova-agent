use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use moray_core::{MorayError, ToolCallResponder, TypedTool};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const READ_CHUNK_SIZE: usize = 8192;

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

    async fn run(
        &self,
        args: ShellArgs,
        responder: &dyn ToolCallResponder,
    ) -> Result<(), MorayError> {
        let command = args.command.trim();
        if command.is_empty() {
            return Err(MorayError::Message("shell: command is empty".into()));
        }

        let run = run_shell_command(self, command, responder);
        timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS), run)
            .await
            .map_err(|_| MorayError::Message("shell: command timed out after 30 seconds".into()))?
    }
}

struct PipeReader<R> {
    reader: BufReader<R>,
    buf: Vec<u8>,
    eof: bool,
    prefix: Option<&'static str>,
    name: &'static str,
}

impl<R: AsyncRead + Unpin> PipeReader<R> {
    fn new(reader: R, prefix: Option<&'static str>, name: &'static str) -> Self {
        Self {
            reader: BufReader::new(reader),
            buf: vec![0u8; READ_CHUNK_SIZE],
            eof: false,
            prefix,
            name,
        }
    }

    async fn read_chunk(&mut self, responder: &dyn ToolCallResponder) -> Result<(), MorayError> {
        // read stdout or stderr chunk by chunk
        let n = self.reader.read(&mut self.buf).await.map_err(|e| {
            MorayError::Message(format!("shell: failed to read {}: {e}", self.name))
        })?;

        // n == 0 means EOF, set eof to true so the while loop in run_shell_command can exit
        if n == 0 {
            self.eof = true;
            return Ok(());
        }

        // convert the chunk to string that is utf8 encoded that can be streamed to the model
        let chunk = String::from_utf8_lossy(&self.buf[..n]).into_owned();

        // add the prefix to the chunk so the model can distinguish stdout and stderr
        // stdout: None
        // stderr: "[stderr] "
        let text = match self.prefix {
            Some(prefix) => format!("{prefix}{chunk}"),
            None => chunk,
        };

        // send the chunk to agent and UI, then the model can see the output in next ReAct loop step
        responder.send_text(text).await;
        Ok(())
    }
}

async fn run_shell_command(
    tool: &ShellTool,
    command: &str,
    responder: &dyn ToolCallResponder,
) -> Result<(), MorayError> {
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-lc").arg(command);
        c
    };

    cmd.current_dir(&tool.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    for (key, value) in &tool.subprocess_env {
        cmd.env(key, value);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| MorayError::Message(format!("shell: failed to execute command: {e}")))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| MorayError::Message("shell: stdout not piped".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| MorayError::Message("shell: stderr not piped".into()))?;

    let mut stdout = PipeReader::new(stdout, None, "stdout");
    let mut stderr = PipeReader::new(stderr, Some("[stderr] "), "stderr");

    while !stdout.eof || !stderr.eof {
        tokio::select! {
            result = stdout.read_chunk(responder), if !stdout.eof => result?,
            result = stderr.read_chunk(responder), if !stderr.eof => result?,
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| MorayError::Message(format!("shell: failed to wait for command: {e}")))?;

    let footer = serde_json::to_string(&serde_json::json!({
        "success": status.success(),
        "exit_code": status.code(),
    }))
    .map_err(|e| MorayError::Message(format!("shell: serialization failed: {e}")))?;
    responder.send_text(format!("\n{footer}")).await;
    Ok(())
}
