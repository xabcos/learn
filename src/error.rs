use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use std::borrow::Cow;

pub type Result<T> = std::result::Result<T, AppError>;

// `Cow<'static, str>` lets the literal messages stay borrowed; dynamic ones still fit as owned.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Config(Cow<'static, str>),
    #[error("{0}")]
    NotFound(Cow<'static, str>),
    #[error("{0}")]
    Conflict(Cow<'static, str>),
    #[error("{0}")]
    Internal(Cow<'static, str>),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl AppError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Http(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Config(_)
            | Self::Internal(_)
            | Self::Sqlx(_)
            | Self::Migrate(_)
            | Self::Json(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// Readers get a plain page: what was not found, or that the site is busy. What went wrong
/// inside stays in the log.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let message = match &self {
            Self::NotFound(message) => message.as_ref(),
            _ => {
                tracing::error!(error = %self, "request failed");
                "页面暂时打不开，请稍后再试"
            }
        };
        (
            status,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            crate::pages::problem(status, message),
        )
            .into_response()
    }
}
