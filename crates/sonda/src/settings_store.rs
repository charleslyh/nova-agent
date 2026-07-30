//! Sonda **settings store**: agents / completions 的磁盘读写、结构校验与合并。
//!
//! Completion 条目含 `id` / `provider` 与 opaque `config`；provider 具体语义由应用 wiring 解释。
//!
//! 调用方通过 [`SondaSettingsStore::load`] 传入 bundled 与 user 两个路径；store 内加载并合并
//! （bundled 为基础，user 为 patch）。写回仅作用于 user 路径。与 [`super::session_catalog`] 配合，
//! 由 [`super::sonda::SondaBuilder::build`] 做跨文件一致性检查。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use super::error::{
    require_argument_nonempty, require_nonempty_field, FileIoError, InvalidArguments,
    InvalidContent, MissingReference,
};

type Result<T> = std::result::Result<T, SondaSettingsStoreError>;

#[derive(Debug, Error)]
pub enum SondaSettingsStoreError {
    #[error(transparent)]
    InvalidContent(#[from] InvalidContent),

    #[error(transparent)]
    MissingReference(#[from] MissingReference),

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
    /// Short description for sub-agent tool manifest listing.
    #[serde(default)]
    pub desc: Option<String>,
}

/// One completion profile: stable `id`, routing `provider`, plus opaque provider configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SondaSettingsCompletionEntry {
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(flatten)]
    pub config: BTreeMap<String, toml::Value>,
}

impl SondaSettingsCompletionEntry {
    /// Read a string field from opaque config without schema validation.
    pub fn config_str(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(|v| v.as_str())
    }

    /// Merged completion row as JSON (`id` plus flattened config keys).
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".to_string(), Value::String(self.id.clone()));
        map.insert(
            "provider".to_string(),
            Value::String(self.provider.clone()),
        );
        for (key, value) in &self.config {
            map.insert(key.clone(), toml_value_to_json(value));
        }
        Value::Object(map)
    }
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
        desc: Option<String>,
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
            entry.desc = desc;
        }

        save(self)?;

        Ok(())
    }

    pub fn create_agent(
        &self,
        name: &str,
        completion_id: &str,
        allowed_tools: Vec<String>,
        character: Option<String>,
        desc: Option<String>,
    ) -> crate::error::Result<String> {
        let name = require_argument_nonempty(name, "name")?;
        let completion_id = require_argument_nonempty(completion_id, "completion_id")?;

        let agent_id = new_agent_id();
        {
            let mut inner = self.inner.write();
            ensure_completion_exists(&inner, completion_id)?;
            inner.agents.push(SondaSettingsAgentEntry {
                id: agent_id.clone(),
                name: name.to_string(),
                completion_id: completion_id.to_string(),
                allowed_tools,
                character,
                desc,
            });
            save_locked(self, &inner)?;
        }

        Ok(agent_id)
    }

    pub fn delete_agent(&self, agent_id: &str) -> crate::error::Result<()> {
        let agent_id = require_argument_nonempty(agent_id, "agent_id")?;
        let mut inner = self.inner.write();
        if !inner.agents.iter().any(|a| a.id == agent_id) {
            return Err(InvalidArguments::new("agent_id", "corresponding agent not found").into());
        }
        if inner.agents.len() <= 1 {
            return Err(InvalidContent::new("cannot delete the last agent").into());
        }
        inner.agents.retain(|a| a.id != agent_id);
        save_locked(self, &inner)?;
        Ok(())
    }

    /// Completion profile for `completion_id` (secrets not materialized).
    pub fn completion_entry(
        &self,
        completion_id: &str,
    ) -> crate::error::Result<SondaSettingsCompletionEntry> {
        let inner = self.inner.read();
        Ok(find_completion_by_id(&inner, completion_id)?.clone())
    }

    /// Completion profile bound to `agent_id` (secrets not materialized).
    pub fn completion_entry_for_agent(
        &self,
        agent_id: &str,
    ) -> crate::error::Result<SondaSettingsCompletionEntry> {
        let inner = self.inner.read();
        let agent = inner
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| InvalidArguments::new("agent_id", "corresponding agent not found"))?;
        Ok(find_completion_for_agent(&inner, agent)?.clone())
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

    /// Returns the agent's optional description for sub-agent tool listing.
    pub fn agent_desc(&self, agent_id: &str) -> crate::error::Result<String> {
        let inner = self.inner.read();
        let agent = inner
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| InvalidArguments::new("agent_id", "corresponding agent not found"))?;
        Ok(agent.desc.clone().unwrap_or_default())
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
        require_nonempty_field(&c.provider, "completions.provider")?;
        if !seen_completions.insert(&c.id) {
            return Err(InvalidContent::new(format!("duplicate completion id `{}`", c.id)).into());
        }
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
        desc: patch.desc.or(base.desc),
    }
}

fn new_agent_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn merge_completion_entry(
    base: SondaSettingsCompletionEntry,
    patch: SondaSettingsCompletionEntry,
) -> SondaSettingsCompletionEntry {
    let mut config = base.config;
    config.extend(patch.config);
    SondaSettingsCompletionEntry {
        id: patch.id,
        provider: pick_nonempty(patch.provider, base.provider),
        config,
    }
}

