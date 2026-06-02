//! Tool manifest catalog loaded from TOML (`description`, `parameters`, optional `label`).
//!
//! The canonical `tools.toml` is an application resource (desktop client `resources/tools.toml`).
//! Load it via [`SondaToolCatalog::open`] using a path from the app layer.

use std::collections::HashMap;
use std::path::Path;

use moray_core::ToolManifest;
use serde::Deserialize;
use thiserror::Error;

/// Errors while parsing or validating a tool catalog file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SondaToolCatalogError {
    #[error("read catalog file: {0}")]
    Io(String),

    #[error("invalid catalog TOML: {0}")]
    Parse(String),

    #[error("catalog validation: {0}")]
    Validation(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolsFile {
    tools: Vec<ToolCatalogRow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolCatalogRow {
    name: String,
    #[serde(default)]
    label: String,
    description: String,
    parameters: String,
}

/// In-memory catalog of tool manifests (canonical registration order).
#[derive(Debug, Clone, Default)]
pub struct SondaToolCatalog {
    order: Vec<String>,
    manifests: HashMap<String, ToolManifest>,
    labels: HashMap<String, String>,
}

impl SondaToolCatalog {
    pub fn from_str(raw: &str) -> Result<Self, SondaToolCatalogError> {
        let file: ToolsFile =
            toml::from_str(raw).map_err(|e| SondaToolCatalogError::Parse(e.to_string()))?;
        Self::from_rows(file.tools)
    }

    pub fn open(path: &Path) -> Result<Self, SondaToolCatalogError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| SondaToolCatalogError::Io(format!("{}: {e}", path.display())))?;
        Self::from_str(&raw)
    }

    fn from_rows(rows: Vec<ToolCatalogRow>) -> Result<Self, SondaToolCatalogError> {
        let mut order = Vec::with_capacity(rows.len());
        let mut manifests = HashMap::new();
        let mut labels = HashMap::new();
        let mut seen = std::collections::HashSet::new();

        for row in rows {
            let name = row.name.trim().to_string();
            if name.is_empty() {
                return Err(SondaToolCatalogError::Validation(
                    "tool name must not be empty".into(),
                ));
            }
            if !seen.insert(name.clone()) {
                return Err(SondaToolCatalogError::Validation(format!(
                    "duplicate tool name `{name}`"
                )));
            }
            if row.description.trim().is_empty() {
                return Err(SondaToolCatalogError::Validation(format!(
                    "tool `{name}` description must not be empty"
                )));
            }
            if row.parameters.trim().is_empty() {
                return Err(SondaToolCatalogError::Validation(format!(
                    "tool `{name}` parameters must not be empty"
                )));
            }
            if serde_json::from_str::<serde_json::Value>(&row.parameters).is_err() {
                return Err(SondaToolCatalogError::Validation(format!(
                    "tool `{name}` parameters is not valid JSON"
                )));
            }

            let label = if row.label.trim().is_empty() {
                name.clone()
            } else {
                row.label.trim().to_string()
            };

            order.push(name.clone());
            labels.insert(name.clone(), label);
            manifests.insert(
                name.clone(),
                ToolManifest {
                    name,
                    description: row.description,
                    parameters: row.parameters,
                },
            );
        }

        Ok(Self {
            order,
            manifests,
            labels,
        })
    }

    pub fn is_known(&self, name: &str) -> bool {
        self.manifests.contains_key(name)
    }

    pub fn manifest(&self, name: &str) -> Option<&ToolManifest> {
        self.manifests.get(name)
    }

    pub fn label<'a>(&'a self, name: &'a str) -> &'a str {
        self.labels
            .get(name)
            .map(String::as_str)
            .unwrap_or(name)
    }

    /// Tool names in catalog order.
    pub fn names(&self) -> &[String] {
        &self.order
    }

    /// All manifests in catalog order.
    pub fn manifests_in_order(&self) -> Vec<ToolManifest> {
        self.order
            .iter()
            .filter_map(|name| self.manifests.get(name).cloned())
            .collect()
    }

    /// Subset catalog preserving order of `names` as listed in the full catalog.
    pub fn filter(&self, names: &[&str]) -> Self {
        let allow: std::collections::HashSet<&str> = names.iter().copied().collect();
        let order: Vec<String> = self
            .order
            .iter()
            .filter(|name| allow.contains(name.as_str()))
            .cloned()
            .collect();
        let manifests = order
            .iter()
            .filter_map(|name| self.manifests.get(name).map(|d| (name.clone(), d.clone())))
            .collect();
        let labels = order
            .iter()
            .filter_map(|name| self.labels.get(name).map(|l| (name.clone(), l.clone())))
            .collect();
        Self {
            order,
            manifests,
            labels,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[tools]]
name = "echo"
label = "Echo"
description = "Echo back"
parameters = '{"type":"object","properties":{"content":{"type":"string"}},"required":["content"]}'

[[tools]]
name = "calc"
description = "Calculator"
parameters = '{"type":"object"}'
"#;

    #[test]
    fn loads_catalog_and_labels() {
        let catalog = SondaToolCatalog::from_str(SAMPLE).expect("valid");
        assert_eq!(catalog.names(), &["echo", "calc"]);
        assert_eq!(catalog.label("echo"), "Echo");
        assert_eq!(catalog.label("calc"), "calc");
    }

    #[test]
    fn rejects_duplicate_names() {
        let raw = r#"
[[tools]]
name = "a"
description = "d"
parameters = '{}'
[[tools]]
name = "a"
description = "d2"
parameters = '{}'
"#;
        let err = SondaToolCatalog::from_str(raw).expect_err("dup");
        assert!(matches!(err, SondaToolCatalogError::Validation(_)));
    }
}
