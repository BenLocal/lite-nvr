//! GB28181 platform config, parsed from environment variables.

use gb28181::{AuthConfig, GbServerConfig};

/// Platform (上级平台) config. Present only when GB support is enabled.
#[derive(Debug, Clone)]
pub struct GbConfig {
    /// Our 20-digit platform GB code (SIP id / realm user).
    pub sip_id: String,
    /// SIP domain / digest realm.
    pub domain: String,
    /// SIP UDP listen port (national standard default 5060).
    pub listen_port: u16,
    /// Shared registration password. `None` = Open auth (no digest challenge).
    pub password: Option<String>,
    /// Local IP advertised in INVITE SDP — where devices send their PS/RTP.
    pub media_ip: String,
}

impl GbConfig {
    /// Parse from a generic getter (pure — unit-testable without touching real env).
    /// Returns `None` unless `NVR_GB_ENABLE == "1"` and both id+domain are set.
    pub fn from_map(get: impl Fn(&str) -> Option<String>) -> Option<GbConfig> {
        let enabled = get("NVR_GB_ENABLE").as_deref() == Some("1");
        if !enabled {
            return None;
        }
        let sip_id = get("NVR_GB_SIP_ID").filter(|s| !s.is_empty())?;
        let domain = get("NVR_GB_DOMAIN").filter(|s| !s.is_empty())?;
        let listen_port = get("NVR_GB_PORT")
            .and_then(|s| s.parse().ok())
            .unwrap_or(5060);
        let password = get("NVR_GB_PASSWORD").filter(|s| !s.is_empty());
        let media_ip = get("NVR_GB_MEDIA_IP")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        Some(GbConfig {
            sip_id,
            domain,
            listen_port,
            password,
            media_ip,
        })
    }

    /// Parse from the real process environment.
    pub fn from_env() -> Option<GbConfig> {
        Self::from_map(|k| std::env::var(k).ok())
    }

    /// Build the crate's server config from this platform config.
    pub fn to_server_config(&self) -> anyhow::Result<GbServerConfig> {
        let listen = format!("0.0.0.0:{}", self.listen_port).parse()?;
        let mut cfg = GbServerConfig::new(self.sip_id.clone(), self.domain.clone(), listen);
        cfg.auth = match &self.password {
            Some(pw) => AuthConfig::Shared(pw.clone()),
            None => AuthConfig::Open,
        };
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // The closure owns `map` (all Strings) and borrows nothing from `pairs`.
    fn getter(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn disabled_when_flag_absent() {
        assert!(GbConfig::from_map(getter(&[("NVR_GB_SIP_ID", "3402...")])).is_none());
    }

    #[test]
    fn parses_full_config_with_defaults() {
        let cfg = GbConfig::from_map(getter(&[
            ("NVR_GB_ENABLE", "1"),
            ("NVR_GB_SIP_ID", "34020000002000000001"),
            ("NVR_GB_DOMAIN", "3402000000"),
        ]))
        .unwrap();
        assert_eq!(cfg.listen_port, 5060); // default
        assert_eq!(cfg.media_ip, "127.0.0.1"); // default
        assert!(cfg.password.is_none()); // Open auth
    }

    #[test]
    fn honors_overrides() {
        let cfg = GbConfig::from_map(getter(&[
            ("NVR_GB_ENABLE", "1"),
            ("NVR_GB_SIP_ID", "34020000002000000001"),
            ("NVR_GB_DOMAIN", "3402000000"),
            ("NVR_GB_PORT", "15060"),
            ("NVR_GB_PASSWORD", "s3cret"),
            ("NVR_GB_MEDIA_IP", "192.168.1.10"),
        ]))
        .unwrap();
        assert_eq!(cfg.listen_port, 15060);
        assert_eq!(cfg.password.as_deref(), Some("s3cret"));
        assert_eq!(cfg.media_ip, "192.168.1.10");
        let sc = cfg.to_server_config().unwrap();
        assert!(matches!(sc.auth, AuthConfig::Shared(_)));
    }

    #[test]
    fn missing_id_or_domain_is_none() {
        assert!(GbConfig::from_map(getter(&[("NVR_GB_ENABLE", "1")])).is_none());
        assert!(
            GbConfig::from_map(getter(&[("NVR_GB_ENABLE", "1"), ("NVR_GB_SIP_ID", "x")])).is_none()
        );
    }
}
