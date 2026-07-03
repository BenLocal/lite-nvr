use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    db::app_db_conn,
    handler::{ApiJsonResult, ok_empty, ok_json},
};

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

async fn login(Json(req): Json<UserLoginRequest>) -> ApiJsonResult<UserLoginResponse> {
    let conn = app_db_conn()?;

    let username = req.username.trim();
    if username.is_empty() || req.password.is_empty() {
        return Err(anyhow::anyhow!("Invalid username or password").into());
    }

    let user = nvr_db::user::get_by_username(username, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Invalid username or password"))?;

    let parsed_hash = PasswordHash::new(&user.password_hash)
        .map_err(|_| anyhow::anyhow!("Invalid username or password"))?;
    Argon2::default()
        .verify_password(req.password.as_bytes(), &parsed_hash)
        .map_err(|_| anyhow::anyhow!("Invalid username or password"))?;

    let token = uuid::Uuid::new_v4().to_string();
    Ok(ok_json(UserLoginResponse { token }))
}

async fn logout() -> ApiJsonResult<()> {
    Ok(ok_empty())
}

async fn user_info() -> ApiJsonResult<String> {
    Ok(ok_json("success".to_string()))
}