fn pick_nonempty(patch: String, base: String) -> String {
    if patch.is_empty() { base } else { patch }
}

fn save_locked(settings: &SondaSettingsStore, inner: &SondaSettingsFile) -> Result<()> {
    let path = settings.user_path.as_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| FileIoError::new("mkdir", parent.to_path_buf(), source))?;
    }
    let s = toml::to_string_pretty(inner).map_err(|e| {
        InvalidContent::new(format!("invalid settings file: {e}"))
    })?;
    std::fs::write(path, s)
        .map_err(|source| FileIoError::new("write", path.to_path_buf(), source))?;
    Ok(())
}

fn save(settings: &SondaSettingsStore) -> Result<()> {
    let inner = settings.inner.read();
    save_locked(settings, &inner)
}

fn ensure_completion_exists(inner: &SondaSettingsFile, completion_id: &str) -> Result<()> {
    if !inner.completions.iter().any(|c| c.id == completion_id) {
        return Err(
            MissingReference::new(format!("unknown completion id `{completion_id}`")).into(),
        );
    }
    Ok(())
}

fn find_completion_by_id<'a>(
    inner: &'a SondaSettingsFile,
    completion_id: &str,
) -> Result<&'a SondaSettingsCompletionEntry> {
    inner
        .completions
        .iter()
        .find(|c| c.id == completion_id)
        .ok_or_else(|| {
            MissingReference::new(format!("unknown completion id `{completion_id}`")).into()
        })
}

fn find_completion_for_agent<'a>(
    inner: &'a SondaSettingsFile,
    agent: &'a SondaSettingsAgentEntry,
) -> Result<&'a SondaSettingsCompletionEntry> {
    find_completion_by_id(inner, agent.completion_id.as_str()).map_err(|_| {
        MissingReference::new(format!(
            "completion `{}` for agent `{}` missing",
            agent.completion_id, agent.id
        ))
        .into()
    })
}

