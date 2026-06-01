use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use moray_session::TurnInput;
use moray_sonda::{SessionCatalogEntry, Sonda, SondaSessionTranscriptsError};
use serde::{Deserialize, Serialize};

use crate::error::{response_with, session_error_response, sonda_error_response};

#[rustfmt::skip]
pub(super) fn router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/",                    get(sessions_catalog).post(sessions_create))
        .route("/{session_id}",        delete(sessions_delete))
        .route("/{session_id}/agent",  get(sessions_get_agent).put(sessions_set_agent))
        .route("/{session_id}/submit", post(sessions_submit))
        .route("/{session_id}/cancel", post(sessions_cancel))
        .route("/{session_id}/reset",  post(sessions_reset))
        .route("/{session_id}/events", get(sessions_get_events))
}

#[derive(Serialize)]
struct SessionsListStub {
    session_id: String,
    name: String,
}

async fn sessions_catalog(State(sonda): State<Arc<Sonda>>) -> impl IntoResponse {
    let entries = sonda
        .session_catalog
        .entries()
        .into_iter()
        .map(|entry: SessionCatalogEntry| SessionsListStub {
            session_id: entry.session_id,
            name: entry.name,
        })
        .collect::<Vec<_>>();
    Json(entries).into_response()
}

#[derive(Deserialize)]
struct SessionCreateReq {
    name: String,
}

#[derive(Serialize)]
struct SessionCreateRes {
    session_id: String,
}

async fn sessions_create(
    State(sonda): State<Arc<Sonda>>,
    Json(req): Json<SessionCreateReq>,
) -> impl IntoResponse {
    match sonda.create_session(req.name.as_str()) {
        Ok(session_id) => Json(SessionCreateRes { session_id }).into_response(),
        Err(e) => sonda_error_response(e),
    }
}

async fn sessions_delete(
    State(sonda): State<Arc<Sonda>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match sonda.delete_session(session_id.as_str()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => sonda_error_response(e),
    }
}

#[derive(Serialize)]
struct SessionsGetAgentRes {
    agent_id: String,
}

async fn sessions_get_agent(
    State(sonda): State<Arc<Sonda>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match sonda
        .session_catalog
        .get_session_agent_id(session_id.as_str())
    {
        Ok(agent_id) => Json(SessionsGetAgentRes { agent_id }).into_response(),
        Err(e) => sonda_error_response(e.into()),
    }
}

#[derive(Deserialize)]
struct SessionSetAgentReq {
    agent_id: String,
}

async fn sessions_set_agent(
    State(sonda): State<Arc<Sonda>>,
    Path(session_id): Path<String>,
    Json(req): Json<SessionSetAgentReq>,
) -> impl IntoResponse {
    match sonda
        .set_session_agent_id(session_id.as_str(), req.agent_id.as_str())
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => sonda_error_response(e),
    }
}

async fn sessions_submit(
    State(sonda): State<Arc<Sonda>>,
    Path(session_id): Path<String>,
    Json(input): Json<TurnInput>,
) -> impl IntoResponse {
    let result = sonda.live_sessions.submit(session_id.as_str(), input).await;

    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => session_error_response(e),
    }
}

async fn sessions_cancel(
    State(sonda): State<Arc<Sonda>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match sonda.live_sessions.cancel(session_id.as_str()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => session_error_response(e),
    }
}

async fn sessions_reset(
    State(sonda): State<Arc<Sonda>>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match sonda.live_sessions.reset(session_id.as_str()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => session_error_response(e),
    }
}

#[derive(Deserialize)]
struct SessionEventsQuery {
    from_seq: Option<u64>,
}

fn parse_from_seq(headers: &HeaderMap, query: Option<u64>) -> u64 {
    if let Some(last_id) = headers.get("last-event-id").and_then(|v| v.to_str().ok()) {
        if let Ok(id) = last_id.parse::<u64>() {
            return id.saturating_add(1);
        }
    }
    query.unwrap_or(0)
}

async fn sessions_get_events(
    State(sonda): State<Arc<Sonda>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<SessionEventsQuery>,
) -> axum::response::Response {
    let sid = session_id.as_str();
    let from_seq = parse_from_seq(&headers, query.from_seq);

    let mut events = match sonda.session_transcripts.subscribe(sid, from_seq) {
        Ok(events) => events,
        Err(SondaSessionTranscriptsError::NotFound) => {
            return response_with(StatusCode::NOT_FOUND, "unknown chat session").into_response();
        }
        Err(e) => {
            return response_with(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let event_stream = async_stream::stream! {
        while let Some(record) = events.recv().await {
            yield Ok::<Event, Infallible>(
                Event::default()
                    .id(record.seq.to_string())
                    .event("session")
                    .json_data(record.event)
                    .unwrap_or_else(|_| Event::default().event("session")),
            );
        }
    };
    Sse::new(event_stream).into_response()
}
