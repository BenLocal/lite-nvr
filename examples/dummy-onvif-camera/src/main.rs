mod config;

mod auth;

mod responses;

mod soap;

mod discovery;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    let (cfg, opts) = config::Args::parse().into_cfg();
    log::info!(
        "dummy-onvif-camera: service {} (rtsp {}, discovery {}, launch_rtsp {})",
        cfg.service_url(),
        cfg.rtsp_url,
        opts.discovery,
        opts.launch_rtsp
    );
    // Server wiring lands in Task 6.
    Ok(())
}