fn toml_value_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(items) => {
            Value::Array(items.iter().map(toml_value_to_json).collect())
        }
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(k, v)| (k.clone(), toml_value_to_json(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use std::collections::HashMap;

    use async_trait::async_trait;
    use futures::Stream;
    use moray_core::{
        ChatCompletion, ChatCompletionFinishReason, ChatCompletionRequestMessage,
        ChatCompletionResponseChunk, ContextEngine, MorayError, Tool, ToolCallAuthorizer,
        ToolCallResponder, ToolManifest,
    };
    use serde_json::Value;
    use crate::{ContextBuilder, SondaCompletionRegistration};
    use crate::transcripts::SondaSessionTranscripts;
    use tempfile::tempdir;

    use crate::error::SondaError;
    use moray_channels::ChannelCatalog;
    use crate::session_catalog::SondaSessionCatalog;
    use crate::sonda::SondaBuilder;
    use moray_skills::SkillsManager;
    use crate::toolbox_factory::{SondaToolRegistration, SondaToolboxFactory};
    use crate::SondaToolCatalog;

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

    struct StubCompletion;

    #[async_trait]
    impl ChatCompletion for StubCompletion {
        async fn completion(
            &self,
            _messages: &[ChatCompletionRequestMessage],
            _tools: &[ToolManifest],
            _stream: bool,
        ) -> std::result::Result<
            std::pin::Pin<
                Box<dyn Stream<Item = std::result::Result<ChatCompletionResponseChunk, MorayError>> + Send>,
            >,
            MorayError,
        > {
            Ok(Box::pin(futures::stream::once(async {
                Ok(ChatCompletionResponseChunk::Done {
                    reason: ChatCompletionFinishReason::Stop,
                    usage: None,
                })
            })))
        }
    }

    fn testing_completion_registrations() -> Vec<SondaCompletionRegistration> {
        vec![SondaCompletionRegistration::new("openai", |_entry| {
            Ok(Arc::new(StubCompletion))
        })]
    }

    struct AllowAllAuthorizer;

    #[async_trait]
    impl ToolCallAuthorizer for AllowAllAuthorizer {
        async fn request(
            &self,
            _call_id: &str,
            _tool_name: &str,
            _args: &Value,
            _responder: Arc<dyn ToolCallResponder>,
        ) -> bool {
            true
        }
    }

    struct StubContextEngine {
        messages: Vec<ChatCompletionRequestMessage>,
    }

    #[async_trait]
    impl ContextEngine for StubContextEngine {
        async fn setup(
            &self,
            _tools: &[ToolManifest],
        ) -> std::result::Result<(), MorayError> {
            Ok(())
        }

        async fn assemble(
            &self,
            _tools: &[ToolManifest],
        ) -> std::result::Result<Vec<ChatCompletionRequestMessage>, MorayError> {
            Ok(self.messages.clone())
        }

        async fn ingest(
            &self,
            _messages: Vec<ChatCompletionRequestMessage>,
        ) -> std::result::Result<(), MorayError> {
            Ok(())
        }

        async fn teardown(&self) -> std::result::Result<(), MorayError> {
            Ok(())
        }

        async fn clear(&self) -> std::result::Result<(), MorayError> {
            Ok(())
        }
    }

    fn testing_context_builder() -> ContextBuilder {
        Arc::new(|_agent_id, messages| {
            Ok(Arc::new(StubContextEngine { messages }) as Arc<dyn ContextEngine>)
        })
    }

    struct StubTool(&'static str);

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &'static str {
            self.0
        }

        async fn call(
            &self,
            _args: Value,
            _responder: &dyn ToolCallResponder,
        ) -> std::result::Result<(), MorayError> {
            Ok(())
        }
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
        let bundled_skills = skills_dir.join("_bundled");
        std::fs::create_dir_all(&bundled_skills).expect("test bundled skills dir");
        let skills = SkillsManager::load(&skills_dir, &bundled_skills).expect("test skills manager");
        let channels_path = data_dir.join("channels.toml");
        let channel_catalog = Arc::new(ChannelCatalog::open(&channels_path, HashMap::new())?);

        let session_workspace = Arc::new(crate::SondaSessionWorkspace::new(sessions_dir.clone()));
        let _ = SondaBuilder::new()
            .settings(settings_store)
            .completion_registrations(testing_completion_registrations())
            .authorizer(Arc::new(AllowAllAuthorizer))
            .context_builder(testing_context_builder())
            .skills(skills)
            .session_catalog(session_catalog)
            .session_transcripts(session_transcripts)
            .channel_catalog(channel_catalog)
            .harness_components(
                session_workspace,
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
provider = "openai"
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
provider = "openai"
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
        let cfg = settings
            .completion_entry_for_agent("z9y8x7w6")
            .expect("same agent as default");
        let cfg_default = settings
            .completion_entry_for_agent("z9y8x7w6")
            .expect("default agent");
        assert_eq!(cfg.config_str("api_key"), cfg_default.config_str("api_key"));
        assert_eq!(cfg.config_str("base_url"), cfg_default.config_str("base_url"));
        assert_eq!(cfg.config_str("model"), cfg_default.config_str("model"));
    }

    #[test]
    fn completion_entry_for_agent_returns_entry() {
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        let settings = open_test_store(&server).unwrap();
        let entry = settings.completion_entry_for_agent("z9y8x7w6").unwrap();
        assert_eq!(entry.id, "a1b2c3d4");
        assert_eq!(entry.provider, "openai");
        assert_eq!(entry.config_str("api_key"), Some("literal-key"));
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
provider = "openai"
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
        assert_eq!(
            catalog.completions[0].config_str("name"),
            Some("Bundled completion")
        );
        assert_eq!(
            catalog.completions[0].config_str("api_key"),
            Some("user-key")
        );
        let entry = store.completion_entry_for_agent("z9y8x7w6").unwrap();
        assert_eq!(entry.config_str("api_key"), Some("user-key"));
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
provider = "openai"
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
provider = "openai"
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
        let authorizer = Arc::new(AllowAllAuthorizer);
        let dir = tempdir().unwrap();
        let server = dir.path().join("server.toml");
        let sessions = dir.path().join("sessions.toml");
        std::fs::write(&server, sample_server_settings_toml()).unwrap();
        std::fs::write(&sessions, r#"default_agent_id = "z9y8x7w6""#).unwrap();
        let settings_store = Arc::new(open_test_store(&server).unwrap());
        let toolbox_factory = testing_toolbox_factory(authorizer, settings_store);
        let err = toolbox_factory
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

    const TESTING_TOOL_NAMES: &[&str] = &[
        "calc",
        "shell",
        "file_read",
        "file_write",
        "web_fetch",
        "web_search",
        "image_create",
        "image_edit",
    ];

    fn testing_registrations(_shell_env: Vec<(String, String)>) -> Vec<SondaToolRegistration> {
        TESTING_TOOL_NAMES
            .iter()
            .map(|&name| SondaToolRegistration::new(name, |_| Arc::new(StubTool(name))))
            .collect()
    }

    fn testing_toolbox_factory(
        authorizer: Arc<dyn moray_core::ToolCallAuthorizer>,
        settings_store: Arc<SondaSettingsStore>,
    ) -> SondaToolboxFactory {
        let workspace = Arc::new(crate::SondaSessionWorkspace::new(
            std::env::temp_dir().join("moray-sonda-test-sessions"),
        ));
        SondaToolboxFactory::new(
            settings_store,
            authorizer,
            testing_tools_catalog(),
            testing_registrations(testing_shell_env()),
            workspace,
        )
        .expect("testing toolbox factory")
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
provider = "openai"
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
provider = "openai"
name = "A"
base_url = "http://127.0.0.1:9/v1"
model = "m1"
api_key = "key-a"

[[completions]]
id = "bbbbbbbb"
provider = "openai"
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
            .update_agent("zzzzzzzz", "Ag", "bbbbbbbb", vec!["calc".into()], None, None)
            .expect("update agent");
        let raw = std::fs::read_to_string(&server).expect("read server.toml");
        assert!(
            !raw.contains("prefill_supported"),
            "unexpected legacy capabilities in output:\n{raw}"
        );
    }
}
