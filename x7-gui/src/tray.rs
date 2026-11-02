//! StatusNotifierItem tray: icon follows the output, menu does the daily things.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem};
use x7::{Command, Feature, Link, Output, Session};

use crate::wake::Wake;

pub const ICON_SIZE: i32 = 22;
const HEADPHONES: &[u8] = include_bytes!("../icons/headphones.argb");
const SPEAKERS: &[u8] = include_bytes!("../icons/speakers.argb");

pub fn icon(output: Option<Output>) -> ksni::Icon {
    let data = match output {
        Some(Output::Headphones) => HEADPHONES,
        _ => SPEAKERS,
    };
    ksni::Icon {
        width: ICON_SIZE,
        height: ICON_SIZE,
        data: data.to_vec(),
    }
}

pub struct X7Tray {
    pub session: Arc<Session>,
    pub wake: Arc<Wake>,
    pub quit: Arc<AtomicBool>,
    /// True while a StatusNotifier host shows our item. The window reads it
    /// to decide whether closing leaves the app here or must quit.
    pub tray_ok: Arc<AtomicBool>,
    /// Started by the session's autostart entry: a missing host is the
    /// GNOME extension still loading, so wait for it, never open a window.
    pub autostart: bool,
}

impl X7Tray {
    fn toggle(&self, feature: Feature) {
        let on = !self.session.state().feature(feature);
        self.session.send(Command::SetFeature(feature, on));
    }
}

impl ksni::Tray for X7Tray {
    fn id(&self) -> String {
        "x7".into()
    }

    fn title(&self) -> String {
        "Sound Blaster X7".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![icon(self.session.state().output)]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let s = self.session.state();
        let text = match (s.link, s.output) {
            (Link::Absent, _) => "X7: not connected".to_owned(),
            (Link::Connecting, _) => "X7: connecting".to_owned(),
            (Link::Ready, Some(Output::Headphones)) => "X7: headphones".to_owned(),
            (Link::Ready, Some(Output::Speakers)) => "X7: speakers".to_owned(),
            (Link::Ready, None) => "X7".to_owned(),
        };
        ksni::ToolTip {
            title: text,
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.wake.show();
    }

    /// The host picked us up (at start, or after the GNOME extension loaded
    /// later than we did). Closing the window leaves the app here.
    fn watcher_online(&self) {
        self.tray_ok.store(true, Ordering::Relaxed);
    }

    /// No host, or it went away: a closed window could not be reopened, so
    /// open one and let closing quit. Under autostart the host is on its
    /// way; the tray icon or a second launch opens the window later.
    /// Returning true keeps ksni retrying.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.tray_ok.store(false, Ordering::Relaxed);
        if !self.autostart {
            self.wake.show();
        }
        true
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let s = self.session.state();
        let enabled = s.link == Link::Ready;
        let selected = match s.output {
            Some(Output::Speakers) => 1,
            _ => 0,
        };
        vec![
            RadioGroup {
                selected,
                select: Box::new(|t: &mut Self, i| {
                    let o = if i == 1 {
                        Output::Speakers
                    } else {
                        Output::Headphones
                    };
                    t.session.send(Command::SetOutput(o));
                }),
                options: vec![
                    RadioItem {
                        label: "Headphones".into(),
                        enabled,
                        ..Default::default()
                    },
                    RadioItem {
                        label: "Speakers".into(),
                        enabled,
                        ..Default::default()
                    },
                ],
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "SBX".into(),
                enabled,
                checked: s.feature(Feature::Sbx),
                activate: Box::new(|t: &mut Self| t.toggle(Feature::Sbx)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Scout Mode".into(),
                enabled,
                checked: s.feature(Feature::Scout),
                activate: Box::new(|t: &mut Self| t.toggle(Feature::Scout)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Open controls".into(),
                activate: Box::new(|t: &mut Self| t.wake.show()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| {
                    t.quit.store(true, Ordering::Relaxed);
                    t.wake.close();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ksni::Tray;
    use std::sync::mpsc::{Receiver, TryRecvError};
    use std::time::Duration;

    fn tray(autostart: bool) -> (X7Tray, Receiver<()>) {
        let (wake, rx) = Wake::new();
        let session = Session::start_with_opener(
            Box::new(|| Err(x7::Error::NoDevice)),
            Duration::from_secs(3600),
            Box::new(|| {}),
        );
        let tray = X7Tray {
            session: Arc::new(session),
            wake,
            quit: Arc::new(AtomicBool::new(false)),
            tray_ok: Arc::new(AtomicBool::new(true)),
            autostart,
        };
        (tray, rx)
    }

    #[test]
    fn offline_opens_the_window_only_outside_autostart() {
        let (by_hand, rx) = tray(false);
        assert!(by_hand.watcher_offline(ksni::OfflineReason::No));
        assert_eq!(rx.try_recv(), Ok(()), "no host by hand: the window opens");
        assert!(!by_hand.tray_ok.load(Ordering::Relaxed));

        let (at_login, rx) = tray(true);
        assert!(
            at_login.watcher_offline(ksni::OfflineReason::No),
            "keeps retrying"
        );
        assert_eq!(
            rx.try_recv(),
            Err(TryRecvError::Empty),
            "autostart waits for the host"
        );
        assert!(!at_login.tray_ok.load(Ordering::Relaxed));
    }

    #[test]
    fn embedded_icons_are_22x22_argb() {
        for output in [Some(Output::Headphones), Some(Output::Speakers), None] {
            let i = icon(output);
            assert_eq!((i.width, i.height), (ICON_SIZE, ICON_SIZE));
            assert_eq!(i.data.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
            // Some pixel is opaque white; the icon is not blank.
            assert!(i.data.chunks(4).any(|p| p == [255, 255, 255, 255]));
        }
    }

    #[test]
    fn embedded_icons_have_a_dark_halo() {
        for output in [Some(Output::Headphones), Some(Output::Speakers)] {
            let i = icon(output);
            // ARGB: a partly transparent, dark pixel is the outline.
            let halo = i
                .data
                .chunks(4)
                .any(|p| p[0] > 0 && p[0] < 255 && p[1] < 64 && p[2] < 64 && p[3] < 64);
            assert!(halo, "{output:?} has no halo");
        }
    }
}
