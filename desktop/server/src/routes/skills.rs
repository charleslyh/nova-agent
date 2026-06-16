use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use moray_skills::{
    InstallSkillError, SkillCatalogEntry, SkillHub, SkillHubError, UninstallSkillError,
    UnregisterSkillError,
};
use moray_sonda::Sonda;
use serde::{Deserialize, Serialize};

use crate::error::response_with;

#[rustfmt::skip]
pub(super) fn router() -> Router<Arc<Sonda>> {
    Router::new()
        .route("/",           get(skills_list))
        .route("/search",     get(skills_search))
        .route("/install",    post(skills_install))
        .route("/{skill_id}", get(skills_get).delete(skills_uninstall))
}

#[derive(Serialize)]
struct GetSkillsRes {
    skills: Vec<SkillCatalogEntry>,
}

/// Server-registered agent skills (bundled + user-installed).
async fn skills_list(State(sonda): State<Arc<Sonda>>) -> impl IntoResponse {
    Json(GetSkillsRes {
        skills: sonda.skills.local().catalog(),
    })
    .into_response()
}

async fn skills_get(
    State(sonda): State<Arc<Sonda>>,
    Path(skill_id): Path<String>,
) -> impl IntoResponse {
    match sonda.skills.local().detail(skill_id.as_str()) {
        Some(detail) => Json(detail).into_response(),
        None => response_with(StatusCode::NOT_FOUND, "unknown skill").into_response(),
    }
}

#[derive(Deserialize)]
struct SkillsSearchQuery {
    q: Option<String>,
}

#[derive(Serialize)]
struct SkillsSearchRes {
    entries: Vec<SkillHubEntryJson>,
    from_remote: bool,
}

#[derive(Serialize)]
struct SkillHubEntryJson {
    slug: String,
    name: String,
    summary: String,
    version: String,
    tags: Vec<String>,
}

async fn skills_search(
    State(sonda): State<Arc<Sonda>>,
    Query(params): Query<SkillsSearchQuery>,
) -> impl IntoResponse {
    let q = params.q.as_deref().unwrap_or("").trim().to_string();

    match sonda.skills.hub().search(&q).await {
        Ok(result) => Json(SkillsSearchRes {
            entries: result
                .entries
                .into_iter()
                .map(|e| SkillHubEntryJson {
                    slug: e.slug,
                    name: e.name,
                    summary: if e.summary.is_empty() {
                        e.description
                    } else {
                        e.summary
                    },
                    version: e.version,
                    tags: e.tags,
                })
                .collect(),
            from_remote: result.from_remote,
        })
        .into_response(),
        Err(err) => response_with(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct SkillsInstallBody {
    slug: String,
    #[serde(default)]
    force: bool,
}

async fn skills_install(
    State(sonda): State<Arc<Sonda>>,
    Json(body): Json<SkillsInstallBody>,
) -> impl IntoResponse {
    let slug = body.slug.trim();
    if slug.is_empty() {
        return response_with(StatusCode::BAD_REQUEST, "field 'slug' is required").into_response();
    }
    if let Err(err) = SkillHub::validate_slug(slug) {
        return response_with(StatusCode::BAD_REQUEST, &err.to_string()).into_response();
    }

    match sonda.skills.install(slug, body.force).await {
        Ok(()) => Json(serde_json::json!({ "status": "ok" })).into_response(),
        Err(InstallSkillError::Hub(SkillHubError::AlreadyDownloaded { slug, path })) => {
            response_with(
                StatusCode::CONFLICT,
                &format!("skill '{slug}' already downloaded at {}", path.display()),
            )
            .into_response()
        }
        Err(InstallSkillError::Hub(err)) => {
            response_with(StatusCode::UNPROCESSABLE_ENTITY, &err.to_string()).into_response()
        }
        Err(InstallSkillError::RegisterFailed(msg)) => response_with(
            StatusCode::INTERNAL_SERVER_ERROR,
            &msg,
        )
        .into_response(),
    }
}

async fn skills_uninstall(
    State(sonda): State<Arc<Sonda>>,
    Path(skill_id): Path<String>,
) -> impl IntoResponse {
    match sonda.skills.uninstall(skill_id.as_str()) {
        Ok(result) => Json(serde_json::json!({ "status": "ok", "slug": result.slug })).into_response(),
        Err(UninstallSkillError::Unregister(UnregisterSkillError::InvalidId)) => {
            response_with(StatusCode::BAD_REQUEST, "invalid skill id").into_response()
        }
        Err(UninstallSkillError::Unregister(UnregisterSkillError::NotFound)) => {
            response_with(StatusCode::NOT_FOUND, "unknown skill").into_response()
        }
        Err(UninstallSkillError::Unregister(UnregisterSkillError::NotRemovable)) => {
            response_with(StatusCode::FORBIDDEN, "skill is not removable").into_response()
        }
        Err(UninstallSkillError::Unregister(UnregisterSkillError::Catalog(msg))) => {
            response_with(StatusCode::INTERNAL_SERVER_ERROR, &msg).into_response()
        }
        Err(UninstallSkillError::Unregister(UnregisterSkillError::Load(err))) => {
            response_with(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()).into_response()
        }
        Err(UninstallSkillError::RemoveFiles(msg)) => {
            response_with(StatusCode::INTERNAL_SERVER_ERROR, &msg).into_response()
        }
    }
}
