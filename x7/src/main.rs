use std::process::exit;

use clap::{Parser, Subcommand};
use x7::protocol::feature_enabled;
use x7::settings::Param;
use x7::{Device, Error, Feature, Output, Value, settings};

#[derive(Parser)]
#[command(
    name = "x7",
    version,
    about = "Control a Creative Sound Blaster X7 over USB"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the active output, or switch it
    Output {
        #[arg(value_parser = ["headphones", "speakers"])]
        target: Option<String>,
    },
    /// SBX Pro Studio: `x7 sbx` lists, `x7 sbx on|off`, `x7 sbx <name> [<value>]`
    Sbx {
        name: Option<String>,
        value: Option<String>,
    },
    /// CrystalVoice: `x7 voice` lists, `x7 voice on|off`, `x7 voice <name> [<value>]`
    Voice {
        name: Option<String>,
        value: Option<String>,
    },
    /// Equalizer: `x7 eq` lists, `x7 eq on|off`, `x7 eq preset [<name>]`, `x7 eq <band> [<dB>]`
    Eq {
        name: Option<String>,
        /// Band gains are negative dB half the time, so `-3` is a value here.
        #[arg(allow_negative_numbers = true)]
        value: Option<String>,
    },
    /// Headphones: `x7 headphones` lists, `x7 headphones <name> [<value>]`
    Headphones {
        name: Option<String>,
        value: Option<String>,
    },
    /// Speakers: `x7 speakers` lists, `x7 speakers <name> [<value>]`
    Speakers {
        name: Option<String>,
        value: Option<String>,
    },
    /// Scout Mode: `x7 scout` shows, `x7 scout on|off` sets
    Scout { state: Option<String> },
    /// Profiles: `x7 profile save <file>`, `x7 profile load <file>`; a bare
    /// name without `/` or `.` means `~/.config/x7/profiles/<name>.toml`
    Profile {
        #[command(subcommand)]
        action: ProfileCmd,
    },
    /// Safe-mode state without sending anything: device node, recordings,
    /// hang marker, log file
    Status,
}

#[derive(Subcommand)]
enum ProfileCmd {
    /// Read every setting from the device into a file
    Save { file: String },
    /// Apply every setting in a file to the device
    Load {
        file: String,
        /// Apply high headphone gain or High Power Amplification if the
        /// file has them (they can harm headphones under 600 ohm)
        #[arg(long)]
        yes: bool,
    },
    /// List saved profiles
    List,
    /// Rename a saved profile; the new name must be free
    Rename { from: String, to: String },
    /// Delete a saved profile
    Delete { name: String },
}

