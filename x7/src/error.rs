use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Sound Blaster X7 found; is it plugged in?")]
    NoDevice,
    #[error(
        "permission denied opening {0}; install 70-soundblaster-x7.rules into /etc/udev/rules.d, reload udev and replug the X7"
    )]
    Permission(PathBuf),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no reply from the device within 500 ms")]
    Timeout,
    #[error("unexpected reply from the device: {0:02x?}")]
    BadReply([u8; 16]),
    #[error("{name}: {value} is outside {min}..{max}")]
    OutOfRange {
        name: &'static str,
        value: f32,
        min: f32,
        max: f32,
    },
    #[error("{name}: unknown value '{value}'")]
    UnknownValue { name: &'static str, value: String },
    #[error("{group}: no parameter named '{name}' (run `x7 {group}` to list them)")]
    UnknownParam { group: &'static str, name: String },
    #[error("profile: {0}")]
    Profile(String),
}
