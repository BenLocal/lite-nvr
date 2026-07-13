use axum::{
    Extension, Json, Router,
    extract::Path,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    auth::{self, AuthUser},
    db::app_db_conn,
    handler::{ApiJsonResult, ok_empty, ok_json},
};

pub fn user_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/info", get(user_info))
        .route("/password", post(change_password))
        .route("/list", get(list_users))
        .route("/add", post(add_user))
        .route("/remove/{username}", post(remove_user))
}

#[derive(Serialize, Deserialize)]
struct UserLoginRequest {
    username: String,
    password: String,
}

#[derive(Serialize, Deserialize)]
struct UserLoginResponse {
    token: String,
    username: String,
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

    if !nvr_db::user::verify_password(&req.password, &user.password_hash) {
        return Err(anyhow::anyhow!("Invalid username or password").into());
    }

    // Opportunistic GC of expired sessions; failure must not block login.
    let _ = nvr_db::session::delete_expired(Utc::now(), &conn).await;

    let token = auth::create_session(username).await?;
    Ok(ok_json(UserLoginResponse {
        token,
        username: username.to_string(),
    }))
}

async fn logout(Extension(user): Extension<AuthUser>) -> ApiJsonResult<()> {
    auth::revoke(&user.token).await?;
    Ok(ok_empty())
}

#[derive(Serialize)]
struct UserInfoResponse {
    username: String,
}

async fn user_info(Extension(user): Extension<AuthUser>) -> ApiJsonResult<UserInfoResponse> {
    Ok(ok_json(UserInfoResponse {
        username: user.username,
    }))
}

#[derive(Deserialize)]
struct ChangePasswordRequest {
    old_password: String,
    new_password: String,
}

async fn change_password(
    Extension(user): Extension<AuthUser>,
    Json(req): Json<ChangePasswordRequest>,
) -> ApiJsonResult<()> {
    if req.new_password.is_empty() {
        return Err(anyhow::anyhow!("New password must not be empty").into());
    }

    let conn = app_db_conn()?;
    let mut record = nvr_db::user::get_by_username(&user.username, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;

    if !nvr_db::user::verify_password(&req.old_password, &record.password_hash) {
        return Err(anyhow::anyhow!("Old password is incorrect").into());
    }

    record.password_hash = nvr_db::user::hash_password(&req.new_password)?;
    record.update_time = Utc::now();
    nvr_db::user::update(&record, &conn).await?;

    // Kick every other session of this user; the current one stays valid.
    auth::revoke_user(&user.username, Some(&user.token)).await?;
    Ok(ok_empty())
}

#[derive(Serialize)]
struct UserListItem {
    username: String,
    create_time: DateTime<Utc>,
    update_time: DateTime<Utc>,
}

async fn list_users() -> ApiJsonResult<Vec<UserListItem>> {
    let conn = app_db_conn()?;
    let mut users: Vec<UserListItem> = nvr_db::user::list(&conn)
        .await?
        .into_iter()
        .map(|u| UserListItem {
            username: u.username,
            create_time: u.create_time,
            update_time: u.update_time,
        })
        .collect();
    users.sort_by(|a, b| a.username.cmp(&b.username));
    Ok(ok_json(users))
}

#[derive(Deserialize)]
struct AddUserRequest {
    username: String,
    password: String,
}

async fn add_user(Json(req): Json<AddUserRequest>) -> ApiJsonResult<()> {
    let username = req.username.trim();
    if username.is_empty() || req.password.is_empty() {
        return Err(anyhow::anyhow!("Username and password must not be empty").into());
    }

    let conn = app_db_conn()?;
    if nvr_db::user::exists(username, &conn).await? {
        return Err(anyhow::anyhow!("User already exists").into());
    }

    let now = Utc::now();
    let user = nvr_db::user::UserInfo {
        username: username.to_string(),
        password_hash: nvr_db::user::hash_password(&req.password)?,
        metadata: std::collections::HashMap::new(),
        create_time: now,
        update_time: now,
    };
    nvr_db::user::insert(&user, &conn).await?;
    Ok(ok_empty())
}

async fn remove_user(
    Extension(user): Extension<AuthUser>,
    Path(username): Path<String>,
) -> ApiJsonResult<()> {
    if username == user.username {
        return Err(anyhow::anyhow!("Cannot remove the currently logged-in user").into());
    }

    let conn = app_db_conn()?;
    if !nvr_db::user::exists(&username, &conn).await? {
        return Err(anyhow::anyhow!("User not found").into());
    }

    nvr_db::user::delete(&username, &conn).await?;
    auth::revoke_user(&username, None).await?;
    Ok(ok_empty())
}
