use std::time::Duration;

use crate::config::TrackSelect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Video,
    Audio,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Selected {
    pub video: Option<usize>,
    pub audio: Option<usize>,
}

/// Resolve the input stream indices to record, honoring the track selection.
/// Errors only when a specifically requested single kind is absent (or when
/// `Both` finds neither video nor audio).
pub(crate) fn select_streams(
    streams: impl IntoIterator<Item = (usize, MediaKind)>,
    tracks: TrackSelect,
) -> anyhow::Result<Selected> {
    let mut video = None;
    let mut audio = None;
    for (idx, kind) in streams {
        match kind {
            MediaKind::Video if video.is_none() => video = Some(idx),
            MediaKind::Audio if audio.is_none() => audio = Some(idx),
            _ => {}
        }
    }
    let want_v = matches!(tracks, TrackSelect::Video | TrackSelect::Both);
    let want_a = matches!(tracks, TrackSelect::Audio | TrackSelect::Both);
    let sel = Selected {
        video: if want_v { video } else { None },
        audio: if want_a { audio } else { None },
    };
    match tracks {
        TrackSelect::Video if sel.video.is_none() => {
            anyhow::bail!("no video stream in source")
        }
        TrackSelect::Audio if sel.audio.is_none() => {
            anyhow::bail!("no audio stream in source")
        }
        TrackSelect::Both if sel.video.is_none() && sel.audio.is_none() => {
            anyhow::bail!("source has neither video nor audio")
        }
        _ => {}
    }
    Ok(sel)
}

/// Exponential backoff: attempt 0 -> base, doubling, capped at max.
pub(crate) fn backoff_delay(attempt: u32, base: Duration, max: Duration) -> Duration {
    let factor = 1u128.checked_shl(attempt).unwrap_or(u128::MAX);
    let ms = base.as_millis().saturating_mul(factor).min(max.as_millis());
    Duration::from_millis(ms as u64)
}

#[cfg(test)]
#[path = "recorder_test.rs"]
mod recorder_test;
