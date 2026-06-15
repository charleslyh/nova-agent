//! CLI-local toolbox: registered [`Tool`] instances for `tool list` / `tool run` / `tool schema`.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use async_trait::async_trait;
use moray_core::{MorayError, Tool, ToolCallResponder, ToolManifest};
use moray_sonda::SondaToolCatalog;
use serde_json::Value;

/// Writes model-visible tool output to stdout as it arrives.
struct PrintingResponder;

#[async_trait]
impl ToolCallResponder for PrintingResponder {
    async fn send_extra(&self, _: Value) -> Result<(), moray_core::ToolboxError> {
        Ok(())
    }

    async fn send_text(&self, text: String) -> Result<(), moray_core::ToolboxError> {
        print!("{text}");
        std::io::stdout().flush().map_err(|e| moray_core::ToolboxError::DeliverFailed {
            reason: e.to_string(),
        })?;
        Ok(())
    }
}

/// Tools exposed by this `moray-cli` binary (catalog metadata + runnable instances).
///
/// [`SondaToolCatalog`] supplies manifests only; listing uses the registered tool set.
/// Catalog rows without a matching tool are dropped at construction.
pub struct CliToolbox {
    catalog: SondaToolCatalog,
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl CliToolbox {
    /// Each tool's [`Tool::name`] must exist in `catalog`. Catalog rows without a tool are ignored.
    pub fn new(
        catalog: SondaToolCatalog,
        tool_instances: impl IntoIterator<Item = Arc<dyn Tool>>,
    ) -> Result<Self, MorayError> {
        let mut tools = HashMap::new();
        for tool in tool_instances {
            let name = tool.name().to_string();
            if tools.contains_key(&name) {
                return Err(MorayError::Message(format!(
                    "duplicate CLI tool `{name}`"
                )));
            }
            tools.insert(name, tool);
        }

        if tools.is_empty() {
            return Err(MorayError::Message(
                "at least one CLI tool must be registered".into(),
            ));
        }

        for name in tools.keys() {
            if name.trim().is_empty() {
                return Err(MorayError::Message(
                    "registered CLI tool name must not be empty".into(),
                ));
            }
            if !catalog.is_known(name) {
                return Err(MorayError::Message(format!(
                    "tool `{name}` is not defined in the CLI tool catalog"
                )));
            }
        }

        let allow: Vec<&str> = tools.keys().map(String::as_str).collect();
        let catalog = catalog.filter(&allow);

        Ok(Self { catalog, tools })
    }

    /// Tool manifests in catalog order (registered tools only).
    pub fn list_tools(&self) -> Vec<ToolManifest> {
        self.catalog
            .names()
            .iter()
            .filter_map(|id| {
                self.tools
                    .contains_key(id)
                    .then(|| self.catalog.manifest(id).cloned())
                    .flatten()
            })
            .collect()
    }

    pub fn manifest(&self, name: &str) -> Option<ToolManifest> {
        if self.tools.contains_key(name) {
            self.catalog.manifest(name).cloned()
        } else {
            None
        }
    }

    pub async fn run(&self, name: &str, arguments: Value) -> Result<(), MorayError> {
        let Some(tool) = self.tools.get(name) else {
            return Err(MorayError::Message(format!("unknown tool {name}")));
        };
        tool.call(arguments, &PrintingResponder).await
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use moray_core::{ToolCallResponder, TypedTool};
    use moray_sonda::SondaToolCatalog;
    use std::sync::Arc;

    use super::CliToolbox;

    const TEST_CATALOG: &str = r#"
[[tools]]
name = "beta"
description = "b"
parameters = '{}'
[[tools]]
name = "alpha"
description = "a"
parameters = '{}'
[[tools]]
name = "gamma"
description = "g"
parameters = '{}'
"#;

    struct BetaTool;
    #[async_trait]
    impl TypedTool for BetaTool {
        type Args = serde_json::Value;
        const NAME: &'static str = "beta";
        async fn run(
            &self,
            _: serde_json::Value,
            responder: &dyn ToolCallResponder,
        ) -> Result<(), moray_core::MorayError> {
            responder.send_text("beta".into()).await;
            Ok(())
        }
    }

    struct AlphaTool;
    #[async_trait]
    impl TypedTool for AlphaTool {
        type Args = serde_json::Value;
        const NAME: &'static str = "alpha";
        async fn run(
            &self,
            _: serde_json::Value,
            responder: &dyn ToolCallResponder,
        ) -> Result<(), moray_core::MorayError> {
            responder.send_text("alpha".into()).await;
            Ok(())
        }
    }

    struct GammaTool;
    #[async_trait]
    impl TypedTool for GammaTool {
        type Args = serde_json::Value;
        const NAME: &'static str = "gamma";
        async fn run(
            &self,
            _: serde_json::Value,
            responder: &dyn ToolCallResponder,
        ) -> Result<(), moray_core::MorayError> {
            responder.send_text("gamma".into()).await;
            Ok(())
        }
    }

    #[test]
    fn list_tools_preserves_catalog_order() {
        let catalog = SondaToolCatalog::from_str(TEST_CATALOG).unwrap();
        let tools: Vec<Arc<dyn moray_core::Tool>> = vec![
            Arc::new(BetaTool),
            Arc::new(AlphaTool),
            Arc::new(GammaTool),
        ];
        let toolbox = CliToolbox::new(catalog, tools).unwrap();

        let names: Vec<_> = toolbox.list_tools().into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["beta", "alpha", "gamma"]);
    }

    #[test]
    fn list_tools_omits_unregistered_catalog_rows() {
        let catalog = SondaToolCatalog::from_str(TEST_CATALOG).unwrap();
        let tools: Vec<Arc<dyn moray_core::Tool>> = vec![Arc::new(BetaTool)];
        let toolbox = CliToolbox::new(catalog, tools).unwrap();
        let names: Vec<_> = toolbox.list_tools().into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["beta"]);
    }

    #[test]
    fn new_rejects_tool_missing_from_catalog() {
        let catalog = SondaToolCatalog::from_str(
            r#"
[[tools]]
name = "echo"
description = "d"
parameters = '{}'
"#,
        )
        .unwrap();

        struct OtherTool;
        #[async_trait]
        impl TypedTool for OtherTool {
            type Args = serde_json::Value;
            const NAME: &'static str = "other";
            async fn run(
                &self,
                _: serde_json::Value,
                responder: &dyn ToolCallResponder,
            ) -> Result<(), moray_core::MorayError> {
                responder.send_text("ok".into()).await;
                Ok(())
            }
        }

        let tools: Vec<Arc<dyn moray_core::Tool>> = vec![Arc::new(OtherTool)];
        match CliToolbox::new(catalog, tools) {
            Err(e) => assert!(e.to_string().contains("other")),
            Ok(_) => panic!("expected missing catalog row"),
        }
    }
}
