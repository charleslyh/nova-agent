//! Sonda **settings store**: agents / completions 的磁盘读写、结构校验，以及按需解析 completion 的 `api_key`。
//!
//! 调用方通过 [`SondaSettingsStore::load`] 传入 bundled 与 user 两个路径；store 内加载并合并
//! （bundled 为基础，user 为 patch）。写回仅作用于 user 路径。与 [`super::session_catalog`] 配合，
//! 由 [`super::sonda::SondaBuilder::build`] 做跨文件一致性检查。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use moray_extensions::completions::Endpoint;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::error::{
    require_argument_nonempty, require_nonempty_field, BadEnvironmentVariable, FileIoError,
    InvalidArguments, InvalidContent, MissingReference,
};

type Result<T> = std::result::Result<T, SondaSettingsStoreError>;

#[derive(Debug, Error)]
pub enum SondaSettingsStoreError {
    #[error(transparent)]
    InvalidContent(#[from] InvalidContent),

    #[error(transparent)]
    MissingReference(#[from] MissingReference),

    #[error(transparent)]
    BadEnvironment(#[from] BadEnvironmentVariable),

    #[error(transparent)]
    FileIo(#[from] FileIoError),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SondaSettingsAgentEntry {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub completion_id: String,
    /// Whitelist of built-in tool names. When empty, the agent has no tools.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Optional persona text for system prompt `{{character}}` substitution.
    #[serde(default)]
    pub character: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SondaSettingsCompletionEntry {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SondaSettingsFile {
    /// System prompt template (`{{character}}` and other placeholders).
    #[serde(default)]
    pub preamble_template: Option<String>,

    #[serde(default)]
    pub agents: Vec<SondaSettingsAgentEntry>,

    #[serde(default)]
    pub completions: Vec<SondaSettingsCompletionEntry>,
}

#[derive(Debug)]
pub struct SondaSettingsStore {
    #[allow(dead_code)]
    bundled_path: PathBuf,
    user_path: PathBuf,
    inner: RwLock<SondaSettingsFile>,
}

impl SondaSettingsStore {
    /// Loads bundled base settings and merges an optional user patch from `user_path`.
    pub fn load(bundled_path: &Path, user_path: &Path) -> Result<SondaSettingsStore> {
        let base = read_settings_file_optional(bundled_path, "bundled settings")?
            .unwrap_or_default();
        let patch = read_settings_file_optional(user_path, "user settings")?.unwrap_or_default();
        let merged = merge_settings(base, patch);
        validate_settings_file(&merged)?;

        Ok(SondaSettingsStore {
            bundled_path: bundled_path.to_path_buf(),
            user_path: user_path.to_path_buf(),
            inner: RwLock::new(merged),
        })
    }

    /// System prompt template after merge (`{{character}}` and other placeholders).
    pub fn preamble_template(&self) -> String {
        self.inner
            .read()
            .preamble_template
            .clone()
            .unwrap_or_default()
    }

    pub fn catalog(&self) -> SondaSettingsFile {
        self.inner.read().clone()
    }

    pub fn update_agent(
        &self,
        agent_id: &str,
        name: &str,
        completion_id: &str,
        allowed_tools: Vec<String>,
        character: Option<String>,
    ) -> crate::error::Result<()> {
        let agent_id = require_argument_nonempty(agent_id, "agent_id")?;
        let name = require_argument_nonempty(name, "name")?;
        let completion_id = require_argument_nonempty(completion_id, "completion_id")?;

        {
            let inner = self.inner.read();
            ensure_completion_exists(&inner, completion_id)?;
        }

        {
            let mut inner = self.inner.write();
            let entry = inner
                .agents
                .iter_mut()
                .find(|a| a.id == agent_id)
                .ok_or_else(|| {
                    InvalidArguments::new("agent_id", "corresponding agent not found")
                })?;

            entry.name = name.to_string();
            entry.completion_id = completion_id.to_string();
            entry.allowed_tools = allowed_tools;
            entry.character = character;
        }

        save(self)?;

        Ok(())
    }

    pub fn resolve_completion_endpoint(
        &self,
        agent_id: &str,
    ) -> crate::error::Result<Endpoint> {
        let inner = self.inner.read();

        let agent = inner
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| InvalidArguments::new("agent_id", "corresponding agent not found"))?;

        let c = find_completion_for_agent(&inner, agent)?;
        let api_key = resolve_raw_api_key(&c.api_key)?;

        Ok(Endpoint::new(api_key, c.base_url.clone(), c.model.clone()))
    }

    pub fn has_agent(&self, agent_id: &str) -> bool {
        self.inner.read().agents.iter().any(|a| a.id == agent_id)
    }

    /// Returns the agent's `allowed_tools` whitelist (empty means no tools).
    pub fn agent_allowed_tools(&self, agent_id: &str) -> crate::error::Result<Vec<String>> {
        let inner = self.inner.read();
        let agent = inner
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| InvalidArguments::new("agent_id", "corresponding agent not found"))?;
        Ok(agent.allowed_tools.clone())
    }

    /// Returns the agent's optional `character` persona text.
    pub fn agent_character(&self, agent_id: &str) -> crate::error::Result<Option<String>> {
        let inner = self.inner.read();
        let agent = inner
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| InvalidArguments::new("agent_id", "corresponding agent not found"))?;
        Ok(agent.character.clone())
    }
}

fn validate_api_key_kind(raw: &Option<String>) -> Result<()> {
    let Some(raw) = raw else {
        return Ok(());
    };

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(InvalidContent::new("literal api_key is empty").into());
    }

    if let Some(rest) = trimmed.strip_prefix("env:") {
        let name = rest.trim();
        if name.is_empty() {
            return Err(InvalidContent::new(
                "api_key env: reference has empty variable name",
            )
            .into());
        }
    }

    Ok(())
}

fn validate_settings_file(settings: &SondaSettingsFile) -> Result<()> {
    if settings.completions.is_empty() || settings.agents.is_empty() {
        return Err(InvalidContent::new(
            "settings must include at least one `completions` and one `agents` entry",
        )
        .into());
    }

    let mut seen_completions = HashSet::new();
    for c in &settings.completions {
        require_nonempty_field(&c.id, "completions.id")?;
        if !seen_completions.insert(&c.id) {
            return Err(InvalidContent::new(format!("duplicate completion id `{}`", c.id)).into());
        }

        require_nonempty_field(&c.base_url, "base_url")?;
        require_nonempty_field(&c.model, "model")?;
        require_nonempty_field(&c.name, "name")?;

        validate_api_key_kind(&c.api_key)?;
    }

    let mut seen_agents = HashSet::new();
    for a in &settings.agents {
        require_nonempty_field(&a.id, "agents.id")?;
        if !seen_agents.insert(&a.id) {
            return Err(InvalidContent::new(format!("duplicate agent id `{}`", a.id)).into());
        }
        require_nonempty_field(&a.completion_id, "agents.completion_id")?;

        if !settings.completions.iter().any(|c| c.id == a.completion_id) {
            return Err(InvalidContent::new(format!(
                "agent `{}` references unknown completion `{}`",
                a.id, a.completion_id
            ))
            .into());
        }
        require_nonempty_field(&a.name, "name")?;
    }

    Ok(())
}

fn read_settings_file_optional(path: &Path, label: &str) -> Result<Option<SondaSettingsFile>> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|source| FileIoError::new("read", path.to_path_buf(), source))?;
    let file: SondaSettingsFile = toml::from_str(&raw).map_err(|e| {
        InvalidContent::new(format!("invalid {label} ({}): {e}", path.display()))
    })?;
    Ok(Some(file))
}

