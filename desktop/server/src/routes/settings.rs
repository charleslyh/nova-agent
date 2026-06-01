use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch};
use axum::{Json, Router};
use moray_sonda::{Sonda, SondaSettingsAgentEntry};
use serde::{Deserialize, Serialize};

use crate::error::sonda_error_response;

#[rustfmt::skip]
pub(super) fn router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/catalog",           get(settings_get_catalog))
        .route("/agents/{agent_id}", patch(settings_update_agent))
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
}

async fn settings_get_catalog(State(sonda): State<Arc<Sonda>>) -> impl IntoResponse {
    let catalog = sonda.settings_store.catalog();

    // 原始的 CompletionEntry 包含 api_key 等敏感信息, 这里只返回 id 和 name
    let completions = catalog
        .completions
        .into_iter()
        .map(|c| CompletionStub {
            id: c.id,
            name: c.name,
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
    ) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => sonda_error_response(e),
    }
}
