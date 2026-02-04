use ffmpeg_bus::{
    decoder::{Decoder, DecoderTask},
    encoder::{Encoder, EncoderTask, Settings},
    frame::RawFrame,
    input::{AvInput, AvInputTask},
};
use ffmpeg_next::{Rational, format::Pixel};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let input = AvInput::new("scripts/test.mp4", None).unwrap();
    let streams = input.streams();
    for (index, stream) in streams.iter() {
        println!(
            "stream: index: {}, id: {:?}",
            index,
            stream.parameters().id()
        );
    }
    let task = AvInputTask::new();

    // let mut receiver = task.subscribe();
    // tokio::spawn(async move {
    //     while let Ok(packet) = receiver.recv().await {
    //         println!(
    //             "packet: index: {}, pts: {:?}, dts: {:?}, data len: {}",
    //             packet.index(),
    //             packet.pts(),
    //             packet.dts(),
    //             packet.size()
    //         );
    //     }
    // });

    //  decoder
    let decoder = Decoder::new(streams.get(&0).unwrap())?;
    let decoder_task = DecoderTask::new();
    //let mut decoder_receiver = decoder_task.subscribe();
    // tokio::spawn(async move {
    //     while let Ok(frame) = decoder_receiver.recv().await {
    //         if let RawFrame::Video(frame) = frame {
    //             println!(
    //                 "video decode frame: width: {}, height: {}, format: {:?}",
    //                 frame.width(),
    //                 frame.height(),
    //                 frame.format()
    //             );
    //         }
    //     }
    // });

    let setting = Settings {
        width: 320,
        height: 240,
        keyframe_interval: 10,
        codec: Some("libx264".to_string()),
        pixel_format: Pixel::YUV420P,
    };
    let encoder = Encoder::new(streams.get(&0).unwrap(), setting, None)?;
    let encoder_task = EncoderTask::new();
    let mut encoder_receiver = encoder_task.subscribe();
    tokio::spawn(async move {
        while let Ok(packet) = encoder_receiver.recv().await {
            println!(
                "video encode packet: pts: {:?}, dts: {:?}, size: {:?}",
                packet.pts(),
                packet.dts(),
                packet.size()
            );
        }
    });

    encoder_task.start(encoder, decoder_task.subscribe()).await;
    decoder_task.start(decoder, task.subscribe()).await;
    task.start(input).await;
    tokio::signal::ctrl_c().await?;
    println!("ctrl+c received");
    task.get_cancel().cancel();

    Ok(())
}
