use ffmpeg_bus::bus::{Bus, InputConfig, OutputConfig, OutputDest};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bus = Bus::new("a");

    let input_config = InputConfig::File {
        path: "scripts/test.mp4".to_string(),
    };
    bus.add_input(input_config).await?;

    // 直接写文件：使用 AvOutput 写真实路径，生成标准 MP4，任何播放器都能打开
    let output_config = OutputConfig {
        id: "mux_file".to_string(),
        dest: OutputDest::File {
            path: "output.mp4".to_string(),
        },
        encode: None,
    };
    let _stream = bus.add_output(output_config).await?;

    println!("start bus, writing to output.mp4 (wait for input EOF or Ctrl+C)");
    tokio::signal::ctrl_c().await?;
    println!("ctrl+c received");
    bus.stop();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    println!("bus stopped");
    Ok(())
}
