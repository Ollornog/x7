//! Words for the window. The settings tables carry CLI identifiers
//! (`surround-level`, `amp-out`); this module turns them into labels,
//! units and readouts. GUI-only: the CLI keeps the identifiers.

use x7::settings::{Kind, Param};

/// Page title for a CLI group.
pub fn page(group: &str) -> &'static str {
    match group {
        "sbx" => "SBX",
        "eq" => "Equalizer",
        "headphones" => "Headphones",
        "speakers" => "Speakers",
        "voice" => "CrystalVoice",
        _ => leak(group),
    }
}

/// Label for a row. Level rows share their feature's label because the
/// window shows the pair on one line.
pub fn row(group: &str, name: &str) -> &'static str {
    match (group, name) {
        ("sbx", "enabled") => "SBX",
        ("sbx", "surround" | "surround-level") => "Surround",
        ("sbx", "crystalizer" | "crystalizer-level") => "Crystalizer",
        ("sbx", "bass" | "bass-level") => "Bass",
        ("sbx", "bass-crossover") => "Crossover",
        ("sbx", "smart-volume" | "smart-volume-level") => "Smart Volume",
        ("sbx", "smart-volume-mode") => "Mode",
        ("sbx", "dialog-plus" | "dialog-plus-level") => "Dialog Plus",
        ("voice", "enabled") => "CrystalVoice",
        ("voice", "noise-reduction") => "Noise reduction",
        ("voice", "echo-cancellation") => "Echo cancellation",
        ("voice", "focus" | "focus-angle") => "Focus",
        ("voice", "smart-volume" | "smart-volume-level") => "Smart Volume",
        ("eq", "enabled") => "Equalizer",
        ("eq", "preamp") => "Pre",
        ("eq", "31hz") => "31",
        ("eq", "62hz") => "62",
        ("eq", "125hz") => "125",
        ("eq", "250hz") => "250",
        ("eq", "500hz") => "500",
        ("eq", "1khz") => "1k",
        ("eq", "2khz") => "2k",
        ("eq", "4khz") => "4k",
        ("eq", "8khz") => "8k",
        ("eq", "16khz") => "16k",
        ("headphones", "gain") => "Gain",
        ("headphones", "direct-mode") => "Direct Mode",
        ("headphones", "direct-mode-spdif") => "Direct Mode for S/PDIF in",
        ("headphones", "high-power-amp") => "High Power Amplification",
        ("speakers", "layout") => "Layout",
        ("speakers", "output-target") => "Output target",
        ("speakers", "front-full-range") => "Front full-range",
        ("speakers", "rear-full-range") => "Rear full-range",
        ("speakers", "bass-redirection") => "Bass redirection",
        ("speakers", "crossover") => "Crossover",
        ("speakers", "subwoofer-gain") => "Subwoofer gain",
        ("speakers", "auto-standby") => "Auto standby",
        _ => leak(name),
    }
}

/// Display name for an `Enum` or `Choice` value.
pub fn value(group: &str, name: &str, value: &str) -> &'static str {
    match (group, name, value) {
        ("sbx", "smart-volume-mode", "normal") => "Normal",
        ("sbx", "smart-volume-mode", "loud") => "Loud",
        ("sbx", "smart-volume-mode", "night") => "Night",
        ("speakers", "layout", "2.0") => "2.0 stereo",
        ("speakers", "layout", "4.0") => "4.0 quad",
        ("speakers", "layout", "5.0") => "5.0 surround",
        ("speakers", "layout", "5.1") => "5.1 surround",
        ("speakers", "front-full-range" | "rear-full-range", "on") => "On",
        ("speakers", "front-full-range" | "rear-full-range", "off") => "Off",
        ("headphones", "gain", "normal") => "Normal, 32 to 300 Ω",
        ("headphones", "gain", "high") => "High, 600 Ω",
        ("speakers", "output-target", "line-out") => "Line out",
        ("speakers", "output-target", "amp-out") => "Amplifier out",
        ("speakers", "output-target", "line-and-amp") => "Line and amplifier",
        _ => leak(value),
    }
}

/// Unit of a float row, which decides its readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Percent,
    Hertz,
    Degrees,
    Decibel,
}

pub fn unit(p: &Param) -> Option<Unit> {
    match p.kind {
        Kind::Float { min, max } if min == 0.0 && max == 1.0 => Some(Unit::Percent),
        Kind::Float { .. } if p.group == "eq" => Some(Unit::Decibel),
        Kind::Float { .. } if p.name.ends_with("crossover") => Some(Unit::Hertz),
        Kind::Float { .. } if p.name == "focus-angle" => Some(Unit::Degrees),
        _ => None,
    }
}

