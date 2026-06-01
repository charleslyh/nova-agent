use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use moray_sonda::Sonda;
use serde::{Deserialize, Serialize};

use crate::error::sonda_error_response;

#[rustfmt::skip]
pub(super) fn router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/",             get(channels_list).post(channels_create))
        .route("/{channel_id}", get(channels_get).put(channels_put).delete(channels_delete))
}

#[derive(Serialize)]
struct ChannelListItem {
    channel_id: String,
    session_id: String,
    #[serde(rename = "type")]
    channel_type: String,
    name: String,
}

async fn channels_list(State(sonda): State<Arc<Sonda>>) -> impl IntoResponse {
    let session_names: std::collections::HashMap<String, String> = sonda
        .session_catalog
        .entries()
        .into_iter()
        .map(|e| (e.session_id, e.name))
        .collect();

    let items = sonda
        .channel_catalog
        .entries()
        .into_iter()
        .map(|entry| ChannelListItem {
            channel_id: entry.channel_id,
            session_id: entry.session_id.clone(),
            channel_type: entry.channel_type,
            name: session_names
                .get(&entry.session_id)
                .cloned()
                .unwrap_or_else(|| entry.session_id.clone()),
        })
        .collect::<Vec<_>>();
    Json(items).into_response()
}

#[derive(Deserialize)]
struct ChannelCreateReq {
    name: String,
    #[serde(rename = "type")]
    channel_type: String,
    data: serde_json::Value,
}

async fn channels_create(
    State(sonda): State<Arc<Sonda>>,
    Json(req): Json<ChannelCreateReq>,
) -> impl IntoResponse {
    match sonda.create_channel(&req.name, &req.channel_type, req.data) {
        Ok(result) => Json(result).into_response(),
        Err(e) => sonda_error_response(e),
    }
}

async fn channels_get(
    State(sonda): State<Arc<Sonda>>,
    Path(channel_id): Path<String>,
) -> impl IntoResponse {
    match sonda.channel_catalog.get(channel_id.as_str()) {
        Ok(mut entry) => {
            entry.data = sonda.channel_catalog.redact_data(&entry);
            Json(entry).into_response()
        }
        Err(e) => sonda_error_response(e.into()),
    }
}

#[derive(Deserialize)]
struct ChannelPutReq {
    data: serde_json::Value,
}

async fn channels_put(
    State(sonda): State<Arc<Sonda>>,
    Path(channel_id): Path<String>,
    Json(req): Json<ChannelPutReq>,
) -> impl IntoResponse {
    match sonda.update_channel(channel_id.as_str(), req.data) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => sonda_error_response(e),
    }
}

async fn channels_delete(
    State(sonda): State<Arc<Sonda>>,
    Path(channel_id): Path<String>,
) -> impl IntoResponse {
    match sonda.delete_channel(channel_id.as_str()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => sonda_error_response(e),
    }
}
