use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use ffmpeg_next::{ChannelLayout, filter, format::Sample, frame::Audio, util::error};
use tokio_util::sync::CancellationToken;

use crate::frame::{RawAudioFrame, RawFrame, RawFrameCmd, RawFrameReceiver, RawFrameSender};

/// 内部状态，由 Read/Write 通过 `Arc<Mutex<>>` 共享。
struct DynamicMixerInner {
    graph: filter::Graph,
    sample_rate: u32,
    sample_fmt: Sample,
    layout: ChannelLayout,
    max_inputs: usize,
}

unsafe impl Send for DynamicMixerInner {}
unsafe impl Sync for DynamicMixerInner {}

pub struct DynamicMixer {
    inner: Arc<Mutex<DynamicMixerInner>>,
}

impl DynamicMixer {
    pub fn new(max_inputs: usize, sample_rate: u32) -> anyhow::Result<Self> {
        let mut graph = filter::Graph::new();
        let layout = ChannelLayout::STEREO;
        let sample_fmt = Sample::I16(ffmpeg_next::format::sample::Type::Packed);

        for i in 0..max_inputs {
            let name = format!("in_{}", i);
            let args = format!(
                "time_base=1/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
                sample_rate, sample_rate, "s16", "stereo"
            );
            graph.add(&filter::find("abuffer").unwrap(), &name, &args)?;
        }

        let amix_args = format!("inputs={}:duration=longest", max_inputs);
        graph.add(&filter::find("amix").unwrap(), "mixer", &amix_args)?;

        for i in 0..max_inputs {
            let mut src = graph.get(&format!("in_{}", i)).unwrap();
            let mut mixer = graph.get("mixer").unwrap();
            src.link(0, &mut mixer, i as u32);
        }

        graph.add(&filter::find("abuffersink").unwrap(), "out", "")?;
        let mut mixer = graph.get("mixer").unwrap();
        let mut sink = graph.get("out").unwrap();
        mixer.link(0, &mut sink, 0);

        graph.validate()?;

        Ok(Self {
            inner: Arc::new(Mutex::new(DynamicMixerInner {
                graph,
                sample_rate,
                sample_fmt,
                layout,
                max_inputs,
            })),
        })
    }

    /// 拆成只读端（拉取混音结果）和只写端（推送输入）。两端可分别在不同线程/任务使用。
    pub fn split(self) -> (DynamicMixerRead, DynamicMixerWrite) {
        let read = DynamicMixerRead {
            inner: Arc::clone(&self.inner),
        };
        let write = DynamicMixerWrite { inner: self.inner };
        (read, write)
    }

    /// 兼容：不 split 时也可直接拉帧（与写端不能并发）。
    pub fn pull_frame(&self) -> anyhow::Result<Option<Audio>> {
        let mut guard = self.inner.lock().unwrap();
        Self::pull_frame_inner(&mut guard.graph)
    }

