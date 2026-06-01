//! Moray CLI sidecar: run built-in tools directly (`moray-cli tool run <name> --args '<json>'`).

mod cli_toolbox;

use std::sync::Arc;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use moray_extensions::tools::WebFetchTool;
use moray_extensions::ToolCatalog;
use moray_core::Tool;
use cli_toolbox::CliToolbox;

#[derive(Parser, Debug)]
#[command(name = "moray-cli", about = "Moray tool sidecar")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Execute registered tools directly
    Tool {
        #[command(subcommand)]
        command: ToolCommand,
    },
}

#[derive(Subcommand, Debug)]
enum ToolCommand {
    /// List tools exposed by this CLI binary
    List,
    /// Run a tool by name
    Run {
        name: String,
        #[arg(long, short, num_args = 1.., allow_hyphen_values = true)]
        args: Option<Vec<String>>,
    },
    /// Print JSON Schema for a tool's parameters
    Schema {
        name: String,
    },
}

/// Must match [`moray_desktop_server::ENV_TOOLS_CATALOG_PATH`] set by the desktop shell tool.
const ENV_TOOLS_CATALOG_PATH: &str = "MORAY_TOOLS_CATALOG_PATH";

fn resolve_tools_catalog_path() -> Result<std::path::PathBuf> {
    let path = std::env::var(ENV_TOOLS_CATALOG_PATH)
        .map_err(|_| anyhow::anyhow!("{ENV_TOOLS_CATALOG_PATH} is not set"))?;
    Ok(std::path::PathBuf::from(path))
}

fn build_cli_toolbox() -> Result<CliToolbox> {
    let catalog_path = resolve_tools_catalog_path()?;
    let catalog = ToolCatalog::open(&catalog_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to load tools catalog from {}: {e}",
            catalog_path.display()
        )
    })?;
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(WebFetchTool)];
    CliToolbox::new(catalog, tools).map_err(|e| anyhow::anyhow!("{e}"))
}

fn parse_tool_args(args: Option<Vec<String>>) -> Result<String> {
    let Some(parts) = args.filter(|p| !p.is_empty()) else {
        return Ok("{}".to_string());
    };
    let raw = if parts.len() == 1 {
        parts[0].clone()
    } else {
        parts.join(" ")
    };
    if serde_json::from_str::<serde_json::Value>(&raw).is_err() {
        anyhow::bail!("Invalid JSON arguments");
    }
    Ok(raw)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let toolbox = build_cli_toolbox()?;

    match cli.command {
        Command::Tool { command } => handle_tool(&toolbox, command).await,
    }
}

async fn handle_tool(toolbox: &CliToolbox, cmd: ToolCommand) -> Result<()> {
    match cmd {
        ToolCommand::List => {
            let tools = toolbox.list_tools();
            if tools.is_empty() {
                println!("No CLI tools registered.");
                return Ok(());
            }
            println!("Available CLI tools ({} total):\n", tools.len());
            println!("  {:<24} DESCRIPTION", "NAME");
            println!("  {:<24} ───────────", "────");
            for manifest in tools {
                println!("  {:<24} {}", manifest.name, manifest.description);
            }
            println!();
            println!("Usage:");
            println!("  moray-cli tool run <name> [--args '<json>']");
            println!("  moray-cli tool schema <name>");
            Ok(())
        }
        ToolCommand::Schema { name } => {
            let Some(manifest) = toolbox.manifest(&name) else {
                bail!(
                    "Tool '{name}' not found. Use 'moray-cli tool list' to see available tools."
                );
            };
            let schema = serde_json::json!({
                "name": manifest.name,
                "description": manifest.description,
                "parameters": serde_json::from_str::<serde_json::Value>(&manifest.parameters)
                    .unwrap_or_else(|_| serde_json::Value::String(manifest.parameters.clone())),
            });
            println!("{}", serde_json::to_string_pretty(&schema)?);
            Ok(())
        }
        ToolCommand::Run { name, args } => {
            let arguments = parse_tool_args(args)?;
            match toolbox.run(&name, &arguments).await {
                Ok(output) => {
                    if !output.is_empty() {
                        println!("{output}");
                    }
                    Ok(())
                }
                Err(e) => bail!("Tool '{name}' failed: {e}"),
            }
        }
    }
}
