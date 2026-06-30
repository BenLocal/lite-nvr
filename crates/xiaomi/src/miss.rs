//! Port of go2rtc `pkg/xiaomi/miss/client.go` — the MTP command/media layer on
//! top of the [`crate::cs2`] transport.
//!
//! After [`Client::connect`] (auth handshake), [`Client::start_media`] asks the
//! camera to stream and [`Client::read_packet`] yields decrypted media
//! [`Packet`]s (codec + payload + timestamp). Commands after auth are ChaCha20
//! encrypted with the shared key ([`crate::crypto`]).
//!
//! Only the `cs2` vendor is wired up (the `tutk` vendor + Dafang ICAM raw
//! commands are not ported yet).

use anyhow::{Result, bail};

use crate::crypto;
use crate::cs2;
use crate::device::MissConnection;

// MTP codec ids.
pub const CODEC_H264: u32 = 4;
pub const CODEC_H265: u32 = 5;
pub const CODEC_PCM: u32 = 1024;
pub const CODEC_PCMU: u32 = 1026;
pub const CODEC_PCMA: u32 = 1027;
pub const CODEC_OPUS: u32 = 1032;

// MTP command ids.
const CMD_AUTH_REQ: u32 = 0x100;
const CMD_VIDEO_START: u32 = 0x102;
const CMD_VIDEO_STOP: u32 = 0x103;
const CMD_AUDIO_START: u32 = 0x104;
const CMD_ENCODED: u32 = 0x1001;

const HDR_SIZE: usize = 32;

/// One demuxed media unit from the camera.
#[derive(Debug, Clone)]
pub struct Packet {
    pub codec_id: u32,
    pub sequence: u32,
    pub flags: u32,
    /// Milliseconds.
    pub timestamp: u64,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn sample_rate(&self) -> u32 {
        if (self.flags >> 3) & 0b1111 != 0 {
            16000
        } else {
            8000
        }
    }
}

pub struct Client {
    conn: cs2::Conn,
    key: [u8; 32],
    model: String,
}

impl Client {
    /// Connect to a camera resolved by [`crate::device::resolve_miss`]. `host` is
    /// the camera's address (its local IP from the device list); `transport` is
    /// "" | "udp" | "tcp".
    pub fn connect(host: &str, miss: &MissConnection, transport: &str) -> Result<Client> {
        let key = crypto::calc_shared_key(&miss.device_public, &hex::encode(&miss.client_private))?;

        let conn = match miss.vendor.as_str() {
            "cs2" => cs2::dial(host, transport)?,
            other => bail!("miss: unsupported vendor {other} (only cs2 ported)"),
        };

        let client = Client {
            conn,
            key,
            model: miss.model.clone(),
        };
        client.login(&hex::encode(&miss.client_public), &miss.sign)?;
        Ok(client)
    }

    fn login(&self, client_public_hex: &str, sign: &str) -> Result<()> {
        let s = format!(
            r#"{{"public_key":"{client_public_hex}","sign":"{sign}","uuid":"","support_encrypt":0}}"#
        );
        self.conn.write_command(CMD_AUTH_REQ, s.as_bytes())?;
        let (_, data) = self.conn.read_command()?;
        if !contains(&data, br#""result":"success""#) {
            bail!("miss: auth failed: {}", String::from_utf8_lossy(&data));
        }
        Ok(())
    }

    pub fn protocol(&self) -> &'static str {
        self.conn.protocol()
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Encrypt and send a command (Client.WriteCommand → cmdEncoded).
    fn write_encrypted(&self, data: &[u8]) -> Result<()> {
        let enc = crypto::encode(data, &self.key);
        self.conn.write_command(CMD_ENCODED, &enc)
    }

    /// Ask the camera to start streaming (generic, non-Dafang models).
    /// `quality`: "" | "auto" | "sd" | "hd"; `audio`: "" | "0" | "1".
    pub fn start_media(&self, channel: &str, quality: &str, audio: &str) -> Result<()> {
        if matches!(self.model.as_str(), "isa.camera.df3" | "isa.camera.isc5c1") {
            bail!("miss: Dafang/Xiaofang ICAM start_media not ported");
        }

        let quality = match quality {
            "" | "hd" => match self.model.as_str() {
                "chuangmi.camera.046c04" | "chuangmi.camera.72ac1" => "3",
                _ => "2",
            },
            "sd" => "1",
            "auto" => "0",
            other => other,
        };
        let audio = if audio.is_empty() { "1" } else { audio };

        let mut data = CMD_VIDEO_START.to_be_bytes().to_vec();
        let json = match channel {
            "" | "0" => format!(r#"{{"videoquality":{quality},"enableaudio":{audio}}}"#),
            _ => {
                format!(r#"{{"videoquality":-1,"videoquality2":{quality},"enableaudio":{audio}}}"#)
            }
        };
        data.extend_from_slice(json.as_bytes());
        self.write_encrypted(&data)
    }

    pub fn stop_media(&self) -> Result<()> {
        self.write_encrypted(&CMD_VIDEO_STOP.to_be_bytes())
    }

    #[allow(dead_code)]
    pub fn start_audio(&self) -> Result<()> {
        self.write_encrypted(&CMD_AUDIO_START.to_be_bytes())
    }

    /// Read one decrypted media packet.
    pub fn read_packet(&self) -> Result<Packet> {
        let (hdr, payload) = self.conn.read_packet()?;
        if hdr.len() < HDR_SIZE {
            bail!("miss: packet header too small");
        }
        let payload = crypto::decode(&payload, &self.key);

        let codec_id = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
        let sequence = u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]);
        let flags = u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]);
        // Dafang/Xiaofang/LoockV2 timestamps are unreliable; the integration may
        // substitute a local clock. We pass through the header timestamp.
        let timestamp = u64::from_le_bytes([
            hdr[16], hdr[17], hdr[18], hdr[19], hdr[20], hdr[21], hdr[22], hdr[23],
        ]);

        Ok(Packet {
            codec_id,
            sequence,
            flags,
            timestamp,
            payload,
        })
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rate_from_flags() {
        let p = |flags| Packet {
            codec_id: CODEC_PCMA,
            sequence: 0,
            flags,
            timestamp: 0,
            payload: vec![],
        };
        assert_eq!(p(0).sample_rate(), 8000);
        assert_eq!(p(0b1000).sample_rate(), 16000); // (flags >> 3) & 0xf != 0
    }

    #[test]
    fn contains_subslice() {
        assert!(contains(
            br#"{"result":"success"}"#,
            br#""result":"success""#
        ));
        assert!(!contains(br#"{"result":"fail"}"#, br#""result":"success""#));
    }
}
