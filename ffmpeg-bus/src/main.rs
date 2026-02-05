use ffmpeg_bus::bus::{Bus, InputConfig, OutputConfig, OutputDest};
use futures_util::stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bus = Bus::new("a");

    let input_config = InputConfig::File {
        path: "scripts/test.mp4".to_string(),
    };
    bus.add_input(input_config).await?;
    let output_config = OutputConfig {
        id: "a".to_string(),
        dest: OutputDest::Raw,
        encode: None,
    };
    let mut stream = bus.add_output(output_config).await?;
    tokio::spawn(async move {
        while let Some(frame) = stream.next().await {
            match frame {
                Some(frame) => {
                    println!("frame: {}", frame.to_string());
                }
                None => break,
            }
        }
        println!("end of output stream");
    });

    println!("start bus");
    tokio::signal::ctrl_c().await?;
    println!("ctrl+c received");
    bus.stop();
    println!("bus stopped");
    Ok(())
}
