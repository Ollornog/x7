//! Byte-level encoding and decoding of the X7 HID protocol. No I/O here.
//!
//! Input reports are 16 bytes, indexed from 0. Byte 0 is the consumer-control
//! bitfield; protocol data starts at byte 1. Example reply to a parameter get:
//! `00 26 01 00 96 08 3f 26 66 66` has group at byte 4, id at 5, float at 6..10.

pub type InputReport = [u8; 16];

/// Read a parameter: `26 01 <group> <id>`.
pub fn encode_get(group: u8, id: u8) -> [u8; 4] {
    [0x26, 0x01, group, id]
}

/// Write a parameter as a big-endian float. The set id is a different number
/// from the get id (see the settings table).
pub fn encode_set(group: u8, set_id: u8, value: f32) -> [u8; 14] {
    let f = value.to_be_bytes();
    [
        0x20, 0x00, 0x16, 0x0a, 0xd5, 0x02, 0x08, set_id, 0x20, group, f[0], f[1], f[2], f[3],
    ]
}

/// Reply to `encode_get`: `00 26 01 00 <group> <id> <f32 BE>`.
pub fn is_get_reply(report: &[u8], group: u8, id: u8) -> bool {
    report.len() >= 10 && report[1..6] == [0x26, 0x01, 0x00, group, id]
}

/// Float at bytes 6..10 of a report that satisfied `is_get_reply`.
///
/// Takes a whole report so a short slice is a compile error, not a panic:
///
/// ```compile_fail
/// let short = [0u8; 4];
/// x7::protocol::decode_get_reply(&short);
/// ```
pub fn decode_get_reply(report: &InputReport) -> f32 {
    f32::from_be_bytes([report[6], report[7], report[8], report[9]])
}

/// Acknowledgement of `encode_set`: `00 20`.
pub fn is_set_ack(report: &[u8]) -> bool {
    report.len() >= 2 && report[1] == 0x20
}

use crate::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    Headphones,
    Speakers,
}

/// Read the current output: `23 4d`, reply `00 23 4d 00 <1|2|4>`.
pub fn encode_output_get() -> [u8; 2] {
    [0x23, 0x4d]
}

/// Switch output. Speakers use the INT_MIN encoding from the captures.
pub fn encode_output_set(output: Output) -> Vec<u8> {
    match output {
        Output::Headphones => vec![0x23, 0x4e, 0x01],
        Output::Speakers => vec![0x23, 0x4e, 0x00, 0x00, 0x00, 0x80],
    }
}

/// `23 4d` reply, also pushed by the device on jack events and speaker switches.
pub fn is_output_reply(report: &[u8]) -> bool {
    report.len() >= 5 && report[1..3] == [0x23, 0x4d]
}

/// Echo of an output set command, carrying the value that was sent. The
/// headphone switch sends only this. Requiring the requested value means a
/// switch the device did not apply ends in `Error::BadReply`.
pub fn is_output_set_echo(report: &[u8], output: Output) -> bool {
    let value: &[u8] = match output {
        Output::Headphones => &[0x01],
        Output::Speakers => &[0x00, 0x00, 0x00, 0x80],
    };
    report.len() >= 4 + value.len()
        && report[1..3] == [0x23, 0x4e]
        && &report[4..4 + value.len()] == value
}

pub fn decode_output_reply(report: &InputReport) -> Result<Output, Error> {
    match report[4] {
        1 => Ok(Output::Headphones),
        // 4 once the speaker layout has rears (round 2 device check 2).
        2 | 4 => Ok(Output::Speakers),
        _ => Err(Error::BadReply(*report)),
    }
}

/// Features toggled through `23 23 <id> <on>`. The id doubles as the bit
/// position (id - 1) in the mask returned by `23 24`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Feature {
    Sbx = 1,
    Scout = 2,
    CrystalVoice = 0x11,
}

pub fn encode_feature_get() -> [u8; 2] {
    [0x23, 0x24]
}

pub fn encode_feature_set(feature: Feature, on: bool) -> [u8; 4] {
    [0x23, 0x23, feature as u8, on as u8]
}

