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

    pub fn set_duration(&mut self, duration: i64) {
        if let Some(p) = Arc::get_mut(&mut self.packet) {
            p.set_duration(duration);
        } else {
            // If Arc is shared, we clone (make_mut)
            Arc::make_mut(&mut self.packet).set_duration(duration);
        }
    }

    pub fn get_mut(&mut self) -> &mut ffmpeg_next::codec::packet::Packet {
        Arc::make_mut(&mut self.packet)
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
            time_base: time_base,
        }
    }
}
