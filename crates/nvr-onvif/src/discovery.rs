use std::time::Duration;

use futures::StreamExt;

use crate::types::{Discovered, OnvifError};

/// Probe the LAN for ONVIF devices via WS-Discovery for `timeout`.
pub async fn discover(timeout: Duration) -> Result<Vec<Discovered>, OnvifError> {
    // `DiscoveryBuilder` uses `&mut self` setters, so bind it before chaining.
    let mut builder = onvif::discovery::DiscoveryBuilder::default();
    builder.duration(timeout);

    let stream = builder
        .run()
        .await
        .map_err(|e| OnvifError::Protocol(format!("discovery: {e:?}")))?;

    let devices = stream
        .map(|d| {
            let endpoints: Vec<String> = d.urls.iter().map(|u| u.to_string()).collect();
            let addr = endpoints.first().and_then(|u| {
                url::Url::parse(u).ok().and_then(|parsed| {
                    parsed.host_str().map(|h| match parsed.port() {
                        Some(p) => format!("{h}:{p}"),
                        None => h.to_string(),
                    })
                })
            });
            Discovered {
                endpoints,
                name: d.name,
                hardware: d.hardware,
                addr,
            }
        })
        .collect::<Vec<_>>()
        .await;

    Ok(devices)
}