    fn pull_frame_inner(graph: &mut filter::Graph) -> anyhow::Result<Option<Audio>> {
        let mut out = Audio::empty();
        let mut out_ctx = graph.get("out").unwrap();
        let mut sink = out_ctx.sink();
        match sink.frame(&mut out) {
            Ok(()) => Ok(Some(out)),
            Err(ffmpeg_next::Error::Eof) => Ok(None),
            Err(ffmpeg_next::Error::Other { errno }) if errno == error::EAGAIN => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// 混音器只读端：仅可拉取混音后的音频帧。
#[derive(Clone)]
pub struct DynamicMixerRead {
    inner: Arc<Mutex<DynamicMixerInner>>,
}

impl DynamicMixerRead {
    /// 从混音器输出端拉取一帧。无数据时返回 `Ok(None)`（EAGAIN/EOF）。
    pub fn pull_frame(&self) -> anyhow::Result<Option<Audio>> {
        let mut guard = self.inner.lock().unwrap();
        DynamicMixer::pull_frame_inner(&mut guard.graph)
    }
}

/// 混音器只写端：仅可向各 slot 推送音频或静音。
#[derive(Clone)]
pub struct DynamicMixerWrite {
    inner: Arc<Mutex<DynamicMixerInner>>,
}

impl DynamicMixerWrite {
    pub fn push_audio(
        &self,
        slot_idx: usize,
        frame: &ffmpeg_next::frame::Audio,
    ) -> anyhow::Result<()> {
        let name = format!("in_{}", slot_idx);
        let mut guard = self.inner.lock().unwrap();
        let mut source = guard.graph.get(&name).unwrap();
        source.source().add(frame)?;
        Ok(())
    }

    pub fn push_silence(
        &self,
        slot_idx: usize,
        samples_count: usize,
        pts: i64,
    ) -> anyhow::Result<()> {
        let mut guard = self.inner.lock().unwrap();
        let mut silence_frame = Audio::new(guard.sample_fmt, samples_count, guard.layout);
        silence_frame.set_rate(guard.sample_rate);
        silence_frame.set_pts(Some(pts));

        for plane in 0..silence_frame.planes() {
            let data = silence_frame.data_mut(plane);
            for byte in data {
                *byte = 0;
            }
        }

        let name = format!("in_{}", slot_idx);
        let mut source = guard.graph.get(&name).unwrap();
        source.source().add(&silence_frame)?;
        Ok(())
    }
}

pub enum DynamicMixerCmd {
    AddInput {
        slot_idx: usize,
        receiver: RawFrameReceiver,
    },
    RemoveInput {
        slot_idx: usize,
    },
}

pub struct DynamicMixerTask {
    cancel: CancellationToken,
    raw_chan: RawFrameSender,
    _sender: Option<tokio::sync::mpsc::Sender<DynamicMixerCmd>>,
}

impl DynamicMixerTask {
    pub fn new() -> Self {
        let cancel = CancellationToken::new();
        let (sender, _) = tokio::sync::broadcast::channel(1024);
        Self {
            cancel,
            raw_chan: sender,
            _sender: None,
        }
    }

    pub async fn add_input(
        &self,
        slot_idx: usize,
        receiver: RawFrameReceiver,
    ) -> anyhow::Result<()> {
        if let Some(sender) = &self._sender {
            sender
                .send(DynamicMixerCmd::AddInput { slot_idx, receiver })
                .await?;
            return Ok(());
        }

        Err(anyhow::anyhow!("audio dynamic mixer task not started"))
    }

    pub async fn remove_input(&self, slot_idx: usize) -> anyhow::Result<()> {
        if let Some(sender) = &self._sender {
            sender
                .send(DynamicMixerCmd::RemoveInput { slot_idx })
                .await?;
            return Ok(());
        }

        Err(anyhow::anyhow!("audio dynamic mixer task not started"))
    }

    /// 订阅混音输出流。
    pub fn subscribe(&self) -> RawFrameReceiver {
        self.raw_chan.subscribe()
    }

    /// 停止任务（取消内部循环）。
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub async fn start(&mut self, mixer: DynamicMixer) -> anyhow::Result<()> {
        let cancel_clone = self.cancel.clone();
        let sender_clone = self.raw_chan.clone();
        let (read, write) = mixer.split();

        let (cmd_sender, mut cmd_receiver) = tokio::sync::mpsc::channel::<DynamicMixerCmd>(1024);
        self._sender = Some(cmd_sender);

        let (input_tx, mut input_rx) = tokio::sync::mpsc::channel::<(usize, RawAudioFrame)>(1024);

        tokio::spawn(async move {
            let handle_cancel = cancel_clone.clone();
            let handle = tokio::task::spawn_blocking(move || {
                Self::mixer_output_loop_sync(read, handle_cancel, sender_clone)
            });

            let mut inputs = HashMap::new();
            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        break;
                    }
                    Some((slot_idx, frame)) = input_rx.recv() => {
                        let _ = write.push_audio(slot_idx, &frame.as_audio());
                        tokio::task::yield_now().await;
                    }
                    Some(cmd) = cmd_receiver.recv() => {
                        match cmd {
                            DynamicMixerCmd::AddInput { slot_idx, mut receiver  } => {
                                let cancel = CancellationToken::new();
                                let input_tx_clone = input_tx.clone();
                                let cancel_clone = cancel.clone();
                                tokio::spawn(async move {
                                    loop {
                                        tokio::select! {
                                            _ = cancel_clone.cancelled() => break,
                                            msg = receiver.recv() => match msg {
                                                Ok(RawFrameCmd::Data(RawFrame::Audio(frame))) => {
                                                    let _ = input_tx_clone.send((slot_idx, frame)).await;
                                                }
                                                Ok(_) => {}
                                                Err(_) => break,
                                            }
                                        }
                                    }
                                });
                                inputs.insert(slot_idx, cancel);
                            }
                            DynamicMixerCmd::RemoveInput { slot_idx } => {
                                if let Some(cancel) = inputs.remove(&slot_idx) {
                                    cancel.cancel();
                                }
                            }
                        }
                    }
                }
            }

            for (_, cancel) in inputs.iter_mut() {
                cancel.cancel();
            }

            let _ = handle.await;
            log::info!("audio dynamic mixer task finished");
        });

        Ok(())
    }

    pub fn mixer_output_loop_sync(
        read: DynamicMixerRead,
        cancel: CancellationToken,
        out: RawFrameSender,
    ) {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            let frame = match read.pull_frame() {
                Ok(Some(f)) => f,
                Ok(None) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(_) => break,
            };
            let _ = out.send(RawFrameCmd::Data(RawFrame::Audio(frame.into())));
        }
    }
}

#[cfg(test)]
#[path = "audio_mixer_test.rs"]
mod audio_mixer_test;