/// `00 23 24 00` + 32-bit little-endian mask of enabled features.
pub fn is_feature_reply(report: &[u8]) -> bool {
    report.len() >= 8 && report[1..3] == [0x23, 0x24]
}

/// `00 23 23 00 <id> <on>` + mask, sent after a set and on button presses.
/// Matches only the requested feature in the requested state, so a set that
/// the device did not apply ends in `Error::BadReply` rather than silent success.
pub fn is_feature_set_echo(report: &[u8], feature: Feature, on: bool) -> bool {
    report.len() >= 6
        && report[1..3] == [0x23, 0x23]
        && report[4] == feature as u8
        && (report[5] != 0) == on
}

/// Mask at bytes 4..8 of a `23 24` reply. Whole report only:
///
/// ```compile_fail
/// let short = [0u8; 4];
/// x7::protocol::decode_feature_mask(&short);
/// ```
pub fn decode_feature_mask(report: &InputReport) -> u32 {
    u32::from_le_bytes([report[4], report[5], report[6], report[7]])
}

pub fn feature_enabled(mask: u32, feature: Feature) -> bool {
    mask & (1 << (feature as u8 - 1)) != 0
}

/// Simple `23 xx` toggles (session 4): the set id and its read id, which
/// is the set id minus one. The device answers a set with the read form
/// `00 23 <read> 00 <on>` followed by the echo `00 23 <set> 00 <on>`.
pub const TOGGLE_IDS: [u8; 8] = [0x42, 0x43, 0x44, 0x45, 0x4a, 0x4b, 0x58, 0x59];

pub fn encode_toggle_set(set: u8, on: bool) -> [u8; 3] {
    [0x23, set, on as u8]
}

pub fn encode_toggle_get(read: u8) -> [u8; 2] {
    [0x23, read]
}

/// `00 23 <id> 00 <on>`; the value is at byte 4.
pub fn is_toggle_reply(report: &[u8], id: u8) -> bool {
    report.len() >= 5 && report[1..3] == [0x23, id]
}

/// `5a`-framed device settings (sessions 4 and 4b).
pub const DEVICE_OUTPUT_TARGET: u8 = 0x2c;
pub const DEVICE_STANDBY: u8 = 0x39;
const DEVICE_ACK: u8 = 0x02;

/// Speaker Output Target: 1 line out, 2 amplifier out, 3 both.
pub fn encode_output_target_set(v: u8) -> [u8; 5] {
    [0x5a, DEVICE_OUTPUT_TARGET, 0x05, 0x00, v]
}

pub fn encode_output_target_get() -> [u8; 4] {
    [0x5a, DEVICE_OUTPUT_TARGET, 0x01, 0x01]
}

/// Auto Standby is flag 0x0b of the `5a 39` settings mask.
pub fn encode_standby_set(on: bool) -> [u8; 6] {
    [0x5a, DEVICE_STANDBY, 0x03, 0x00, 0x0b, on as u8]
}

pub fn encode_standby_get() -> [u8; 4] {
    [0x5a, DEVICE_STANDBY, 0x01, 0x01]
}

/// Get-form reply `00 5a <cmd> <len> 01 ...`.
pub fn is_device_value(report: &[u8], cmd: u8) -> bool {
    report.len() >= 9 && report[1..3] == [0x5a, cmd] && report[4] == 0x01
}

/// `00 5a 02 <len> <cmd> ...`, sent after a `5a` set.
pub fn is_device_ack(report: &[u8], cmd: u8) -> bool {
    report.len() >= 5 && report[1..3] == [0x5a, DEVICE_ACK] && report[4] == cmd
}

/// The value of a get-form reply as the row's raw float: the byte at 5 for
/// the output target, flag 0x0b of the little-endian mask at 5..9 for
/// standby (captured `00 08` after on).
pub fn decode_device_value(report: &InputReport, cmd: u8) -> f32 {
    match cmd {
        DEVICE_STANDBY => {
            let mask = u32::from_le_bytes([report[5], report[6], report[7], report[8]]);
            ((mask >> 0x0b) & 1) as f32
        }
        _ => report[5] as f32,
    }
}