fn merge_settings(mut base: SondaSettingsFile, patch: SondaSettingsFile) -> SondaSettingsFile {
    if let Some(template) = patch.preamble_template {
        base.preamble_template = Some(template);
    }
    merge_agents(&mut base.agents, patch.agents);
    merge_completions(&mut base.completions, patch.completions);
    base
}

fn merge_agents(base: &mut Vec<SondaSettingsAgentEntry>, patch: Vec<SondaSettingsAgentEntry>) {
    for item in patch {
        let id = item.id.as_str();
        if let Some(existing) = base.iter_mut().find(|entry| entry.id == id) {
            let merged = merge_agent_entry(existing.clone(), item);
            *existing = merged;
        } else {
            base.push(item);
        }
    }
}

fn merge_completions(
    base: &mut Vec<SondaSettingsCompletionEntry>,
    patch: Vec<SondaSettingsCompletionEntry>,
) {
    for item in patch {
        let id = item.id.as_str();
        if let Some(existing) = base.iter_mut().find(|entry| entry.id == id) {
            let merged = merge_completion_entry(existing.clone(), item);
            *existing = merged;
        } else {
            base.push(item);
        }
    }
}

fn merge_agent_entry(
    base: SondaSettingsAgentEntry,
    patch: SondaSettingsAgentEntry,
) -> SondaSettingsAgentEntry {
    SondaSettingsAgentEntry {
        id: patch.id,
        name: pick_nonempty(patch.name, base.name),
        completion_id: pick_nonempty(patch.completion_id, base.completion_id),
        allowed_tools: if patch.allowed_tools.is_empty() {
            base.allowed_tools
        } else {
            patch.allowed_tools
        },
        character: patch.character.or(base.character),
    }
}

