//! API error type: everything a client can see, with correct status codes.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden: {0}")]
    Forbidden(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("paypal error: {0}")]
    PayPal(#[from] kindly_paypal::PayPalError),

    #[error("domain error: {0}")]
    Domain(#[from] kindly_core::CoreError),

    #[error("database error: {0}")]
    Database(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl From<crate::portal::PortalTokenError> for ApiError {
    fn from(error: crate::portal::PortalTokenError) -> Self {
        match error {
            crate::portal::PortalTokenError::Expired => {
                ApiError::Forbidden("this payment link has expired".into())
            }
            crate::portal::PortalTokenError::BadSignature
            | crate::portal::PortalTokenError::Malformed => {
                ApiError::Forbidden("this payment link is not valid".into())
            }
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::RowNotFound => ApiError::NotFound("row".into()),
            other => ApiError::Database(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        ApiError::Internal(format!("json: {error}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::BadRequest(_) | ApiError::Domain(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::PayPal(error) => match error {
                kindly_paypal::PayPalError::NotFound(_) => StatusCode::NOT_FOUND,
                kindly_paypal::PayPalError::SignatureInvalid(_) => StatusCode::UNAUTHORIZED,
                kindly_paypal::PayPalError::Config(_) => StatusCode::SERVICE_UNAVAILABLE,
                kindly_paypal::PayPalError::Api { status, .. } => {
                    StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
                }
                _ => StatusCode::BAD_GATEWAY,
            },
            ApiError::Database(_) | ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let body = Json(json!({
            "error": {
                "message": self.to_string(),
                "kind": match &self {
                    ApiError::NotFound(_) => "not_found",
                    ApiError::BadRequest(_) => "bad_request",
                    ApiError::Unauthorized => "unauthorized",
                    ApiError::Forbidden(_) => "forbidden",
                    ApiError::Conflict(_) => "conflict",
                    ApiError::PayPal(_) => "paypal",
                    ApiError::Domain(_) => "domain",
                    ApiError::Database(_) => "database",
                    ApiError::Internal(_) => "internal",
                },
            }
        }));
        (status, body).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
