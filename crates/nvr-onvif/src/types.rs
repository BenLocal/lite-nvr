use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Discovered {
    pub endpoints: Vec<String>,
    pub name: Option<String>,
    pub hardware: Option<String>,
    pub addr: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub serial: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Profile {
    pub token: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    pub fps: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Preset {
    pub token: String,
    pub name: String,
}

/// Continuous-move velocity; each axis clamped to -1.0..=1.0 (0 = no motion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PtzVelocity {
    pub pan: f32,
    pub tilt: f32,
    pub zoom: f32,
}

impl PtzVelocity {
    pub fn new(pan: f32, tilt: f32, zoom: f32) -> Self {
        let c = |v: f32| v.clamp(-1.0, 1.0);
        Self {
            pan: c(pan),
            tilt: c(tilt),
            zoom: c(zoom),
        }
    }
}

#[derive(Debug)]
pub enum OnvifError {
    Connect(String),
    Auth,
    NoPtzService,
    NoProfile(String),
    Protocol(String),
}

impl std::fmt::Display for OnvifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnvifError::Connect(s) => write!(f, "connect failed: {s}"),
            OnvifError::Auth => write!(f, "authentication rejected"),
            OnvifError::NoPtzService => write!(f, "camera has no PTZ service"),
            OnvifError::NoProfile(t) => write!(f, "profile not found: {t}"),
            OnvifError::Protocol(s) => write!(f, "onvif protocol error: {s}"),
        }
    }
}

impl std::error::Error for OnvifError {}

#[cfg(test)]
#[path = "types_test.rs"]
mod types_test;