fn merge_completion_entry(
    base: SondaSettingsCompletionEntry,
    patch: SondaSettingsCompletionEntry,
) -> SondaSettingsCompletionEntry {
    SondaSettingsCompletionEntry {
        id: patch.id,
        name: pick_nonempty(patch.name, base.name),
        base_url: pick_nonempty(patch.base_url, base.base_url),
        model: pick_nonempty(patch.model, base.model),
        api_key: patch.api_key.or(base.api_key),
    }
}

fn pick_nonempty(patch: String, base: String) -> String {
    if patch.is_empty() { base } else { patch }
}

fn save(settings: &SondaSettingsStore) -> Result<()> {
    let path = settings.user_path.as_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| FileIoError::new("mkdir", parent.to_path_buf(), source))?;
    }
    let inner = settings.inner.read();
    let s = toml::to_string_pretty(&*inner).map_err(|e| {
        InvalidContent::new(format!("invalid settings file: {e}"))
    })?;
    drop(inner);
    std::fs::write(path, s)
        .map_err(|source| FileIoError::new("write", path.to_path_buf(), source))?;
    Ok(())
}

#[derive(Debug)]
enum ApiKeyCell {
    Literal(String),
    EnvVar(String),
    Omitted,
}

fn read_env_trimmed(name: &str) -> std::result::Result<String, BadEnvironmentVariable> {
    let value = std::env::var(name).map_err(|_| BadEnvironmentVariable::not_set(name))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(BadEnvironmentVariable::empty(name));
    }
    Ok(value.to_string())
}

fn classify_api_key_cell(raw: &Option<String>) -> Result<ApiKeyCell> {
    let Some(raw) = raw else {
        return Ok(ApiKeyCell::Omitted);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(InvalidContent::new("literal api_key is empty after trim").into());
    }
    if let Some(rest) = trimmed.strip_prefix("env:") {
        let name = rest.trim();
        if name.is_empty() {
            return Err(InvalidContent::new(
                "api_key env: reference has empty variable name",
            )
            .into());
        }
        return Ok(ApiKeyCell::EnvVar(name.to_string()));
    }
    Ok(ApiKeyCell::Literal(trimmed.to_string()))
}

fn materialize_api_key(cell: &ApiKeyCell) -> Result<String> {
    match cell {
        ApiKeyCell::Literal(s) => Ok(s.clone()),
        ApiKeyCell::Omitted => Ok("".to_string()),
        ApiKeyCell::EnvVar(name) => Ok(read_env_trimmed(name)?),
    }
}

fn resolve_raw_api_key(raw: &Option<String>) -> Result<String> {
    materialize_api_key(&classify_api_key_cell(raw)?)
}

