//! A fake X7 for running the GUI without the device (`x7-gui --fake`).
//!
//! One thread plays the device over a datagram socket pair: it reads the
//! 65-byte Output reports the session writes, keeps every setting in memory,
//! and answers with the same 16-byte Input reports the real X7 sends
//! (`docs/protocol.md`). Unknown commands get no reply, as on the device.
//! Nothing here touches hidraw.

use std::collections::HashMap;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixDatagram;

use crate::protocol::{self, Feature, InputReport};
use crate::settings::{self, Kind, Wire};
use crate::transport::Device;

/// Open a fake device. The thread behind it ends when the `Device` drops.
pub fn device() -> Device {
    let (ours, theirs) = UnixDatagram::pair().expect("socket pair");
    std::thread::Builder::new()
        .name("x7-fake".into())
        .spawn(move || Fake::new().run(theirs))
        .expect("spawn fake device");
    Device::from_file(File::from(OwnedFd::from(ours)))
}

struct Fake {
    output: protocol::Output,
    features: u32,
    /// Raw float per (hid_group, get_id).
    params: HashMap<(u8, u8), f32>,
    /// Toggle state per read id.
    toggles: HashMap<u8, bool>,
    output_target: u8,
    auto_standby: bool,
}

/// A plausible power-on value for a row: levels at two thirds, bands flat,
/// other ranges (the crossover) mid-way, choices at their first entry.
fn initial(kind: Kind) -> f32 {
    match kind {
        Kind::Bool | Kind::Enum(_) => 0.0,
        Kind::Float { min, .. } if min < 0.0 => 0.0,
        Kind::Float { max, .. } if max <= 1.0 => 0.67,
        Kind::Float { min, max } => (min + max) / 2.0,
        Kind::Choice(choices) => choices[0].1,
    }
}

fn report(bytes: &[u8]) -> InputReport {
    let mut r = [0u8; 16];
    r[..bytes.len()].copy_from_slice(bytes);
    r
}

impl Fake {
    fn new() -> Fake {
        let params = settings::PARAMS
            .iter()
            .map(|p| ((p.hid_group, p.get_id), initial(p.kind)))
            .collect();
        let toggles = settings::SWITCHES
            .iter()
            .filter_map(|s| match s.wire {
                Wire::Toggle { read, .. } => Some((read, false)),
                _ => None,
            })
            .collect();
        Fake {
            output: protocol::Output::Headphones,
            features: 1 << (Feature::Sbx as u8 - 1),
            params,
            toggles,
            output_target: 2,
            auto_standby: true,
        }
    }

    fn run(mut self, sock: UnixDatagram) {
        let mut buf = [0u8; 65];
        // A read error means the session closed its end.
        while let Ok(n) = sock.recv(&mut buf) {
            let cmd = &buf[1..n.max(1)];
            for reply in self.handle(cmd) {
                if sock.send(&reply).is_err() {
                    return;
                }
            }
        }
    }

    /// The reports the device sends for one command, in order.
    fn handle(&mut self, cmd: &[u8]) -> Vec<InputReport> {
        match cmd {
            [0x26, 0x01, group, id, ..] => {
                let raw = self.params.get(&(*group, *id)).copied().unwrap_or(0.0);
                let f = raw.to_be_bytes();
                vec![report(&[
                    0x00, 0x26, 0x01, 0x00, *group, *id, f[0], f[1], f[2], f[3],
                ])]
            }
            [
                0x20,
                0x00,
                0x16,
                0x0a,
                0xd5,
                0x02,
                0x08,
                set_id,
                0x20,
                group,
                f0,
                f1,
                f2,
                f3,
                ..,
            ] => {
                let raw = f32::from_be_bytes([*f0, *f1, *f2, *f3]);
                if let Some(p) = settings::PARAMS
                    .iter()
                    .find(|p| p.hid_group == *group && p.set_id == *set_id)
                {
                    self.params.insert((p.hid_group, p.get_id), raw);
                }
                vec![report(&[0x00, 0x20])]
            }
            [0x23, 0x4d, ..] => vec![self.output_reply()],
            [0x23, 0x4e, 0x01, ..] => {
                self.output = protocol::Output::Headphones;
                vec![report(&[0x00, 0x23, 0x4e, 0x00, 0x01])]
            }
            [0x23, 0x4e, 0x00, 0x00, 0x00, 0x80, ..] => {
                self.output = protocol::Output::Speakers;
                vec![report(&[0x00, 0x23, 0x4e, 0x00, 0x00, 0x00, 0x00, 0x80])]
            }
            [0x23, 0x24, ..] => {
                let m = self.features.to_le_bytes();
                vec![report(&[0x00, 0x23, 0x24, 0x00, m[0], m[1], m[2], m[3]])]
            }
            [0x23, 0x23, id, on, ..] => {
                let bit = 1u32
                    .checked_shl(u32::from(*id).wrapping_sub(1))
                    .unwrap_or(0);
                if *on != 0 {
                    self.features |= bit;
                } else {
                    self.features &= !bit;
                }
                // The echo of a host-sent set carries zeros where a push
                // carries the mask.
                vec![report(&[0x00, 0x23, 0x23, 0x00, *id, *on])]
            }
            [0x5a, protocol::DEVICE_OUTPUT_TARGET, 0x05, 0x00, v, ..] => {
                self.output_target = *v;
                vec![report(&[
                    0x00,
                    0x5a,
                    0x02,
                    0x0a,
                    protocol::DEVICE_OUTPUT_TARGET,
                    0x00,
                    0x00,
                    *v,
                ])]
            }
            [0x5a, protocol::DEVICE_OUTPUT_TARGET, 0x01, 0x01, ..] => vec![report(&[
                0x00,
                0x5a,
                protocol::DEVICE_OUTPUT_TARGET,
                0x05,
                0x01,
                self.output_target,
            ])],
            [0x5a, protocol::DEVICE_STANDBY, 0x03, 0x00, 0x0b, on, ..] => {
                self.auto_standby = *on != 0;
                vec![report(&[0x00, 0x5a, 0x02, 0x02, protocol::DEVICE_STANDBY])]
            }
            [0x5a, protocol::DEVICE_STANDBY, 0x01, 0x01, ..] => {
                let m = ((self.auto_standby as u32) << 0x0b).to_le_bytes();
                vec![report(&[
                    0x00,
                    0x5a,
                    protocol::DEVICE_STANDBY,
                    0x05,
                    0x01,
                    m[0],
                    m[1],
                    m[2],
                    m[3],
                ])]
            }
            // A toggle set is answered with the read form, then the echo.
            [0x23, set, on, ..] if self.toggles.contains_key(&(set - 1)) => {
                let read = set - 1;
                let on = *on != 0;
                self.toggles.insert(read, on);
                vec![
                    report(&[0x00, 0x23, read, 0x00, on as u8]),
                    report(&[0x00, 0x23, *set, 0x00, on as u8]),
                ]
            }
            // Set and read ids never collide (read is set - 1), so the
            // zero-padded report can match on the id alone.
            [0x23, read, ..] if self.toggles.contains_key(read) => {
                vec![report(&[0x00, 0x23, *read, 0x00, self.toggles[read] as u8])]
            }
            _ => vec![],
        }
    }

