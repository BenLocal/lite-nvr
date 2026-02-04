use ffmpeg_bus::{
    decoder::{Decoder, DecoderTask},
    encoder::{Encoder, EncoderTask, Settings},
    input::{AvInput, AvInputTask},
    output::AvOutput,
    packet::RawPacketCmd,
};
use ffmpeg_next::format::Pixel;

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
    //         match packet {
    //             RawPacketCmd::Data(p) => {
    //                 println!(
    //                     "packet: index: {}, pts: {:?}, dts: {:?}, data len: {}",
    //                     p.index(),
    //                     p.pts(),
    //                     p.dts(),
    //                     p.size()
    //                 );
    //             }
    //             RawPacketCmd::EOF => break,
    //         }
    //     }
    // });

    //  decoder
    let decoder = Decoder::new(streams.get(&0).unwrap())?;
    let decoder_task = DecoderTask::new();
    // let mut decoder_receiver = decoder_task.subscribe();
    // tokio::spawn(async move {
    //     while let Ok(frame) = decoder_receiver.recv().await {
    //         match frame {
    //             RawFrameCmd::Data(frame) => {
    //                 if let RawFrame::Video(frame) = frame {
    //                     println!(
    //                         "video decode frame: width: {}, height: {}, format: {:?}",
    //                         frame.width(),
    //                         frame.height(),
    //                         frame.format()
    //                     );
    //                 }
    //             }
    //             RawFrameCmd::EOF => break,
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

    let mut output = AvOutput::new("output.mp4", None, None)?;
    output.add_stream(streams.get(&0).unwrap())?;
    let mut encoder_receiver = encoder_task.subscribe();
    tokio::spawn(async move {
        while let Ok(packet) = encoder_receiver.recv().await {
            match packet {
                RawPacketCmd::Data(packet) => {
                    println!(
                        "video encode packet: pts: {:?}, dts: {:?}, size: {:?}",
                        packet.pts(),
                        packet.dts(),
                        packet.size()
                    );
                    let _ = output.write_packet(0, packet);
                }
                RawPacketCmd::EOF => break,
            }
        }

        println!("finish output");
        output.finish().unwrap();
    });

    encoder_task.start(encoder, decoder_task.subscribe()).await;
    decoder_task.start(decoder, task.subscribe()).await;
    task.start(input).await;

    println!("start");
    tokio::signal::ctrl_c().await?;
    task.stop();
    decoder_task.stop();
    encoder_task.stop();
    println!("ctrl+c received");

    Ok(())
}
