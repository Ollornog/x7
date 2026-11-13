//! The parameter table: one row per SBX and CrystalVoice setting, taken from
//! the decode tables in `docs/protocol.md`.

use crate::Error;
use crate::protocol::{self, InputReport};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Float {
        min: f32,
        max: f32,
    },
    Enum(&'static [&'static str]),
    /// Named raw values: parsed by name, sent as the listed float, read
    /// back by matching the float.
    Choice(&'static [(&'static str, f32)]),
}

#[derive(Debug)]
pub struct Param {
    /// CLI group: "sbx" or "voice".
    pub group: &'static str,
    /// CLI name, kebab-case.
    pub name: &'static str,
    /// HID parameter group: 0x96 SBX, 0x95 CrystalVoice.
    pub hid_group: u8,
    pub get_id: u8,
    pub set_id: u8,
    pub kind: Kind,
}

const LEVEL: Kind = Kind::Float { min: 0.0, max: 1.0 };
const BAND: Kind = Kind::Float {
    min: -12.0,
    max: 12.0,
};
/// The device stores 1.0 for an unticked Full-Range box (capture 04b).
const FULL_RANGE: Kind = Kind::Choice(&[("on", 0.0), ("off", 1.0)]);

const fn row(
    group: &'static str,
    name: &'static str,
    hid_group: u8,
    get_id: u8,
    set_id: u8,
    kind: Kind,
) -> Param {
    Param {
        group,
        name,
        hid_group,
        get_id,
        set_id,
        kind,
    }
}

pub static PARAMS: &[Param] = &[
    row("sbx", "surround", 0x96, 0x00, 0x00, Kind::Bool),
    row("sbx", "surround-level", 0x96, 0x01, 0x02, LEVEL),
    row("sbx", "dialog-plus", 0x96, 0x02, 0x04, Kind::Bool),
    row("sbx", "dialog-plus-level", 0x96, 0x03, 0x06, LEVEL),
    row("sbx", "smart-volume", 0x96, 0x04, 0x08, Kind::Bool),
    row("sbx", "smart-volume-level", 0x96, 0x05, 0x0a, LEVEL),
    row(
        "sbx",
        "smart-volume-mode",
        0x96,
        0x06,
        0x0c,
        Kind::Enum(&["normal", "loud", "night"]),
    ),
    row("sbx", "crystalizer", 0x96, 0x07, 0x0e, Kind::Bool),
    row("sbx", "crystalizer-level", 0x96, 0x08, 0x10, LEVEL),
    row(
        "sbx",
        "bass-crossover",
        0x96,
        0x17,
        0x2e,
        Kind::Float {
            min: 10.0,
            max: 500.0,
        },
    ),
    row("sbx", "bass", 0x96, 0x18, 0x30, Kind::Bool),
    row("sbx", "bass-level", 0x96, 0x19, 0x32, LEVEL),
    row("voice", "echo-cancellation", 0x95, 0x00, 0x00, Kind::Bool),
    row("voice", "noise-reduction", 0x95, 0x04, 0x08, Kind::Bool),
    row("voice", "focus", 0x95, 0x06, 0x0c, Kind::Bool),
    row(
        "voice",
        "focus-angle",
        0x95,
        0x08,
        0x10,
        Kind::Float {
            min: 20.0,
            max: 180.0,
        },
    ),
    row("voice", "smart-volume", 0x95, 0x2c, 0x58, Kind::Bool),
    row("voice", "smart-volume-level", 0x95, 0x2d, 0x5a, LEVEL),
    row("eq", "enabled", 0x96, 0x09, 0x12, Kind::Bool),
    row(
        "eq",
        "preamp",
        0x96,
        0x0a,
        0x14,
        Kind::Float {
            min: -6.0,
            max: 6.0,
        },
    ),
    row("eq", "31hz", 0x96, 0x0b, 0x16, BAND),
    row("eq", "62hz", 0x96, 0x0c, 0x18, BAND),
    row("eq", "125hz", 0x96, 0x0d, 0x1a, BAND),
    row("eq", "250hz", 0x96, 0x0e, 0x1c, BAND),
    row("eq", "500hz", 0x96, 0x0f, 0x1e, BAND),
    row("eq", "1khz", 0x96, 0x10, 0x20, BAND),
    row("eq", "2khz", 0x96, 0x11, 0x22, BAND),
    row("eq", "4khz", 0x96, 0x12, 0x24, BAND),
    row("eq", "8khz", 0x96, 0x13, 0x26, BAND),
    row("eq", "16khz", 0x96, 0x14, 0x28, BAND),
    // Group 0x80 float alone changes the configuration (round 2 check 2).
    row(
        "speakers",
        "layout",
        0x80,
        0x04,
        0x08,
        // 1 + subwoofer + 2 x center + 4 x rear (the macOS panel's
        // SpeakerAdvancedConfiguration, alcatron/mb-x7-control PROTOCOL.md
        // section 3). The Windows captures showed 1, 5, 7 and 8.
        Kind::Choice(&[
            ("2.0", 1.0),
            ("2.1", 2.0),
            ("3.0", 3.0),
            ("3.1", 4.0),
            ("4.0", 5.0),
            ("4.1", 6.0),
            ("5.0", 7.0),
            ("5.1", 8.0),
        ]),
    ),
    row("speakers", "front-full-range", 0x96, 0x1a, 0x34, FULL_RANGE),
    row("speakers", "rear-full-range", 0x96, 0x1b, 0x36, FULL_RANGE),
    row("speakers", "bass-redirection", 0x96, 0x15, 0x2a, Kind::Bool),
    // The device keeps any float; 10 Hz is the smallest useful crossover.
    row(
        "speakers",
        "crossover",
        0x96,
        0x16,
        0x2c,
        Kind::Float {
            min: 10.0,
            max: 500.0,
        },
    ),
    row("speakers", "subwoofer-gain", 0x96, 0x1e, 0x3c, Kind::Bool),
];

