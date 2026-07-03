//! nvr-compositor — composite several live sources into ONE video.
//!
//! Every source is decoded hot; the latest frame of each is held. On a fixed
//! output clock, a libavfilter graph (per-source `scale` → chained `overlay`
//! onto a black canvas) fuses them into one frame, which a single persistent
//! encoder + muxer publishes to ZLM. Sources may have different sizes and frame
//! rates (hold-last-frame + CFR output). Layout is free-form rectangles
//! (`{x,y,w,h}` per region), with a grid helper — so mosaic, video wall, and
//! picture-in-picture are all just different region sets.
//!
//! Built on `ffmpeg-bus`'s public building blocks plus `ffmpeg-next`'s filter
//! graph; it does not modify ffmpeg-bus.

pub mod compositor;
pub mod layout;
pub mod source;

pub use compositor::{Compositor, CompositorConfig, Director, SourceFeed};
pub use layout::{Layout, Region};
pub use source::Source;
