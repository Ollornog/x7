//! x7-gui: a tray icon plus a control window over one device session.
//!
//! Threads: main runs eframe (winit requires it), once per window, and
//! waits in the tray between windows; the session owns the device; ksni
//! runs the tray; a notifier thread turns session changes into tray
//! updates so no D-Bus work happens on the device thread.

mod instance;
mod labels;
mod theme;
mod tray;
mod wake;
mod widgets;
mod window;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;

use eframe::egui;
use ksni::blocking::TrayMethods;
use x7::Session;

/// Which device the session talks to. `--fake` is the in-memory X7 for
/// working on the window without the hardware; `--fake=absent` never finds
/// a device, for the not-connected state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Real,
    Fake,
    Absent,
}

/// What the arguments ask for. `autostart` is for the session's autostart
/// entry: stay in the tray and never open a window on your own, so a start
/// before the tray host is up (GNOME loads its extension after autostarted
/// apps) waits for the host instead of opening the window at login.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Launch {
    Run { mode: Mode, autostart: bool },
    Version,
}

const USAGE: &str = "usage: x7-gui [--fake | --fake=absent] [--autostart] [--version]";

fn parse_args(args: impl Iterator<Item = String>) -> Result<Launch, String> {
    let mut mode = Mode::Real;
    let mut autostart = false;
    for arg in args {
        match arg.as_str() {
            "--fake" => mode = Mode::Fake,
            "--fake=absent" => mode = Mode::Absent,
            "--autostart" => autostart = true,
            "--version" => return Ok(Launch::Version),
            other => return Err(format!("x7-gui: unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(Launch::Run { mode, autostart })
}

fn start_session(mode: Mode, on_change: Box<dyn Fn() + Send>) -> Session {
    let delay = std::time::Duration::from_secs(2);
    match mode {
        Mode::Real => Session::start(on_change),
        Mode::Fake => {
            Session::start_with_opener(Box::new(|| Ok(x7::fake::device())), delay, on_change)
        }
        Mode::Absent => {
            Session::start_with_opener(Box::new(|| Err(x7::Error::NoDevice)), delay, on_change)
        }
    }
}

/// The name the window, the tray and the launcher show. The binaries, the
/// SNI id (`x7`) and the app id (`x7-gui`) keep their names: installed
/// autostart entries, the udev rule and the Omarchy widget refer to them.
pub const APP_NAME: &str = "X7 Sound Blaster";

fn main() -> eframe::Result {
    let (mode, autostart) = match parse_args(std::env::args().skip(1)) {
        Ok(Launch::Run { mode, autostart }) => (mode, autostart),
        Ok(Launch::Version) => {
            println!("x7-gui {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Err(text) => {
            eprintln!("{text}");
            std::process::exit(2);
        }
    };
    let quit = Arc::new(AtomicBool::new(false));
    // False until a StatusNotifier host reports us online (see tray.rs).
    let tray_ok = Arc::new(AtomicBool::new(false));
    let (wake, wake_rx) = wake::Wake::new();
    let (notify_tx, notify_rx) = sync_channel::<()>(1);

    // One instance per session; a second launch shows the first one's window.
    // A fake session runs beside the real tray app, so it makes no claim
    // and opens its window at once instead of waiting in the tray.
    let _bus = match mode {
        Mode::Fake | Mode::Absent => {
            wake.show();
            None
        }
        Mode::Real => match instance::claim(wake.clone()) {
            instance::Claim::First(guard) => Some(guard),
            instance::Claim::AlreadyRunning => return Ok(()),
            instance::Claim::ShowFailed(why) => {
                eprintln!(
                    "x7-gui: another instance is running but did not show its window ({why})"
                );
                std::process::exit(1);
            }
            instance::Claim::NoBus(e) => {
                eprintln!(
                    "x7-gui: no session bus ({e}); running without the single-instance guard"
                );
                None
            }
        },
    };

    let session = Arc::new(start_session(
        mode,
        Box::new({
            let wake = wake.clone();
            move || {
                wake.repaint();
                let _ = notify_tx.try_send(());
            }
        }),
    ));

    // The tray lives as long as the process; windows come and go under it.
    let tray = tray::X7Tray {
        session: session.clone(),
        wake: wake.clone(),
        quit: quit.clone(),
        tray_ok: tray_ok.clone(),
        autostart,
    };
    // A missing host is temporary (GNOME loads its extension after
    // autostarted apps); ksni then reports it through watcher_offline
    // and keeps retrying. Only a dead session bus fails here.
    //
    // Assume the host is reachable before spawning: ksni calls
    // `watcher_online` only after an offline period, so a host
    // present from the start would never set this, and a
    // `watcher_offline` fired during startup must not be overwritten
    // by a store after `spawn` returns.
    tray_ok.store(true, Ordering::Relaxed);
    let tray_handle = match tray.assume_sni_available(true).spawn() {
        Ok(handle) => {
            let h = handle.clone();
            std::thread::Builder::new()
                .name("x7-tray-notify".into())
                .spawn(move || {
                    while notify_rx.recv().is_ok() {
                        if h.update(|_| {}).is_none() {
                            break;
                        }
                    }
                })
                .expect("spawn notifier");
            Some(handle)
        }
        Err(e) => {
            tray_ok.store(false, Ordering::Relaxed);
            eprintln!("x7-gui: no tray available ({e}); running as a plain window");
            wake.show();
            None
        }
    };

    // A window on demand. Wayland cannot hide one, so closing destroys it
    // and the next show builds a new one; `run_native` returns in between.
    let result = loop {
        if wake_rx.recv().is_err() || quit.load(Ordering::Relaxed) {
            break Ok(());
        }
        wake.opening();
        let r = eframe::run_native("x7-gui", window_options(), {
            let session = session.clone();
            let quit = quit.clone();
            let tray_ok = tray_ok.clone();
            let wake = wake.clone();
            Box::new(move |cc| {
                let ctx = cc.egui_ctx.clone();
                theme::prepare(
                    &ctx,
                    theme::load_appearance(&theme::gui_config_path()),
                    theme::desktop_theme(),
                );
                wake.opened(&ctx);
                Ok(Box::new(window::Controls::new(
                    session, quit, tray_ok, wake,
                )))
            })
        });
        wake.closing();
        if r.is_err() || quit.load(Ordering::Relaxed) {
            break r;
        }
    };

    if let Some(h) = tray_handle {
        let _ = h.shutdown();
    }
    session.shutdown();
    result
}

fn window_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id("x7-gui")
            .with_inner_size([440.0, 540.0])
            .with_min_inner_size([440.0, 480.0]),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Launch, String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }

    fn run(mode: Mode) -> Launch {
        Launch::Run {
            mode,
            autostart: false,
        }
    }

    #[test]
    fn the_fake_flag_picks_the_mode_and_anything_else_is_refused() {
        assert_eq!(parse(&[]), Ok(run(Mode::Real)));
        assert_eq!(parse(&["--fake"]), Ok(run(Mode::Fake)));
        assert_eq!(parse(&["--fake=absent"]), Ok(run(Mode::Absent)));
        let err = parse(&["--fake=loud"]).unwrap_err();
        assert!(err.contains("--fake=loud") && err.contains(USAGE), "{err}");
    }

    #[test]
    fn autostart_is_a_flag_beside_the_mode() {
        assert_eq!(
            parse(&["--autostart"]),
            Ok(Launch::Run {
                mode: Mode::Real,
                autostart: true
            })
        );
        assert_eq!(
            parse(&["--fake", "--autostart"]),
            Ok(Launch::Run {
                mode: Mode::Fake,
                autostart: true
            })
        );
        assert!(USAGE.contains("--autostart"));
    }

    #[test]
    fn version_wins_over_everything_else() {
        assert_eq!(parse(&["--version"]), Ok(Launch::Version));
        assert_eq!(parse(&["--fake", "--version"]), Ok(Launch::Version));
        assert!(USAGE.contains("--version"));
    }
}
