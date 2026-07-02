//! Minimal Annex-B H.264 parsing: split a byte stream into NAL units and group
//! them into access units (one coded picture each). Just enough to feed the PS
//! muxer — no slice-header parsing, we rely on the simple structure our sample
//! clip has (one slice per picture, non-VCL NALs preceding their VCL NAL).

/// The 5-bit `nal_unit_type` of a NAL (payload WITHOUT the start code).
pub fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map(|b| b & 0x1f).unwrap_or(0)
}

/// A VCL NAL (coded slice) — its arrival completes an access unit.
fn is_vcl(t: u8) -> bool {
    matches!(t, 1 | 5)
}

/// An IDR slice (`nal_unit_type == 5`) — marks a keyframe access unit.
fn is_idr(t: u8) -> bool {
    t == 5
}

/// One coded picture: its NAL units (each WITHOUT a start code) and whether it
/// is a keyframe (carries an IDR, and — with `repeat-headers` — its SPS/PPS).
pub struct AccessUnit {
    pub nals: Vec<Vec<u8>>,
    pub keyframe: bool,
}

/// Iterate the NAL units of an Annex-B buffer, yielding each payload WITHOUT its
/// start code and with trailing zero bytes (the leading `00` of a 4-byte start
/// code, or `cabac_zero_word`s) trimmed.
fn iter_nals(buf: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut pos = find_start_code(buf, 0);
    while let Some(sc) = pos {
        let nal_start = sc + 3;
        let next = find_start_code(buf, nal_start);
        let mut nal_end = next.unwrap_or(buf.len());
        // A 4-byte start code (00 00 00 01) is found as the 3-byte code 00 00 01
        // preceded by an extra 00; trim any trailing zeros so they don't cling
        // to the previous NAL.
        while nal_end > nal_start && buf[nal_end - 1] == 0 {
            nal_end -= 1;
        }
        if nal_end > nal_start {
            nals.push(&buf[nal_start..nal_end]);
        }
        pos = next;
    }
    nals
}

/// Position of the next `00 00 01` start-code prefix at or after `from`.
fn find_start_code(buf: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Group an Annex-B stream into access units. Non-VCL NALs (SPS/PPS/SEI/AUD)
/// accumulate and attach to the next VCL NAL, which closes the unit.
pub fn parse_access_units(buf: &[u8]) -> Vec<AccessUnit> {
    let mut aus = Vec::new();
    let mut cur: Vec<Vec<u8>> = Vec::new();
    let mut cur_key = false;
    for nal in iter_nals(buf) {
        let t = nal_type(nal);
        cur.push(nal.to_vec());
        cur_key |= is_idr(t);
        if is_vcl(t) {
            aus.push(AccessUnit {
                nals: std::mem::take(&mut cur),
                keyframe: cur_key,
            });
            cur_key = false;
        }
    }
    aus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_groups_access_units() {
        // SPS(7) PPS(8) IDR(5) | non-IDR(1) — two access units, first keyframe.
        let stream = [
            0, 0, 0, 1, 0x67, 0xAA, // SPS (4-byte start code)
            0, 0, 1, 0x68, 0xBB, // PPS (3-byte start code)
            0, 0, 0, 1, 0x65, 0x11, 0x22, // IDR slice
            0, 0, 0, 1, 0x41, 0x33, // non-IDR slice
        ];
        let aus = parse_access_units(&stream);
        assert_eq!(aus.len(), 2);
        assert!(aus[0].keyframe);
        assert_eq!(aus[0].nals.len(), 3); // SPS, PPS, IDR
        assert_eq!(nal_type(&aus[0].nals[0]), 7);
        assert!(!aus[1].keyframe);
        assert_eq!(aus[1].nals.len(), 1);
        assert_eq!(nal_type(&aus[1].nals[0]), 1);
        // trailing zeros of the 4-byte start codes must not cling to a NAL
        assert_eq!(aus[0].nals[0], vec![0x67, 0xAA]);
    }
}
