use std::sync::Arc;

use bytes::Bytes;
use ffmpeg_next::Rational;

pub type RawPacketSender = tokio::sync::broadcast::Sender<RawPacketCmd>;
pub type RawPacketReceiver = tokio::sync::broadcast::Receiver<RawPacketCmd>;

#[derive(Clone)]
pub enum RawPacketCmd {
    Data(RawPacket),
    EOF,
}

#[derive(Clone)]
pub struct RawPacket {
    packet: Arc<ffmpeg_next::codec::packet::Packet>,
    time_base: Rational,
}

impl RawPacket {
    pub fn pts(&self) -> Option<i64> {
        self.packet.pts()
    }

    pub fn dts(&self) -> Option<i64> {
        self.packet.dts()
    }

    pub fn size(&self) -> usize {
        self.packet.size()
    }

    pub fn index(&self) -> usize {
        self.packet.stream()
    }

    pub fn data(&self) -> Bytes {
        self.packet
            .data()
            .map(Bytes::copy_from_slice)
            .unwrap_or_default()
    }

    pub fn is_key(&self) -> bool {
        self.packet.is_key()
    }

    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    pub fn set_duration(&mut self, duration: i64) -> anyhow::Result<()> {
        self.get_mut()?.set_duration(duration);
        Ok(())
    }

    /// Detach the header while retaining the payload buffer and its destructor.
    /// SDK Packet::clone also makes the payload writable, which invalidates
    /// nested frame ownership for WRAPPED_AVFRAME packets.
    pub fn get_mut(&mut self) -> anyhow::Result<&mut ffmpeg_next::codec::packet::Packet> {
        if Arc::get_mut(&mut self.packet).is_none() {
            self.packet = Arc::new(self.referenced_packet()?);
        }
        Arc::get_mut(&mut self.packet)
            .ok_or_else(|| anyhow::anyhow!("packet header is still shared"))
    }

    pub(crate) fn referenced_packet(&self) -> anyhow::Result<ffmpeg_next::codec::packet::Packet> {
        use ffmpeg_next::codec::packet::{Mut, Ref};
        let mut packet = ffmpeg_next::Packet::empty();
        // SAFETY: both AVPacket headers are valid; the source is kept alive
        // during the call. av_packet_ref retains its buffer, including the
        // custom destructor used by wrapped frames, without copying bytes.
        let result =
            unsafe { ffmpeg_next::ffi::av_packet_ref(packet.as_mut_ptr(), self.packet.as_ptr()) };
        if result < 0 {
            return Err(ffmpeg_next::Error::from(result).into());
        }
        Ok(packet)
    }

    /// Get a reference to the inner packet (for BSF and other FFmpeg operations).
    pub fn packet(&self) -> &ffmpeg_next::codec::packet::Packet {
        &self.packet
    }
}

impl From<(ffmpeg_next::codec::packet::Packet, Rational)> for RawPacket {
    fn from((packet, time_base): (ffmpeg_next::codec::packet::Packet, Rational)) -> Self {
        Self {
            packet: Arc::new(packet),
            time_base,
        }
    }
}

#[cfg(test)]
#[path = "packet_test.rs"]
mod packet_test;
