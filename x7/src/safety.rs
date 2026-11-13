//! Safe mode: what the transport checks before it writes to the X7.
//!
//! An X7 once stopped answering after a single read (`23 4d`): the HID
//! `SET_REPORT` never completed, and from then on the device answered no
//! control request at all, not even a USB reset, until its power was cut.
//! Shortly before, the same device had been recording, and the host
//! controller had reported buffer overruns on its capture endpoint. The
//! firmware version was unknown. See `docs/safety.md`.
//!
//! So the transport (1) sends nothing while a capture stream of the X7's
//! ALSA card is open, (2) sends a write only after a read was answered,
//! (3) treats a missing reply or a failed or slow write as a hang: it stops
//! at once, never retries, never resets, and leaves a marker so that no
//! other x7 process talks to the same enumeration of the device either.
//! A power cycle re-enumerates the X7 with a new device number, which
//! clears the marker by itself.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Which recordings to look for before a write.
#[derive(Clone, Debug)]
pub enum Capture {
    /// No check: the in-memory fake and test devices.
    Off,
    /// `/proc/asound/cardN` of the X7.
    Card(PathBuf),
    /// The X7 has no ALSA card bound: refuse, the device is not in its
    /// normal state.
    Missing,
}

impl Capture {
    /// The capture substreams that are open, as `pcm0c/sub0: RUNNING`.
    /// Empty means none. `Missing` is an error so callers refuse.
    pub fn open_streams(&self) -> io::Result<Vec<String>> {
        match self {
            Capture::Off => Ok(Vec::new()),
            Capture::Missing => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the X7 has no ALSA card bound",
            )),
            Capture::Card(dir) => open_captures(dir),
        }
    }
}

/// Every `pcm*c/sub*/status` under a card directory that is not `closed`.
pub fn open_captures(card: &Path) -> io::Result<Vec<String>> {
    let mut open = Vec::new();
    let mut pcms: Vec<_> = fs::read_dir(card)?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("pcm") && n.ends_with('c'))
        .collect();
    pcms.sort();
    for pcm in pcms {
        let mut subs: Vec<_> = fs::read_dir(card.join(&pcm))?
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("sub"))
            .collect();
        subs.sort();
        for sub in subs {
            let status = fs::read_to_string(card.join(&pcm).join(&sub).join("status"))?;
            let first = status.lines().next().unwrap_or("").trim();
            if first != "closed" {
                let state = status
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("state:"))
                    .map_or(first, str::trim);
                open.push(format!("{pcm}/{sub}: {state}"));
            }
        }
    }
    Ok(open)
}

/// One enumeration of the device: bus and device number plus the boot,
/// so a reboot or a re-plug is a different id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceId {
    pub bus: u32,
    pub dev: u32,
    pub boot: String,
}

impl DeviceId {
    pub fn key(&self) -> String {
        format!("{} {} {}", self.bus, self.dev, self.boot)
    }
}

/// Where a hidraw node sits in sysfs, resolved once at open.
#[derive(Clone, Debug)]
pub struct Location {
    pub id: Option<DeviceId>,
    pub capture: Capture,
}

/// From `/sys/class/hidraw/hidrawN` up to the USB device: its bus and
/// device number, and the ALSA card bound to one of its interfaces.
/// `proc_asound` is `/proc/asound` outside tests.
pub fn locate(hidraw_sysfs: &Path, proc_asound: &Path, boot_id: &str) -> Location {
    let usb = fs::canonicalize(hidraw_sysfs.join("device"))
        .ok()
        .and_then(|hid| hid.parent().and_then(Path::parent).map(Path::to_path_buf));
    let Some(usb) = usb else {
        return Location {
            id: None,
            capture: Capture::Missing,
        };
    };
    let read_num = |name: &str| {
        fs::read_to_string(usb.join(name))
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
    };
    let id = match (read_num("busnum"), read_num("devnum")) {
        (Some(bus), Some(dev)) => Some(DeviceId {
            bus,
            dev,
            boot: boot_id.trim().to_owned(),
        }),
        _ => None,
    };
    Location {
        id,
        capture: find_card(&usb)
            .map(|n| Capture::Card(proc_asound.join(format!("card{n}"))))
            .unwrap_or(Capture::Missing),
    }
}

/// `<usb>/<usb-name>:<cfg>.<if>/sound/cardN` for any interface.
fn find_card(usb: &Path) -> Option<u32> {
    let name = usb.file_name()?.to_string_lossy().into_owned();
    let prefix = format!("{name}:");
    let mut ifaces: Vec<_> = fs::read_dir(usb)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
        .map(|e| e.path())
        .collect();
    ifaces.sort();
    ifaces.iter().find_map(|iface| {
        fs::read_dir(iface.join("sound"))
            .ok()?
            .filter_map(Result::ok)
            .find_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_prefix("card")
                    .and_then(|n| n.parse().ok())
            })
    })
}

pub fn boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default()
}

