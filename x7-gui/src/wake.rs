//! The one control window's switch, shared by the tray, the bus and the
//! main loop. Wayland cannot hide a window, so "back to the tray" means
//! the window is destroyed and a new one is built on demand; `main` runs
//! eframe once per window and blocks on this switch in between.

use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};

use eframe::egui;

enum Window {
    /// No window; the loop waits on the channel.
    None,
    /// The loop took a wake and eframe is building the window. A show
    /// now is answered by that window, not by a queued wake, which the
    /// loop would read only after the user closed it and so reopen it.
    Opening { close: bool },
    /// Up, from its first frame to its close frame.
    Open(egui::Context),
}

pub struct Wake {
    /// Wakes the loop in `main`; one slot, so repeated requests collapse.
    tx: SyncSender<()>,
    window: Mutex<Window>,
}

impl Wake {
    pub fn new() -> (Arc<Wake>, Receiver<()>) {
        let (tx, rx) = sync_channel(1);
        let wake = Arc::new(Wake {
            tx,
            window: Mutex::new(Window::None),
        });
        (wake, rx)
    }

    /// The loop took a wake and is about to build a window.
    pub fn opening(&self) {
        *self.window.lock().unwrap() = Window::Opening { close: false };
    }

    /// The window is up: show, close and repaint reach it directly. A
    /// close that arrived while it was being built is delivered now.
    pub fn opened(&self, ctx: &egui::Context) {
        let mut w = self.window.lock().unwrap();
        if let Window::Opening { close: true } = *w {
            Self::close_window(ctx);
        }
        *w = Window::Open(ctx.clone());
    }

    /// The window is on its last frame: requests go to the loop again,
    /// so a show that lands during the close opens a fresh window.
    pub fn closing(&self) {
        *self.window.lock().unwrap() = Window::None;
    }

    fn close_window(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    /// Bring the window up: focus the open one, wait for the one being
    /// built, or have the loop open one.
    pub fn show(&self) {
        match &*self.window.lock().unwrap() {
            Window::Open(ctx) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
            Window::Opening { .. } => {}
            Window::None => {
                let _ = self.tx.try_send(());
            }
        }
    }

    /// Close the open window, or wake the loop so it reads the quit flag.
    pub fn close(&self) {
        match &mut *self.window.lock().unwrap() {
            Window::Open(ctx) => Self::close_window(ctx),
            Window::Opening { close } => *close = true,
            Window::None => {
                let _ = self.tx.try_send(());
            }
        }
    }

    /// The session changed: the open window, if any, draws again.
    pub fn repaint(&self) {
        if let Window::Open(ctx) = &*self.window.lock().unwrap() {
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::TryRecvError;

    #[test]
    fn show_wakes_the_loop_while_no_window_is_open() {
        let (wake, rx) = Wake::new();
        wake.show();
        assert_eq!(rx.try_recv(), Ok(()), "the loop opens a window");
    }

    #[test]
    fn repeated_shows_collapse_into_one_wake() {
        let (wake, rx) = Wake::new();
        wake.show();
        wake.show();
        assert_eq!(rx.try_recv(), Ok(()));
        assert_eq!(
            rx.try_recv(),
            Err(TryRecvError::Empty),
            "one window, not two"
        );
    }

    #[test]
    fn show_focuses_the_open_window_instead_of_waking() {
        let (wake, rx) = Wake::new();
        let ctx = eframe::egui::Context::default();
        wake.opened(&ctx);
        wake.show();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        assert!(
            ctx.has_requested_repaint(),
            "the window is asked to come up"
        );
    }

    #[test]
    fn a_show_while_the_window_is_opening_leaves_no_stale_wake() {
        let (wake, rx) = Wake::new();
        wake.show();
        assert_eq!(rx.try_recv(), Ok(()), "the loop takes the wake");
        wake.opening();
        wake.show();
        assert_eq!(
            rx.try_recv(),
            Err(TryRecvError::Empty),
            "the window being built is the answer; a queued wake would \
             reopen it after the user closes it"
        );
    }

    #[test]
    fn a_close_while_opening_closes_the_window_once_it_is_up() {
        let (wake, rx) = Wake::new();
        wake.opening();
        wake.close();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty), "no stale wake");
        let ctx = eframe::egui::Context::default();
        wake.opened(&ctx);
        assert!(ctx.has_requested_repaint(), "Quit still takes effect");
    }

    #[test]
    fn a_show_after_the_window_started_closing_wakes_the_loop() {
        let (wake, rx) = Wake::new();
        let ctx = eframe::egui::Context::default();
        wake.opened(&ctx);
        wake.closing();
        wake.show();
        assert_eq!(rx.try_recv(), Ok(()), "a new window, not the dying one");
    }

    #[test]
    fn close_reaches_the_window_or_wakes_the_loop() {
        let (wake, rx) = Wake::new();
        wake.close();
        assert_eq!(rx.try_recv(), Ok(()), "the loop sees the quit flag");
        let ctx = eframe::egui::Context::default();
        wake.opened(&ctx);
        wake.close();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
        assert!(ctx.has_requested_repaint(), "the window is told to close");
    }

    #[test]
    fn repaint_reaches_only_an_open_window() {
        let (wake, rx) = Wake::new();
        wake.repaint();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty), "nothing to paint");
        let ctx = eframe::egui::Context::default();
        wake.opened(&ctx);
        wake.repaint();
        assert!(ctx.has_requested_repaint());
    }
}
