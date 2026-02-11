use axum::{
    Json, Router,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::handler::ApiJsonResult;

pub fn user_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/info", get(user_info))
}

#[derive(Serialize, Deserialize)]
struct UserLoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
struct UserLoginResponse {
    token: String,
}

async fn index() -> &'static str {
    "user route!"
}

async fn login(Json(_req): Json<UserLoginRequest>) -> ApiJsonResult<UserLoginResponse> {
    Ok(Json(UserLoginResponse {
        token: "token".to_string(),
    }))
}

async fn logout() -> Json<String> {
    Json("success".to_string())
}

async fn user_info() -> Json<String> {
    Json("success".to_string())
}
