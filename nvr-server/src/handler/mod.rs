use axum::{
    Json,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use reqwest::StatusCode;

pub mod device;
pub mod media_pipe;
pub mod playback;
pub mod system;
pub mod user;

pub type ApiResult<T> = Result<T, ApiError>;
pub type ApiJsonResult<T> = ApiResult<Json<BaseResponse<T>>>;

#[derive(Debug, Serialize)]
pub struct BaseResponse<T> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
}

pub fn ok_json<T>(data: T) -> Json<BaseResponse<T>>
where
    T: Serialize,
{
    Json(BaseResponse {
        code: 0,
        message: "success".to_string(),
        data: Some(data),
    })
}

pub fn ok_empty() -> Json<BaseResponse<()>> {
    Json(BaseResponse {
        code: 0,
        message: "success".to_string(),
        data: None,
    })
}

pub struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        log::error!("ApiError: {:?}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(BaseResponse::<()> {
                code: 500,
                message: self.0.to_string(),
                data: None,
            }),
        ).into_response()
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