pub fn find(group: &str, name: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.group == group && p.name == name)
}

pub fn in_group(group: &str) -> impl Iterator<Item = &'static Param> {
    let group = group.to_owned();
    PARAMS.iter().filter(move |p| p.group == group)
}

/// The ten EQ band rows in device order (set ids 16..28).
pub const EQ_BAND_NAMES: [&str; 10] = [
    "31hz", "62hz", "125hz", "250hz", "500hz", "1khz", "2khz", "4khz", "8khz", "16khz",
];

pub fn eq_bands() -> [&'static Param; 10] {
    EQ_BAND_NAMES.map(|n| find("eq", n).expect("eq band row"))
}

/// A client-side EQ preset: the Windows app writes preamp 0 and these ten
/// band gains in dB, in the app's list order. Flat and Rock are from the
/// capture; the other eight are read off the Windows sliders.
#[derive(Debug)]
pub struct EqPreset {
    pub name: &'static str,
    pub bands: [f32; 10],
}

pub static EQ_PRESETS: &[EqPreset] = &[
    EqPreset {
        name: "acoustic",
        bands: [0.0, 1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 2.0, 2.0, 2.0],
    },
    EqPreset {
        name: "classical",
        bands: [0.0, 6.0, 6.0, 3.0, 0.0, 0.0, 0.0, 0.0, 3.0, 3.0],
    },
    EqPreset {
        name: "country",
        bands: [-1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 2.0, 3.0, 4.0],
    },
    EqPreset {
        name: "dance",
        bands: [-1.0, 2.0, 3.0, 4.0, -1.0, -1.0, 0.0, 0.0, 4.0, 4.0],
    },
    EqPreset {
        name: "flat",
        bands: [0.0; 10],
    },
    EqPreset {
        name: "jazz",
        bands: [0.0, 0.0, 1.0, 4.0, 4.0, 4.0, 0.0, 1.0, 3.0, 3.0],
    },
    EqPreset {
        name: "new-age",
        bands: [0.0, 2.0, 2.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
    },
    EqPreset {
        name: "pop",
        bands: [-2.0, 0.0, 2.0, 2.0, 0.0, -1.0, -1.0, 0.0, 3.0, 6.0],
    },
    // The capture, 11.688 s onward.
    EqPreset {
        name: "rock",
        bands: [-1.0, -1.0, 1.0, 2.0, -1.0, -1.0, 0.0, 0.0, 4.0, 4.0],
    },
    EqPreset {
        name: "vocal",
        bands: [-2.0, -1.0, -1.0, 0.0, 3.0, 4.0, 3.0, 0.0, 0.0, 1.0],
    },
];

pub fn eq_preset(name: &str) -> Option<&'static EqPreset> {
    EQ_PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
}

