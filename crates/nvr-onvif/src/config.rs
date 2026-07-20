use serde::{Deserialize, Serialize};

/// Connection config for one ONVIF camera. This is exactly what an
/// `input_type == "onvif"` device stores in its `input_value` JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnvifConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Chosen media profile token; `None` = use the first profile.
    #[serde(default)]
    pub profile_token: Option<String>,
}

impl OnvifConfig {
    /// The ONVIF device-management service URL (the well-known entry point).
    pub fn service_url(&self) -> String {
        format!("http://{}:{}/onvif/device_service", self.host, self.port)
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
