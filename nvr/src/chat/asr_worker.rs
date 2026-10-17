//! One session's speech recognizer on its own thread: decoding is CPU-heavy
//! and must not stall the async runtime. Results go back to the session as
//! [`Event`]s tagged with the push-to-talk `epoch` they belong to.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use nvr_asr::{AsrEngine, AsrModels};
use tokio::sync::mpsc::UnboundedSender;

use super::session::Event;

enum Cmd {
    Audio(Vec<f32>),
    /// Drop any buffered speech and start tagging results with this epoch.
    Reset(u64),
    /// End of the utterance: emit the remaining text, then `Flushed`.
    Flush,
}

/// Stops the thread when dropped.
pub(super) struct AsrWorker {
    cmds: Sender<Cmd>,
}

impl AsrWorker {
    pub(super) fn spawn(
        models: Arc<AsrModels>,
        events: UnboundedSender<Event>,
    ) -> anyhow::Result<Self> {
        let engine = AsrEngine::new(models)?;
        let (cmds, rx) = channel();
        std::thread::Builder::new()
            .name("chat-asr".into())
            .spawn(move || {
                let mut engine = engine;
                let mut epoch = 0;
                while let Ok(cmd) = rx.recv() {
                    let out: Vec<Event> = match cmd {
                        Cmd::Audio(samples) => engine
                            .accept(&samples)
                            .into_iter()
                            .map(|t| Event::Asr { epoch, t })
                            .collect(),
                        Cmd::Reset(next) => {
                            engine.flush();
                            epoch = next;
                            continue;
                        }
                        Cmd::Flush => engine
                            .flush()
                            .into_iter()
                            .map(|t| Event::Asr { epoch, t })
                            .chain([Event::Flushed { epoch }])
                            .collect(),
                    };
                    for event in out {
                        // The session is gone.
                        if events.send(event).is_err() {
                            return;
                        }
                    }
                }
            })?;
        Ok(Self { cmds })
    }

    pub(super) fn audio(&self, samples: Vec<f32>) {
        let _ = self.cmds.send(Cmd::Audio(samples));
    }

    pub(super) fn reset(&self, epoch: u64) {
        let _ = self.cmds.send(Cmd::Reset(epoch));
    }

    pub(super) fn flush(&self) {
        let _ = self.cmds.send(Cmd::Flush);
    }
}