fn ensure_completion_exists(inner: &SondaSettingsFile, completion_id: &str) -> Result<()> {
    if !inner.completions.iter().any(|c| c.id == completion_id) {
        return Err(
            MissingReference::new(format!("unknown completion id `{completion_id}`")).into(),
        );
    }
    Ok(())
}

fn find_completion_for_agent<'a>(
    inner: &'a SondaSettingsFile,
    agent: &'a SondaSettingsAgentEntry,
) -> Result<&'a SondaSettingsCompletionEntry> {
    inner
        .completions
        .iter()
        .find(|c| c.id == agent.completion_id)
        .ok_or_else(|| {
            MissingReference::new(format!(
                "completion `{}` for agent `{}` missing",
                agent.completion_id, agent.id
            ))
            .into()
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use std::collections::HashMap;

    use moray_extensions::auths::AlwaysAsking;
    use crate::transcripts::SondaSessionTranscripts;
    use tempfile::tempdir;

    use crate::error::SondaError;
    use moray_channels::ChannelCatalog;
    use crate::session_catalog::SondaSessionCatalog;
    use crate::sonda::SondaBuilder;
    use moray_skillhub::SkillHub;
    use crate::skill_center::{SkillCenter, SkillDirKind, SkillDirSource};
    use crate::harness::{SondaSessionHarness, SondaToolRegistration};
    use crate::SondaToolCatalog;
    use moray_extensions::tools::{
        CalcTool, FileReadTool, FileWriteTool, ImageCreateTool, ImageEditTool, ShellTool,
        WebFetchTool, WebSearchTool,
    };
    use moray_core::TypedTool;

    use super::*;

    fn missing_bundled_path(user_path: &Path) -> PathBuf {
        user_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("__missing_bundled_settings.toml")
    }

    fn open_test_store(user_path: &Path) -> Result<SondaSettingsStore> {
        SondaSettingsStore::load(&missing_bundled_path(user_path), user_path)
    }

    fn build_test_sonda(server_path: &Path, sessions_path: &Path) -> crate::error::Result<()> {
        let data_dir = server_path
            .parent()
            .expect("server.toml parent")
            .to_path_buf();
        let settings_store = Arc::new(open_test_store(server_path)?);
        let session_catalog = Arc::new(SondaSessionCatalog::open(sessions_path)?);
        let sessions_dir = data_dir.join("sessions");
        let session_transcripts = Arc::new(SondaSessionTranscripts::new(sessions_dir.clone()));
        let skills_dir = data_dir.join("skills");
        std::fs::create_dir_all(&skills_dir).expect("test skills dir");
        let skill_center = SkillCenter::load([SkillDirSource::new(
            skills_dir.clone(),
            SkillDirKind::User,
        )])
        .expect("test skills center");
        let skill_hub = SkillHub::new(skills_dir);
        let channels_path = data_dir.join("channels.toml");
        let channel_catalog = Arc::new(ChannelCatalog::open(&channels_path, HashMap::new())?);

        let _ = SondaBuilder::new()
            .settings(settings_store)
            .skill_center(skill_center)
            .skill_hub(skill_hub)
            .session_catalog(session_catalog)
            .session_transcripts(session_transcripts)
            .channel_catalog(channel_catalog)
            .harness_components(
                sessions_dir.clone(),
                testing_tools_catalog(),
                testing_registrations(testing_shell_env()),
            )
            .channel_factories(HashMap::new())
            .build()?;
        Ok(())
    }

    fn load_settings_docs(server_path: &Path, sessions_path: &Path) -> crate::error::Result<()> {
        build_test_sonda(server_path, sessions_path)
    }

    fn sample_server_settings_toml() -> &'static str {
        r#"
[[completions]]
id = "a1b2c3d4"
name = "Test completion"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "literal-key"

[[agents]]
id = "z9y8x7w6"
name = "Default"
completion_id = "a1b2c3d4"
"#
    }

    #[test]
    fn sessions_rejects_ambiguous_default_agent_key() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"
default_agent = "z9y8x7w6"
"#,
        )
        .unwrap();
        let r = load_settings_docs(&server, &sessions);
        assert!(matches!(
            r,
            Err(SondaError::SessionCatalog(
                crate::session_catalog::SessionCatalogError::InvalidContent(content)
            )) if content.message.contains("default_agent")
        ));
    }

    #[test]
    fn sessions_rejects_bindings_key() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"