/// The hang marker: one file whose first line is the `DeviceId` key of the
/// enumeration that stopped answering, the second line the reason.
#[derive(Clone, Debug)]
pub struct Marker {
    path: PathBuf,
}

impl Marker {
    pub fn at(path: PathBuf) -> Marker {
        Marker { path }
    }

    /// `$XDG_RUNTIME_DIR/x7/hang`, else `~/.cache/x7/hang`.
    pub fn default_path() -> Option<PathBuf> {
        if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(d).join("x7/hang"));
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/x7/hang"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The reason recorded for this enumeration, if it hung.
    pub fn hung(&self, id: &DeviceId) -> Option<String> {
        let text = fs::read_to_string(&self.path).ok()?;
        let mut lines = text.lines();
        (lines.next()? == id.key()).then(|| lines.next().unwrap_or("").to_owned())
    }

    /// A marker for another enumeration is stale: the device was power
    /// cycled or re-plugged since. Remove it.
    pub fn clear_if_stale(&self, id: &DeviceId) -> io::Result<bool> {
        match fs::read_to_string(&self.path) {
            Ok(text) if text.lines().next() != Some(&id.key()) => {
                fs::remove_file(&self.path)?;
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn set(&self, id: &DeviceId, reason: &str) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("new");
        fs::write(
            &tmp,
            format!("{}\n{}\n", id.key(), reason.replace('\n', " ")),
        )?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("x7-safety-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn status(card: &Path, pcm: &str, sub: &str, text: &str) {
        let d = card.join(pcm).join(sub);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("status"), text).unwrap();
    }

    #[test]
    fn closed_captures_and_playback_streams_do_not_count() {
        let card = tmp("closed");
        status(&card, "pcm0c", "sub0", "closed\n");
        status(&card, "pcm1c", "sub0", "closed\n");
        status(&card, "pcm0p", "sub0", "state: RUNNING\nowner_pid   : 1\n");
        assert!(open_captures(&card).unwrap().is_empty());
    }

    #[test]
    fn an_open_capture_is_listed_with_its_state() {
        let card = tmp("open");
        status(&card, "pcm0c", "sub0", "closed\n");
        status(&card, "pcm1c", "sub0", "state: RUNNING\nowner_pid   : 42\n");
        assert_eq!(open_captures(&card).unwrap(), ["pcm1c/sub0: RUNNING"]);
    }

    #[test]
    fn a_missing_card_refuses() {
        assert!(Capture::Missing.open_streams().is_err());
        assert!(Capture::Off.open_streams().unwrap().is_empty());
    }

    /// A sysfs tree like the real one: hidraw -> HID device under the HID
    /// interface, which sits in the USB device next to the audio interface.
    fn sysfs(root: &Path, card: Option<u32>) -> PathBuf {
        let usb = root.join("devices/usb1/1-2");
        let hid = usb.join("1-2:1.4/0003:041E:323A.0001");
        fs::create_dir_all(&hid).unwrap();
        fs::write(usb.join("busnum"), "1\n").unwrap();
        fs::write(usb.join("devnum"), "7\n").unwrap();
        if let Some(n) = card {
            fs::create_dir_all(usb.join(format!("1-2:1.0/sound/card{n}"))).unwrap();
        }
        let hidraw = root.join("class/hidraw/hidraw0");
        fs::create_dir_all(&hidraw).unwrap();
        std::os::unix::fs::symlink(&hid, hidraw.join("device")).unwrap();
        hidraw
    }

    #[test]
    fn locate_finds_bus_device_and_card() {
        let root = tmp("locate");
        let hidraw = sysfs(&root, Some(1));
        let loc = locate(&hidraw, Path::new("/proc/asound"), "abc\n");
        assert_eq!(
            loc.id,
            Some(DeviceId {
                bus: 1,
                dev: 7,
                boot: "abc".into()
            })
        );
        assert!(matches!(loc.capture, Capture::Card(p) if p == Path::new("/proc/asound/card1")));
    }

    #[test]
    fn locate_without_a_card_is_missing() {
        let root = tmp("nocard");
        let hidraw = sysfs(&root, None);
        let loc = locate(&hidraw, Path::new("/proc/asound"), "abc");
        assert!(matches!(loc.capture, Capture::Missing));
    }

    #[test]
    fn marker_holds_for_its_enumeration_only() {
        let dir = tmp("marker");
        let m = Marker::at(dir.join("x7/hang"));
        let id = DeviceId {
            bus: 1,
            dev: 2,
            boot: "b".into(),
        };
        assert_eq!(m.hung(&id), None);
        m.set(&id, "no reply to 23 4d").unwrap();
        assert_eq!(m.hung(&id).as_deref(), Some("no reply to 23 4d"));
        assert!(!m.clear_if_stale(&id).unwrap());
        // Power cycled: a new device number.
        let fresh = DeviceId {
            dev: 7,
            ..id.clone()
        };
        assert_eq!(m.hung(&fresh), None);
        assert!(m.clear_if_stale(&fresh).unwrap());
        assert_eq!(m.hung(&id), None);
    }
}
