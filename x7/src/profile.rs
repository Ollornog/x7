//! Profiles: every row's value in a TOML file, saved and loaded as a set.
//!
//! Sections are the CLI groups, keys are the row names, `enabled` is the
//! group's master (the `23 23` feature for sbx and voice, the `enabled`
//! row for eq). Output and Scout Mode are not part of a profile.

use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value as Toml};

use crate::Error;
use crate::protocol::{Feature, feature_enabled};
use crate::session::{Command, State};
use crate::settings::{self, Kind, Param, Switch, Value};
use crate::transport::Device;

/// Groups in file and apply order, with the feature behind `enabled`.
const GROUPS: [(&str, Option<Feature>); 5] = [
    ("sbx", Some(Feature::Sbx)),
    ("voice", Some(Feature::CrystalVoice)),
    ("eq", None),
    ("headphones", None),
    ("speakers", None),
];

#[derive(Clone, Debug)]
pub enum Entry {
    /// A group master: `sbx on`.
    Master(&'static str, Feature, bool),
    Param(&'static Param, Value),
    Switch(&'static Switch, Value),
}

impl Entry {
    /// `name value`, as the CLI listings print it.
    pub fn line(&self) -> String {
        match self {
            Entry::Master(group, _, on) => {
                format!("{group} {}", if *on { "on" } else { "off" })
            }
            // The eq `enabled` row is the group's master line: `eq on`.
            Entry::Param(p, v) if p.name == "enabled" => format!("{} {}", p.group, p.format(v)),
            Entry::Param(p, v) => format!("{} {}", p.name, p.format(v)),
            Entry::Switch(s, v) => format!("{} {}", s.name, s.format(v)),
        }
    }

    /// The session command that applies this entry.
    pub fn command(&self) -> Command {
        match self {
            Entry::Master(_, f, on) => Command::SetFeature(*f, *on),
            Entry::Param(p, v) => Command::SetParam(p, v.clone()),
            Entry::Switch(s, v) => Command::SetSwitch(s, v.clone()),
        }
    }

    /// Apply this entry over the request functions (CLI path).
    pub fn apply(&self, dev: &mut Device) -> Result<(), Error> {
        match self {
            Entry::Master(_, f, on) => crate::set_feature(dev, *f, *on),
            Entry::Param(p, v) => crate::set_param(dev, p, v),
            Entry::Switch(s, v) => crate::set_switch(dev, s, v),
        }
    }
}

/// Every row in order; the unit `Profile::from_state` and `read_device` walk.
enum Key {
    Master(&'static str, Feature),
    Param(&'static Param),
    Switch(&'static Switch),
}

fn keys() -> Vec<Key> {
    let mut keys = Vec::new();
    for (group, master) in GROUPS {
        if let Some(f) = master {
            keys.push(Key::Master(group, f));
        }
        keys.extend(settings::in_group(group).map(Key::Param));
        keys.extend(settings::switches_in_group(group).map(Key::Switch));
    }
    keys
}

#[derive(Clone, Debug, Default)]
pub struct Profile {
    entries: Vec<Entry>,
}

impl Profile {
    /// Entries in apply order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Lines of entries that can harm headphones when applied unasked:
    /// High headphone gain (600 ohm only) and High Power Amplification on.
    /// The GUI asks before applying them; `x7 profile load` needs `--yes`.
    pub fn risky(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| match e {
                Entry::Switch(s, v) => {
                    matches!(
                        (s.group, s.name),
                        ("headphones", "gain") | ("headphones", "high-power-amp")
                    ) && s.to_raw(v) != 0.0
                }
                _ => false,
            })
            .map(Entry::line)
            .collect()
    }

    /// Every row the state knows a value for; rows still unknown are skipped,
    /// as is a `Choice` row whose device value matches no name.
    pub fn from_state(state: &State) -> Profile {
        let entries = keys()
            .into_iter()
            .filter_map(|k| match k {
                Key::Master(group, f) => Some(Entry::Master(group, f, state.feature(f))),
                Key::Param(p) => state
                    .param(p)
                    .filter(|v| p.validate(v).is_ok())
                    .map(|v| Entry::Param(p, v)),
                Key::Switch(s) => state
                    .switch(s)
                    .filter(|v| s.validate(v).is_ok())
                    .map(|v| Entry::Switch(s, v)),
            })
            .collect();
        Profile { entries }
    }

    /// Every row read from the device, except a `Choice` row whose value
    /// matches no name: it cannot be written back, so it is left out.
    pub fn read_device(dev: &mut Device) -> Result<Profile, Error> {
        let mask = crate::get_features(dev)?;
        let mut entries = Vec::new();
        for k in keys() {
            match k {
                Key::Master(group, f) => {
                    entries.push(Entry::Master(group, f, feature_enabled(mask, f)))
                }
                Key::Param(p) => {
                    let v = crate::get_param(dev, p)?;
                    if p.validate(&v).is_ok() {
                        entries.push(Entry::Param(p, v));
                    }
                }
                Key::Switch(s) => {
                    let v = crate::get_switch(dev, s)?;
                    if s.validate(&v).is_ok() {
                        entries.push(Entry::Switch(s, v));
                    }
                }
            }
        }
        Ok(Profile { entries })
    }

    pub fn to_toml(&self) -> String {
        let mut root = Table::new();
        for e in &self.entries {
            let (group, name, value) = match e {
                Entry::Master(group, _, on) => (*group, "enabled", Toml::Boolean(*on)),
                Entry::Param(p, v) => (p.group, p.name, toml_value(p.kind, v)),
                Entry::Switch(s, v) => (s.group, s.name, toml_value(s.kind, v)),
            };
            root.entry(group)
                .or_insert_with(|| Toml::Table(Table::new()))
                .as_table_mut()
                .expect("sections are tables")
                .insert(name.to_owned(), value);
        }
        format!("# Sound Blaster X7 profile, written by x7\n\n{root}")
    }

    /// Resolve every key to a row and validate every value; nothing is
    /// applied if any line is wrong. Missing keys are skipped.
    pub fn parse(text: &str) -> Result<Profile, Error> {
        let table: Table = text
            .parse()
            .map_err(|e: toml::de::Error| Error::Profile(e.to_string()))?;
        for key in table.keys() {
            if !GROUPS.iter().any(|(g, _)| *g == key.as_str()) {
                return Err(Error::Profile(format!("unknown section [{key}]")));
            }
        }
        let mut entries = Vec::new();
        for (group, master) in GROUPS {
            let Some(section) = table.get(group) else {
                continue;
            };
            let section = section
                .as_table()
                .ok_or_else(|| Error::Profile(format!("[{group}] is not a section")))?;
            for key in section.keys() {
                let known = (master.is_some() && key == "enabled")
                    || settings::find(group, key).is_some()
                    || settings::find_switch(group, key).is_some();
                if !known {
                    return Err(Error::Profile(format!(
                        "[{group}] has no setting named '{key}'"
                    )));
                }
            }
            if let Some(f) = master
                && let Some(v) = section.get("enabled")
            {
                let on = v
                    .as_bool()
                    .ok_or_else(|| wrong(group, "enabled", "true or false", v))?;
                entries.push(Entry::Master(group, f, on));
            }
            for p in settings::in_group(group) {
                if let Some(v) = section.get(p.name) {
                    entries.push(Entry::Param(p, convert(group, p.name, p.kind, v)?));
                }
            }
            for s in settings::switches_in_group(group) {
                if let Some(v) = section.get(s.name) {
                    entries.push(Entry::Switch(s, convert(group, s.name, s.kind, v)?));
                }
            }
        }
        Ok(Profile { entries })
    }
}

fn toml_value(kind: Kind, v: &Value) -> Toml {
    match v {
        Value::Bool(b) => Toml::Boolean(*b),
        // Through the f32 Display so 0.67 stays 0.67, not 0.6700000166893005.
        Value::Float(f) => Toml::Float(f.to_string().parse().unwrap_or(f64::from(*f))),
        Value::Enum(_) => Toml::String(kind.format(v)),
    }
}

fn wrong(group: &str, name: &str, expected: &str, got: &Toml) -> Error {
    Error::Profile(format!("[{group}] {name}: expected {expected}, got {got}"))
}

fn convert(group: &str, name: &'static str, kind: Kind, v: &Toml) -> Result<Value, Error> {
    let value = match kind {
        Kind::Bool => Value::Bool(
            v.as_bool()
                .ok_or_else(|| wrong(group, name, "true or false", v))?,
        ),
        Kind::Float { .. } => {
            let f = v
                .as_float()
                .or_else(|| v.as_integer().map(|i| i as f64))
                .ok_or_else(|| wrong(group, name, "a number", v))?;
            Value::Float(f as f32)
        }
        Kind::Enum(_) | Kind::Choice(_) => kind.parse(
            name,
            v.as_str()
                .ok_or_else(|| wrong(group, name, "a name in quotes", v))?,
        )?,
    };
    kind.validate(name, &value)?;
    Ok(value)
}

/// `$XDG_CONFIG_HOME/x7/profiles`, or `~/.config/x7/profiles`.
pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("x7").join("profiles")
}

/// The file for a profile name inside `dir()`. A name is one path component
/// without dots, so the CLI can tell a name from a path: no separators, no
/// `.`, not empty.
pub fn path_for(name: &str) -> Result<PathBuf, Error> {
    path_in(&dir(), name)
}

fn path_in(dir: &Path, name: &str) -> Result<PathBuf, Error> {
    let name = name.trim();
    if name.is_empty() || name.contains(['/', '\\', '.']) {
        return Err(Error::Profile(format!("'{name}' is not a profile name")));
    }
    Ok(dir.join(format!("{name}.toml")))
}

/// Give a saved profile a new name. Never overwrites: the new name must be
/// free.
pub fn rename(from: &str, to: &str) -> Result<(), Error> {
    rename_in(&dir(), from, to)
}

fn rename_in(dir: &Path, from: &str, to: &str) -> Result<(), Error> {
    let src = path_in(dir, from)?;
    let dst = path_in(dir, to)?;
    if !src.is_file() {
        return Err(Error::Profile(format!(
            "no profile named '{}'",
            from.trim()
        )));
    }
    if dst.exists() {
        return Err(Error::Profile(format!(
            "a profile named '{}' exists already",
            to.trim()
        )));
    }
    fs::rename(&src, &dst)?;
    Ok(())
}

/// Delete a saved profile by name. Only files in the profiles directory.
pub fn delete(name: &str) -> Result<(), Error> {
    delete_in(&dir(), name)
}

fn delete_in(dir: &Path, name: &str) -> Result<(), Error> {
    let path = path_in(dir, name)?;
    if !path.is_file() {
        return Err(Error::Profile(format!(
            "no profile named '{}'",
            name.trim()
        )));
    }
    fs::remove_file(&path)?;
    Ok(())
}

/// Saved profile names, sorted.
pub fn list() -> Vec<String> {
    list_in(&dir())
}

/// Names `load` accepts: `*.toml` stems that `path_for` resolves, so a
/// hand-placed `my.v2.toml` is not offered and then refused.
fn list_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .filter_map(|p| p.file_stem()?.to_str().map(str::to_owned))
                // `path_for` trims, so " desk" would load desk.toml: keep
                // only stems it leaves unchanged.
                .filter(|name| name.trim() == name && path_for(name).is_ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Write through a temporary file next to the destination and rename it
/// into place, so a crash mid-write never leaves a truncated profile.
pub fn save(path: &Path, profile: &Profile) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    let written = fs::write(&tmp, profile.to_toml()).and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Profile, Error> {
    Profile::parse(&fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Link, State};
    use crate::settings::{self, Wire};

    /// A state with one value per group, enough to cover every entry kind.
    fn state() -> State {
        let mut s = State {
            link: Link::Ready,
            features: 0x0001_0001, // sbx and voice on
            ..State::default()
        };
        let put = |s: &mut State, group: &str, name: &str, raw: f32| {
            let p = settings::find(group, name).unwrap();
            s.params.insert((p.hid_group, p.get_id), raw);
        };
        put(&mut s, "sbx", "surround-level", 0.67);
        put(&mut s, "sbx", "smart-volume-mode", 2.0);
        put(&mut s, "eq", "enabled", 1.0);
        put(&mut s, "eq", "31hz", -1.0);
        put(&mut s, "speakers", "layout", 5.0);
        put(&mut s, "speakers", "front-full-range", 1.0);
        s.switches.insert(
            Wire::Toggle {
                set: 0x45,
                read: 0x44,
            },
            1.0,
        );
        s.switches.insert(Wire::AutoStandby, 0.0);
        s
    }

    fn lines(p: &Profile) -> Vec<String> {
        p.entries().iter().map(Entry::line).collect()
    }

    #[test]
    fn from_state_keeps_file_order_and_skips_unknown_rows() {
        let p = Profile::from_state(&state());
        assert_eq!(
            lines(&p),
            vec![
                "sbx on",
                "surround-level 0.67",
                "smart-volume-mode night",
                "voice on",
                "eq on",
                "31hz -1.00",
                "gain high",
                "layout 4.0",
                "front-full-range off",
                "auto-standby off",
            ]
        );
    }

    #[test]
    fn to_toml_round_trips_through_parse() {
        let p = Profile::from_state(&state());
        let text = p.to_toml();
        assert!(text.starts_with("# Sound Blaster X7 profile"), "{text}");
        assert!(
            text.contains(
                "[sbx]\nenabled = true\nsurround-level = 0.67\nsmart-volume-mode = \"night\"\n"
            ),
            "{text}"
        );
        assert!(
            text.contains("[eq]\nenabled = true\n31hz = -1.0\n"),
            "{text}"
        );
        assert!(text.contains("[headphones]\ngain = \"high\"\n"), "{text}");
        assert!(
            text.contains(
                "[speakers]\nlayout = \"4.0\"\nfront-full-range = \"off\"\nauto-standby = false\n"
            ),
            "{text}"
        );
        let back = Profile::parse(&text).unwrap();
        assert_eq!(lines(&back), lines(&p));
    }

    #[test]
    fn parse_orders_entries_by_table_not_by_file() {
        let text = "[speakers]\nfront-full-range = \"off\"\nlayout = \"5.1\"\n[sbx]\nsurround = false\nenabled = true\n";
        let p = Profile::parse(text).unwrap();
        assert_eq!(
            lines(&p),
            vec![
                "sbx on",
                "surround off",
                "layout 5.1",
                "front-full-range off"
            ]
        );
    }

    #[test]
    fn parse_rejects_unknown_section_key_and_type() {
        let e = Profile::parse("[mixer]\nvolume = 1\n").unwrap_err();
        assert!(e.to_string().contains("[mixer]"), "{e}");
        let e = Profile::parse("[sbx]\nloudness = true\n").unwrap_err();
        assert!(e.to_string().contains("loudness"), "{e}");
        let e = Profile::parse("[sbx]\nsurround = \"yes\"\n").unwrap_err();
        assert!(e.to_string().contains("surround"), "{e}");
        let e = Profile::parse("[headphones]\nenabled = true\n").unwrap_err();
        assert!(
            e.to_string().contains("enabled"),
            "no master in headphones: {e}"
        );
        let e = Profile::parse("[sbx]\nsurround-level = 1.5\n").unwrap_err();
        assert!(matches!(e, Error::OutOfRange { .. }), "{e}");
        let e = Profile::parse("[headphones]\ngain = \"loud\"\n").unwrap_err();
        assert!(matches!(e, Error::UnknownValue { .. }), "{e}");
        let e = Profile::parse("not toml at all = = =\n").unwrap_err();
        assert!(matches!(e, Error::Profile(_)), "{e}");
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("x7-profiles-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn rename_moves_a_profile_and_never_overwrites() {
        let d = tmp_dir("rename");
        fs::write(d.join("evening.toml"), "[eq]\nenabled = true\n").unwrap();
        fs::write(d.join("keep.toml"), "[eq]\nenabled = false\n").unwrap();
        rename_in(&d, "evening", "night").unwrap();
        assert_eq!(list_in(&d), ["keep", "night"]);
        let e = rename_in(&d, "night", "keep").unwrap_err();
        assert!(e.to_string().contains("exists already"), "{e}");
        assert_eq!(
            fs::read_to_string(d.join("keep.toml")).unwrap(),
            "[eq]\nenabled = false\n",
            "the existing profile is untouched"
        );
        assert!(rename_in(&d, "missing", "x").is_err());
        assert!(rename_in(&d, "night", "../escape").is_err());
        assert!(rename_in(&d, "night", "a.b").is_err());
        assert_eq!(list_in(&d), ["keep", "night"]);
    }

    #[test]
    fn delete_removes_only_the_named_profile() {
        let d = tmp_dir("delete");
        fs::write(d.join("a.toml"), "").unwrap();
        fs::write(d.join("b.toml"), "").unwrap();
        delete_in(&d, "a").unwrap();
        assert_eq!(list_in(&d), ["b"]);
        assert!(delete_in(&d, "a").is_err(), "gone already");
        assert!(delete_in(&d, "../b").is_err());
        assert_eq!(list_in(&d), ["b"]);
    }

    #[test]
    fn risky_lists_high_gain_and_high_power_only() {
        let p = Profile::parse(
            "[headphones]\ngain = \"high\"\nhigh-power-amp = true\ndirect-mode = true\n",
        )
        .unwrap();
        assert_eq!(p.risky(), ["gain high", "high-power-amp on"]);
        let p =
            Profile::parse("[headphones]\ngain = \"normal\"\nhigh-power-amp = false\n").unwrap();
        assert!(p.risky().is_empty());
    }

    /// Load another profile, then the first one again, over the CLI path
    /// against the in-memory X7: every value comes back. A profile holds
    /// only settings the device accepts back, nothing one-way.
    #[test]
    fn loading_a_profile_and_loading_back_restores_every_value() {
        let mut dev = crate::fake::device();
        let before = Profile::read_device(&mut dev).unwrap();
        let other = Profile::parse(
            "[sbx]\nenabled = true\nsurround = true\nsurround-level = 0.3\n\
             [eq]\nenabled = false\n31hz = -4.0\n\
             [speakers]\nlayout = \"2.1\"\ncrossover = 80.0\noutput-target = \"line-and-amp\"\n",
        )
        .unwrap();
        for e in other.entries() {
            e.apply(&mut dev).unwrap();
        }
        let changed = Profile::read_device(&mut dev).unwrap();
        assert_ne!(changed.to_toml(), before.to_toml());
        for e in before.entries() {
            e.apply(&mut dev).unwrap();
        }
        let after = Profile::read_device(&mut dev).unwrap();
        assert_eq!(after.to_toml(), before.to_toml());
    }

    #[test]
    fn unmatched_choice_value_is_left_out() {
        // A layout float the table does not list (9.0, no such layout) is
        // shown by the CLI as a number but cannot be applied, so a profile
        // never contains it.
        let mut s = state();
        let l = settings::find("speakers", "layout").unwrap();
        s.params.insert((l.hid_group, l.get_id), 9.0);
        let p = Profile::from_state(&s);
        assert!(
            !lines(&p).iter().any(|l| l.starts_with("layout")),
            "{:?}",
            lines(&p)
        );
        assert!(Profile::parse(&p.to_toml()).is_ok());
    }

    #[test]
    fn integer_is_accepted_for_a_float_row() {
        let p = Profile::parse("[eq]\npreamp = 0\n1khz = -3\n").unwrap();
        assert_eq!(lines(&p), vec!["preamp 0.00", "1khz -3.00"]);
    }

    #[test]
    fn partial_file_yields_only_its_entries() {
        let p = Profile::parse("[voice]\nenabled = false\n").unwrap();
        assert_eq!(lines(&p), vec!["voice off"]);
        let p = Profile::parse("").unwrap();
        assert!(p.entries().is_empty());
    }

    #[test]
    fn entry_commands_match_their_rows() {
        let p = Profile::parse(
            "[speakers]\nlayout = \"4.0\"\n[sbx]\nenabled = false\nsurround = true\n[headphones]\ngain = \"high\"\n",
        )
        .unwrap();
        let cmds: Vec<String> = p
            .entries()
            .iter()
            .map(|e| format!("{:?}", e.command()))
            .collect();
        assert!(cmds[0].starts_with("SetFeature(Sbx, false)"), "{}", cmds[0]);
        assert!(cmds[1].starts_with("SetParam("), "{}", cmds[1]);
        assert!(cmds[2].starts_with("SetSwitch("), "{}", cmds[2]);
        // The layout is a plain parameter row (round 2 device check 2).
        assert!(
            cmds[3].starts_with("SetParam(") && cmds[3].contains("\"layout\""),
            "{}",
            cmds[3]
        );
    }

    #[test]
    fn path_for_rejects_names_that_leave_the_directory() {
        assert!(
            path_for("gaming")
                .unwrap()
                .ends_with("x7/profiles/gaming.toml")
        );
        assert!(path_for("../etc").is_err());
        assert!(path_for(".hidden").is_err());
        assert!(
            path_for("preset.v2").is_err(),
            "a dot would make the CLI read it as a path"
        );
        assert!(path_for("a/b").is_err());
        assert!(path_for("").is_err());
        assert!(path_for("  ").is_err());
    }

    #[test]
    fn save_and_load_through_a_temp_dir() {
        let dir = std::env::temp_dir().join(format!("x7-profile-test-{}", std::process::id()));
        let path = dir.join("one.toml");
        let p = Profile::from_state(&state());
        save(&path, &p).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(lines(&back), lines(&p));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(load(&path), Err(Error::Io(_))));
    }

    #[test]
    fn save_replaces_the_file_atomically() {
        use std::os::unix::fs::MetadataExt;
        let dir = std::env::temp_dir().join(format!("x7-profile-atomic-{}", std::process::id()));
        let path = dir.join("one.toml");
        let p = Profile::from_state(&state());
        save(&path, &p).unwrap();
        let first = std::fs::metadata(&path).unwrap().ino();
        save(&path, &p).unwrap();
        // A rename over the destination gives a new inode; a truncating
        // write keeps the old one and leaves a half-written file on a crash.
        assert_ne!(std::fs::metadata(&path).unwrap().ino(), first);
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["one.toml"], "no temporary file is left behind");
        // A destination that cannot be replaced fails without leaving a temp.
        let blocked = dir.join("sub");
        std::fs::create_dir(&blocked).unwrap();
        assert!(matches!(save(&blocked, &p), Err(Error::Io(_))));
        assert!(!dir.join("sub.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_leaves_out_names_load_would_refuse() {
        let dir = std::env::temp_dir().join(format!("x7-profiles-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["desk.toml", "my.v2.toml", "notes.txt", " desk.toml"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let names = list_in(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            names,
            vec!["desk".to_owned()],
            "a dotted stem fails path_for; a padded stem would load another file"
        );
    }
}
