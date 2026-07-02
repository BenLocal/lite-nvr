//! GB28181 MPEG-2 Program Stream muxer. Each access unit is wrapped as:
//!
//! ```text
//! [pack header 0xBA] [PSM 0xBC — keyframes only] [PES 0xE0 …]
//! ```
//!
//! The PES payload is the picture's H.264 elementary stream (NALs with 4-byte
//! Annex-B start codes). This is the framing ZLMediaKit's RTP/PS demuxer on the
//! NVR expects; it reads the PSM to learn the video is H.264 (stream_type 0x1B).

use crate::h264::AccessUnit;

/// program_mux_rate is in units of 50 bytes/s. A fixed, comfortably-large value
/// keeps decoder buffering happy; the NVR does not rely on its accuracy.
const MUX_RATE: u32 = 25_200; // ~10 Mbit/s ceiling

/// Assemble one access unit into a PS byte stream, timestamped at `pts` (90 kHz).
pub fn mux_access_unit(au: &AccessUnit, pts: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    write_pack_header(&mut out, pts, MUX_RATE);
    if au.keyframe {
        write_psm(&mut out);
    }
    // Elementary stream for this picture: each NAL with a 4-byte start code.
    let mut es = Vec::new();
    for nal in &au.nals {
        es.extend_from_slice(&[0, 0, 0, 1]);
        es.extend_from_slice(nal);
    }
    write_pes(&mut out, 0xE0, &es, pts);
    out
}

/// MPEG-2 pack_header (14 bytes): 0x000001BA + SCR (base 33-bit / ext 9-bit) +
/// program_mux_rate + stuffing. Bit layout per ISO/IEC 13818-1 §2.5.3.
fn write_pack_header(out: &mut Vec<u8>, scr_base: u64, mux_rate: u32) {
    let s = scr_base & 0x1_FFFF_FFFF; // 33 bits
    let ext: u16 = 0; // scr_extension (9 bits)
    out.extend_from_slice(&[0x00, 0x00, 0x01, 0xBA]);
    out.push(0x44 | (((s >> 30) & 0x07) as u8) << 3 | ((s >> 28) & 0x03) as u8);
    out.push(((s >> 20) & 0xFF) as u8);
    out.push((((s >> 15) & 0x1F) as u8) << 3 | 0x04 | ((s >> 13) & 0x03) as u8);
    out.push(((s >> 5) & 0xFF) as u8);
    out.push(((s & 0x1F) as u8) << 3 | 0x04 | (((ext >> 7) & 0x03) as u8));
    out.push((((ext & 0x7F) as u8) << 1) | 0x01);
    out.push(((mux_rate >> 14) & 0xFF) as u8);
    out.push(((mux_rate >> 6) & 0xFF) as u8);
    out.push((((mux_rate & 0x3F) as u8) << 2) | 0x03);
    out.push(0xF8); // reserved (5 bits) + pack_stuffing_length = 0
}

/// program_stream_map (PSM): declares one elementary stream — H.264
/// (stream_type 0x1B) carried on elementary_stream_id 0xE0. Sent on keyframes.
fn write_psm(out: &mut Vec<u8>) {
    let mut psm: Vec<u8> = Vec::with_capacity(20);
    psm.extend_from_slice(&[0x00, 0x00, 0x01, 0xBC]);
    psm.extend_from_slice(&[0x00, 0x00]); // program_stream_map_length — filled below
    psm.push(0xE0); // current_next_indicator=1, reserved, program_stream_map_version=0
    psm.push(0xFF); // reserved (7 bits) + marker
    psm.extend_from_slice(&[0x00, 0x00]); // program_stream_info_length = 0
    psm.extend_from_slice(&[0x00, 0x04]); // elementary_stream_map_length = 4
    psm.extend_from_slice(&[0x1B, 0xE0, 0x00, 0x00]); // H.264, es_id 0xE0, es_info_len 0

    let body_len = (psm.len() - 6 + 4) as u16; // bytes after the length field, incl. CRC
    psm[4] = (body_len >> 8) as u8;
    psm[5] = (body_len & 0xFF) as u8;
    let crc = crc32_mpeg(&psm);
    psm.extend_from_slice(&crc.to_be_bytes());
    out.extend_from_slice(&psm);
}

