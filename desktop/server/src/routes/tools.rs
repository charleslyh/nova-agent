use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use moray_sonda::{Sonda, SondaToolCatalogEntry};
use serde::Serialize;

use crate::error::tool_call_auth_error_response;

#[rustfmt::skip]
pub(super) fn router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/", get(tools_list))
}

#[rustfmt::skip]
pub(super) fn auth_router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/{call_id}", post(tools_reply_auth))
}

#[derive(Serialize)]
struct GetToolsRes {
    tools: Vec<SondaToolCatalogEntry>,
}

/// Server-registered built-in tools (from harness registration, not TOML settings).
async fn tools_list(State(sonda): State<Arc<Sonda>>) -> impl IntoResponse {
    Json(GetToolsRes {
        tools: sonda.toolbox_factory.tools(),
    })
    .into_response()
}

async fn tools_reply_auth(
    State(sonda): State<Arc<Sonda>>,
    Path(call_id): Path<String>,
    Json(data): Json<serde_json::Value>,
) -> impl IntoResponse {
    let result = sonda.auth_resolver.reply(&call_id, data).await;

    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => tool_call_auth_error_response(e),
    }
}
