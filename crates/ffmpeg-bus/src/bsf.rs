use anyhow::Result;
use bytes::{Bytes, BytesMut};
use ffmpeg_next::{Rational, codec::Parameters};

use crate::packet::RawPacket;

/// Reads extradata from codec parameters via the raw AVCodecParameters pointer.
/// Returns None if extradata is null or empty.
fn get_extradata(codec_params: &Parameters) -> Option<&[u8]> {
    unsafe {
        // AVCodecParameters has extradata (uint8_t*) and extradata_size (int)
        let p = codec_params.as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
        let extradata_ptr = (*p).extradata;
        if extradata_ptr.is_null() {
            return None;
        }
        let size = (*p).extradata_size;
        if size <= 0 {
            return None;
        }
        Some(std::slice::from_raw_parts(extradata_ptr, size as usize))
    }
}

/// Check if the codec parameters indicate AVCC/HVCC format (needs conversion to Annex B).
///
/// Returns true if the stream is in MP4 container format and needs BSF conversion.
/// Returns false if already in Annex B format or format cannot be determined.
pub fn needs_annexb_conversion(codec_params: &Parameters) -> bool {
    let extradata = match get_extradata(codec_params) {
        Some(d) if !d.is_empty() => d,
        _ => return false,
    };
    if extradata.len() < 4 {
        return false;
    }

    // Check for Annex B start codes
    if (extradata[0] == 0x00
        && extradata[1] == 0x00
        && extradata[2] == 0x00
        && extradata[3] == 0x01)
        || (extradata[0] == 0x00 && extradata[1] == 0x00 && extradata[2] == 0x01)
    {
        return false;
    }

    // AVCC typically has configurationVersion = 1 as first byte
    if extradata[0] == 0x01 && extradata.len() >= 7 {
        return true;
    }

    false
}

/// Check if packet data is in Annex B format by looking at the start codes.
pub fn is_annexb_packet(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    if data[0] == 0x00 && data[1] == 0x00 && data[2] == 0x00 && data[3] == 0x01 {
        return true;
    }
    if data[0] == 0x00 && data[1] == 0x00 && data[2] == 0x01 {
        return true;
    }
    false
}

/// Annex B start code (4-byte)
const START_CODE: &[u8] = &[0x00, 0x00, 0x00, 0x01];

/// Converts H.264/HEVC from AVCC (length-prefixed) to Annex B (start-code) format.
/// Does not use FFmpeg BSF; ffmpeg_next does not expose AVBSFContext in its ffi.
pub struct AvccToAnnexB {
    time_base: Rational,
}

impl AvccToAnnexB {
    pub fn new(_codec_params: &Parameters, time_base: Rational) -> Result<Self> {
        Ok(Self { time_base })
    }

    /// Convert one raw packet from AVCC to Annex B.
    /// Input: 4-byte big-endian length then NAL bytes, repeated.
    /// Output: 0x00 0x00 0x00 0x01 then NAL bytes for each NAL.
    pub fn filter_packet(&mut self, packet: &RawPacket) -> Result<FilteredPacket> {
        let data = packet.data();
        if data.is_empty() {
            return Ok(FilteredPacket {
                data: Bytes::new(),
                pts: packet.pts(),
                dts: packet.dts(),
                is_key: packet.is_key(),
                size: 0,
            });
        }
        if is_annexb_packet(&data) {
            return Ok(FilteredPacket {
                data: data.clone(),
                pts: packet.pts(),
                dts: packet.dts(),
                is_key: packet.is_key(),
                size: data.len(),
            });
        }
        let out = convert_avcc_to_annexb(&data);
        let len = out.len();
        Ok(FilteredPacket {
            data: out,
            pts: packet.pts(),
            dts: packet.dts(),
            is_key: packet.is_key(),
            size: len,
        })
    }

    pub fn time_base(&self) -> Rational {
        self.time_base
    }
}

