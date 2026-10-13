use serde::Serialize;

pub const TERMINAL_SOURCE: &str = "alsa:rockchipes8388";
const PHYSICAL_INPUTS: [(&str, &str); 3] = [
    ("rockchipes8388", "端子音频（左右双声道）"),
    ("D6911codec", "HDMI 1 音频"),
    ("rockchiphdmiin", "HDMI 2 音频"),
];

pub fn capture_address(id: &str) -> Option<String> {
    PHYSICAL_INPUTS
        .iter()
        .find(|(card, _)| id == format!("alsa:{card}"))
        .map(|(card, _)| format!("hw:CARD={card},DEV=0"))
}

fn physical_inputs(cards: &str, pcm: &str) -> Vec<SourceOption> {
    PHYSICAL_INPUTS
        .iter()
        .filter_map(|(card, name)| {
            let index = cards.lines().find_map(|line| {
                let (head, tail) = line.split_once('[')?;
                let (id, _) = tail.split_once(']')?;
                (id.trim() == *card)
                    .then(|| head.trim().parse::<usize>().ok())
                    .flatten()
            })?;
            let captures = pcm.lines().any(|line| {
                let Some((device, description)) = line.split_once(':') else {
                    return false;
                };
                let Some((card_index, device_index)) = device.trim().split_once('-') else {
                    return false;
                };
                card_index.parse::<usize>().ok() == Some(index)
                    && device_index.parse::<usize>().ok() == Some(0)
                    && description.contains("capture")
            });
            captures.then(|| SourceOption {
                id: format!("alsa:{card}"),
                name: (*name).to_string(),
                kind: "alsa",
            })
        })
        .collect()
}

#[derive(Serialize)]
pub struct SourceOption {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
}

pub async fn available() -> anyhow::Result<Vec<SourceOption>> {
    let (cards, pcm) = tokio::join!(
        tokio::fs::read_to_string("/proc/asound/cards"),
        tokio::fs::read_to_string("/proc/asound/pcm")
    );
    Ok(physical_inputs(
        &cards.unwrap_or_default(),
        &pcm.unwrap_or_default(),
    ))
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
        if let Some(address) = capture_address(id) {
            super::MIXER
                .ensure_capture_source(id, &address, "alsa")
                .await?;
        } else {
            super::MIXER.ensure_shared_source(id);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "sources_test.rs"]
mod sources_test;
