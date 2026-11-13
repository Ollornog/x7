//! The hidraw device: find it, write Output reports, read Input reports.
//!
//! Commands are 64-byte Output reports. The X7 has no interrupt OUT endpoint,
//! so a hidraw `write()` becomes the `SET_REPORT` control transfer the Windows
//! app used. Replies and device-initiated events are 16-byte Input reports on
//! the interrupt IN endpoint, which hidraw `read()` returns one at a time.
//!
//! Every write goes through the safe mode in `crate::safety`: no write while
//! the X7 records, writes only after an answered read, and a hang (no reply,
//! a failed or a slow write) stops all traffic to this enumeration of the
//! device without any retry. Every step is logged (`crate::log`).

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout};

use crate::Error;
use crate::log::{Log, hex};
use crate::protocol::{self, InputReport};
use crate::safety::{self, Capture, DeviceId, Marker};

const HID_ID_LINE: &str = "HID_ID=0003:0000041E:0000323A";
const OUTPUT_LEN: usize = 64;
pub const REPLY_TIMEOUT: Duration = Duration::from_millis(500);
/// A healthy `SET_REPORT` completes in a few ms; the kernel gives up after
/// 5 s. A write slower than this counts as a hang even if it completed.
pub const SLOW_WRITE: Duration = Duration::from_millis(1000);

/// What the transport reads and writes: a hidraw node, or a socket for the
/// fake and the tests. A trait so a test can stand in for a device whose
/// control transfer hangs.
pub trait Port: Send {
    fn write_report(&mut self, buf: &[u8]) -> io::Result<()>;
    /// One report into `buf`, waiting up to `timeout`. `None` on timeout,
    /// `Some(0)` at EOF.
    fn read_report(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<Option<usize>>;
}

impl Port for File {
    fn write_report(&mut self, buf: &[u8]) -> io::Result<()> {
        self.write_all(buf)
    }

    fn read_report(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
        let mut fds = [PollFd::new(self.as_fd(), PollFlags::POLLIN)];
        let timeout = PollTimeout::try_from(timeout).unwrap_or(PollTimeout::MAX);
        if nix::poll::poll(&mut fds, timeout).map_err(io::Error::from)? == 0 {
            return Ok(None);
        }
        Ok(Some(self.read(buf)?))
    }
}

pub struct Device {
    port: Box<dyn Port>,
    capture: Capture,
    /// The enumeration and where its hang marker lives; `None` for fakes.
    marker: Option<(DeviceId, Marker)>,
    /// A read was answered: writes may go out.
    verified: bool,
    /// The read waiting for its answer.
    pending_read: Option<Vec<u8>>,
    /// Set once the device hung; every later send refuses.
    tripped: Option<String>,
    log: Log,
}

/// Copy a hidraw read into a fixed 16-byte report, padding or truncating.
fn normalize(buf: &[u8]) -> InputReport {
    let mut r = [0u8; 16];
    let n = buf.len().min(16);
    r[..n].copy_from_slice(&buf[..n]);
    r
}

fn find_hidraw() -> Result<(PathBuf, PathBuf), Error> {
    for entry in fs::read_dir("/sys/class/hidraw")? {
        let entry = entry?;
        let uevent = fs::read_to_string(entry.path().join("device/uevent")).unwrap_or_default();
        if uevent.lines().any(|l| l == HID_ID_LINE) {
            return Ok((PathBuf::from("/dev").join(entry.file_name()), entry.path()));
        }
    }
    Err(Error::NoDevice)
}

/// The text a hang leaves for the person at the device.
pub fn hang_advice(reason: &str) -> String {
    format!(
        "{reason}. Power-cycle the X7: unplug its power supply for 10 seconds, then plug it back in. \
         x7 sends nothing more to it until then; no reset, no retry"
    )
}

/// What `x7 status` shows. Opens nothing for traffic and sends nothing:
/// the node, the enumeration, recordings, the hang marker, the log.
pub fn status() -> Result<Vec<(&'static str, String)>, Error> {
    let (path, sysfs) = find_hidraw()?;
    let loc = safety::locate(&sysfs, Path::new("/proc/asound"), &safety::boot_id());
    let mut rows = vec![("device", path.display().to_string())];
    let access = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(_) => "ok".to_owned(),
        Err(e) => format!("no ({e})"),
    };
    rows.push(("access", access));
    rows.push((
        "usb",
        loc.id.as_ref().map_or("unknown".into(), |id| {
            format!("bus {} device {}", id.bus, id.dev)
        }),
    ));
    rows.push((
        "recording",
        match &loc.capture {
            Capture::Card(dir) => match safety::open_captures(dir) {
                Ok(open) if open.is_empty() => format!("none ({})", dir.display()),
                Ok(open) => format!("{} - HID commands wait", open.join(", ")),
                Err(e) => format!("cannot check {}: {e}", dir.display()),
            },
            Capture::Missing => "no ALSA card bound - HID commands refused".into(),
            Capture::Off => "not checked".into(),
        },
    ));
    let hang = match (loc.id.as_ref(), Marker::default_path()) {
        (Some(id), Some(p)) => match Marker::at(p.clone()).hung(id) {
            Some(reason) => format!("yes, since {reason}; power-cycle the X7 ({})", p.display()),
            None => "no".into(),
        },
        _ => "unknown".into(),
    };
    rows.push(("hung", hang));
    rows.push((
        "log",
        Log::default_path().map_or("none".into(), |p| p.display().to_string()),
    ));
    Ok(rows)
}

