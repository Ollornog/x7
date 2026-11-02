//! One x7-gui per session. The first instance owns a well-known name on the
//! session bus and answers `Show`; a second launch asks it to show its
//! window and exits, so a launcher click never makes a second tray icon.
//!
//! ksni builds zbus on its tokio backend, so the bus work here runs on a
//! one-thread tokio runtime of its own that lives as long as the claim.

use std::sync::Arc;
use std::time::Duration;

use zbus::fdo::{RequestNameFlags, RequestNameReply};

use crate::wake::Wake;

pub const NAME: &str = "io.github.stamrin.x7gui";
const PATH: &str = "/io/github/stamrin/x7gui";
/// How long a second launch waits for the running instance to answer Show.
const SHOW_TIMEOUT: Duration = Duration::from_secs(3);

/// Outcome of claiming the name at startup.
pub enum Claim {
    /// We own the name; drop the guard only when the process ends.
    First(Guard),
    /// Another instance owns it and showed its window.
    AlreadyRunning,
    /// Another instance owns it but did not answer Show.
    ShowFailed(String),
    /// No session bus: run unguarded.
    NoBus(String),
}

/// Keeps the connection, and the runtime that serves it, alive.
pub struct Guard {
    _conn: zbus::Connection,
    _rt: tokio::runtime::Runtime,
}

/// The object the running instance serves. A `Show` that arrives before
/// the first window exists waits in the switch until `main` gets there.
struct Shower {
    wake: Arc<Wake>,
}

#[zbus::interface(name = "io.github.stamrin.x7gui")]
impl Shower {
    fn show(&self) {
        self.wake.show();
    }
}

pub fn claim(wake: Arc<Wake>) -> Claim {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("x7-bus")
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return Claim::NoBus(e.to_string()),
    };
    let shower = Shower { wake };
    let owned: zbus::Result<Result<zbus::Connection, Option<String>>> = rt.block_on(async {
        let conn = zbus::Connection::session().await?;
        // Serve before claiming, so a Show that follows the claim is answered.
        conn.object_server().at(PATH, shower).await?;
        // With DoNotQueue, zbus reports a taken name as an error, not as
        // `RequestNameReply::Exists`.
        match conn
            .request_name_with_flags(NAME, RequestNameFlags::DoNotQueue.into())
            .await
        {
            Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(Ok(conn)),
            Ok(_) | Err(zbus::Error::NameTaken) => {
                let call = conn.call_method(Some(NAME), PATH, Some(NAME), "Show", &());
                let failure = match tokio::time::timeout(SHOW_TIMEOUT, call).await {
                    Ok(Ok(_)) => None,
                    Ok(Err(e)) => Some(e.to_string()),
                    Err(_) => Some(format!("no answer within {SHOW_TIMEOUT:?}")),
                };
                Ok(Err(failure))
            }
            Err(e) => Err(e),
        }
    });
    match owned {
        Ok(Ok(conn)) => Claim::First(Guard {
            _conn: conn,
            _rt: rt,
        }),
        Ok(Err(None)) => Claim::AlreadyRunning,
        Ok(Err(Some(why))) => Claim::ShowFailed(why),
        Err(e) => Claim::NoBus(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_before_the_window_exists_is_kept_for_the_loop() {
        let (wake, rx) = Wake::new();
        let shower = Shower { wake: wake.clone() };
        shower.show();
        assert_eq!(rx.try_recv(), Ok(()), "nothing to show yet: wake the loop");
        let ctx = eframe::egui::Context::default();
        wake.opened(&ctx);
        shower.show();
        assert!(
            rx.try_recv().is_err(),
            "focused directly once a window exists"
        );
        assert!(ctx.has_requested_repaint());
    }
}
