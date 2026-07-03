//! Live-stream liveness check backed by ZLM's own media-source registry. A
//! stream key `(app, stream)` is live iff ZLM currently has a matching source
//! registered (publishing). Queried on demand via `MediaSource::for_each`, so
//! there is no local state to keep in sync with `on_media_changed` events.

use rszlm::obj::{MediaSource, Track};
use serde::Serialize;

#[derive(Clone, Copy, Default)]
pub struct MediaCache;

/// A snapshot of one registered source, extracted from a `MediaSource` while it
/// is valid (i.e. inside `for_each`), so it can be handed out and serialized.
#[derive(Debug, Clone, Serialize)]
pub struct MediaInfo {
    pub schema: String,
    pub app: String,
    pub stream: String,
    pub reader_count: i32,
    pub total_reader_count: i32,
    pub tracks: Vec<TrackInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackInfo {
    pub codec_id: i32,
    pub codec_name: String,
    pub bit_rate: i32,
    pub is_video: bool,
    // Video-only (0 for audio tracks).
    pub width: i32,
    pub height: i32,
    pub fps: i32,
    // Audio-only (0 for video tracks).
    pub sample_rate: i32,
    pub channels: i32,
    pub sample_bit: i32,
}

impl TrackInfo {
    fn from_track(track: &Track) -> Self {
        let is_video = track.is_video();
        let (width, height, fps) = if is_video {
            (track.video_width(), track.video_height(), track.video_fps())
        } else {
            (0, 0, 0)
        };
        let (sample_rate, channels, sample_bit) = if is_video {
            (0, 0, 0)
        } else {
            (
                track.audio_sample_rate(),
                track.audio_channel(),
                track.audio_sample_bit(),
            )
        };
        Self {
            codec_id: track.get_codec_id(),
            codec_name: track.get_codec_name(),
            bit_rate: track.get_bit_rate(),
            is_video,
            width,
            height,
            fps,
            sample_rate,
            channels,
            sample_bit,
        }
    }
}

impl MediaCache {
    /// Whether ZLM currently has a source registered for `(app, stream)`.
    /// Schema/vhost agnostic — matches on app+stream like the old cache key.
    pub fn is_live(&self, app: &str, stream: &str) -> bool {
        let mut found = false;
        MediaSource::for_each(
            |_src| found = true,
            None, // any schema
            None, // any vhost
            Some(app),
            Some(stream),
        );
        found
    }

    /// Media info (reader counts + tracks) of the source registered for
    /// `(app, stream)`, or `None` if nothing is publishing. Returns the first
    /// match if the stream is registered under several schemas.
    // Reserved for a future consumer (e.g. a `GET /gb/streams/{id}/info` API);
    // no caller yet.
    #[allow(dead_code)]
    pub fn media_info(&self, app: &str, stream: &str) -> Option<MediaInfo> {
        let mut info: Option<MediaInfo> = None;
        MediaSource::for_each(
            |src| {
                if info.is_some() {
                    return;
                }
                let tracks = (0..src.track_count())
                    .filter_map(|i| src.get_track(i))
                    .map(|track| TrackInfo::from_track(&track))
                    .collect();
                info = Some(MediaInfo {
                    schema: src.schema(),
                    app: src.app(),
                    stream: src.stream(),
                    reader_count: src.reader_count(),
                    total_reader_count: src.total_reader_count(),
                    tracks,
                });
            },
            None, // any schema
            None, // any vhost
            Some(app),
            Some(stream),
        );
        info
    }
}
