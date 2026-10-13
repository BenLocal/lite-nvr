// device_test is a child module of `device` (this file), so `super` == device.
use super::{DetectConfig, DeviceConfig, DeviceInfo};

#[test]
fn device_without_config_deserializes_to_default() {
    // A row written before this field exists must still load, with detection off.
    let json = r#"{
        "id":"cam1","name":"Cam 1","input_type":"rtsp","input_value":"rtsp://x",
        "description":"","include_audio":false,"record":true,
        "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"
    }"#;
    let dev: DeviceInfo = serde_json::from_str(json).expect("legacy row must parse");
    assert!(dev.config.detect.is_none());
}

#[test]
fn detect_config_round_trips() {
    let cfg = DeviceConfig {
        detect: Some(DetectConfig {
            enabled: true,
            models: vec!["yolov8n".to_string()],
            sample_every_ms: 500,
            min_confidence: 0.4,
        }),
        ..DeviceConfig::default()
    };
    let s = serde_json::to_string(&cfg).unwrap();
    let back: DeviceConfig = serde_json::from_str(&s).unwrap();
    let d = back.detect.expect("detect present");
    assert!(d.enabled);
    assert_eq!(d.models, vec!["yolov8n".to_string()]);
    assert_eq!(d.sample_every_ms, 500);
    assert!((d.min_confidence - 0.4).abs() < 1e-6);
}

#[tokio::test]
async fn output_settings_survive_database_reopen() -> anyhow::Result<()> {
    use super::{AudioCodec, AudioMixMode, OutputConfig, VideoCodec};
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("devices.db");
    let db = turso::Builder::new_local(path.to_str().unwrap())
        .build()
        .await?;
    let conn = db.connect()?;
    conn.execute_batch(include_str!("../migrations/20260210_init.sql"))
        .await?;
    let device = DeviceInfo {
        id: "output-test".into(),
        name: "Output test".into(),
        input_type: "rtsp".into(),
        input_value: "rtsp://localhost/test".into(),
        description: String::new(),
        include_audio: true,
        record: false,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        config: DeviceConfig {
            output: Some(OutputConfig {
                use_global_audio: Some(false),
                audio_bus_id: Some("main".into()),
                audio_mode: Some(AudioMixMode::AddOriginal),
                video_codec: Some(VideoCodec::H265),
                width: Some(640),
                height: Some(360),
                fps: Some(15),
                max_bitrate: Some(500_000),
                audio_codec: Some(AudioCodec::Opus),
                sample_rate: Some(24000),
            }),
            ..DeviceConfig::default()
        },
    };
    super::upsert(&device, &conn).await?;
    drop(conn);
    drop(db);
    let db = turso::Builder::new_local(path.to_str().unwrap())
        .build()
        .await?;
    let restored = super::get(&device.id, &db.connect()?).await?.unwrap();
    assert_eq!(restored.config.output, device.config.output);
    Ok(())
}