/// The readout column text, padded so the column never changes width.
pub fn readout(unit: Unit, raw: f32) -> String {
    match unit {
        Unit::Percent => format!("{:>3.0} %", raw * 100.0),
        Unit::Hertz => format!("{raw:>3.0} Hz"),
        Unit::Degrees => format!("{raw:>3.0}°"),
        Unit::Decibel => {
            // Round first so -0.4 and -0.0 both read as zero with a plus.
            let n = raw.round();
            let n = if n == 0.0 { 0.0 } else { n };
            format!("{n:>+3.0}")
        }
    }
}

/// Unknown identifiers come back as themselves. They are `'static` in
/// practice (table names), but the signature takes `&str`, so the rare
/// unknown is leaked rather than panicking. Never hit by a table row.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use x7::settings::{PARAMS, SWITCHES};

    #[test]
    fn every_row_has_a_label_that_is_not_its_identifier() {
        for p in PARAMS {
            let l = row(p.group, p.name);
            assert_ne!(l, p.name, "{}.{} has no label", p.group, p.name);
        }
        for s in SWITCHES {
            let l = row(s.group, s.name);
            assert_ne!(l, s.name, "{}.{} has no label", s.group, s.name);
        }
        for g in ["sbx", "voice", "eq"] {
            assert_ne!(row(g, "enabled"), "enabled");
        }
    }

    #[test]
    fn every_enum_and_choice_name_has_a_value_label() {
        let kinds = PARAMS
            .iter()
            .map(|p| (p.group, p.name, p.kind))
            .chain(SWITCHES.iter().map(|s| (s.group, s.name, s.kind)));
        for (g, n, k) in kinds {
            let names: Vec<&str> = match k {
                Kind::Enum(names) => names.to_vec(),
                Kind::Choice(choices) => choices.iter().map(|(n, _)| *n).collect(),
                _ => continue,
            };
            for v in names {
                let l = value(g, n, v);
                assert_ne!(l, v, "{g}.{n} value {v} has no label");
            }
        }
    }

    #[test]
    fn every_float_row_has_a_unit() {
        for p in PARAMS {
            if let Kind::Float { .. } = p.kind {
                assert!(unit(p).is_some(), "{}.{} has no unit", p.group, p.name);
            }
        }
    }

    #[test]
    fn units_by_row() {
        let f = |g, n| unit(x7::settings::find(g, n).unwrap()).unwrap();
        assert_eq!(f("sbx", "surround-level"), Unit::Percent);
        assert_eq!(f("sbx", "bass-crossover"), Unit::Hertz);
        assert_eq!(f("speakers", "crossover"), Unit::Hertz);
        assert_eq!(f("voice", "focus-angle"), Unit::Degrees);
        assert_eq!(f("eq", "1khz"), Unit::Decibel);
        assert_eq!(f("eq", "preamp"), Unit::Decibel);
    }

    #[test]
    fn readouts_are_padded_to_the_column() {
        assert_eq!(readout(Unit::Percent, 0.67), " 67 %");
        assert_eq!(readout(Unit::Percent, 1.0), "100 %");
        assert_eq!(readout(Unit::Hertz, 80.0), " 80 Hz");
        assert_eq!(readout(Unit::Hertz, 500.0), "500 Hz");
        assert_eq!(readout(Unit::Degrees, 90.0), " 90°");
        assert_eq!(readout(Unit::Decibel, 4.0), " +4");
        assert_eq!(readout(Unit::Decibel, -12.0), "-12");
    }

    #[test]
    fn readout_normalises_negative_zero() {
        assert_eq!(readout(Unit::Decibel, -0.0), " +0");
        assert_eq!(readout(Unit::Decibel, -0.4), " +0");
    }

    #[test]
    fn unknown_inputs_fall_through_unchanged() {
        assert_eq!(row("sbx", "nosuch"), "nosuch");
        assert_eq!(value("sbx", "smart-volume-mode", "nosuch"), "nosuch");
        assert_eq!(page("nosuch"), "nosuch");
    }

    #[test]
    fn page_names() {
        assert_eq!(page("sbx"), "SBX");
        assert_eq!(page("eq"), "Equalizer");
        assert_eq!(page("voice"), "CrystalVoice");
        assert_eq!(page("headphones"), "Headphones");
        assert_eq!(page("speakers"), "Speakers");
    }
}