    fn output_reply(&self) -> InputReport {
        let v = match self.output {
            protocol::Output::Headphones => 1,
            protocol::Output::Speakers => 2,
        };
        report(&[0x00, 0x23, 0x4d, 0x00, v])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{PARAMS, SWITCHES, Value};
    use crate::{Command, Link, Session};
    use std::time::{Duration, Instant};

    fn param(name: &str) -> &'static settings::Param {
        PARAMS.iter().find(|p| p.name == name).unwrap()
    }

    fn switch(name: &str) -> &'static settings::Switch {
        SWITCHES.iter().find(|s| s.name == name).unwrap()
    }

    #[test]
    fn every_row_reads_back_and_a_set_is_read_back_changed() {
        let mut dev = device();
        for p in PARAMS {
            crate::get_param(&mut dev, p).unwrap();
        }
        for s in SWITCHES {
            crate::get_switch(&mut dev, s).unwrap();
        }
        let band = param("62hz");
        crate::set_param(&mut dev, band, &Value::Float(4.0)).unwrap();
        assert_eq!(crate::get_param(&mut dev, band).unwrap(), Value::Float(4.0));

        let gain = switch("gain");
        crate::set_switch(&mut dev, gain, &Value::Enum(1)).unwrap();
        assert_eq!(crate::get_switch(&mut dev, gain).unwrap(), Value::Enum(1));

        let target = switch("output-target");
        crate::set_switch(&mut dev, target, &Value::Enum(2)).unwrap();
        assert_eq!(crate::get_switch(&mut dev, target).unwrap(), Value::Enum(2));

        let standby = switch("auto-standby");
        crate::set_switch(&mut dev, standby, &Value::Bool(false)).unwrap();
        assert_eq!(
            crate::get_switch(&mut dev, standby).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn output_and_features_follow_their_sets() {
        let mut dev = device();
        assert_eq!(
            crate::get_output(&mut dev).unwrap(),
            protocol::Output::Headphones
        );
        crate::set_output(&mut dev, protocol::Output::Speakers).unwrap();
        assert_eq!(
            crate::get_output(&mut dev).unwrap(),
            protocol::Output::Speakers
        );

        let mask = crate::get_features(&mut dev).unwrap();
        assert!(protocol::feature_enabled(mask, Feature::Sbx));
        assert!(!protocol::feature_enabled(mask, Feature::Scout));
        crate::set_feature(&mut dev, Feature::Scout, true).unwrap();
        let mask = crate::get_features(&mut dev).unwrap();
        assert!(protocol::feature_enabled(mask, Feature::Scout));
    }

    #[test]
    fn a_session_over_the_fake_settles_ready_with_every_row() {
        let session = Session::start_with_opener(
            Box::new(|| Ok(device())),
            Duration::from_millis(10),
            Box::new(|| {}),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.state().link != Link::Ready {
            assert!(Instant::now() < deadline, "{:?}", session.state());
            std::thread::sleep(Duration::from_millis(10));
        }
        let s = session.state();
        assert_eq!(s.params.len(), PARAMS.len());
        assert_eq!(s.switches.len(), SWITCHES.len());
        assert_eq!(s.output, Some(protocol::Output::Headphones));

        session.send(Command::SetParam(param("bass"), Value::Bool(true)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while session.state().param(param("bass")) != Some(Value::Bool(true)) {
            assert!(Instant::now() < deadline, "{:?}", session.state());
            std::thread::sleep(Duration::from_millis(10));
        }
        session.shutdown();
    }
}