/// The preset whose bands equal `bands` exactly, if any.
pub fn eq_preset_matching(bands: [f32; 10]) -> Option<&'static EqPreset> {
    EQ_PRESETS.iter().find(|p| p.bands == bands)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Float(f32),
    Enum(usize),
}

impl Kind {
    /// Parse CLI text into a validated value for a row named `name`.
    pub fn parse(self, name: &'static str, text: &str) -> Result<Value, Error> {
        let unknown = || Error::UnknownValue {
            name,
            value: text.to_owned(),
        };
        match self {
            Kind::Bool => match text.to_ascii_lowercase().as_str() {
                "on" => Ok(Value::Bool(true)),
                "off" => Ok(Value::Bool(false)),
                _ => Err(unknown()),
            },
            Kind::Float { min, max } => {
                let v: f32 = text.parse().map_err(|_| unknown())?;
                // `!(a && b)` rather than `a || b` so NaN fails the range check.
                if !(v >= min && v <= max) {
                    return Err(Error::OutOfRange {
                        name,
                        value: v,
                        min,
                        max,
                    });
                }
                // Normalise -0.0 so it prints and compares as 0.
                Ok(Value::Float(v + 0.0))
            }
            Kind::Enum(names) => names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(text))
                .map(Value::Enum)
                .ok_or_else(unknown),
            Kind::Choice(choices) => choices
                .iter()
                .position(|(n, _)| n.eq_ignore_ascii_case(text))
                .map(Value::Enum)
                .ok_or_else(unknown),
        }
    }

    /// Check a value built in code (not via `parse`) against this kind.
    pub fn validate(self, name: &'static str, value: &Value) -> Result<(), Error> {
        match (self, value) {
            (Kind::Bool, Value::Bool(_)) => Ok(()),
            (Kind::Float { min, max }, Value::Float(v)) if *v >= min && *v <= max => Ok(()),
            (Kind::Float { min, max }, Value::Float(v)) => Err(Error::OutOfRange {
                name,
                value: *v,
                min,
                max,
            }),
            (Kind::Enum(names), Value::Enum(i)) if *i < names.len() => Ok(()),
            (Kind::Choice(choices), Value::Enum(i)) if *i < choices.len() => Ok(()),
            _ => Err(Error::UnknownValue {
                name,
                value: format!("{value:?}"),
            }),
        }
    }

    /// Render a value the way the CLI prints it.
    pub fn format(self, value: &Value) -> String {
        match (self, value) {
            (_, Value::Bool(b)) => if *b { "on" } else { "off" }.to_owned(),
            (_, Value::Float(f)) => format!("{f:.2}"),
            (Kind::Enum(names), Value::Enum(i)) => match names.get(*i) {
                Some(n) => (*n).to_owned(),
                None => format!("unknown({i})"),
            },
            (Kind::Choice(choices), Value::Enum(i)) => match choices.get(*i) {
                Some((n, _)) => (*n).to_owned(),
                None => format!("unknown({i})"),
            },
            (_, Value::Enum(i)) => format!("unknown({i})"),
        }
    }

    /// The float sent to the device.
    pub fn to_raw(self, value: &Value) -> f32 {
        match (self, value) {
            (_, Value::Bool(b)) => *b as u8 as f32,
            (_, Value::Float(f)) => *f,
            (Kind::Choice(choices), Value::Enum(i)) => choices.get(*i).map_or(0.0, |(_, r)| *r),
            (_, Value::Enum(i)) => *i as f32,
        }
    }

    /// Interpret a float read from the device according to this kind.
    pub fn from_raw(self, raw: f32) -> Value {
        match self {
            Kind::Bool => Value::Bool(raw != 0.0),
            Kind::Float { .. } => Value::Float(raw),
            Kind::Enum(_) => Value::Enum(raw.round().max(0.0) as usize),
            // An unmatched float is shown as a number; `validate` rejects it.
            Kind::Choice(choices) => choices
                .iter()
                .position(|(_, r)| *r == raw)
                .map_or(Value::Float(raw), Value::Enum),
        }
    }
}

