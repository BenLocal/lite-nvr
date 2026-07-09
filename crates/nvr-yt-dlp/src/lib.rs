//! nvr-yt-dlp — resolve platform live/VOD page URLs into playable stream URLs.
//!
//! Platforms like Douyin/Bilibili/Twitch don't hand out stable pull addresses:
//! their CDN URLs are temporary and signed, expire/rotate, and must be
//! re-extracted from the room page every time. This crate shells out to the
//! external, community-maintained `yt-dlp` binary to do that extraction, so a
//! stored room URL can be re-resolved into a fresh stream URL at pipe start
//! *and on every reconnect* — never persist the resolved URL.

mod resolve;

pub use resolve::{ResolvedStream, YT_DLP_BIN_ENV, YtDlp, YtDlpError};
