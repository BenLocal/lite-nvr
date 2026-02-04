use ffmpeg_bus::{
    decoder::{Decoder, DecoderTask},
    frame::RawFrame,
    input::{AvInput, AvInputTask},
};

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
    decoder_task.start(decoder, task.subscribe()).await;
    let mut decoder_receiver = decoder_task.subscribe();
    tokio::spawn(async move {
        while let Ok(frame) = decoder_receiver.recv().await {
            if let RawFrame::Video(frame) = frame {
                println!(
                    "video frame: width: {}, height: {}, format: {:?}",
                    frame.width(),
                    frame.height(),
                    frame.format()
                );
            }
        }
    });

    task.start(input).await;
    tokio::signal::ctrl_c().await?;
    println!("ctrl+c received");
    task.get_cancel().cancel();

    Ok(())
}