[bindings]
foo = "z9y8x7w6"
"#,
        )
        .unwrap();
        let r = load_settings_docs(&server, &sessions);
        assert!(matches!(
            r,
            Err(SondaError::SessionCatalog(
                crate::session_catalog::SessionCatalogError::InvalidContent(content)
            )) if content.message.contains("bindings")
        ));
    }

    #[test]
    fn sessions_default_agent_id_roundtrip() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"

[[entries]]
session_id = "default"

[[entries]]
session_id = "other"
"#,
        )
        .unwrap();
        load_settings_docs(&server, &sessions).unwrap();
        let store = SondaSessionCatalog::open(&sessions).unwrap();
        assert_eq!(store.get_session_agent_id("default").unwrap(), "z9y8x7w6");
        assert_eq!(store.get_session_agent_id("other").unwrap(), "z9y8x7w6");
        let mut ids: Vec<String> = store
            .entries()
            .into_iter()
            .map(|e| e.session_id)
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["default".to_string(), "other".to_string()]);
    }

    #[test]
    fn set_session_agent_errors_when_no_sessions_entry() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();
        load_settings_docs(&server, &sessions).unwrap();
        let store = SondaSessionCatalog::open(&sessions).unwrap();
        let err = store
            .set_session_agent_id("default", "z9y8x7w6")
            .expect_err("expected error");
        assert!(matches!(
            err,
            crate::session_catalog::SessionCatalogError::MissingReference(reference)
                if reference.message.contains("default")
                    && reference.message.contains("[[entries]]"),
        ));
    }

    #[test]
    fn get_session_agent_id_errors_when_no_sessions_entry() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();
        load_settings_docs(&server, &sessions).unwrap();
        let store = SondaSessionCatalog::open(&sessions).unwrap();
        let err = store
            .get_session_agent_id("orphan")
            .expect_err("expected error");
        assert!(matches!(
            err,
            crate::session_catalog::SessionCatalogError::MissingReference(reference)
                if reference.message.contains("orphan")
                    && reference.message.contains("[[entries]]"),
        ));
    }

    #[test]
    fn store_add_session_entry_persists_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions_path = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(
            &sessions_path,
            r#"default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();

        let store = SondaSessionCatalog::open(&sessions_path).unwrap();
        store.add_session_entry("new-tab", "new-tab").unwrap();
        store.add_session_entry("new-tab", "new-tab").unwrap();

        let reloaded = SondaSessionCatalog::open(&sessions_path).unwrap();
        assert!(reloaded.has_session("new-tab"));
        let raw = std::fs::read_to_string(&sessions_path).expect("read sessions");
        assert_eq!(
            raw.matches("session_id = \"new-tab\"").count(),
            1,
            "add_session_entry should be idempotent (single row)"
        );
        assert!(raw.contains("new-tab"));
    }

    #[test]
    fn set_session_agent_id_succeeds_after_sessions_entry_added() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(
            &server,
            r#"
[[completions]]
id = "a1b2c3d4"
name = "Test completion"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "literal-key"

[[agents]]
id = "z9y8x7w6"
name = "Default"
completion_id = "a1b2c3d4"

[[agents]]
id = "b2c3d4e5"
name = "Alt"
completion_id = "a1b2c3d4"
"#,
        )
        .unwrap();
        std::fs::write(
            &sessions,
            r#"default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();
        let store = SondaSessionCatalog::open(&sessions).unwrap();
        store.add_session_entry("pinned", "pinned").unwrap();
        store
            .set_session_agent_id("pinned", "b2c3d4e5")
            .expect("set_session_row_agent_id");
        assert_eq!(
            SondaSessionCatalog::open(&sessions)
                .unwrap()
                .get_session_agent_id("pinned")
                .unwrap(),
            "b2c3d4e5"
        );
        let raw = std::fs::read_to_string(&sessions).expect("read sessions");
        assert!(raw.contains("pinned"));
    }

    #[test]
    fn sessions_entry_without_agent_id_uses_default_for_resolve() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"

