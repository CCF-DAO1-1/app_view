use color_eyre::eyre::Error;
use common_x::restful::axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    ValidateFailed(String),
    NotFound,
    ExecSqlFailed(String),
    CallPdsFailed(String),
    Unknown(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error, error_message) = match self {
            AppError::ValidateFailed(msg) => (StatusCode::BAD_REQUEST, "ValidateFailed", msg),
            AppError::NotFound => (StatusCode::NOT_FOUND, "NotFound", "NOT_FOUND".to_owned()),
            AppError::ExecSqlFailed(_msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "ExecSqlFailed",
                "ExecSqlFailed".to_string(),
            ),
            AppError::CallPdsFailed(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "CallPdsFailed",
                json!({"pds": msg}).to_string(),
            ),
            AppError::Unknown(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "ServerError", msg),
        };
        let body = Json(json!({
            "code": status.as_u16(),
            "error": error,
            "message": error_message,
        }));
        (status, body).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<Error>,
{
    fn from(err: E) -> Self {
        Self::Unknown(err.into().to_string())
    }
}
