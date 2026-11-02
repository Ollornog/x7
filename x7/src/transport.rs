//! The hidraw device: find it, write Output reports, read Input reports.
//!
//! Commands are 64-byte Output reports. The X7 has no interrupt OUT endpoint,
//! so a hidraw `write()` becomes the `SET_REPORT` control transfer the Windows
//! app used. Replies and device-initiated events are 16-byte Input reports on
//! the interrupt IN endpoint, which hidraw `read()` returns one at a time.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nix::poll::{PollFd, PollFlags, PollTimeout};

use crate::Error;
use crate::protocol::InputReport;

const HID_ID_LINE: &str = "HID_ID=0003:0000041E:0000323A";
const OUTPUT_LEN: usize = 64;
pub const REPLY_TIMEOUT: Duration = Duration::from_millis(500);

pub struct Device {
    file: File,
}

/// Copy a hidraw read into a fixed 16-byte report, padding or truncating.
fn normalize(buf: &[u8]) -> InputReport {
    let mut r = [0u8; 16];
    let n = buf.len().min(16);
    r[..n].copy_from_slice(&buf[..n]);
    r
}

fn find_hidraw() -> Result<PathBuf, Error> {
    for entry in fs::read_dir("/sys/class/hidraw")? {
        let entry = entry?;
        let uevent = fs::read_to_string(entry.path().join("device/uevent")).unwrap_or_default();
        if uevent.lines().any(|l| l == HID_ID_LINE) {
            return Ok(PathBuf::from("/dev").join(entry.file_name()));
        }
    }
    Err(Error::NoDevice)
}

impl Device {
    pub(crate) fn from_file(file: File) -> Device {
        Device { file }
    }

    pub fn open() -> Result<Device, Error> {
        let path = find_hidraw()?;
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(file) => Ok(Device { file }),
            Err(e) if e.kind() == ErrorKind::PermissionDenied => Err(Error::Permission(path)),
            Err(e) => Err(e.into()),
        }
    }

    /// Write one Output report. Byte 0 is the report id (0, unnumbered), which
    /// the kernel strips before the control transfer.
    pub fn send(&mut self, payload: &[u8]) -> Result<(), Error> {
        assert!(
            payload.len() <= OUTPUT_LEN,
            "payload longer than one report"
        );
        let mut buf = [0u8; OUTPUT_LEN + 1];
        buf[1..1 + payload.len()].copy_from_slice(payload);
        self.file.write_all(&buf)?;
        Ok(())
    }

    /// Wait up to `timeout` for one Input report. `None` on timeout.
    pub fn recv(&mut self, timeout: Duration) -> Result<Option<InputReport>, Error> {
        let mut fds = [PollFd::new(self.file.as_fd(), PollFlags::POLLIN)];
        let timeout = PollTimeout::try_from(timeout).unwrap_or(PollTimeout::MAX);
        if nix::poll::poll(&mut fds, timeout).map_err(std::io::Error::from)? == 0 {
            return Ok(None);
        }
        let mut buf = [0u8; 64];
        let n = self.file.read(&mut buf)?;
        if n == 0 {
            // EOF: the device is gone. Never treat it as a report, or a
            // drain loop would spin on a permanently readable fd.
            return Err(std::io::Error::from(ErrorKind::UnexpectedEof).into());
        }
        Ok(Some(normalize(&buf[..n])))
    }

    /// Drop every report already queued, so a reply left over from an earlier
    /// command on a long-lived handle cannot be mistaken for the next one.
    fn drain(&mut self) -> Result<(), Error> {
        while self.recv(Duration::ZERO)?.is_some() {}
        Ok(())
    }

    /// Send `cmd` and return the first report within `REPLY_TIMEOUT` for which
    /// `matches` is true. Reports queued before the send, and unsolicited
    /// pushes after it, are dropped.
    pub fn request(
        &mut self,
        cmd: &[u8],
        matches: impl Fn(&[u8]) -> bool,
    ) -> Result<InputReport, Error> {
        self.drain()?;
        self.send(cmd)?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            match self.recv(remaining)? {
                None => return Err(Error::Timeout),
                Some(report) if matches(&report) => return Ok(report),
                Some(_) => continue,
            }
        }
    }
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

    #[test]
    fn request_discards_replies_queued_before_the_send() {
        let (mut dev, mut x7) = fake_pair();
        // A stale `23 4d` reply is already waiting when the request starts.
        let mut stale = [0u8; 16];
        stale[..5].copy_from_slice(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
        x7.write_all(&stale).unwrap();
        let r = dev.request(&[0x23, 0x4d], |r| r[1..3] == [0x23, 0x4d]);
        assert!(
            matches!(r, Err(Error::Timeout)),
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
}