[[entries]]
session_id = "tab-a"
"#,
        )
        .unwrap();
        load_settings_docs(&server, &sessions).unwrap();
        let store = SondaSessionCatalog::open(&sessions).unwrap();
        assert_eq!(store.get_session_agent_id("tab-a").unwrap(), "z9y8x7w6");
        let settings = open_test_store(&server).unwrap();
        let ep = settings
            .resolve_completion_endpoint("z9y8x7w6")
            .expect("same agent as default");
        let ep_default = settings
            .resolve_completion_endpoint("z9y8x7w6")
            .expect("default agent");
        assert_eq!(ep.api_key, ep_default.api_key);
        assert_eq!(ep.api_base, ep_default.api_base);
        assert_eq!(ep.model, ep_default.model);
    }

    const TEST_RESOLVE_ENV_CACHED: &str = "MORAY_DESKTOP_SETTINGS_TEST_CACHED";

    #[test]
    fn resolve_reads_env_each_call() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        let body = sample_server_settings_toml().replace(
            "api_key = \"literal-key\"\n",
            &format!("api_key = \"env:{TEST_RESOLVE_ENV_CACHED}\"\n"),
        );
        std::fs::write(&server, body).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();
        std::env::set_var(TEST_RESOLVE_ENV_CACHED, "ok-key");
        load_settings_docs(&server, &sessions).unwrap();
        let settings = open_test_store(&server).unwrap();
        let t = settings.resolve_completion_endpoint("z9y8x7w6").unwrap();
        assert_eq!(t.api_key, "ok-key");
        std::env::remove_var(TEST_RESOLVE_ENV_CACHED);
        let r = settings.resolve_completion_endpoint("z9y8x7w6");
        assert!(matches!(
            r,
            Err(SondaError::SettingsStore(SondaSettingsStoreError::BadEnvironment(
                BadEnvironmentVariable::NotSet { name }
            ))) if name == TEST_RESOLVE_ENV_CACHED
        ));
    }

    const TEST_RESOLVE_ENV_MISSING: &str = "MORAY_DESKTOP_SETTINGS_TEST_MISSING";

    #[test]
    fn load_ok_when_api_key_env_missing_resolve_fails() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        let body = sample_server_settings_toml().replace(
            "api_key = \"literal-key\"\n",
            &format!("api_key = \"env:{TEST_RESOLVE_ENV_MISSING}\"\n"),
        );
        std::fs::write(&server, body).unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();
        std::env::remove_var(TEST_RESOLVE_ENV_MISSING);
        load_settings_docs(&server, &sessions).unwrap();
        let settings = open_test_store(&server).unwrap();
        let r = settings.resolve_completion_endpoint("z9y8x7w6");
        assert!(matches!(
            r,
            Err(SondaError::SettingsStore(SondaSettingsStoreError::BadEnvironment(
                BadEnvironmentVariable::NotSet { name }
            ))) if name == TEST_RESOLVE_ENV_MISSING
        ));
    }

    #[test]
    fn load_merges_bundled_base_with_user_patch() {
        let dir = tempdir().unwrap();
        let bundled = dir.path().join("bundled.toml");
        let user = dir.path().join("user.toml");
        std::fs::write(
            &bundled,
            r#"
preamble_template = "bundled {{character}}"

[[completions]]
id = "a1b2c3d4"
name = "Bundled completion"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "bundled-key"

[[agents]]
id = "z9y8x7w6"
name = "Bundled"
completion_id = "a1b2c3d4"
"#,
        )
        .unwrap();
        std::fs::write(
            &user,
            r#"
preamble_template = "user {{character}}"

[[completions]]
id = "a1b2c3d4"
api_key = "user-key"

[[agents]]
id = "z9y8x7w6"
name = "Patched"
"#,
        )
        .unwrap();

        let store = SondaSettingsStore::load(&bundled, &user).unwrap();
        assert_eq!(store.preamble_template(), "user {{character}}");
        let catalog = store.catalog();
        assert_eq!(catalog.agents[0].name, "Patched");
        assert_eq!(catalog.agents[0].completion_id, "a1b2c3d4");
        assert_eq!(catalog.completions[0].name, "Bundled completion");
        assert_eq!(catalog.completions[0].api_key.as_deref(), Some("user-key"));
        let endpoint = store.resolve_completion_endpoint("z9y8x7w6").unwrap();
        assert_eq!(endpoint.api_key, "user-key");
    }

    #[test]
    fn load_rejects_unknown_allowed_tool() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(
            &server,
            r#"
[[completions]]
id = "a1b2c3d4"
name = "Test completion"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "literal-key"

[[agents]]
id = "z9y8x7w6"
name = "Default"
completion_id = "a1b2c3d4"
allowed_tools = ["calc", "not_a_tool"]
"#,
        )
        .unwrap();
        std::fs::write(
            &sessions,
            r#"default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();
        let r = load_settings_docs(&server, &sessions);
        assert!(matches!(
            r,
            Err(SondaError::InvalidContent(content))
                if content.message.contains("unknown tool")
                    && content.message.contains("not_a_tool")
        ));
    }

    #[test]
    fn load_rejects_duplicate_allowed_tool() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(
            &server,
            r#"
[[completions]]
id = "a1b2c3d4"
name = "Test completion"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "literal-key"

[[agents]]
id = "z9y8x7w6"
name = "Default"
completion_id = "a1b2c3d4"
allowed_tools = ["calc", "calc"]
"#,
        )
        .unwrap();
        std::fs::write(
            &sessions,
            r#"default_agent_id = "z9y8x7w6"
"#,
        )
        .unwrap();
        let r = load_settings_docs(&server, &sessions);
        assert!(matches!(
            r,
            Err(SondaError::InvalidContent(content)) if content.message.contains("duplicate")
        ));
    }

    #[test]
    fn update_agent_persists_allowed_tools() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        let store = open_test_store(&server).unwrap();
        store
            .update_agent(
                "z9y8x7w6",
                "Default",
                "a1b2c3d4",
                vec!["shell".into(), "calc".into()],
                None,
            )
            .expect("update agent");
        let reloaded = open_test_store(&server).unwrap();
        assert_eq!(
            reloaded.agent_allowed_tools("z9y8x7w6").unwrap(),
            vec!["shell".to_string(), "calc".to_string()]
        );
    }

    #[test]
    fn update_agent_rejects_unknown_allowed_tool() {
        let authorizer = Arc::new(AlwaysAsking::new());
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(&sessions, r#"default_agent_id = "z9y8x7w6""#).unwrap();
        let settings_store = Arc::new(open_test_store(&server).unwrap());
        let session_catalog = Arc::new(SondaSessionCatalog::open(&sessions).unwrap());
        let factory = testing_session_harness(
            settings_store,
            session_catalog,
            authorizer,
            std::env::temp_dir().join("moray-sonda-test-sessions"),
        )
        .expect("factory");
        let err = factory
            .validate_allowed_tools(&["nope".into()])
            .expect_err("unknown tool");
        assert!(err.to_string().contains("unknown tool"));
    }

    const TESTING_TOOLS_CATALOG_TOML: &str = r#"
[[tools]]
name = "calc"
description = "calc"
parameters = '{}'
[[tools]]
name = "shell"
description = "shell"
parameters = '{}'
[[tools]]
name = "file_read"
description = "file_read"
parameters = '{}'
[[tools]]
name = "file_write"
description = "file_write"
parameters = '{}'
[[tools]]
name = "web_fetch"
description = "web_fetch"
parameters = '{}'
[[tools]]
name = "web_search"
description = "web_search"
parameters = '{}'
[[tools]]
name = "image_create"
description = "image_create"
parameters = '{}'
[[tools]]
name = "image_edit"
description = "image_edit"
parameters = '{}'
"#;

    fn testing_tools_catalog() -> SondaToolCatalog {
        SondaToolCatalog::from_str(TESTING_TOOLS_CATALOG_TOML).expect("testing tools catalog")
    }

    fn testing_shell_env() -> Vec<(String, String)> {
        vec![
            ("CLI".into(), "/tmp/moray-cli-test".into()),
            (
                "MORAY_TOOLS_CATALOG_PATH".into(),
                "/tmp/moray-test-tools.toml".into(),
            ),
        ]
    }

    fn testing_registrations(shell_env: Vec<(String, String)>) -> Vec<SondaToolRegistration> {
        vec![
            SondaToolRegistration::new(CalcTool::NAME, |_| Arc::new(CalcTool)),
            SondaToolRegistration::new(ShellTool::NAME, move |dir| {
                Arc::new(ShellTool::new(dir, shell_env.clone()))
            }),
            SondaToolRegistration::new(FileReadTool::NAME, |dir| Arc::new(FileReadTool::new(dir))),
            SondaToolRegistration::new(FileWriteTool::NAME, |dir| Arc::new(FileWriteTool::new(dir))),
            SondaToolRegistration::new(WebFetchTool::NAME, |_| Arc::new(WebFetchTool)),
            SondaToolRegistration::new(WebSearchTool::NAME, |_| Arc::new(WebSearchTool)),
            SondaToolRegistration::new(ImageCreateTool::NAME, |_| Arc::new(ImageCreateTool)),
            SondaToolRegistration::new(ImageEditTool::NAME, |dir| Arc::new(ImageEditTool::new(dir))),
        ]
    }

    fn testing_session_harness(
        settings_store: Arc<SondaSettingsStore>,
        session_catalog: Arc<SondaSessionCatalog>,
        authorizer: Arc<dyn moray_core::ToolCallAuthorizer>,
        sessions_dir: std::path::PathBuf,
    ) -> crate::error::Result<Arc<SondaSessionHarness>> {
        let workspace = Arc::new(crate::SondaSessionWorkspace::new(sessions_dir));
        Ok(Arc::new(SondaSessionHarness::new(
            settings_store,
            session_catalog,
            authorizer,
            testing_tools_catalog(),
            workspace,
            testing_registrations(testing_shell_env()),
        )?))
    }

    #[test]
    fn agent_allowed_tools_roundtrip() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        std::fs::write(
            &server,
            r#"
[[completions]]
id = "a1b2c3d4"
name = "Test completion"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "literal-key"

[[agents]]
id = "z9y8x7w6"
name = "Default"
completion_id = "a1b2c3d4"
allowed_tools = ["shell", "calc"]
"#,
        )
        .unwrap();
        let store = open_test_store(&server).unwrap();
        assert_eq!(
            store.agent_allowed_tools("z9y8x7w6").unwrap(),
            vec!["shell".to_string(), "calc".to_string()]
        );
    }

    #[test]
    fn write_server_roundtrip_omits_legacy_capabilities_section() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(
            &server,
            r#"
[[completions]]
id = "aaaaaaaa"
name = "A"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "key-a"

[[completions]]
id = "bbbbbbbb"
name = "B"
base_url = "http://127.0.0.1:9/v1"
model = "m2"
api_key = "literal-key"

[[agents]]
id = "zzzzzzzz"
name = "Ag"
completion_id = "aaaaaaaa"
"#,
        )
        .unwrap();
        std::fs::write(
            &sessions,
            r#"
default_agent_id = "zzzzzzzz"
"#,
        )
        .unwrap();
        load_settings_docs(&server, &sessions).unwrap();
        let settings = open_test_store(&server).unwrap();
        settings
            .update_agent("zzzzzzzz", "Ag", "bbbbbbbb", vec!["calc".into()], None)
            .expect("update agent");
        let raw = std::fs::read_to_string(&server).expect("read server.toml");
        assert!(
            !raw.contains("prefill_supported"),
            "unexpected legacy capabilities in output:\n{raw}"
        );
    }
}