impl Param {
    pub fn parse(&self, text: &str) -> Result<Value, Error> {
        self.kind.parse(self.name, text)
    }
    pub fn validate(&self, value: &Value) -> Result<(), Error> {
        self.kind.validate(self.name, value)
    }
    pub fn format(&self, value: &Value) -> String {
        self.kind.format(value)
    }
    pub fn to_raw(&self, value: &Value) -> f32 {
        self.kind.to_raw(value)
    }
    pub fn from_raw(&self, raw: f32) -> Value {
        self.kind.from_raw(raw)
    }
}

/// How a `Switch` row reaches the device. Not a group/id float parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Wire {
    /// `23 <set> <on>` sets, `23 <read>` reads (session 4).
    Toggle { set: u8, read: u8 },
    /// `5a 2c`: 1 line out, 2 amplifier out, 3 both (session 4).
    OutputTarget,
    /// `5a 39`, flag 0x0b (session 4b).
    AutoStandby,
}

impl Wire {
    pub fn get_bytes(self) -> Vec<u8> {
        match self {
            Wire::Toggle { read, .. } => protocol::encode_toggle_get(read).to_vec(),
            Wire::OutputTarget => protocol::encode_output_target_get().to_vec(),
            Wire::AutoStandby => protocol::encode_standby_get().to_vec(),
        }
    }

    pub fn set_bytes(self, raw: f32) -> Vec<u8> {
        match self {
            Wire::Toggle { set, .. } => protocol::encode_toggle_set(set, raw != 0.0).to_vec(),
            Wire::OutputTarget => protocol::encode_output_target_set(raw as u8).to_vec(),
            Wire::AutoStandby => protocol::encode_standby_set(raw != 0.0).to_vec(),
        }
    }

    /// The report that completes a set of `raw`: the toggle echo carrying
    /// the value, or the `5a 02` acknowledgement.
    pub fn is_set_done(self, report: &[u8], raw: f32) -> bool {
        match self {
            Wire::Toggle { set, .. } => {
                protocol::is_toggle_reply(report, set) && (report[4] != 0) == (raw != 0.0)
            }
            Wire::OutputTarget => protocol::is_device_ack(report, protocol::DEVICE_OUTPUT_TARGET),
            Wire::AutoStandby => protocol::is_device_ack(report, protocol::DEVICE_STANDBY),
        }
    }

    /// The reply to `get_bytes`.
    pub fn is_get_reply(self, report: &[u8]) -> bool {
        match self {
            Wire::Toggle { read, .. } => protocol::is_toggle_reply(report, read),
            Wire::OutputTarget => protocol::is_device_value(report, protocol::DEVICE_OUTPUT_TARGET),
            Wire::AutoStandby => protocol::is_device_value(report, protocol::DEVICE_STANDBY),
        }
    }

    /// The raw float in a report that satisfied `is_get_reply`.
    pub fn decode(self, report: &InputReport) -> f32 {
        match self {
            Wire::Toggle { .. } => (report[4] != 0) as u8 as f32,
            Wire::OutputTarget => {
                protocol::decode_device_value(report, protocol::DEVICE_OUTPUT_TARGET)
            }
            Wire::AutoStandby => protocol::decode_device_value(report, protocol::DEVICE_STANDBY),
        }
    }
}

/// A setting that is not a float parameter: headphone and amp toggles and
/// the two `5a` device settings.
#[derive(Debug)]
pub struct Switch {
    pub group: &'static str,
    pub name: &'static str,
    pub wire: Wire,
    pub kind: Kind,
}

const fn switch(group: &'static str, name: &'static str, wire: Wire, kind: Kind) -> Switch {
    Switch {
        group,
        name,
        wire,
        kind,
    }
}

pub static SWITCHES: &[Switch] = &[
    switch(
        "headphones",
        "gain",
        Wire::Toggle {
            set: 0x45,
            read: 0x44,
        },
        Kind::Choice(&[("normal", 0.0), ("high", 1.0)]),
    ),
    switch(
        "headphones",
        "direct-mode",
        Wire::Toggle {
            set: 0x43,
            read: 0x42,
        },
        Kind::Bool,
    ),
    switch(
        "headphones",
        "direct-mode-spdif",
        Wire::Toggle {
            set: 0x4b,
            read: 0x4a,
        },
        Kind::Bool,
    ),
    switch(
        "headphones",
        "high-power-amp",
        Wire::Toggle {
            set: 0x59,
            read: 0x58,
        },
        Kind::Bool,
    ),
    switch(
        "speakers",
        "output-target",
        Wire::OutputTarget,
        Kind::Choice(&[("line-out", 1.0), ("amp-out", 2.0), ("line-and-amp", 3.0)]),
    ),
    switch("speakers", "auto-standby", Wire::AutoStandby, Kind::Bool),
];

