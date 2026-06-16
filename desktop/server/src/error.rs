use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use moray_channels::ChannelCatalogError;
use moray_core::{MorayError, ToolCallAuthError};
use moray_sonda::{
    FileIoError, InvalidContent, MissingReference, SessionCatalogError, SondaError,
    SondaSessionError, SondaSettingsStoreError, SondaToolCatalogError,
};
use serde::Serialize;

#[derive(Serialize)]
struct JsonErrorBody {
    message: String,
}

pub(crate) fn response_with(status: StatusCode, message: impl Into<String>) -> impl IntoResponse {
    (
        status,
        Json(JsonErrorBody {
            message: message.into(),
        }),
    )
}

pub(crate) fn tool_call_auth_error_response(err: ToolCallAuthError) -> Response {
    response_with(StatusCode::NOT_FOUND, err.to_string()).into_response()
}

pub(crate) fn sonda_error_response(err: SondaError) -> Response {
    let (status, message) = match &err {
        SondaError::InvalidArguments(e) => (StatusCode::BAD_REQUEST, e.to_string()),
        SondaError::InvalidContent(e) => invalid_content_http_status(e),
        SondaError::MissingReference(e) => missing_reference_http_status(e),
        SondaError::FileIo(e) => file_io_http_status(e),
        SondaError::BadEnvironmentVariable(e) => internal_error_http_status(e),
        SondaError::SettingsStore(e) => settings_store_http_status(e),
        SondaError::SessionCatalog(e) => session_catalog_http_status(e),
        SondaError::ChannelCatalog(e) => channel_catalog_http_status(e),
        SondaError::ToolCatalog(e) => tool_catalog_http_status(e),
        SondaError::Skills(e) => internal_error_http_status(e),
    };
    response_with(status, message).into_response()
}

fn invalid_content_http_status(e: &InvalidContent) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.message.clone())
}

fn missing_reference_http_status(e: &MissingReference) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, e.message.clone())
}

fn file_io_http_status(e: &FileIoError) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn internal_error_http_status(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn settings_store_http_status(err: &SondaSettingsStoreError) -> (StatusCode, String) {
    match err {
        SondaSettingsStoreError::InvalidContent(e) => invalid_content_http_status(e),
        SondaSettingsStoreError::MissingReference(e) => missing_reference_http_status(e),
        SondaSettingsStoreError::FileIo(e) => file_io_http_status(e),
    }
}

fn session_catalog_http_status(err: &SessionCatalogError) -> (StatusCode, String) {
    match err {
        SessionCatalogError::InvalidContent(e) => invalid_content_http_status(e),
        SessionCatalogError::MissingReference(e) => missing_reference_http_status(e),
        SessionCatalogError::FileIo(e) => file_io_http_status(e),
    }
}

fn tool_catalog_http_status(err: &SondaToolCatalogError) -> (StatusCode, String) {
    match err {
        SondaToolCatalogError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
        SondaToolCatalogError::Parse(msg) | SondaToolCatalogError::Io(msg) => {
            (StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
        }
    }
}

fn channel_catalog_http_status(err: &ChannelCatalogError) -> (StatusCode, String) {
    match err {
        ChannelCatalogError::InvalidContent(e) => (StatusCode::BAD_REQUEST, e.message.clone()),
        ChannelCatalogError::MissingReference(e) => (StatusCode::NOT_FOUND, e.message.clone()),
        ChannelCatalogError::FileIo(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

pub(crate) fn session_error_response(err: SondaSessionError) -> Response {
    match err {
        SondaSessionError::UnknownSession => {
            response_with(StatusCode::NOT_FOUND, "unknown chat session").into_response()
        }
        SondaSessionError::Moray(MorayError::Busy) => {
            response_with(StatusCode::CONFLICT, "session is busy").into_response()
        }
        SondaSessionError::Moray(e) => {
            response_with(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
