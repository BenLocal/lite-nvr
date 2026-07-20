use onvif::soap::client::{AuthType, Client, ClientBuilder, Credentials};
use schema::onvif as onvif_xsd;
use url::Url;

use crate::config::OnvifConfig;
use crate::types::{DeviceInfo, OnvifError, Preset, Profile, PtzVelocity};

/// A connected ONVIF camera: per-service SOAP clients resolved once at connect.
pub struct OnvifCamera {
    devicemgmt: Client,
    media: Client,
    ptz: Option<Client>,
    /// Fallback profile token (first profile) used when a caller passes None.
    default_profile: String,
}

fn creds(cfg: &OnvifConfig) -> Option<Credentials> {
    if cfg.username.is_empty() {
        None
    } else {
        Some(Credentials {
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        })
    }
}

fn client_at(url: &str, cfg: &OnvifConfig) -> Result<Client, OnvifError> {
    let uri = Url::parse(url).map_err(|e| OnvifError::Connect(format!("{e:?}")))?;
    Ok(ClientBuilder::new(&uri)
        .credentials(creds(cfg))
        .auth_type(AuthType::Any)
        .build())
}

fn proto(e: impl std::fmt::Debug) -> OnvifError {
    OnvifError::Protocol(format!("{e:?}"))
}

impl OnvifCamera {
    pub async fn connect(cfg: &OnvifConfig) -> Result<OnvifCamera, OnvifError> {
        let devicemgmt = client_at(&cfg.service_url(), cfg)?;

        // Resolve media/ptz service addresses from GetCapabilities.
        let caps = schema::devicemgmt::get_capabilities(&devicemgmt, &Default::default())
            .await
            .map_err(proto)?;
        let media_addr = caps
            .capabilities
            .media
            .first()
            .map(|m| m.x_addr.clone())
            .ok_or_else(|| OnvifError::Protocol("device advertises no media service".into()))?;
        let media = client_at(&media_addr, cfg)?;
        let ptz = caps
            .capabilities
            .ptz
            .first()
            .map(|p| client_at(&p.x_addr, cfg))
            .transpose()?;

        // First profile token = default.
        let profiles = schema::media::get_profiles(&media, &Default::default())
            .await
            .map_err(proto)?;
        let default_profile = profiles
            .profiles
            .first()
            .map(|p| p.token.0.clone())
            .ok_or_else(|| OnvifError::Protocol("device has no media profiles".into()))?;

        Ok(OnvifCamera {
            devicemgmt,
            media,
            ptz,
            default_profile,
        })
    }

    pub async fn device_info(&self) -> Result<DeviceInfo, OnvifError> {
        let i = schema::devicemgmt::get_device_information(&self.devicemgmt, &Default::default())
            .await
            .map_err(proto)?;
        Ok(DeviceInfo {
            manufacturer: i.manufacturer,
            model: i.model,
            firmware: i.firmware_version,
            serial: i.serial_number,
        })
    }

    pub async fn profiles(&self) -> Result<Vec<Profile>, OnvifError> {
        let resp = schema::media::get_profiles(&self.media, &Default::default())
            .await
            .map_err(proto)?;
        Ok(resp
            .profiles
            .iter()
            .map(|p| {
                let (width, height, codec) = p
                    .video_encoder_configuration
                    .as_ref()
                    .map(|v| {
                        (
                            v.resolution.width.max(0) as u32,
                            v.resolution.height.max(0) as u32,
                            format!("{:?}", v.encoding),
                        )
                    })
                    .unwrap_or((0, 0, String::new()));
                Profile {
                    token: p.token.0.clone(),
                    name: p.name.0.clone(),
                    width,
                    height,
                    video_codec: codec,
                    fps: 0.0,
                }
            })
            .collect())
    }

    pub async fn stream_uri(&self, profile: Option<&str>) -> Result<String, OnvifError> {
        let token = profile.unwrap_or(&self.default_profile).to_string();
        let req = schema::media::GetStreamUri {
            profile_token: onvif_xsd::ReferenceToken(token),
            stream_setup: onvif_xsd::StreamSetup {
                stream: onvif_xsd::StreamType::RtpUnicast,
                transport: onvif_xsd::Transport {
                    protocol: onvif_xsd::TransportProtocol::Rtsp,
                    tunnel: vec![],
                },
            },
        };
        let resp = schema::media::get_stream_uri(&self.media, &req)
            .await
            .map_err(proto)?;
        Ok(resp.media_uri.uri)
    }

    pub async fn ptz_move(&self, v: PtzVelocity) -> Result<(), OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::ContinuousMove {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
            velocity: onvif_xsd::Ptzspeed {
                pan_tilt: Some(onvif_xsd::Vector2D {
                    x: v.pan as f64,
                    y: v.tilt as f64,
                    space: None,
                }),
                zoom: Some(onvif_xsd::Vector1D {
                    x: v.zoom as f64,
                    space: None,
                }),
            },
            timeout: None,
        };
        schema::ptz::continuous_move(ptz, &req)
            .await
            .map_err(proto)?;
        Ok(())
    }

    pub async fn ptz_stop(&self) -> Result<(), OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::Stop {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
            pan_tilt: Some(true),
            zoom: Some(true),
        };
        schema::ptz::stop(ptz, &req).await.map_err(proto)?;
        Ok(())
    }

    pub async fn presets(&self) -> Result<Vec<Preset>, OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::GetPresets {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
        };
        let resp = schema::ptz::get_presets(ptz, &req).await.map_err(proto)?;
        Ok(resp
            .preset
            .iter()
            .filter_map(|p| {
                p.token.as_ref().map(|t| Preset {
                    token: t.0.clone(),
                    name: p.name.as_ref().map(|n| n.0.clone()).unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn goto_preset(&self, token: &str) -> Result<(), OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::GotoPreset {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
            preset_token: onvif_xsd::ReferenceToken(token.to_string()),
            speed: None,
        };
        schema::ptz::goto_preset(ptz, &req).await.map_err(proto)?;
        Ok(())
    }
}