/// PES packet(s) for `stream_id`, carrying `payload` with a PTS (90 kHz). Splits
/// across packets when the payload exceeds the 16-bit PES length field; only the
/// first packet carries the PTS.
fn write_pes(out: &mut Vec<u8>, stream_id: u8, payload: &[u8], pts: u64) {
    // Max payload so PES_packet_length (= 3 header + 5 PTS + payload) fits 0xFFFF.
    const MAX_FIRST: usize = 0xFFFF - 8;
    const MAX_CONT: usize = 0xFFFF - 3;
    let mut off = 0;
    let mut first = true;
    loop {
        let cap = if first { MAX_FIRST } else { MAX_CONT };
        let end = (off + cap).min(payload.len());
        let chunk = &payload[off..end];
        let hdr_len: u8 = if first { 5 } else { 0 };
        let pes_len = 3 + hdr_len as usize + chunk.len();
        out.extend_from_slice(&[0x00, 0x00, 0x01, stream_id]);
        out.push((pes_len >> 8) as u8);
        out.push((pes_len & 0xFF) as u8);
        out.push(0x80); // '10' marker, no scrambling/priority/alignment flags
        out.push(if first { 0x80 } else { 0x00 }); // PTS_DTS_flags = '10' (PTS only)
        out.push(hdr_len);
        if first {
            write_pts(out, pts);
        }
        out.extend_from_slice(chunk);
        off = end;
        first = false;
        if off >= payload.len() {
            break;
        }
    }
}

/// The 5-byte PTS field of a PES header (guard nibble `0010` for PTS-only).
fn write_pts(out: &mut Vec<u8>, pts: u64) {
    let p = pts & 0x1_FFFF_FFFF; // 33 bits
    out.push(0x21 | (((p >> 30) & 0x07) as u8) << 1);
    out.push(((p >> 22) & 0xFF) as u8);
    out.push((((p >> 15) & 0x7F) as u8) << 1 | 0x01);
    out.push(((p >> 7) & 0xFF) as u8);
    out.push((((p & 0x7F) as u8) << 1) | 0x01);
}

/// MPEG-2 systems CRC-32 (poly 0x04C11DB7, MSB-first, init 0xFFFFFFFF, no final
/// XOR) — used for the PSM's trailing CRC.
fn crc32_mpeg(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_header_is_well_formed() {
        let mut out = Vec::new();
        write_pack_header(&mut out, 90_000, MUX_RATE);
        assert_eq!(out.len(), 14);
        assert_eq!(&out[0..4], &[0x00, 0x00, 0x01, 0xBA]);
        // marker bits must be set (bit2 of the two middle SCR bytes)
        assert_eq!(out[6] & 0x04, 0x04);
        assert_eq!(out[8] & 0x04, 0x04);
    }

    #[test]
    fn psm_declares_h264() {
        let mut out = Vec::new();
        write_psm(&mut out);
        assert_eq!(&out[0..4], &[0x00, 0x00, 0x01, 0xBC]);
        // length field = remaining bytes (incl. 4-byte CRC)
        let len = ((out[4] as usize) << 8) | out[5] as usize;
        assert_eq!(len, out.len() - 6);
        // stream_type 0x1B (H.264) on es_id 0xE0
        assert!(out.windows(2).any(|w| w == [0x1B, 0xE0]));
    }

    #[test]
    fn pes_length_matches_payload() {
        let mut out = Vec::new();
        write_pes(&mut out, 0xE0, &[0xAA; 100], 90_000);
        assert_eq!(&out[0..4], &[0x00, 0x00, 0x01, 0xE0]);
        let pes_len = ((out[4] as usize) << 8) | out[5] as usize;
        assert_eq!(pes_len, 3 + 5 + 100); // flags(3) + PTS(5) + payload
        assert_eq!(out.len(), 6 + pes_len);
    }

    #[test]
    fn crc32_mpeg_known_vector() {
        // Standard MPEG CRC-32 of "123456789" is 0x0376E6E7.
        assert_eq!(crc32_mpeg(b"123456789"), 0x0376_E6E7);
    }
}
