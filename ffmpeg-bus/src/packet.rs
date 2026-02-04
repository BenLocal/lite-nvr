use std::sync::Arc;

use ffmpeg_next::Rational;

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

    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    pub fn get_mut(&mut self) -> &mut ffmpeg_next::codec::packet::Packet {
        Arc::make_mut(&mut self.packet)
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