pub fn find_switch(group: &str, name: &str) -> Option<&'static Switch> {
    SWITCHES.iter().find(|s| s.group == group && s.name == name)
}

pub fn switches_in_group(group: &str) -> impl Iterator<Item = &'static Switch> {
    let group = group.to_owned();
    SWITCHES.iter().filter(move |s| s.group == group)
}

impl Switch {
    pub fn parse(&self, text: &str) -> Result<Value, Error> {
        self.kind.parse(self.name, text)
    }
    pub fn validate(&self, value: &Value) -> Result<(), Error> {
        self.kind.validate(self.name, value)
    }
    pub fn format(&self, value: &Value) -> String {
        self.kind.format(value)
    }
    pub fn to_raw(&self, value: &Value) -> f32 {
        self.kind.to_raw(value)
    }
    pub fn from_raw(&self, raw: f32) -> Value {
        self.kind.from_raw(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(group: &str, name: &str) -> &'static Param {
        find(group, name).expect("row exists")
    }

    #[test]
    fn table_has_the_spec_rows() {
        assert_eq!(in_group("sbx").count(), 12);
        assert_eq!(in_group("voice").count(), 6);
        let c = p("sbx", "crystalizer-level");
        assert_eq!((c.hid_group, c.get_id, c.set_id), (0x96, 0x08, 0x10));
        let f = p("voice", "focus-angle");
        assert_eq!((f.hid_group, f.get_id, f.set_id), (0x95, 0x08, 0x10));
        assert!(find("sbx", "focus-angle").is_none());
        assert!(find("eq", "bass").is_none());
    }

    #[test]
    fn names_are_unique_within_a_group() {
        for a in PARAMS {
            let dups = PARAMS
                .iter()
                .filter(|b| b.group == a.group && b.name == a.name)
                .count();
            assert_eq!(dups, 1, "{}/{} duplicated", a.group, a.name);
        }
        for s in SWITCHES {
            assert!(
                find(s.group, s.name).is_none(),
                "{}/{} is in both tables",
                s.group,
                s.name
            );
            let dups = SWITCHES
                .iter()
                .filter(|b| b.group == s.group && b.name == s.name)
                .count();
            assert_eq!(dups, 1, "{}/{} duplicated", s.group, s.name);
        }
    }

    #[test]
    fn bool_parse_and_format() {
        let b = p("sbx", "crystalizer");
        assert_eq!(b.parse("on").unwrap(), Value::Bool(true));
        assert_eq!(b.parse("OFF").unwrap(), Value::Bool(false));
        assert!(matches!(b.parse("1"), Err(Error::UnknownValue { .. })));
        assert_eq!(b.format(&Value::Bool(true)), "on");
        assert_eq!(b.to_raw(&Value::Bool(true)), 1.0);
        assert_eq!(b.from_raw(0.0), Value::Bool(false));
        assert_eq!(b.from_raw(1.0), Value::Bool(true));
    }

    #[test]
    fn float_parse_checks_range() {
        let f = p("sbx", "bass-crossover");
        assert_eq!(f.parse("80").unwrap(), Value::Float(80.0));
        assert_eq!(f.parse("10").unwrap(), Value::Float(10.0));
        assert_eq!(f.parse("500").unwrap(), Value::Float(500.0));
        assert!(matches!(f.parse("501"), Err(Error::OutOfRange { .. })));
        assert!(matches!(f.parse("9.99"), Err(Error::OutOfRange { .. })));
        assert!(matches!(f.parse("abc"), Err(Error::UnknownValue { .. })));
        assert_eq!(f.format(&Value::Float(80.0)), "80.00");
        assert_eq!(f.to_raw(&Value::Float(80.0)), 80.0);
        assert_eq!(f.from_raw(42.5), Value::Float(42.5));
    }

    #[test]
    fn float_rejects_nan_and_infinity() {
        let f = p("sbx", "crystalizer-level");
        assert!(matches!(f.parse("nan"), Err(Error::OutOfRange { .. })));
        assert!(matches!(f.parse("inf"), Err(Error::OutOfRange { .. })));
        assert!(matches!(f.parse("-inf"), Err(Error::OutOfRange { .. })));
        assert_eq!(f.parse("-0.0").unwrap(), Value::Float(0.0));
    }

    #[test]
    fn enum_parse_format_and_raw() {
        let m = p("sbx", "smart-volume-mode");
        assert_eq!(m.parse("night").unwrap(), Value::Enum(2));
        assert_eq!(m.parse("Loud").unwrap(), Value::Enum(1));
        assert!(matches!(m.parse("quiet"), Err(Error::UnknownValue { .. })));
        assert_eq!(m.format(&Value::Enum(0)), "normal");
        assert_eq!(m.to_raw(&Value::Enum(2)), 2.0);
        assert_eq!(m.from_raw(1.0), Value::Enum(1));
    }

    #[test]
    fn validate_rejects_wrong_variant_and_range() {
        // Library callers can build a Value directly; set_param must still check it.
        let f = p("sbx", "crystalizer-level");
        assert!(f.validate(&Value::Float(0.5)).is_ok());
        assert!(matches!(
            f.validate(&Value::Float(1.5)),
            Err(Error::OutOfRange { .. })
        ));
        assert!(matches!(
            f.validate(&Value::Bool(true)),
            Err(Error::UnknownValue { .. })
        ));
        let m = p("sbx", "smart-volume-mode");
        assert!(m.validate(&Value::Enum(2)).is_ok());
        assert!(matches!(
            m.validate(&Value::Enum(3)),
            Err(Error::UnknownValue { .. })
        ));
        let b = p("sbx", "crystalizer");
        assert!(matches!(
            b.validate(&Value::Float(1.0)),
            Err(Error::UnknownValue { .. })
        ));
    }

    #[test]
    fn enum_out_of_table_formats_as_unknown() {
        let m = p("sbx", "smart-volume-mode");
        assert_eq!(m.from_raw(5.0), Value::Enum(5));
        assert_eq!(m.format(&Value::Enum(5)), "unknown(5)");
        assert_eq!(m.from_raw(-1.0), Value::Enum(0));
    }

    #[test]
    fn eq_rows_match_session_5() {
        assert_eq!(in_group("eq").count(), 12);
        let e = p("eq", "enabled");
        assert_eq!(
            (e.hid_group, e.get_id, e.set_id, e.kind),
            (0x96, 0x09, 0x12, Kind::Bool)
        );
        let pre = p("eq", "preamp");
        assert_eq!((pre.get_id, pre.set_id), (0x0a, 0x14));
        assert_eq!(
            pre.kind,
            Kind::Float {
                min: -6.0,
                max: 6.0
            }
        );
        let bands = eq_bands();
        assert_eq!(bands[0].name, "31hz");
        assert_eq!(bands[9].name, "16khz");
        for (i, b) in bands.iter().enumerate() {
            assert_eq!(b.get_id, 0x0b + i as u8, "{}", b.name);
            assert_eq!(b.set_id, 0x16 + 2 * i as u8, "{}", b.name);
            assert_eq!(
                b.kind,
                Kind::Float {
                    min: -12.0,
                    max: 12.0
                }
            );
        }
    }

    #[test]
    fn rock_preset_first_band_encodes_as_captured() {
        // 05-eq.txt 11.709 s: band 31 Hz to -1 dB.
        let rock = eq_preset("rock").unwrap();
        assert_eq!(
            rock.bands,
            [-1.0, -1.0, 1.0, 2.0, -1.0, -1.0, 0.0, 0.0, 4.0, 4.0]
        );
        let band = eq_bands()[0];
        assert_eq!(
            crate::protocol::encode_set(band.hid_group, band.set_id, rock.bands[0]),
            [
                0x20, 0x00, 0x16, 0x0a, 0xd5, 0x02, 0x08, 0x16, 0x20, 0x96, 0xbf, 0x80, 0x00, 0x00
            ]
        );
        assert!(eq_preset("Rock").is_some(), "case-insensitive");
        assert!(eq_preset("metal").is_none());
    }

    #[test]
    fn the_ten_presets_are_in_list_order_and_tell_apart() {
        let names: Vec<&str> = EQ_PRESETS.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "acoustic",
                "classical",
                "country",
                "dance",
                "flat",
                "jazz",
                "new-age",
                "pop",
                "rock",
                "vocal"
            ]
        );
        for (i, a) in EQ_PRESETS.iter().enumerate() {
            for b in &EQ_PRESETS[i + 1..] {
                assert_ne!(
                    a.bands, b.bands,
                    "{} and {} share one band set",
                    a.name, b.name
                );
            }
            assert!(
                a.bands.iter().all(|db| (-12.0..=12.0).contains(db)),
                "{} has a band outside the slider range",
                a.name
            );
        }
    }

    #[test]
    fn eq_preset_matching_is_exact() {
        assert_eq!(eq_preset_matching([0.0; 10]).unwrap().name, "flat");
        let rock = eq_preset("rock").unwrap();
        assert_eq!(eq_preset_matching(rock.bands).unwrap().name, "rock");
        let mut off = rock.bands;
        off[3] = 2.5;
        assert!(eq_preset_matching(off).is_none());
    }

    fn sw(group: &str, name: &str) -> &'static Switch {
        find_switch(group, name).expect("switch exists")
    }

    #[test]
    fn choice_parse_format_and_raw() {
        let g = sw("headphones", "gain");
        assert_eq!(
            g.wire,
            Wire::Toggle {
                set: 0x45,
                read: 0x44
            }
        );
        assert_eq!(g.parse("high").unwrap(), Value::Enum(1));
        assert_eq!(g.parse("NORMAL").unwrap(), Value::Enum(0));
        assert!(matches!(g.parse("loud"), Err(Error::UnknownValue { .. })));
        assert_eq!(g.to_raw(&Value::Enum(1)), 1.0);
        assert_eq!(g.from_raw(0.0), Value::Enum(0));
        assert_eq!(g.format(&Value::Enum(1)), "high");
        // An unmatched raw is shown as a number and can never be sent.
        assert_eq!(g.from_raw(3.0), Value::Float(3.0));
        assert_eq!(g.format(&Value::Float(3.0)), "3.00");
        assert!(matches!(
            g.validate(&Value::Float(3.0)),
            Err(Error::UnknownValue { .. })
        ));
        assert!(matches!(
            g.validate(&Value::Enum(2)),
            Err(Error::UnknownValue { .. })
        ));
        assert!(g.validate(&Value::Enum(1)).is_ok());
        let t = sw("speakers", "output-target");
        assert_eq!(t.wire, Wire::OutputTarget);
        assert_eq!(t.parse("line-and-amp").unwrap(), Value::Enum(2));
        assert_eq!(t.to_raw(&Value::Enum(2)), 3.0);
        assert_eq!(t.from_raw(1.0), Value::Enum(0));
    }

    #[test]
    fn switch_table_matches_session_4() {
        assert_eq!(switches_in_group("headphones").count(), 4);
        assert_eq!(switches_in_group("speakers").count(), 2);
        assert_eq!(
            sw("headphones", "direct-mode").wire,
            Wire::Toggle {
                set: 0x43,
                read: 0x42
            }
        );
        assert_eq!(
            sw("headphones", "direct-mode-spdif").wire,
            Wire::Toggle {
                set: 0x4b,
                read: 0x4a
            }
        );
        assert_eq!(
            sw("headphones", "high-power-amp").wire,
            Wire::Toggle {
                set: 0x59,
                read: 0x58
            }
        );
        assert_eq!(sw("speakers", "auto-standby").wire, Wire::AutoStandby);
        assert!(find_switch("sbx", "gain").is_none());
    }

    #[test]
    fn wire_bytes_match_the_captures() {
        let gain = Wire::Toggle {
            set: 0x45,
            read: 0x44,
        };
        assert_eq!(gain.set_bytes(1.0), vec![0x23, 0x45, 0x01]);
        assert_eq!(gain.set_bytes(0.0), vec![0x23, 0x45, 0x00]);
        assert_eq!(gain.get_bytes(), vec![0x23, 0x44]);
        assert!(gain.is_set_done(&[0x00, 0x23, 0x45, 0x00, 0x01], 1.0));
        assert!(
            !gain.is_set_done(&[0x00, 0x23, 0x44, 0x00, 0x01], 1.0),
            "read form is not the echo"
        );
        assert!(
            !gain.is_set_done(&[0x00, 0x23, 0x45, 0x00, 0x00], 1.0),
            "other value"
        );
        assert!(gain.is_get_reply(&[0x00, 0x23, 0x44, 0x00, 0x01]));
        let mut r = [0u8; 16];
        r[..5].copy_from_slice(&[0x00, 0x23, 0x44, 0x00, 0x01]);
        assert_eq!(gain.decode(&r), 1.0);
        assert_eq!(
            Wire::OutputTarget.set_bytes(3.0),
            vec![0x5a, 0x2c, 0x05, 0x00, 0x03]
        );
        assert_eq!(Wire::OutputTarget.get_bytes(), vec![0x5a, 0x2c, 0x01, 0x01]);
        assert!(
            Wire::OutputTarget.is_set_done(&[0x00, 0x5a, 0x02, 0x0a, 0x2c, 0x00, 0x00, 0x03], 3.0)
        );
        assert!(Wire::OutputTarget.is_get_reply(&[0x00, 0x5a, 0x2c, 0x05, 0x01, 0x03, 0, 0, 0]));
        assert_eq!(
            Wire::AutoStandby.set_bytes(1.0),
            vec![0x5a, 0x39, 0x03, 0x00, 0x0b, 0x01]
        );
        assert_eq!(Wire::AutoStandby.get_bytes(), vec![0x5a, 0x39, 0x01, 0x01]);
        assert!(Wire::AutoStandby.is_set_done(&[0x00, 0x5a, 0x02, 0x02, 0x39], 1.0));
        let mut on = [0u8; 16];
        on[..7].copy_from_slice(&[0x00, 0x5a, 0x39, 0x05, 0x01, 0x00, 0x08]);
        assert!(Wire::AutoStandby.is_get_reply(&on));
        assert_eq!(Wire::AutoStandby.decode(&on), 1.0);
        let mut off = [0u8; 16];
        off[..5].copy_from_slice(&[0x00, 0x5a, 0x39, 0x05, 0x01]);
        assert_eq!(Wire::AutoStandby.decode(&off), 0.0);
    }

    #[test]
    fn speaker_rows_match_session_4b() {
        assert_eq!(in_group("speakers").count(), 6);
        let l = p("speakers", "layout");
        assert_eq!((l.hid_group, l.get_id, l.set_id), (0x80, 0x04, 0x08));
        assert_eq!(l.parse("5.1").unwrap(), Value::Enum(7));
        assert_eq!(l.to_raw(&Value::Enum(4)), 5.0);
        assert_eq!(l.from_raw(7.0), Value::Enum(6));
        assert_eq!(l.format(&Value::Enum(0)), "2.0");
        // 2.1 is raw 2.0: one front pair plus the subwoofer.
        assert_eq!(l.to_raw(&l.parse("2.1").unwrap()), 2.0);
        assert_eq!(l.format(&l.from_raw(2.0)), "2.1");
        for (name, raw) in [("3.0", 3.0), ("3.1", 4.0), ("4.1", 6.0)] {
            assert_eq!(l.to_raw(&l.parse(name).unwrap()), raw, "{name}");
        }
        // Raw 1.0 is the unticked Full-Range box (04b: "Front Full-Range off").
        let f = p("speakers", "front-full-range");
        assert_eq!((f.get_id, f.set_id), (0x1a, 0x34));
        assert_eq!(f.to_raw(&f.parse("off").unwrap()), 1.0);
        assert_eq!(f.from_raw(0.0), Value::Enum(0));
        assert_eq!(f.format(&f.from_raw(0.0)), "on");
        let r = p("speakers", "rear-full-range");
        assert_eq!((r.get_id, r.set_id), (0x1b, 0x36));
        assert_eq!(p("speakers", "bass-redirection").set_id, 0x2a);
        let c = p("speakers", "crossover");
        assert_eq!((c.get_id, c.set_id), (0x16, 0x2c));
        assert_eq!(
            c.kind,
            Kind::Float {
                min: 10.0,
                max: 500.0
            }
        );
        assert_eq!(p("speakers", "subwoofer-gain").get_id, 0x1e);
    }
}
