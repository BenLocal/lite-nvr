//! GB/T 28181 front-end control (`<PTZCmd>`) — the 8-byte pan/tilt/zoom/preset
//! command. Pure + media-agnostic: this only builds the hex string that goes
//! into a MANSCDP `DeviceControl` message.
//!
//! The 8 bytes (emitted as 16 uppercase hex chars):
//!
//! | Byte | Meaning |
//! |------|---------|
//! | B0 | `0xA5` fixed leading magic |
//! | B1 | `0x0F` = version `0` (high nibble) + checkbit `(0xA+0x5+0)&0xF` (low) |
//! | B2 | address low byte (single-camera default `0x01`) |
//! | B3 | command byte — movement bits OR preset opcode (below) |
//! | B4 | pan (horizontal) speed `0x00–0xFF` (0 for presets) |
//! | B5 | tilt (vertical) speed `0x00–0xFF` (preset **number** for presets) |
//! | B6 | high nibble = zoom speed `0x0–0xF`; low nibble = address high bits (0) |
//! | B7 | checksum = `(B0+B1+B2+B3+B4+B5+B6) & 0xFF` |
//!
//! B3 movement bits (combinable): `0x01` zoom-out, `0x02` zoom-in, `0x04` down,
//! `0x08` up, `0x10` left, `0x20` right; all-zero = stop.
//! B3 preset opcodes: `0x81` set, `0x82` call, `0x83` delete (number in B5).

/// A front-end control command. `Move` carries independent direction + zoom
/// bits (any combination; all-false = stop); presets carry a 1..=255 number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtzCommand {
    /// Continuous move. Any combination of directions/zoom; all-false = stop.
    /// `pan_speed`/`tilt_speed` are 0..=255; `zoom_speed` is 0..=15 (4 bits).
    Move {
        up: bool,
        down: bool,
        left: bool,
        right: bool,
        zoom_in: bool,
        zoom_out: bool,
        pan_speed: u8,
        tilt_speed: u8,
        zoom_speed: u8,
    },
    /// Save the current position as preset `n` (1..=255).
    PresetSet(u8),
    /// Move to preset `n`.
    PresetCall(u8),
    /// Delete preset `n`.
    PresetDelete(u8),
}

impl PtzCommand {
    /// The all-stop move (every direction/zoom bit clear, speeds 0).
    pub fn stop() -> Self {
        PtzCommand::Move {
            up: false,
            down: false,
            left: false,
            right: false,
            zoom_in: false,
            zoom_out: false,
            pan_speed: 0,
            tilt_speed: 0,
            zoom_speed: 0,
        }
    }
}

/// Encode a command into the 16-hex-char GB `<PTZCmd>` string.
/// `address` is the camera address low byte (single-camera default `1`).
pub fn encode_ptz_cmd(cmd: &PtzCommand, address: u8) -> String {
    let mut b = [0u8; 8];
    b[0] = 0xA5;
    // B1: high nibble = version(0); low nibble = checkbit = (0xA + 0x5 + 0) & 0xF.
    b[1] = 0x0F;
    b[2] = address;
    match cmd {
        PtzCommand::Move {
            up,
            down,
            left,
            right,
            zoom_in,
            zoom_out,
            pan_speed,
            tilt_speed,
            zoom_speed,
        } => {
            let mut c = 0u8;
            if *zoom_out {
                c |= 0x01;
            }
            if *zoom_in {
                c |= 0x02;
            }
            if *down {
                c |= 0x04;
            }
            if *up {
                c |= 0x08;
            }
            if *left {
                c |= 0x10;
            }
            if *right {
                c |= 0x20;
            }
            b[3] = c;
            b[4] = *pan_speed;
            b[5] = *tilt_speed;
            // B6: high nibble = zoom speed; low nibble = address high 4 bits (0).
            b[6] = (*zoom_speed & 0x0F) << 4;
        }
        PtzCommand::PresetSet(n) => {
            b[3] = 0x81;
            b[5] = *n;
        }
        PtzCommand::PresetCall(n) => {
            b[3] = 0x82;
            b[5] = *n;
        }
        PtzCommand::PresetDelete(n) => {
            b[3] = 0x83;
            b[5] = *n;
        }
    }
    // B7: checksum = sum of the first 7 bytes, mod 256.
    b[7] = b[..7].iter().fold(0u8, |acc, &x| acc.wrapping_add(x));
    b.iter().map(|x| format!("{x:02X}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_is_all_zero_command_byte() {
        // B3 = 0x00; checksum = (0xA5+0x0F+0x01) & 0xFF = 0xB5.
        assert_eq!(encode_ptz_cmd(&PtzCommand::stop(), 1), "A50F0100000000B5");
    }

    #[test]
    fn move_up_sets_bit3_and_tilt_speed() {
        let cmd = PtzCommand::Move {
            up: true,
            tilt_speed: 0x20,
            down: false,
            left: false,
            right: false,
            zoom_in: false,
            zoom_out: false,
            pan_speed: 0,
            zoom_speed: 0,
        };
        // B3=0x08 (up), B5=0x20 (tilt speed); checksum 0xDD.
        assert_eq!(encode_ptz_cmd(&cmd, 1), "A50F0108002000DD");
    }

    #[test]
    fn move_up_right_combines_bits() {
        let cmd = PtzCommand::Move {
            up: true,
            right: true,
            down: false,
            left: false,
            zoom_in: false,
            zoom_out: false,
            pan_speed: 0x10,
            tilt_speed: 0x10,
            zoom_speed: 0,
        };
        // B3 = 0x08 | 0x20 = 0x28.
        let hex = encode_ptz_cmd(&cmd, 1);
        assert_eq!(&hex[6..8], "28");
    }

    #[test]
    fn zoom_in_sets_bit1_and_zoom_speed_high_nibble() {
        let cmd = PtzCommand::Move {
            zoom_in: true,
            zoom_speed: 5,
            up: false,
            down: false,
            left: false,
            right: false,
            zoom_out: false,
            pan_speed: 0,
            tilt_speed: 0,
        };
        let hex = encode_ptz_cmd(&cmd, 1);
        assert_eq!(&hex[6..8], "02"); // B3 zoom in
        assert_eq!(&hex[12..14], "50"); // B6 high nibble = zoom speed 5
    }

    #[test]
    fn preset_call_uses_0x82_and_number_in_b5() {
        // B3=0x82, B5=0x05; checksum 0x3C.
        assert_eq!(
            encode_ptz_cmd(&PtzCommand::PresetCall(5), 1),
            "A50F01820005003C"
        );
    }

    #[test]
    fn preset_set_and_delete_opcodes() {
        assert_eq!(&encode_ptz_cmd(&PtzCommand::PresetSet(1), 1)[6..8], "81");
        assert_eq!(&encode_ptz_cmd(&PtzCommand::PresetDelete(1), 1)[6..8], "83");
    }

    #[test]
    fn checksum_wraps_mod_256() {
        // A high preset number that pushes the sum past 255 must wrap.
        let hex = encode_ptz_cmd(&PtzCommand::PresetCall(255), 1);
        // sum = 0xA5+0x0F+0x01+0x82+0x00+0xFF+0x00 = 0x236 -> &0xFF = 0x36.
        assert_eq!(&hex[14..16], "36");
    }
}
