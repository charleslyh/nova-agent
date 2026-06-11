use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use moray_sonda::{Sonda, SondaSettingsAgentEntry};
use serde::{Deserialize, Serialize};

use crate::error::sonda_error_response;

#[rustfmt::skip]
pub(super) fn router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/catalog",           get(settings_get_catalog))
        .route("/agents",            post(settings_create_agent))
        .route("/agents/{agent_id}", patch(settings_update_agent).delete(settings_delete_agent))
}

/// Public `GET /settings/catalog` completion row: `id` + `name` only (no URLs, models, or API keys).
/// TODO: 如果将来返回完整的 CompletionEntry, 需要考虑敏感信息的安全性问题
#[derive(Serialize)]
pub(crate) struct CompletionStub {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
struct SettingsGetCatalogRes {
    agents: Vec<SondaSettingsAgentEntry>,
    completions: Vec<CompletionStub>,
}

#[derive(Deserialize)]
struct SettingsUpdateAgentReq {
    name: String,
    completion_id: String,
    allowed_tools: Vec<String>,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    desc: Option<String>,
}

#[derive(Deserialize)]
struct SettingsCreateAgentReq {
    name: String,
    completion_id: String,
    #[serde(default)]
    allowed_tools: Vec<String>,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    desc: Option<String>,
}

#[derive(Serialize)]
struct SettingsCreateAgentRes {
    id: String,
}

async fn settings_get_catalog(State(sonda): State<Arc<Sonda>>) -> impl IntoResponse {
    let catalog = sonda.settings_store.catalog();

    // 原始的 CompletionEntry 包含 api_key 等敏感信息, 这里只返回 id 和 name
    let completions = catalog
        .completions
        .into_iter()
        .map(|c| CompletionStub {
            id: c.id.clone(),
            name: c.config_str("name").unwrap_or("").to_string(),
        })
        .collect();

    Json(SettingsGetCatalogRes {
        agents: catalog.agents,
        completions,
    })
    .into_response()
}

async fn settings_update_agent(
    State(sonda): State<Arc<Sonda>>,
    Path(agent_id): Path<String>,
    Json(req): Json<SettingsUpdateAgentReq>,
) -> impl IntoResponse {
    match sonda.update_agent(
        &agent_id,
        &req.name,
        &req.completion_id,
        req.allowed_tools,
        req.character,
        req.desc,
    ) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => sonda_error_response(e),
    }
}

async fn settings_create_agent(
    State(sonda): State<Arc<Sonda>>,
    Json(req): Json<SettingsCreateAgentReq>,
) -> impl IntoResponse {
    match sonda.create_agent(
        &req.name,
        &req.completion_id,
        req.allowed_tools,
        req.character,
        req.desc,
    ) {
        Ok(id) => Json(SettingsCreateAgentRes { id }).into_response(),
        Err(e) => sonda_error_response(e),
    }
}

async fn settings_delete_agent(
    State(sonda): State<Arc<Sonda>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    match sonda.delete_agent(agent_id.as_str()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => sonda_error_response(e),
    }
}
