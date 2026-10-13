use serde::Serialize;

pub const TERMINAL_SOURCE: &str = "alsa:rockchipes8388";

#[derive(Serialize)]
pub struct SourceOption {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
}

pub async fn available() -> anyhow::Result<Vec<SourceOption>> {
    let conn = crate::db::app_db_conn()?;
    let mut sources: Vec<_> = nvr_db::device::list(&conn)
        .await?
        .into_iter()
        .filter(|d| d.include_audio && d.input_type != "xiaomi")
        .map(|d| SourceOption {
            id: d.id,
            name: d.name,
            kind: "device",
        })
        .collect();
    if tokio::fs::read_to_string("/proc/asound/cards")
        .await
        .unwrap_or_default()
        .contains("[rockchipes8388")
    {
        sources.push(SourceOption {
            id: TERMINAL_SOURCE.to_string(),
            name: "端子音频（左右双声道）".to_string(),
            kind: "alsa",
        });
    }
    Ok(sources)
}

pub async fn ensure(ids: &[String]) -> anyhow::Result<()> {
    let sources = available().await?;
    for id in ids {
        anyhow::ensure!(
            sources.iter().any(|s| &s.id == id),
            "音频源不存在或未启用原音：{id}"
        );
    }
    // Validate the entire input set before starting any capture.
    for id in ids {
        if id == TERMINAL_SOURCE {
            super::MIXER
                .ensure_capture_source(id, "hw:CARD=rockchipes8388,DEV=0", "alsa")
                .await?;
        } else {
            super::MIXER.ensure_shared_source(id);
        }
    }
    if let Some(bridge) = crate::gb::bridge() {
        // Unknown/non-GB mappings are ignored by looking up the stored type
        // once for the whole set, not one database call per source.
        let conn = crate::db::app_db_conn()?;
        let devices = nvr_db::device::list(&conn).await?;
        for device in devices
            .iter()
            .filter(|d| d.input_type == "gb28181" && ids.contains(&d.id))
        {
            bridge.ensure_mixer_source(&device.id).await?;
        }
    }
    Ok(())
}