impl Device {
    /// A device without safe-mode checks or log: the in-memory fake and
    /// the tests. Writes still wait for an answered read.
    pub(crate) fn from_port(port: Box<dyn Port>) -> Device {
        Device {
            port,
            capture: Capture::Off,
            marker: None,
            verified: false,
            pending_read: None,
            tripped: None,
            log: Log::off(),
        }
    }

    pub(crate) fn from_file(file: File) -> Device {
        Self::from_port(Box::new(file))
    }

    /// Open the X7's hidraw node. Sends nothing. Refuses an enumeration
    /// that hung before (`Error::Unresponsive`).
    pub fn open() -> Result<Device, Error> {
        let log = Log::standard();
        let (path, sysfs) = find_hidraw()?;
        let loc = safety::locate(&sysfs, Path::new("/proc/asound"), &safety::boot_id());
        let marker = match (loc.id, Marker::default_path()) {
            (Some(id), Some(p)) => Some((id, Marker::at(p))),
            _ => None,
        };
        if let Some((id, m)) = &marker {
            if let Some(reason) = m.hung(id) {
                log.line(&format!(
                    "open {}: refused, marked hung: {reason}",
                    path.display()
                ));
                return Err(Error::Unresponsive(hang_advice(&format!(
                    "the X7 stopped answering earlier ({reason})"
                ))));
            }
            if m.clear_if_stale(id).unwrap_or(false) {
                log.line("hang marker of an earlier enumeration removed");
            }
        }
        let file = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                return Err(Error::Permission(path));
            }
            Err(e) => return Err(e.into()),
        };
        log.line(&format!(
            "open {} usb={} capture={:?}",
            path.display(),
            marker
                .as_ref()
                .map_or("unknown".into(), |(id, _)| format!("{}:{}", id.bus, id.dev)),
            loc.capture
        ));
        Ok(Device {
            port: Box::new(file),
            capture: loc.capture,
            marker,
            verified: false,
            pending_read: None,
            tripped: None,
            log,
        })
    }

    pub fn log(&self) -> &Log {
        &self.log
    }

    pub fn is_tripped(&self) -> bool {
        self.tripped.is_some()
    }

    /// The enumeration this handle belongs to (`bus dev boot`), if known.
    pub fn identity(&self) -> Option<String> {
        self.marker.as_ref().map(|(id, _)| id.key())
    }

    /// The device hung: stop all traffic, tell other x7 processes, log it.
    pub fn trip(&mut self, reason: &str) -> Error {
        if self.tripped.is_none() {
            self.log
                .line(&format!("HANG: {reason}; all traffic stopped"));
            if let Some((id, m)) = &self.marker
                && let Err(e) = m.set(id, reason)
            {
                self.log.line(&format!("could not write hang marker: {e}"));
            }
            self.tripped = Some(reason.to_owned());
        }
        Error::Unresponsive(hang_advice(&format!("the X7 stopped answering ({reason})")))
    }

    /// The checks before a write, without writing: hang, recording.
    pub fn ready_to_send(&self) -> Result<(), Error> {
        if let Some(reason) = &self.tripped {
            return Err(Error::Unresponsive(hang_advice(&format!(
                "the X7 stopped answering ({reason})"
            ))));
        }
        match self.capture.open_streams() {
            Ok(open) if open.is_empty() => Ok(()),
            Ok(open) => Err(Error::CaptureOpen(open.join(", "))),
            Err(e) => Err(Error::CaptureOpen(format!("cannot check: {e}"))),
        }
    }

    /// Write one Output report. Byte 0 is the report id (0, unnumbered), which
    /// the kernel strips before the control transfer.
    pub fn send(&mut self, payload: &[u8]) -> Result<(), Error> {
        assert!(
            payload.len() <= OUTPUT_LEN,
            "payload longer than one report"
        );
        if let Err(e) = self.ready_to_send() {
            self.log.line(&format!("refused {}: {e}", hex(payload)));
            return Err(e);
        }
        let read = protocol::is_read_command(payload);
        if !read && !self.verified {
            self.log.line(&format!(
                "refused {}: write before an answered read",
                hex(payload)
            ));
            return Err(Error::NotVerified);
        }
        let mut buf = [0u8; OUTPUT_LEN + 1];
        buf[1..1 + payload.len()].copy_from_slice(payload);
        let kind = if read { "read" } else { "write" };
        self.log.line(&format!("tx {kind} {}", hex(payload)));
        let started = Instant::now();
        let result = self.port.write_report(&buf);
        let took = started.elapsed();
        match result {
            Err(e) if e.raw_os_error() == Some(nix::libc::ENODEV) => {
                self.log.line(&format!("tx failed, device gone: {e}"));
                Err(e.into())
            }
            Err(e) => Err(self.trip(&format!(
                "write of {} failed after {} ms: {e}",
                hex(payload),
                took.as_millis()
            ))),
            Ok(()) if took > SLOW_WRITE => Err(self.trip(&format!(
                "write of {} took {} ms",
                hex(payload),
                took.as_millis()
            ))),
            Ok(()) => {
                if read {
                    self.pending_read = Some(payload.to_vec());
                }
                Ok(())
            }
        }
    }

    /// Wait up to `timeout` for one Input report. `None` on timeout.
    pub fn recv(&mut self, timeout: Duration) -> Result<Option<InputReport>, Error> {
        let mut buf = [0u8; 64];
        let Some(n) = self.port.read_report(&mut buf, timeout)? else {
            return Ok(None);
        };
        if n == 0 {
            // EOF: the device is gone. Never treat it as a report, or a
            // drain loop would spin on a permanently readable fd.
            self.log.line("rx EOF, device gone");
            return Err(io::Error::from(ErrorKind::UnexpectedEof).into());
        }
        let report = normalize(&buf[..n]);
        self.log.line(&format!("rx {}", hex(&report)));
        if let Some(cmd) = &self.pending_read
            && protocol::answers_read(cmd, &report)
        {
            if !self.verified {
                self.log.line("first read answered: writes allowed");
            }
            self.verified = true;
            self.pending_read = None;
        }
        Ok(Some(report))
    }

    /// Drop every report already queued, so a reply left over from an earlier
    /// command on a long-lived handle cannot be mistaken for the next one.
    fn drain(&mut self) -> Result<(), Error> {
        while self.recv(Duration::ZERO)?.is_some() {}
        Ok(())
    }

    /// Send `cmd` and return the first report within `REPLY_TIMEOUT` for which
    /// `matches` is true. Reports queued before the send, and unsolicited
    /// pushes after it, are dropped. A write is preceded by the output read
    /// when no read was answered yet. No reply means the device hung: no
    /// retry, and nothing more is sent.
    pub fn request(
        &mut self,
        cmd: &[u8],
        matches: impl Fn(&[u8]) -> bool,
    ) -> Result<InputReport, Error> {
        if !self.verified && !protocol::is_read_command(cmd) {
            self.log.line("probe read before the first write");
            self.request(&protocol::encode_output_get(), protocol::is_output_reply)?;
        }
        self.drain()?;
        self.send(cmd)?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        // A protocol report after the send proves the device is alive even
        // when none matches (a set it did not apply): that is a rejection,
        // not a hang. Knob reports and releases prove nothing.
        let mut answered: Option<InputReport> = None;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let report = if remaining.is_zero() {
                None
            } else {
                self.recv(remaining)?
            };
            match report {
                None => {
                    if let Some(last) = answered {
                        self.log
                            .line(&format!("no matching reply to {}, device alive", hex(cmd)));
                        return Err(Error::BadReply(last));
                    }
                    return Err(self.trip(&format!(
                        "no reply to {} within {} ms",
                        hex(cmd),
                        REPLY_TIMEOUT.as_millis()
                    )));
                }
                Some(report) if matches(&report) => return Ok(report),
                Some(report) => {
                    if report[0] == 0 && report[1..].iter().any(|&b| b != 0) {
                        answered = Some(report);
                    }
                }
            }
        }
    }

    /// The device with its marker and capture check pointed at test paths.
    #[cfg(test)]
    pub(crate) fn with_safety(mut self, capture: Capture, id: DeviceId, marker: Marker) -> Device {
        self.capture = capture;
        self.marker = Some((id, marker));
        self
    }

    /// Tests that start mid-conversation: a read was answered already.
    #[cfg(test)]
    pub(crate) fn assume_verified(mut self) -> Device {
        self.verified = true;
        self
    }
}

