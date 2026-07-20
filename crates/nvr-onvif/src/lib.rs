//! ONVIF client wrapper: discovery, profiles, stream-URI, PTZ.

pub mod camera;
pub mod config;
pub mod types;
pub mod uri;

pub use camera::OnvifCamera;
pub use config::OnvifConfig;
pub use types::{DeviceInfo, Discovered, OnvifError, Preset, Profile, PtzVelocity};
pub use uri::inject_credentials;
