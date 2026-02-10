use axum::{
    Json,
    response::{IntoResponse, Response},
};
use reqwest::StatusCode;

pub mod media_pipe;
pub mod user;

pub type ApiResult<T> = Result<T, ApiError>;
pub type ApiJsonResult<T> = ApiResult<Json<T>>;

pub struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        eprintln!("ApiError: {:?}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Manager went wrong because service inner error"),
        )
            .into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
