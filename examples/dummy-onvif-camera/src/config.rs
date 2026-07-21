use clap::Parser;

/// Everything the SOAP responses and discovery need to describe this device.
#[derive(Clone, Debug)]
pub struct DeviceCfg {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub rtsp_url: String,
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub serial: String,
}

impl DeviceCfg {
    /// The device-management service URL (also advertised for media + ptz).
    pub fn service_url(&self) -> String {
        format!("http://{}:{}/onvif/device_service", self.host, self.port)
    }
}

/// Runtime toggles that aren't part of the device description.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeOpts {
    pub launch_rtsp: bool,
    pub discovery: bool,
}

#[derive(Parser, Debug)]
#[command(
    name = "dummy-onvif-camera",
    about = "Simulated ONVIF camera for testing nvr-onvif."
)]
pub struct Args {
    /// Advertised media/service IP.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// ONVIF HTTP port.
    #[arg(long, default_value_t = 8000)]
    pub port: u16,
    /// Required WS-Security username.
    #[arg(long, default_value = "admin")]
    pub username: String,
    /// Required WS-Security password.
    #[arg(long, default_value = "admin")]
    pub password: String,
    /// URL returned by GetStreamUri.
    #[arg(long, default_value = "rtsp://127.0.0.1:9554/live/test1")]
    pub rtsp_url: String,
    /// Also spawn dummy-rtsp-camera as a child.
    #[arg(long, default_value_t = false)]
    pub launch_rtsp: bool,
    /// Disable the WS-Discovery responder (it runs by default).
    #[arg(long, default_value_t = false)]
    pub no_discovery: bool,
    #[arg(long, default_value = "lite-nvr")]
    pub manufacturer: String,
    #[arg(long, default_value = "dummy-onvif-camera")]
    pub model: String,
    #[arg(long, default_value = "0.1")]
    pub firmware: String,
    #[arg(long, default_value = "SN-0001")]
    pub serial: String,
}

impl Args {
    pub fn into_cfg(self) -> (DeviceCfg, RuntimeOpts) {
        let opts = RuntimeOpts {
            launch_rtsp: self.launch_rtsp,
            discovery: !self.no_discovery,
        };
        let cfg = DeviceCfg {
            host: self.host,
            port: self.port,
            username: self.username,
            password: self.password,
            rtsp_url: self.rtsp_url,
            manufacturer: self.manufacturer,
            model: self.model,
            firmware: self.firmware,
            serial: self.serial,
        };
        (cfg, opts)
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