fn main() {
    // Rust ignores SIGPIPE, so `x7 sbx | head -1` would panic on the closed
    // pipe. Restore the default so the process exits quietly like other CLIs.
    // SAFETY: called before any other thread exists; SigDfl needs no handler.
    unsafe {
        let _ = nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGPIPE,
            nix::sys::signal::SigHandler::SigDfl,
        );
    }
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("x7: {e}");
        exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Error> {
    // Commands that never touch the device run before it is opened.
    match &cli.cmd {
        Cmd::Status => {
            for (k, v) in x7::transport::status()? {
                println!("{k:<10} {v}");
            }
            return Ok(());
        }
        Cmd::Profile {
            action: ProfileCmd::List,
        } => {
            for name in x7::profile::list() {
                println!("{name}");
            }
            return Ok(());
        }
        Cmd::Profile {
            action: ProfileCmd::Rename { from, to },
        } => {
            x7::profile::rename(from, to)?;
            println!("renamed {} to {}", from.trim(), to.trim());
            return Ok(());
        }
        Cmd::Profile {
            action: ProfileCmd::Delete { name },
        } => {
            x7::profile::delete(name)?;
            println!("deleted {}", name.trim());
            return Ok(());
        }
        Cmd::Profile {
            action: ProfileCmd::Load { file, yes: false },
        } => {
            // Refuse before opening the device, so nothing is sent.
            let risky = x7::profile::load(&profile_path(file)?)?.risky();
            if !risky.is_empty() {
                return Err(Error::Profile(format!(
                    "{} sets {}, which can harm headphones under 600 ohm; add --yes to apply it",
                    file,
                    risky.join(", ")
                )));
            }
        }
        _ => {}
    }
    let mut dev = Device::open()?;
    match cli.cmd {
        Cmd::Output { target: None } => {
            println!("{}", output_name(x7::get_output(&mut dev)?));
        }
        Cmd::Output { target: Some(t) } => {
            let output = if t == "speakers" {
                Output::Speakers
            } else {
                Output::Headphones
            };
            x7::set_output(&mut dev, output)?;
            println!("{}", output_name(x7::get_output(&mut dev)?));
        }
        Cmd::Sbx { name, value } => {
            group(&mut dev, "sbx", Master::Feature(Feature::Sbx), name, value)?
        }
        Cmd::Voice { name, value } => group(
            &mut dev,
            "voice",
            Master::Feature(Feature::CrystalVoice),
            name,
            value,
        )?,
        Cmd::Eq { name, value } if name.as_deref() == Some("preset") => eq_preset(&mut dev, value)?,
        Cmd::Eq { name, value } => group(&mut dev, "eq", Master::Row(eq_enabled()), name, value)?,
        Cmd::Headphones { name, value } => {
            group(&mut dev, "headphones", Master::None, name, value)?
        }
        Cmd::Speakers { name, value } => group(&mut dev, "speakers", Master::None, name, value)?,
        Cmd::Scout { state: None } => {
            let mask = x7::get_features(&mut dev)?;
            println!("scout {}", on_off(feature_enabled(mask, Feature::Scout)));
        }
        Cmd::Profile {
            action: ProfileCmd::Save { file },
        } => {
            let path = profile_path(&file)?;
            let p = x7::Profile::read_device(&mut dev)?;
            x7::profile::save(&path, &p)?;
            println!("saved {} settings to {}", p.entries().len(), path.display());
        }
        Cmd::Profile {
            action: ProfileCmd::Load { file, .. },
        } => {
            let p = x7::profile::load(&profile_path(&file)?)?;
            for e in p.entries() {
                e.apply(&mut dev)?;
                println!("{}", e.line());
            }
        }
        Cmd::Status
        | Cmd::Profile {
            action: ProfileCmd::List | ProfileCmd::Rename { .. } | ProfileCmd::Delete { .. },
        } => unreachable!("handled before the device is opened"),
        Cmd::Scout { state: Some(state) } => {
            let on = master_state(&state).ok_or_else(|| Error::UnknownValue {
                name: "scout",
                value: state.clone(),
            })?;
            x7::set_feature(&mut dev, Feature::Scout, on)?;
            println!("scout {}", on_off(on));
        }
    }
    Ok(())
}

