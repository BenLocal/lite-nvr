//! nvr-switcher — a seamless director / vision-mixer.
//!
//! Many input sources are decoded in parallel and kept hot; a selector feeds
//! only the *active* source's frames into ONE persistent encoder + muxer that
//! is published to ZLM. Because the encoder and muxer never restart and the
//! output runs on its own continuous CFR timeline, switching the program from
//! source A to source B does not unpublish/republish the ZLM stream — the
//! player keeps playing without interruption. Each switch forces an IDR so the
//! decoder re-syncs immediately.
//!
//! Built entirely on `ffmpeg-bus`'s public building blocks (`input`, `decoder`,
//! `encoder`, `scaler`, `output`); it does not modify that crate.

pub mod program;
pub mod source;
pub mod switcher;

pub use program::ProgramConfig;
pub use switcher::Switcher;
