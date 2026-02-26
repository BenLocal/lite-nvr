use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub username: String,
    pub password_hash: String,
    pub metadata: HashMap<String, String>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}
