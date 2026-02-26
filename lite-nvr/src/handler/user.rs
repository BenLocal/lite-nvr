use axum::{
    Json, Router,
    routing::{get, post},
};
use nvr_db::{kv::Kv, user::UserInfo};
use serde::{Deserialize, Serialize};

use crate::{
    db::app_db_conn,
    handler::{ApiError, ApiJsonResult},
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

    let key = req.username;
    let user = nvr_db::kv::by_module_and_key("user", &key, &conn).await?;
    let kv = user.ok_or(anyhow::anyhow!("User not found"))?;
    let _user: UserInfo = serde_json::from_str(&kv.value.unwrap_or_default())?;

    // let hash = argon2::argon2id13::Argon2::default()
    //     .hash_password(req.password.as_bytes(), &user.password_hash.as_bytes())?;
    // if hash != user.password_hash {
    //     return Err(anyhow::anyhow!("Invalid password").into());
    // }
    let token = uuid::Uuid::new_v4().to_string();
    Ok(Json(UserLoginResponse { token }))
}

async fn logout() -> Json<String> {
    Json("success".to_string())
}

async fn user_info() -> Json<String> {
    Json("success".to_string())
}