/// `23 <id>` reads: output, feature mask, available features, jack, and the
/// read ids of the `TOGGLE_IDS` pairs (each set id minus one).
const READ_23: [u8; 8] = [0x4d, 0x24, 0x25, 0x12, 0x42, 0x44, 0x4a, 0x58];

/// Is this command one of the known reads? Everything else counts as a
/// write. The device sees 64 bytes padded with zeros, so `23 43` and
/// `23 43 00` are the same (Direct Mode off) and a read must be followed by
/// nothing but zeros. Used by the transport's safe mode: writes go out only
/// after a read has been answered.
pub fn is_read_command(cmd: &[u8]) -> bool {
    let tail_zero = |from: usize| cmd.iter().skip(from).all(|&b| b == 0);
    match cmd {
        [0x23, id, ..] => READ_23.contains(id) && tail_zero(2),
        [0x26, 0x01, _, _, ..] => tail_zero(4),
        [0x5a, _, 0x01, 0x01 | 0x02, ..] => tail_zero(4),
        _ => false,
    }
}

/// Does `report` answer the read `cmd`? `23 xx` replies `00 23 xx ...`,
/// `26 01 g i` replies `00 26 01 00 g i ...`, `5a c 01 ..` replies
/// `00 5a c ...`. False for anything that is not a read.
pub fn answers_read(cmd: &[u8], report: &[u8]) -> bool {
    if !is_read_command(cmd) || report.len() < 6 || report[0] != 0x00 {
        return false;
    }
    match cmd {
        [0x23, id, ..] => report[1..3] == [0x23, *id],
        [0x26, 0x01, g, i, ..] => report[1..6] == [0x26, 0x01, 0x00, *g, *i],
        [0x5a, c, ..] => report[1..3] == [0x5a, *c],
        _ => false,
    }
}

/// Everything the device sends on its own, plus replies, in decoded form.
/// Intended for a GUI that keeps the device open; the CLI does not use it.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    VolumeUp,
    VolumeDown,
    Mute,
    /// All-zero report sent ~10 ms after each knob or button report.
    Release,
    ParamPush {
        group: u8,
        id: u8,
        value: f32,
    },
    FeatureChanged {
        feature: u8,
        on: bool,
        mask: u32,
    },
    /// Mask reply to a `23 24` read.
    Features(u32),
    /// `00 20`, acknowledgement of a parameter set.
    SetAck,
    OutputChanged(Output),
    /// Headphone jack present (`23 12 00 04`) or removed (`23 12`).
    Jack(bool),
    /// `00 23 <id> 00 <on>` for one of `TOGGLE_IDS`: the read form or the
    /// set echo of a headphone/amp toggle.
    Toggle {
        id: u8,
        on: bool,
    },
    /// Get-form reply of a `5a` device setting, as the row's raw float.
    DeviceValue {
        cmd: u8,
        raw: f32,
    },
    /// `00 5a 02 <len> <cmd>`, acknowledgement of a `5a` set.
    DeviceAck(u8),
    Unknown(InputReport),
}

