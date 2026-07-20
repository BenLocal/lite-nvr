use quick_xml::Reader;
use quick_xml::events::Event;

use crate::auth::{self, UsernameToken};
use crate::config::DeviceCfg;
use crate::responses;

pub struct Reply {
    pub status: u16,
    pub body: String,
}

const OPS: &[&str] = &[
    "GetCapabilities",
    "GetDeviceInformation",
    "GetProfiles",
    "GetStreamUri",
    "ContinuousMove",
    "Stop",
    "GetPresets",
    "GotoPreset",
];

/// Detect the ONVIF operation by looking for its request element's local name.
/// GetPresets must be checked before GetProfiles etc.; we test the whole set and
/// return the first that appears as an element local-name in the body.
pub fn detect_op(body: &str) -> Option<&'static str> {
    let mut reader = Reader::from_str(body);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.local_name();
                let local = String::from_utf8_lossy(name.as_ref()).to_string();
                if let Some(op) = OPS.iter().find(|op| **op == local) {
                    return Some(op);
                }
            }
            Ok(Event::Eof) => return None,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// Extract the WS-Security UsernameToken fields (PasswordDigest mode) if present.
pub fn extract_token(body: &str) -> Option<UsernameToken> {
    let mut reader = Reader::from_str(body);
    let mut buf = Vec::new();
    let (mut username, mut digest, mut nonce, mut created) = (None, None, None, None);
    let mut cur: Option<&'static str> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                cur = match local.as_str() {
                    "Username" => Some("Username"),
                    "Password" => Some("Password"),
                    "Nonce" => Some("Nonce"),
                    "Created" => Some("Created"),
                    _ => None,
                };
            }
            Ok(Event::Text(t)) => {
                if let Some(field) = cur {
                    let val = t.unescape().unwrap_or_default().to_string();
                    match field {
                        "Username" => username = Some(val),
                        "Password" => digest = Some(val),
                        "Nonce" => nonce = Some(val),
                        "Created" => created = Some(val),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(_)) => cur = None,
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
    Some(UsernameToken {
        username: username?,
        password_digest: digest?,
        nonce: nonce?,
        created: created?,
    })
}

/// Authenticate + dispatch. Every operation requires a valid UsernameToken.
pub fn handle(body: &str, cfg: &DeviceCfg) -> Reply {
    let Some(op) = detect_op(body) else {
        return Reply {
            status: 500,
            body: responses::fault_action_not_supported(),
        };
    };

    let authed = extract_token(body)
        .map(|t| auth::verify(&t, &cfg.username, &cfg.password))
        .unwrap_or(false);
    if !authed {
        log::warn!("onvif {op}: rejected (bad/missing UsernameToken)");
        return Reply {
            status: 400,
            body: responses::fault_not_authorized(),
        };
    }

    let body = match op {
        "GetCapabilities" => responses::get_capabilities(&cfg.service_url()),
        "GetDeviceInformation" => responses::device_information(cfg),
        "GetProfiles" => responses::get_profiles(),
        "GetStreamUri" => responses::get_stream_uri(&cfg.rtsp_url),
        "GetPresets" => responses::get_presets(),
        "ContinuousMove" => {
            log::info!("onvif PTZ: ContinuousMove");
            responses::ptz_ack("tptz:ContinuousMoveResponse")
        }
        "Stop" => {
            log::info!("onvif PTZ: Stop");
            responses::ptz_ack("tptz:StopResponse")
        }
        "GotoPreset" => {
            log::info!("onvif PTZ: GotoPreset");
            responses::ptz_ack("tptz:GotoPresetResponse")
        }
        _ => unreachable!("op is from OPS"),
    };
    Reply { status: 200, body }
}

#[cfg(test)]
#[path = "soap_test.rs"]
mod soap_test;
