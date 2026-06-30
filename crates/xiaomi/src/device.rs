//! Port of go2rtc `internal/xiaomi/xiaomi.go` device resolution (miss path).
//!
//! After [`crate::cloud::Cloud`] is authenticated, [`list_cameras`] discovers a
//! user's cameras (their `did` + `model`) and [`resolve_miss`] resolves one into
//! a [`MissConnection`] — the keys + vendor params the TUTK CS2 transport
//! (phase 4) needs to dial the camera.

use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::cloud::Cloud;
use crate::crypto;

/// The Xiaomi Home app sid used for auth (`AppXiaomiHome`).
pub const APP_XIAOMI_HOME: &str = "xiaomiio";

/// miio cloud base URL for a region (empty = mainland China).
pub fn base_url(region: &str) -> String {
    match region {
        "de" | "i2" | "ru" | "sg" | "us" => format!("https://{region}.api.io.mi.com/app"),
        _ => "https://api.io.mi.com/app".to_string(),
    }
}

/// A device from the cloud device list.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Device {
    pub did: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub mac: String,
    #[serde(default, rename = "localip")]
    pub ip: String,
}

impl Device {
    pub fn has_camera(&self) -> bool {
        self.model.contains(".camera.")
            || self.model.contains(".cateye.")
            || self.model.contains(".feeder.")
    }
}

/// List the user's camera devices (`/v2/home/device_list_page`).
pub fn list_cameras(cloud: &Cloud, region: &str) -> Result<Vec<Device>> {
    #[derive(serde::Deserialize)]
    struct DeviceList {
        #[serde(default)]
        list: Vec<Device>,
    }
    let body = cloud.request(
        &base_url(region),
        "/v2/home/device_list_page",
        "{}",
        &HashMap::new(),
    )?;
    let v: DeviceList = serde_json::from_slice(&body)?;
    Ok(v.list.into_iter().filter(Device::has_camera).collect())
}

/// Resolved miss/TUTK-CS2 connection parameters for one camera.
#[derive(Debug, Clone)]
pub struct MissConnection {
    pub did: String,
    pub model: String,
    /// Our ephemeral Curve25519 keypair (raw bytes).
    pub client_public: Vec<u8>,
    pub client_private: Vec<u8>,
    /// Device's public key (hex) and the cloud signature over our app data.
    pub device_public: String,
    pub sign: String,
    /// Vendor transport name: `tutk` / `agora` / `cs2` / `mtp` / numeric.
    pub vendor: String,
    /// P2P UID (TUTK vendor id == 1).
    pub uid: Option<String>,
}

/// Resolve a camera via `/v2/device/miss_get_vendor` (`getMissURL`).
pub fn resolve_miss(cloud: &Cloud, region: &str, did: &str, model: &str) -> Result<MissConnection> {
    let (client_public, client_private) = crypto::generate_key();
    let params = format!(
        r#"{{"app_pubkey":"{}","did":"{}","support_vendors":"TUTK_CS2_MTP"}}"#,
        hex::encode(&client_public),
        did
    );

    let body = match cloud.request(
        &base_url(region),
        "/v2/device/miss_get_vendor",
        &params,
        &HashMap::new(),
    ) {
        Ok(body) => body,
        Err(e) if e.to_string().contains("no available vendor support") => {
            // go2rtc falls back to the legacy path here; not ported yet.
            bail!("xiaomi: device {did} has no miss/CS2 vendor (legacy path not ported)");
        }
        Err(e) => return Err(e),
    };

    #[derive(serde::Deserialize)]
    struct Resp {
        vendor: VendorInfo,
        #[serde(default)]
        public_key: String,
        #[serde(default)]
        sign: String,
    }
    #[derive(serde::Deserialize)]
    struct VendorInfo {
        vendor: u8,
        #[serde(default)]
        vendor_params: VendorParams,
    }
    #[derive(Default, serde::Deserialize)]
    struct VendorParams {
        #[serde(default)]
        p2p_id: String,
    }

    let v: Resp = serde_json::from_slice(&body)?;
    Ok(MissConnection {
        did: did.to_string(),
        model: model.to_string(),
        client_public,
        client_private,
        device_public: v.public_key,
        sign: v.sign,
        vendor: vendor_name(v.vendor.vendor),
        uid: if v.vendor.vendor == 1 {
            Some(v.vendor.vendor_params.p2p_id)
        } else {
            None
        },
    })
}

fn vendor_name(id: u8) -> String {
    match id {
        1 => "tutk".to_string(),
        3 => "agora".to_string(),
        4 => "cs2".to_string(),
        6 => "mtp".to_string(),
        other => other.to_string(),
    }
}

/// Wake a battery/doorbell camera before connecting (`wakeUpCamera`).
pub fn wake_up_camera(cloud: &Cloud, region: &str, did: &str) -> Result<()> {
    let params = r#"{"id":1,"method":"wakeup","params":{"video":"1"}}"#;
    cloud.request(
        &base_url(region),
        &format!("/home/rpc/{did}"),
        params,
        &HashMap::new(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_regions() {
        assert_eq!(base_url(""), "https://api.io.mi.com/app");
        assert_eq!(base_url("cn"), "https://api.io.mi.com/app");
        assert_eq!(base_url("us"), "https://us.api.io.mi.com/app");
        assert_eq!(base_url("de"), "https://de.api.io.mi.com/app");
    }

    #[test]
    fn vendor_names() {
        assert_eq!(vendor_name(1), "tutk");
        assert_eq!(vendor_name(4), "cs2");
        assert_eq!(vendor_name(6), "mtp");
        assert_eq!(vendor_name(9), "9");
    }

    #[test]
    fn device_has_camera() {
        let cam = Device {
            did: "1".into(),
            name: "Cam".into(),
            model: "chuangmi.camera.ipc019".into(),
            mac: String::new(),
            ip: String::new(),
        };
        assert!(cam.has_camera());
        let plug = Device {
            model: "zimi.powerstrip.v2".into(),
            ..cam.clone()
        };
        assert!(!plug.has_camera());
    }

    #[test]
    fn parse_miss_vendor_response_shape() {
        // The `result` JSON that resolve_miss deserializes.
        let body = br#"{"vendor":{"vendor":1,"vendor_params":{"p2p_id":"ABC123"}},"public_key":"deadbeef","sign":"sig"}"#;
        #[derive(serde::Deserialize)]
        struct Resp {
            vendor: VendorInfo,
            public_key: String,
            sign: String,
        }
        #[derive(serde::Deserialize)]
        struct VendorInfo {
            vendor: u8,
            vendor_params: VendorParams,
        }
        #[derive(serde::Deserialize)]
        struct VendorParams {
            p2p_id: String,
        }
        let v: Resp = serde_json::from_slice(body).unwrap();
        assert_eq!(v.vendor.vendor, 1);
        assert_eq!(v.vendor.vendor_params.p2p_id, "ABC123");
        assert_eq!(v.public_key, "deadbeef");
        assert_eq!(vendor_name(v.vendor.vendor), "tutk");
        assert_eq!(v.sign, "sig");
    }
}