pub fn decode_report(report: &InputReport) -> Report {
    match report[0] {
        0x01 => return Report::VolumeUp,
        0x02 => return Report::VolumeDown,
        0x04 => return Report::Mute,
        _ => {}
    }
    if report.iter().all(|&b| b == 0) {
        return Report::Release;
    }
    match report[1..3] {
        [0x26, 0x01] => Report::ParamPush {
            group: report[4],
            id: report[5],
            value: decode_get_reply(report),
        },
        [0x23, 0x23] => Report::FeatureChanged {
            feature: report[4],
            on: report[5] != 0,
            mask: u32::from_le_bytes([report[6], report[7], report[8], report[9]]),
        },
        [0x20, _] => Report::SetAck,
        [0x23, 0x24] => Report::Features(decode_feature_mask(report)),
        [0x23, 0x4d] => match decode_output_reply(report) {
            Ok(output) => Report::OutputChanged(output),
            Err(_) => Report::Unknown(*report),
        },
        [0x23, 0x12] => Report::Jack(report[4] == 0x04),
        [0x23, id] if TOGGLE_IDS.contains(&id) => Report::Toggle {
            id,
            on: report[4] != 0,
        },
        [0x5a, DEVICE_ACK] => Report::DeviceAck(report[4]),
        [0x5a, cmd]
            if (cmd == DEVICE_OUTPUT_TARGET || cmd == DEVICE_STANDBY) && report[4] == 0x01 =>
        {
            Report::DeviceValue {
                cmd,
                raw: decode_device_value(report, cmd),
            }
        }
        _ => Report::Unknown(*report),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 16-byte report from the bytes a capture listing shows
    /// (listings trim trailing zeros).
    fn report(bytes: &[u8]) -> InputReport {
        let mut r = [0u8; 16];
        r[..bytes.len()].copy_from_slice(bytes);
        r
    }

    #[test]
    fn reads_are_told_from_writes() {
        for read in [
            &[0x23, 0x4d][..],
            &[0x23, 0x24],
            &[0x23, 0x25],
            &[0x23, 0x12],
            &[0x23, 0x42],
            &[0x23, 0x58, 0x00, 0x00],
            &[0x26, 0x01, 0x96, 0x08],
            &[0x26, 0x01, 0x96, 0x00],
            &[0x5a, 0x2c, 0x01, 0x01],
            &[0x5a, 0x2c, 0x01, 0x02],
            &[0x5a, 0x39, 0x01, 0x01],
        ] {
            assert!(is_read_command(read), "{read:02x?} is a read");
        }
        for write in [
            &encode_output_set(Output::Speakers)[..],
            &encode_output_set(Output::Headphones),
            &encode_feature_set(Feature::Sbx, false),
            &encode_set(0x96, 0x10, 0.25),
            // `23 43 00` pads to the same bytes as `23 43`: Direct Mode off.
            &encode_toggle_set(0x43, false),
            &[0x23, 0x43],
            &encode_output_target_set(2),
            &encode_standby_set(true),
            // A read id followed by data is not the read.
            &[0x23, 0x4d, 0x01],
            &[0x26, 0x01, 0x96, 0x08, 0x01],
            &[0x5a, 0x2c, 0x05, 0x00, 0x02],
            &[],
        ] {
            assert!(!is_read_command(write), "{write:02x?} is a write");
        }
    }

    #[test]
    fn a_read_is_answered_only_by_its_own_reply() {
        let output = report(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
        assert!(answers_read(&[0x23, 0x4d], &output));
        assert!(!answers_read(&[0x23, 0x24], &output));
        let get = report(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x08, 0x3f, 0x26, 0x66, 0x66]);
        assert!(answers_read(&[0x26, 0x01, 0x96, 0x08], &get));
        assert!(!answers_read(&[0x26, 0x01, 0x96, 0x0a], &get));
        let target = report(&[0x00, 0x5a, 0x2c, 0x05, 0x01, 0x02, 0, 0, 0]);
        assert!(answers_read(&[0x5a, 0x2c, 0x01, 0x01], &target));
        // Knob, release, and a write never verify.
        assert!(!answers_read(&[0x23, 0x4d], &report(&[0x01])));
        assert!(!answers_read(&[0x23, 0x4d], &report(&[])));
        assert!(!answers_read(
            &[0x23, 0x4e, 0x01],
            &report(&[0x00, 0x23, 0x4e, 0x00, 0x01])
        ));
    }

    #[test]
    fn get_encodes_as_captured() {
        assert_eq!(encode_get(0x96, 0x08), [0x26, 0x01, 0x96, 0x08]);
    }

    #[test]
    fn set_encodes_as_captured() {
        // Crystalizer level (set id 0x10) to 0.25 = 0x3e800000
        assert_eq!(
            encode_set(0x96, 0x10, 0.25),
            [
                0x20, 0x00, 0x16, 0x0a, 0xd5, 0x02, 0x08, 0x10, 0x20, 0x96, 0x3e, 0x80, 0x00, 0x00
            ]
        );
    }

    #[test]
    fn get_reply_matches_group_and_id() {
        let r = report(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x08, 0x3f, 0x26, 0x66, 0x66]);
        assert!(is_get_reply(&r, 0x96, 0x08));
    }

    #[test]
    fn push_for_other_id_is_not_the_reply() {
        // Unsolicited push the device sends after an output switch.
        let push = report(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x1a]);
        assert!(!is_get_reply(&push, 0x96, 0x08));
        // Same id in the other group is not it either.
        let other_group = report(&[0x00, 0x26, 0x01, 0x00, 0x95, 0x08]);
        assert!(!is_get_reply(&other_group, 0x96, 0x08));
    }

    #[test]
    fn set_ack_is_not_a_get_reply() {
        let ack = report(&[0x00, 0x20]);
        assert!(!is_get_reply(&ack, 0x96, 0x08));
        assert!(is_set_ack(&ack));
    }

    #[test]
    fn decodes_big_endian_float() {
        let r = report(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x08, 0x3f, 0x26, 0x66, 0x66]);
        assert!((decode_get_reply(&r) - 0.65).abs() < 1e-6);
        // Trimmed listing `3f 80` means 0x3f800000 = 1.0
        let one = report(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x00, 0x3f, 0x80]);
        assert_eq!(decode_get_reply(&one), 1.0);
    }

    #[test]
    fn short_slices_never_match() {
        assert!(!is_get_reply(&[0x00, 0x26], 0x96, 0x08));
        assert!(!is_set_ack(&[]));
    }

    #[test]
    fn output_commands_encode_as_captured() {
        assert_eq!(encode_output_get(), [0x23, 0x4d]);
        assert_eq!(
            encode_output_set(Output::Headphones),
            vec![0x23, 0x4e, 0x01]
        );
        assert_eq!(
            encode_output_set(Output::Speakers),
            vec![0x23, 0x4e, 0x00, 0x00, 0x00, 0x80]
        );
    }

    #[test]
    fn output_reply_decodes() {
        let hp = report(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
        let spk = report(&[0x00, 0x23, 0x4d, 0x00, 0x02]);
        assert!(is_output_reply(&hp));
        assert_eq!(decode_output_reply(&hp).unwrap(), Output::Headphones);
        assert_eq!(decode_output_reply(&spk).unwrap(), Output::Speakers);
        let bad = report(&[0x00, 0x23, 0x4d, 0x00, 0x07]);
        assert!(matches!(decode_output_reply(&bad), Err(Error::BadReply(_))));
    }

    #[test]
    fn output_set_echo_and_read_reply_do_not_cross_match() {
        // Echo after `23 4e 01` (headphones); no `23 4d` follows it.
        let echo = report(&[0x00, 0x23, 0x4e, 0x00, 0x01]);
        assert!(is_output_set_echo(&echo, Output::Headphones));
        assert!(!is_output_reply(&echo));
        // `23 4d` push after a speaker switch must not end a set early.
        let push = report(&[0x00, 0x23, 0x4d, 0x00, 0x02]);
        assert!(!is_output_set_echo(&push, Output::Speakers));
        // Speaker echo keeps the INT_MIN payload.
        let spk_echo = report(&[0x00, 0x23, 0x4e, 0x00, 0x00, 0x00, 0x00, 0x80]);
        assert!(is_output_set_echo(&spk_echo, Output::Speakers));
        // An echo carrying the other output's value is not our echo.
        assert!(!is_output_set_echo(&echo, Output::Speakers));
        assert!(!is_output_set_echo(&spk_echo, Output::Headphones));
    }

    #[test]
    fn feature_commands_encode_as_captured() {
        assert_eq!(encode_feature_get(), [0x23, 0x24]);
        assert_eq!(
            encode_feature_set(Feature::Sbx, true),
            [0x23, 0x23, 0x01, 0x01]
        );
        assert_eq!(
            encode_feature_set(Feature::Sbx, false),
            [0x23, 0x23, 0x01, 0x00]
        );
        assert_eq!(
            encode_feature_set(Feature::Scout, true),
            [0x23, 0x23, 0x02, 0x01]
        );
        assert_eq!(
            encode_feature_set(Feature::CrystalVoice, false),
            [0x23, 0x23, 0x11, 0x00]
        );
    }

    #[test]
    fn feature_mask_decodes_bits() {
        // App launch: SBX and CrystalVoice on.
        let r = report(&[0x00, 0x23, 0x24, 0x00, 0x01, 0x00, 0x01, 0x00]);
        assert!(is_feature_reply(&r));
        let mask = decode_feature_mask(&r);
        assert_eq!(mask, 0x0001_0001);
        assert!(feature_enabled(mask, Feature::Sbx));
        assert!(!feature_enabled(mask, Feature::Scout));
        assert!(feature_enabled(mask, Feature::CrystalVoice));
        // Live read with CrystalVoice off.
        let r2 = report(&[0x00, 0x23, 0x24, 0x00, 0x01]);
        assert!(!feature_enabled(
            decode_feature_mask(&r2),
            Feature::CrystalVoice
        ));
    }

    #[test]
    fn feature_set_echo_matches_its_feature_only() {
        let echo = report(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x01, 0x01, 0x00, 0x01]);
        assert!(is_feature_set_echo(&echo, Feature::Sbx, true));
        // Opposite state or other feature must not count as success.
        assert!(!is_feature_set_echo(&echo, Feature::Sbx, false));
        assert!(!is_feature_set_echo(&echo, Feature::Scout, true));
        assert!(!is_feature_reply(&echo));
    }

    #[test]
    fn consumer_control_reports() {
        assert_eq!(decode_report(&report(&[0x01])), Report::VolumeUp);
        assert_eq!(decode_report(&report(&[0x02])), Report::VolumeDown);
        assert_eq!(decode_report(&report(&[0x04])), Report::Mute);
        assert_eq!(decode_report(&report(&[])), Report::Release);
    }

    #[test]
    fn param_push_report() {
        let r = report(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x07, 0x3f, 0x80]);
        assert_eq!(
            decode_report(&r),
            Report::ParamPush {
                group: 0x96,
                id: 0x07,
                value: 1.0
            }
        );
    }

    #[test]
    fn feature_changed_report_from_button_press() {
        // Session 6: SBX button turned SBX off while CrystalVoice stayed on.
        let r = report(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]);
        assert_eq!(
            decode_report(&r),
            Report::FeatureChanged {
                feature: 1,
                on: false,
                mask: 0x0001_0000
            }
        );
    }

    #[test]
    fn output_and_jack_reports() {
        assert_eq!(
            decode_report(&report(&[0x00, 0x23, 0x4d, 0x00, 0x02])),
            Report::OutputChanged(Output::Speakers)
        );
        assert_eq!(
            decode_report(&report(&[0x00, 0x23, 0x12, 0x00, 0x04])),
            Report::Jack(true)
        );
        assert_eq!(
            decode_report(&report(&[0x00, 0x23, 0x12])),
            Report::Jack(false)
        );
    }

    #[test]
    fn unknown_report_is_preserved() {
        let raw = report(&[0x00, 0x23, 0x4c, 0x00, 0x07, 0x00, 0x00, 0x80]);
        assert_eq!(decode_report(&raw), Report::Unknown(raw));
        // A malformed output value is Unknown rather than a panic or error.
        let bad = report(&[0x00, 0x23, 0x4d, 0x00, 0x09]);
        assert_eq!(decode_report(&bad), Report::Unknown(bad));
    }

    #[test]
    fn set_ack_report() {
        assert_eq!(decode_report(&report(&[0x00, 0x20])), Report::SetAck);
    }

    #[test]
    fn features_report_from_mask_read() {
        // Live read with SBX and CrystalVoice on.
        let r = report(&[0x00, 0x23, 0x24, 0x00, 0x01, 0x00, 0x01, 0x00]);
        assert_eq!(decode_report(&r), Report::Features(0x0001_0001));
    }

    #[test]
    fn toggle_commands_encode_as_captured() {
        // 04-hp-spk-misc.txt: gain high `23 45 01`, read `23 44`.
        assert_eq!(encode_toggle_set(0x45, true), [0x23, 0x45, 0x01]);
        assert_eq!(encode_toggle_set(0x43, false), [0x23, 0x43, 0x00]);
        assert_eq!(encode_toggle_get(0x44), [0x23, 0x44]);
        assert!(is_toggle_reply(&[0x00, 0x23, 0x44, 0x00, 0x01], 0x44));
        assert!(!is_toggle_reply(&[0x00, 0x23, 0x45, 0x00, 0x01], 0x44));
        assert!(!is_toggle_reply(&[0x00, 0x23], 0x44));
    }

    #[test]
    fn toggle_reports_decode_read_form_and_echo() {
        assert_eq!(
            decode_report(&report(&[0x00, 0x23, 0x44, 0x00, 0x01])),
            Report::Toggle { id: 0x44, on: true }
        );
        assert_eq!(
            decode_report(&report(&[0x00, 0x23, 0x45, 0x00, 0x01])),
            Report::Toggle { id: 0x45, on: true }
        );
        assert_eq!(
            decode_report(&report(&[0x00, 0x23, 0x58])),
            Report::Toggle {
                id: 0x58,
                on: false
            }
        );
        // The jack-related `23 4c` push is not a toggle.
        let other = report(&[0x00, 0x23, 0x4c, 0x00, 0x07, 0x00, 0x00, 0x80]);
        assert_eq!(decode_report(&other), Report::Unknown(other));
    }

    #[test]
    fn device_commands_encode_as_captured() {
        assert_eq!(encode_output_target_set(3), [0x5a, 0x2c, 0x05, 0x00, 0x03]);
        assert_eq!(encode_output_target_get(), [0x5a, 0x2c, 0x01, 0x01]);
        assert_eq!(
            encode_standby_set(true),
            [0x5a, 0x39, 0x03, 0x00, 0x0b, 0x01]
        );
        assert_eq!(
            encode_standby_set(false),
            [0x5a, 0x39, 0x03, 0x00, 0x0b, 0x00]
        );
        assert_eq!(encode_standby_get(), [0x5a, 0x39, 0x01, 0x01]);
    }

    #[test]
    fn device_value_and_ack_reports() {
        let target = report(&[0x00, 0x5a, 0x2c, 0x05, 0x01, 0x03]);
        assert!(is_device_value(&target, DEVICE_OUTPUT_TARGET));
        assert_eq!(decode_device_value(&target, DEVICE_OUTPUT_TARGET), 3.0);
        assert_eq!(
            decode_report(&target),
            Report::DeviceValue {
                cmd: 0x2c,
                raw: 3.0
            }
        );
        let on = report(&[0x00, 0x5a, 0x39, 0x05, 0x01, 0x00, 0x08]);
        assert_eq!(
            decode_report(&on),
            Report::DeviceValue {
                cmd: 0x39,
                raw: 1.0
            }
        );
        let off = report(&[0x00, 0x5a, 0x39, 0x05, 0x01]);
        assert_eq!(
            decode_report(&off),
            Report::DeviceValue {
                cmd: 0x39,
                raw: 0.0
            }
        );
        let ack = report(&[0x00, 0x5a, 0x02, 0x0a, 0x2c, 0x00, 0x00, 0x03]);
        assert!(is_device_ack(&ack, DEVICE_OUTPUT_TARGET));
        assert!(!is_device_ack(&ack, DEVICE_STANDBY));
        assert_eq!(decode_report(&ack), Report::DeviceAck(0x2c));
        assert_eq!(
            decode_report(&report(&[0x00, 0x5a, 0x02, 0x02, 0x39])),
            Report::DeviceAck(0x39)
        );
        // The launch-time capability reply `5a 2c 0a 02 ...` is neither.
        let caps = report(&[
            0x00, 0x5a, 0x2c, 0x0a, 0x02, 0x02, 0x03, 0x00, 0x00, 0x00, 0x01,
        ]);
        assert_eq!(decode_report(&caps), Report::Unknown(caps));
    }

    #[test]
    fn output_read_with_rears_is_speakers() {
        // Round 2 device check 2: `23 4d` answers 04 once the layout has rears.
        let r = report(&[0x00, 0x23, 0x4d, 0x00, 0x04]);
        assert_eq!(decode_output_reply(&r).unwrap(), Output::Speakers);
        assert_eq!(decode_report(&r), Report::OutputChanged(Output::Speakers));
    }
}