/// Converts AVCC (4-byte length + NAL) to Annex B (start code + NAL).
/// Public for use by pipe when forwarding to ZLMediaKit.
pub fn convert_avcc_to_annexb(avcc: &[u8]) -> Bytes {
    let mut out = BytesMut::new();
    let mut i = 0;
    while i + 4 <= avcc.len() {
        let len = (u32::from(avcc[i]) << 24
            | u32::from(avcc[i + 1]) << 16
            | u32::from(avcc[i + 2]) << 8
            | u32::from(avcc[i + 3])) as usize;
        i += 4;
        if len == 0 || i + len > avcc.len() {
            break;
        }
        out.extend_from_slice(START_CODE);
        out.extend_from_slice(&avcc[i..i + len]);
        i += len;
    }
    out.freeze()
}

/// The decoder parameter sets carried in AVCC / HVCC extradata (H.264 SPS +
/// PPS; HEVC VPS + SPS + PPS), as one Annex B buffer. MP4 sources keep these
/// only in extradata, not in-band, so a consumer of the converted Annex B
/// stream (e.g. ZLMediaKit) needs them prepended to keyframes. `None` when
/// the stream is not H.264/HEVC or the extradata is not AVCC/HVCC.
pub fn parameter_sets_annexb(codec_params: &Parameters) -> Option<Bytes> {
    let extradata = get_extradata(codec_params)?;
    match codec_params.id() {
        ffmpeg_next::codec::Id::H264 => avcc_parameter_sets(extradata),
        ffmpeg_next::codec::Id::HEVC => hvcc_parameter_sets(extradata),
        _ => None,
    }
}

/// Bounds-checked big-endian reader over extradata (malformed input yields
/// `None` instead of panicking).
struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn at(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    fn u8(&mut self) -> Option<u8> {
        let v = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }

    fn u16(&mut self) -> Option<usize> {
        Some(usize::from(self.u8()?) << 8 | usize::from(self.u8()?))
    }

    fn bytes(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(len)?;
        let v = self.data.get(self.pos..end)?;
        self.pos = end;
        Some(v)
    }
}

/// Append `count` 16-bit-length-prefixed NAL units from `r` as Annex B.
fn push_nalus(r: &mut ByteReader<'_>, count: usize, out: &mut BytesMut) -> Option<()> {
    for _ in 0..count {
        let len = r.u16()?;
        let nal = r.bytes(len)?;
        out.extend_from_slice(START_CODE);
        out.extend_from_slice(nal);
    }
    Some(())
}

/// AVCDecoderConfigurationRecord: version(1) profile compat level
/// lengthSize(1) | numSPS(1) [len(2) sps]* numPPS(1) [len(2) pps]*.
fn avcc_parameter_sets(extradata: &[u8]) -> Option<Bytes> {
    if extradata.first() != Some(&1) {
        return None;
    }
    let mut r = ByteReader::at(extradata, 5);
    let mut out = BytesMut::new();
    let sps = usize::from(r.u8()? & 0x1f);
    push_nalus(&mut r, sps, &mut out)?;
    let pps = usize::from(r.u8()?);
    push_nalus(&mut r, pps, &mut out)?;
    (!out.is_empty()).then(|| out.freeze())
}

/// HEVCDecoderConfigurationRecord: 22 header bytes, numArrays(1), then per
/// array: type(1) numNalus(2) [len(2) nalu]*. Keeps VPS/SPS/PPS (32/33/34).
fn hvcc_parameter_sets(extradata: &[u8]) -> Option<Bytes> {
    if extradata.first() != Some(&1) {
        return None;
    }
    let mut r = ByteReader::at(extradata, 22);
    let mut out = BytesMut::new();
    let arrays = r.u8()?;
    for _ in 0..arrays {
        let nal_type = r.u8()? & 0x3f;
        let count = r.u16()?;
        if (32..=34).contains(&nal_type) {
            push_nalus(&mut r, count, &mut out)?;
        } else {
            let mut skipped = BytesMut::new();
            push_nalus(&mut r, count, &mut skipped)?;
        }
    }
    (!out.is_empty()).then(|| out.freeze())
}

/// A packet in Annex B format.
#[derive(Debug, Clone)]
pub struct FilteredPacket {
    pub data: Bytes,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub is_key: bool,
    pub size: usize,
}

impl FilteredPacket {
    pub fn size(&self) -> usize {
        if self.size > 0 {
            self.size
        } else {
            self.data.len()
        }
    }
}

#[cfg(test)]
#[path = "bsf_test.rs"]
mod bsf_test;