/// A device that was unplugged: writes fail with ENODEV, reads see EOF.
#[cfg(test)]
pub(crate) fn gone_device() -> Device {
    struct Gone;
    impl Port for Gone {
        fn write_report(&mut self, _: &[u8]) -> io::Result<()> {
            Err(io::Error::from_raw_os_error(nix::libc::ENODEV))
        }
        fn read_report(&mut self, _: &mut [u8], _: Duration) -> io::Result<Option<usize>> {
            Ok(Some(0))
        }
    }
    Device::from_port(Box::new(Gone))
}

/// A Device over one end of a socket pair; the other end plays the X7.
#[cfg(test)]
pub(crate) fn fake_pair() -> (Device, std::os::unix::net::UnixStream) {
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    (
        Device::from_file(File::from(std::os::fd::OwnedFd::from(ours))),
        theirs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn request_discards_replies_queued_before_the_send() {
        let (mut dev, mut x7) = fake_pair();
        // A stale `23 4d` reply is already waiting when the request starts.
        let mut stale = [0u8; 16];
        stale[..5].copy_from_slice(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
        x7.write_all(&stale).unwrap();
        let r = dev.request(&[0x23, 0x4d], |r| r[1..3] == [0x23, 0x4d]);
        assert!(
            matches!(r, Err(Error::Unresponsive(_))),
            "stale reply was returned: {r:?}"
        );
        // The command itself still went out: report id byte + 64 data bytes.
        let mut sent = [0u8; 65];
        x7.read_exact(&mut sent).unwrap();
        assert_eq!(&sent[..3], &[0x00, 0x23, 0x4d]);
    }

    #[test]
    fn request_returns_the_reply_that_arrives_after_the_send() {
        let (mut dev, mut x7) = fake_pair();
        let replier = std::thread::spawn(move || {
            let mut sent = [0u8; 65];
            x7.read_exact(&mut sent).unwrap();
            let mut reply = [0u8; 16];
            reply[..5].copy_from_slice(&[0x00, 0x23, 0x4d, 0x00, 0x02]);
            x7.write_all(&reply).unwrap();
        });
        let r = dev
            .request(&[0x23, 0x4d], |r| r[1..3] == [0x23, 0x4d])
            .unwrap();
        assert_eq!(r[4], 0x02);
        replier.join().unwrap();
    }

    #[test]
    fn request_fails_instead_of_spinning_when_the_device_is_gone() {
        let (mut dev, x7) = fake_pair();
        drop(x7); // peer closed: every read now returns 0 bytes
        let r = dev.request(&[0x23, 0x4d], |r| r[1..3] == [0x23, 0x4d]);
        assert!(matches!(r, Err(Error::Io(_))), "got {r:?}");
    }

    #[test]
    fn normalize_pads_short_reads() {
        let r = normalize(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
        assert_eq!(&r[..5], &[0x00, 0x23, 0x4d, 0x00, 0x01]);
        assert!(r[5..].iter().all(|&b| b == 0));
    }

    #[test]
    fn normalize_truncates_long_reads() {
        let long = [0xaa; 64];
        let r = normalize(&long);
        assert_eq!(r, [0xaa; 16]);
    }

    #[test]
    fn normalize_empty_is_release() {
        assert_eq!(normalize(&[]), [0u8; 16]);
    }

    /// A stand-in for the hidraw node of an X7 in the state seen on
    /// 2026-11-13: the `SET_REPORT` of the first command never completes,
    /// the kernel gives up after its control timeout (shortened here) and
    /// `write()` returns ETIMEDOUT. Every later write fails with EPROTO.
    /// Counts the writes, so a test can prove nothing was sent again.
    struct HungX7 {
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
        stall: Duration,
    }

    impl Port for HungX7 {
        fn write_report(&mut self, buf: &[u8]) -> io::Result<()> {
            let first = {
                let mut w = self.writes.lock().unwrap();
                w.push(buf[1..].to_vec());
                w.len() == 1
            };
            if first {
                std::thread::sleep(self.stall);
                Err(io::Error::from_raw_os_error(nix::libc::ETIMEDOUT))
            } else {
                Err(io::Error::from_raw_os_error(nix::libc::EPROTO))
            }
        }
        fn read_report(&mut self, _: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            Ok(None)
        }
    }

    /// A device that takes every write and never answers.
    struct SilentX7 {
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl Port for SilentX7 {
        fn write_report(&mut self, buf: &[u8]) -> io::Result<()> {
            self.writes.lock().unwrap().push(buf[1..].to_vec());
            Ok(())
        }
        fn read_report(&mut self, _: &mut [u8], timeout: Duration) -> io::Result<Option<usize>> {
            std::thread::sleep(timeout);
            Ok(None)
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("x7-transport-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn id(dev: u32) -> DeviceId {
        DeviceId {
            bus: 1,
            dev,
            boot: "boot".into(),
        }
    }

    fn card(dir: &Path, capture_status: &str) -> Capture {
        let sub = dir.join("card1/pcm0c/sub0");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("status"), capture_status).unwrap();
        Capture::Card(dir.join("card1"))
    }

    #[test]
    fn the_hang_stops_all_traffic_and_marks_the_enumeration() {
        let dir = tmp("hang");
        let writes = Arc::new(Mutex::new(Vec::new()));
        let marker = Marker::at(dir.join("hang"));
        let mut dev = Device::from_port(Box::new(HungX7 {
            writes: writes.clone(),
            stall: Duration::from_millis(20),
        }))
        .with_safety(card(&dir, "closed\n"), id(2), marker.clone());

        let r = dev.request(&[0x23, 0x4d], |r| r[1..3] == [0x23, 0x4d]);
        let Err(Error::Unresponsive(text)) = r else {
            panic!("expected a hang, got {r:?}")
        };
        assert!(text.contains("Power-cycle the X7"), "{text}");
        assert!(dev.is_tripped());
        assert_eq!(writes.lock().unwrap().len(), 1, "exactly one write");
        assert!(marker.hung(&id(2)).unwrap().contains("23 4d"));

        // Nothing goes out any more: no retry, no other command.
        assert!(matches!(
            dev.send(&[0x23, 0x24]),
            Err(Error::Unresponsive(_))
        ));
        assert!(matches!(
            dev.request(&[0x23, 0x4d], |_| true),
            Err(Error::Unresponsive(_))
        ));
        assert_eq!(writes.lock().unwrap().len(), 1, "still exactly one write");
    }

    #[test]
    fn a_missing_reply_is_a_hang_without_retry() {
        let dir = tmp("silent");
        let writes = Arc::new(Mutex::new(Vec::new()));
        let marker = Marker::at(dir.join("hang"));
        let mut dev = Device::from_port(Box::new(SilentX7 {
            writes: writes.clone(),
        }))
        .with_safety(Capture::Off, id(2), marker.clone());
        let r = dev.request(&[0x23, 0x4d], |r| r[1..3] == [0x23, 0x4d]);
        assert!(matches!(r, Err(Error::Unresponsive(_))), "{r:?}");
        assert!(dev.send(&[0x23, 0x4d]).is_err());
        assert_eq!(writes.lock().unwrap().len(), 1);
        assert!(marker.hung(&id(2)).is_some());
    }

    #[test]
    fn a_slow_write_counts_as_a_hang() {
        struct SlowX7;
        impl Port for SlowX7 {
            fn write_report(&mut self, _: &[u8]) -> io::Result<()> {
                std::thread::sleep(SLOW_WRITE + Duration::from_millis(50));
                Ok(())
            }
            fn read_report(&mut self, _: &mut [u8], _: Duration) -> io::Result<Option<usize>> {
                Ok(None)
            }
        }
        let mut dev = Device::from_port(Box::new(SlowX7));
        assert!(matches!(
            dev.send(&[0x23, 0x4d]),
            Err(Error::Unresponsive(_))
        ));
        assert!(dev.is_tripped());
    }

    #[test]
    fn nothing_is_sent_while_the_x7_records() {
        let dir = tmp("capture");
        let writes = Arc::new(Mutex::new(Vec::new()));
        let capture = card(&dir, "state: RUNNING\nowner_pid   : 42\n");
        let mut dev = Device::from_port(Box::new(SilentX7 {
            writes: writes.clone(),
        }))
        .with_safety(capture, id(2), Marker::at(dir.join("hang")));
        let r = dev.send(&[0x23, 0x4d]);
        assert!(
            matches!(&r, Err(Error::CaptureOpen(s)) if s.contains("pcm0c/sub0: RUNNING")),
            "{r:?}"
        );
        assert!(writes.lock().unwrap().is_empty());
        assert!(!dev.is_tripped(), "a recording is no hang");
        // The recording ends: the read goes out.
        fs::write(dir.join("card1/pcm0c/sub0/status"), "closed\n").unwrap();
        dev.send(&[0x23, 0x4d]).unwrap();
        assert_eq!(writes.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_missing_alsa_card_refuses_too() {
        let dir = tmp("nocard");
        let mut dev =
            fake_pair()
                .0
                .with_safety(Capture::Missing, id(2), Marker::at(dir.join("hang")));
        assert!(matches!(
            dev.send(&[0x23, 0x4d]),
            Err(Error::CaptureOpen(_))
        ));
    }

    #[test]
    fn a_write_waits_for_an_answered_read() {
        let (mut dev, mut x7) = fake_pair();
        // A set before any read is refused and nothing reaches the device.
        let set = protocol::encode_feature_set(protocol::Feature::Sbx, true);
        assert!(matches!(dev.send(&set), Err(Error::NotVerified)));
        // An unrelated report does not count as an answer.
        dev.send(&[0x23, 0x4d]).unwrap();
        let mut sent = [0u8; 65];
        x7.read_exact(&mut sent).unwrap();
        let mut knob = [0u8; 16];
        knob[0] = 0x01;
        x7.write_all(&knob).unwrap();
        dev.recv(Duration::from_millis(50)).unwrap();
        assert!(matches!(dev.send(&set), Err(Error::NotVerified)));
        // The read's own answer does.
        let mut reply = [0u8; 16];
        reply[..5].copy_from_slice(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
        x7.write_all(&reply).unwrap();
        dev.recv(Duration::from_millis(50)).unwrap();
        dev.send(&set).unwrap();
    }

    #[test]
    fn request_reads_first_before_a_write() {
        let (mut dev, mut x7) = fake_pair();
        let device = std::thread::spawn(move || {
            let mut sent = [0u8; 65];
            x7.read_exact(&mut sent).unwrap();
            let first = sent[1..3].to_vec();
            let mut reply = [0u8; 16];
            reply[..5].copy_from_slice(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
            x7.write_all(&reply).unwrap();
            x7.read_exact(&mut sent).unwrap();
            let second = sent[1..5].to_vec();
            let mut echo = [0u8; 16];
            echo[..7].copy_from_slice(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x01, 0x01]);
            x7.write_all(&echo).unwrap();
            (first, second)
        });
        let f = protocol::Feature::Sbx;
        dev.request(&protocol::encode_feature_set(f, true), |r| {
            protocol::is_feature_set_echo(r, f, true)
        })
        .unwrap();
        let (first, second) = device.join().unwrap();
        assert_eq!(first, [0x23, 0x4d], "the probe read went first");
        assert_eq!(second, [0x23, 0x23, 0x01, 0x01]);
    }

    #[test]
    fn a_vanished_device_is_gone_not_hung() {
        struct GoneX7;
        impl Port for GoneX7 {
            fn write_report(&mut self, _: &[u8]) -> io::Result<()> {
                Err(io::Error::from_raw_os_error(nix::libc::ENODEV))
            }
            fn read_report(&mut self, _: &mut [u8], _: Duration) -> io::Result<Option<usize>> {
                Ok(None)
            }
        }
        let mut dev = Device::from_port(Box::new(GoneX7));
        assert!(matches!(dev.send(&[0x23, 0x4d]), Err(Error::Io(_))));
        assert!(!dev.is_tripped());
    }
}
