use ffmpeg_next::codec::Id;
use rszlm::obj::CodecId;

use super::{video_payload, zlm_codec_id};

fn test_media(id: &str) -> std::sync::Arc<rszlm::media::Media> {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        rszlm::init::EnvInitBuilder::default()
            .log_level(0)
            .log_mask(0)
            .thread_num(2)
            .build();
    });
    std::sync::Arc::new(rszlm::media::Media::new_with_default_vhost(
        "regression",
        id,
        0.0,
        false,
        false,
    ))
}

#[tokio::test]
async fn regression_missing_audio_first_packet_does_not_block_video() {
    use super::*;
    use ffmpeg_bus::frame::VideoFrame;
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

    let media = test_media("missing-audio-first-packet");
    let registered = Arc::new(AtomicBool::new(false));
    let registration = registered.clone();
    media.on_regist(move |source, is_registered| {
        if is_registered && source.schema() == "rtsp" {
            registration.store(true, Ordering::Release);
        }
    });
    let coordinator = ZlmTrackCoordinator::new(media.clone(), 2);
    let mut parameters = ffmpeg_next::codec::Parameters::new();
    // SAFETY: this test owns the parameters and initializes its codec metadata.
    unsafe {
        (*parameters.as_mut_ptr()).codec_type = ffmpeg_next::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
        (*parameters.as_mut_ptr()).codec_id = ffmpeg_next::ffi::AVCodecID::AV_CODEC_ID_H264;
    }
    let av = AvStream::new(0, parameters, (1, 1000).into(), (25, 1).into());
    let progressed = Arc::new(AtomicBool::new(false));
    let progress = progressed.clone();
    let video = futures::stream::iter([0, 40].map(|pts| {
        Some(VideoFrame {
            // Real SPS/PPS/IDR lets the test observe RTSP registration too.
            data: include_bytes!("testdata/black.h264").to_vec().into(),
            width: 16,
            height: 16,
            format: 0,
            pts,
            dts: pts,
            is_key: true,
            codec_id: 0,
        })
    }))
    .chain(futures::stream::poll_fn(move |_| {
        progress.store(true, Ordering::Release);
        std::task::Poll::Pending
    }));
    let handle = tokio::spawn(forward_raw_packet_stream_to_zlm(
        Box::pin(video),
        av,
        media,
        Some(coordinator.clone()),
        OutputAvType::Video,
    ));
    let result = tokio::time::timeout(Duration::from_secs(4), async {
        while !progressed.load(Ordering::Acquire) || !registered.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    handle.abort();
    let _ = handle.await;
    assert!(
        result.is_ok(),
        "video did not publish: progressed={}, registered={}, completed={}",
        progressed.load(Ordering::Acquire),
        registered.load(Ordering::Acquire),
        coordinator.state.lock().unwrap().completed,
    );
    assert!(coordinator.state.lock().unwrap().completed);
    assert!(
        coordinator
            .register_track(Track::new(
                CodecId::G711A,
                Some(CodecArgs::Audio(AudioCodecArgs {
                    sample_rate: 8000,
                    channels: 1
                })),
            ))
            .is_none(),
        "a late audio track must not change the published track set"
    );
    assert_eq!(coordinator.state.lock().unwrap().registered, 1);
}

#[tokio::test]
async fn ready_tracks_and_rejected_outputs_release_the_barrier_without_timeout() {
    use super::*;
    use std::time::Duration;

    for reject_second in [false, true] {
        let media = test_media(if reject_second {
            "rejected-track"
        } else {
            "ready-tracks"
        });
        let coordinator = ZlmTrackCoordinator::new(media, 2);
        let mut video_rx = coordinator
            .register_track(Track::new(
                CodecId::JPEG,
                Some(CodecArgs::Video(VideoCodecArgs {
                    width: 16,
                    height: 16,
                    fps: 25.0,
                })),
            ))
            .unwrap();
        assert!(!*video_rx.borrow());
        let audio_rx = if reject_second {
            coordinator.expect_one_less();
            None
        } else {
            coordinator.register_track(Track::new(
                CodecId::G711A,
                Some(CodecArgs::Audio(AudioCodecArgs {
                    sample_rate: 8000,
                    channels: 1,
                })),
            ))
        };
        tokio::time::timeout(
            Duration::from_millis(200),
            coordinator.wait_complete(&mut video_rx),
        )
        .await
        .unwrap();
        if let Some(mut rx) = audio_rx {
            tokio::time::timeout(
                Duration::from_millis(200),
                coordinator.wait_complete(&mut rx),
            )
            .await
            .unwrap();
        }
        assert!(coordinator.state.lock().unwrap().completed);
    }
}

#[test]
fn test_zlm_codec_id_maps_camera_codecs() {
    assert!(matches!(zlm_codec_id(Id::H264), Some(CodecId::H264)));
    assert!(matches!(zlm_codec_id(Id::HEVC), Some(CodecId::H265)));
    assert!(matches!(zlm_codec_id(Id::AAC), Some(CodecId::AAC)));
    assert!(matches!(zlm_codec_id(Id::PCM_ALAW), Some(CodecId::G711A)));
    assert!(matches!(zlm_codec_id(Id::PCM_MULAW), Some(CodecId::G711U)));
    assert!(matches!(zlm_codec_id(Id::OPUS), Some(CodecId::Opus)));
}

#[test]
fn test_zlm_codec_id_rejects_unsupported() {
    assert!(zlm_codec_id(Id::ADPCM_G726).is_none());
    assert!(zlm_codec_id(Id::MP3).is_none());
}

#[test]
fn test_video_payload_annexb_keyframe_gets_parameter_sets() {
    let annexb = [0, 0, 0, 1, 0x65, 0x88];
    let out = video_payload(&annexb, true, false, Some(&[0, 0, 0, 1, 0x67]));
    assert_eq!(out.as_ref(), &[0, 0, 0, 1, 0x67, 0, 0, 0, 1, 0x65, 0x88]);
    assert_eq!(
        video_payload(&annexb, false, false, Some(&[0, 0, 0, 1, 0x67])).as_ref(),
        &annexb
    );
}

#[test]
fn test_video_payload_avcc_keyframe_gets_parameter_sets() {
    let avcc = [0, 0, 0, 2, 0x65, 0x88];
    let sets = [0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68];
    let key = video_payload(&avcc, true, true, Some(&sets));
    assert_eq!(
        key.as_ref(),
        &[
            0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68, 0, 0, 0, 1, 0x65, 0x88
        ]
    );
    let delta = video_payload(&[0, 0, 0, 2, 0x41, 0x9a], false, true, Some(&sets));
    assert_eq!(
        delta.as_ref(),
        &[0, 0, 0, 1, 0x41, 0x9a],
        "non-key: no sets"
    );
}