fn output_name(o: Output) -> &'static str {
    match o {
        Output::Headphones => "headphones",
        Output::Speakers => "speakers",
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// `on` / `off` in any case, or None if the text is something else (a
/// parameter name, for instance).
fn master_state(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// A bare name is a profile in the shared directory; anything with a `/`
/// or a `.` is a path as typed.
fn profile_path(file: &str) -> Result<std::path::PathBuf, Error> {
    if file.contains('/') || file.contains('.') {
        Ok(std::path::PathBuf::from(file))
    } else {
        x7::profile::path_for(file)
    }
}

/// What `on|off` and the first listing line mean for a group.
#[derive(Clone, Copy)]
enum Master {
    /// A `23 23` feature: sbx, voice.
    Feature(Feature),
    /// A Bool row: the EQ's `enabled`.
    Row(&'static Param),
    /// No master: headphones, speakers.
    None,
}

fn eq_enabled() -> &'static Param {
    settings::find("eq", "enabled").expect("eq enabled row")
}

/// Shared handler for every `<group> [name [value]]` subcommand.
fn group(
    dev: &mut Device,
    group: &'static str,
    master: Master,
    name: Option<String>,
    value: Option<String>,
) -> Result<(), Error> {
    match (name.as_deref(), value) {
        (None, _) => {
            match master {
                Master::Feature(f) => {
                    let mask = x7::get_features(dev)?;
                    println!("{group} {}", on_off(feature_enabled(mask, f)));
                }
                Master::Row(p) => {
                    let v = x7::get_param(dev, p)?;
                    println!("{group} {}", p.format(&v));
                }
                Master::None => {}
            }
            for p in settings::in_group(group) {
                if matches!(master, Master::Row(m) if std::ptr::eq(m, p)) {
                    continue; // already printed as the master line
                }
                let v = x7::get_param(dev, p)?;
                println!("{} {}", p.name, p.format(&v));
            }
            for s in settings::switches_in_group(group) {
                let v = x7::get_switch(dev, s)?;
                println!("{} {}", s.name, s.format(&v));
            }
        }
        (Some(state), None) if master_state(state).is_some() && !matches!(master, Master::None) => {
            let on = master_state(state).unwrap_or(false);
            match master {
                Master::Feature(f) => x7::set_feature(dev, f, on)?,
                Master::Row(p) => x7::set_param(dev, p, &Value::Bool(on))?,
                Master::None => {}
            }
            println!("{group} {}", on_off(on));
        }
        (Some(n), value) => {
            if let Some(p) = settings::find(group, n) {
                if let Some(text) = value {
                    x7::set_param(dev, p, &p.parse(&text)?)?;
                }
                let v = x7::get_param(dev, p)?;
                println!("{} {}", p.name, p.format(&v));
            } else if let Some(s) = settings::find_switch(group, n) {
                if let Some(text) = value {
                    x7::set_switch(dev, s, &s.parse(&text)?)?;
                }
                let v = x7::get_switch(dev, s)?;
                println!("{} {}", s.name, s.format(&v));
            } else {
                return Err(Error::UnknownParam {
                    group,
                    name: n.to_owned(),
                });
            }
        }
    }
    Ok(())
}

/// `x7 eq preset [<name>]`: write a preset (preamp 0, then the ten bands,
/// as the Windows app does), then print the preset the bands now match.
fn eq_preset(dev: &mut Device, name: Option<String>) -> Result<(), Error> {
    if let Some(name) = name {
        let preset = settings::eq_preset(&name).ok_or_else(|| {
            let known: Vec<&str> = settings::EQ_PRESETS.iter().map(|p| p.name).collect();
            Error::UnknownValue {
                name: "preset",
                value: format!("{name} (presets: {})", known.join(", ")),
            }
        })?;
        let preamp = settings::find("eq", "preamp").expect("eq preamp row");
        x7::set_param(dev, preamp, &Value::Float(0.0))?;
        for (band, db) in settings::eq_bands().iter().zip(preset.bands) {
            x7::set_param(dev, band, &Value::Float(db))?;
        }
    }
    let mut bands = [0.0; 10];
    for (slot, band) in bands.iter_mut().zip(settings::eq_bands()) {
        *slot = band.to_raw(&x7::get_param(dev, band)?);
    }
    let name = settings::eq_preset_matching(bands).map_or("custom", |p| p.name);
    println!("preset {name}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_path_resolves_bare_names_only() {
        assert_eq!(
            profile_path("a/b.toml").unwrap(),
            std::path::PathBuf::from("a/b.toml")
        );
        assert_eq!(
            profile_path("./x").unwrap(),
            std::path::PathBuf::from("./x")
        );
        assert!(
            profile_path("gaming")
                .unwrap()
                .ends_with("x7/profiles/gaming.toml")
        );
        assert!(profile_path("").is_err());
    }

    #[test]
    fn master_state_is_case_insensitive() {
        assert_eq!(master_state("on"), Some(true));
        assert_eq!(master_state("OFF"), Some(false));
        assert_eq!(master_state("Off"), Some(false));
        assert_eq!(master_state("crystalizer"), None);
        assert_eq!(master_state(""), None);
    }
}
