//! Extensions-backed wiring for [`SondaBuilder`] (completions, secrets, auth, context).

use std::sync::Arc;

use moray_core::{ChatCompletion, ContextEngine, ToolCallAuthorizer};
use moray_extensions::auths::AlwaysAsking;
use moray_extensions::completions::{Endpoint, OpenAIChatCompletion};
use moray_extensions::context::CompositeContextEngineBuilder;
use moray_extensions::preambles::{SkillsSection, TemplatedPreamblerBuilder};
use moray_skills::SkillsManager;
use moray_sonda::{
    BadEnvironmentVariable, ContextBuilder, InvalidContent, SondaCompletionRegistration,
    SondaError, SondaSettingsCompletionEntry, SondaSettingsStore, RUN_SUB_AGENT_TOOL_NAME,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct OpenAICompletionConfig {
    base_url: String,
    model: String,
    api_key: Option<String>,
}

pub fn completion_registrations() -> Vec<SondaCompletionRegistration> {
    vec![SondaCompletionRegistration::new("openai", build_openai_completion)]
}

pub fn authorizer() -> Arc<dyn ToolCallAuthorizer> {
    Arc::new(AlwaysAsking::with_auto_allow([RUN_SUB_AGENT_TOOL_NAME]))
}

pub fn context_builder(
    settings_store: Arc<SondaSettingsStore>,
    skills: SkillsManager,
) -> ContextBuilder {
    Arc::new(move |agent_id: &str, messages| {
        let agent_id = agent_id.to_string();
        let settings = settings_store.clone();
        let skills = skills.clone();

        let preambler = TemplatedPreamblerBuilder::default()
            .template(settings_store.preamble_template())
            .subst_dyn("character", move || {
                settings
                    .agent_character(&agent_id)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            })
            .section(SkillsSection::new(move || {
                // Re-read the skill catalog on every setup: skills may be installed or removed
                // mid-session; the value is frozen for that run when the context engine runs setup.
                skills.local().all()
            }))
            .build();

        Ok(Arc::new(
            CompositeContextEngineBuilder::new()
                .messages(messages)
                .preamble(Arc::new(preambler))
                .build(),
        ) as Arc<dyn ContextEngine>)
    })
}

fn build_openai_completion(entry: SondaSettingsCompletionEntry) -> Result<Arc<dyn ChatCompletion>, SondaError> {
    let config = entry.to_json_value();
    let cfg: OpenAICompletionConfig = serde_json::from_value(config).map_err(|e| {
        InvalidContent::new(format!("openai completion config: {e}"))
    })?;
    let api_key = materialize_api_key(cfg.api_key.as_ref())?;
    Ok(Arc::new(OpenAIChatCompletion::new(Endpoint::new(
        api_key,
        cfg.base_url,
        cfg.model,
    ))))
}

fn materialize_api_key(raw: Option<&String>) -> Result<String, SondaError> {
    let Some(raw) = raw else {
        return Ok(String::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(InvalidContent::new("literal api_key is empty after trim").into());
    }
    if let Some(rest) = trimmed.strip_prefix("env:") {
        let name = rest.trim();
        if name.is_empty() {
            return Err(
                InvalidContent::new("api_key env: reference has empty variable name").into(),
            );
        }
        return Ok(read_env_trimmed(name)?);
    }
    Ok(trimmed.to_string())
}

fn read_env_trimmed(name: &str) -> Result<String, SondaError> {
    let value = std::env::var(name).map_err(|_| BadEnvironmentVariable::not_set(name))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(BadEnvironmentVariable::empty(name).into());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ENV: &str = "MORAY_WIRING_TEST_API_KEY";

    #[test]
    fn materialize_literal_api_key() {
        let key = "literal-key".to_string();
        assert_eq!(
            materialize_api_key(Some(&key)).unwrap(),
            "literal-key"
        );
    }

    #[test]
    fn materialize_env_api_key() {
        std::env::set_var(TEST_ENV, "ok-key");
        let raw = format!("env:{TEST_ENV}");
        assert_eq!(materialize_api_key(Some(&raw)).unwrap(), "ok-key");
        std::env::remove_var(TEST_ENV);
        assert!(materialize_api_key(Some(&raw)).is_err());
    }
}
