//! The traffic log: every command, reply, check and refusal, one line each.
//!
//! `$X7_LOG`, else `$XDG_STATE_HOME/x7/x7.log`, else
//! `~/.local/state/x7/x7.log`. Kept below 1 MiB by moving the full file to
//! `x7.log.1`. Times are UTC. Logging never fails a command: a log that
//! cannot be written is skipped.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LEN: u64 = 1 << 20;

#[derive(Clone, Debug, Default)]
pub struct Log {
    path: Option<PathBuf>,
}

impl Log {
    /// No log: the fake and test devices.
    pub fn off() -> Log {
        Log { path: None }
    }

    pub fn at(path: PathBuf) -> Log {
        Log { path: Some(path) }
    }

    pub fn default_path() -> Option<PathBuf> {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        if let Some(p) = env("X7_LOG") {
            return Some(PathBuf::from(p));
        }
        if let Some(d) = env("XDG_STATE_HOME") {
            return Some(PathBuf::from(d).join("x7/x7.log"));
        }
        env("HOME").map(|h| PathBuf::from(h).join(".local/state/x7/x7.log"))
    }

    pub fn standard() -> Log {
        Log {
            path: Self::default_path(),
        }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.path.as_ref()
    }

    pub fn line(&self, msg: &str) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if fs::metadata(path).is_ok_and(|m| m.len() > MAX_LEN) {
            let _ = fs::rename(path, path.with_extension("log.1"));
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{} [{}] {msg}", timestamp(), std::process::id());
        }
    }
}

/// Hex without separators' noise: `23 4d`, trailing zeros trimmed.
pub fn hex(bytes: &[u8]) -> String {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let shown = &bytes[..end.max(1).min(bytes.len())];
    shown
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `2026-11-13T14:35:11.683Z`.
pub fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format_utc(now.as_secs(), now.subsec_millis())
}

fn format_utc(secs: u64, millis: u32) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_format() {
        assert_eq!(format_utc(0, 0), "1970-01-01T00:00:00.000Z");
        // 2026-11-13 13:35:11 UTC
        assert_eq!(format_utc(1_794_576_911, 683), "2026-11-13T13:35:11.683Z");
        assert_eq!(format_utc(951_782_400, 5), "2000-02-29T00:00:00.005Z");
    }

    #[test]
    fn hex_trims_trailing_zeros() {
        assert_eq!(hex(&[0x23, 0x4d, 0, 0]), "23 4d");
        assert_eq!(hex(&[0, 0]), "00");
        assert_eq!(hex(&[]), "");
    }

    #[test]
    fn lines_are_appended_and_the_file_rotates() {
        let dir = std::env::temp_dir().join(format!("x7-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("x7.log");
        let log = Log::at(path.clone());
        log.line("one");
        log.line("two");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.lines().count() == 2 && text.contains("] one") && text.contains("] two"));
        fs::write(&path, vec![b'x'; (MAX_LEN + 1) as usize]).unwrap();
        log.line("three");
        assert!(fs::read_to_string(&path).unwrap().ends_with("three\n"));
        assert!(path.with_extension("log.1").exists());
    }
}
