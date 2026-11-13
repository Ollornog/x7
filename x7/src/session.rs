//! A long-lived device session for GUIs: one thread owns the hidraw handle,
//! every report becomes state, commands run one at a time with a deadline.
//!
//! The worker is a `step()` function so tests drive it synchronously over a
//! socket-pair fake device; `Session::start` runs it on a thread.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::Error;
use crate::protocol::{self, Feature, Output, Report};
use crate::settings::{self, Param, Switch, Value, Wire};
use crate::transport::Device;

pub const REPLY_DEADLINE: Duration = Duration::from_millis(500);
const RECV_SLICE: Duration = Duration::from_millis(10);
/// Quiet time after opening before the first command: the X7 takes about
/// two seconds after enumerating before its firmware answers, and a read it
/// misses now counts as a hang, so nothing is sent before this.
pub const SETTLE: Duration = Duration::from_secs(3);
/// How often a send paused for a recording looks again.
const CAPTURE_RECHECK: Duration = Duration::from_secs(1);

/// Where the device is, as far as the session can tell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    /// No hidraw node, or it could not be opened.
    #[default]
    Absent,
    /// The node is open but the first refresh has not completed: the X7
    /// takes about two seconds after enumerating before its firmware
    /// answers, and a silent device is reopened rather than written off.
    Connecting,
    /// The first refresh completed; the state mirrors the device.
    Ready,
    /// The device stopped answering (no reply, a failed or slow write).
    /// Nothing is sent to it until it is power-cycled; the session reopens
    /// it only once it has re-enumerated. See `crate::safety`.
    Unresponsive,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub link: Link,
    pub output: Option<Output>,
    /// Feature mask as read from `23 24`; test with `feature()`.
    pub features: u32,
    /// Raw float per (hid_group, get_id).
    pub params: HashMap<(u8, u8), f32>,
    /// Raw float per switch row, keyed by its wire.
    pub switches: HashMap<Wire, f32>,
    pub jack: Option<bool>,
    pub last_error: Option<String>,
}

impl State {
    pub fn feature(&self, f: Feature) -> bool {
        protocol::feature_enabled(self.features, f)
    }

    pub fn param(&self, p: &Param) -> Option<Value> {
        self.params
            .get(&(p.hid_group, p.get_id))
            .map(|&raw| p.from_raw(raw))
    }

    pub fn switch(&self, s: &Switch) -> Option<Value> {
        self.switches.get(&s.wire).map(|&raw| s.from_raw(raw))
    }
}

#[derive(Debug)]
pub enum Command {
    SetOutput(Output),
    SetFeature(Feature, bool),
    SetParam(&'static Param, Value),
    SetSwitch(&'static Switch, Value),
    Refresh,
}

/// One unit of device traffic. Public `Command`s expand into these.
#[derive(Clone, Copy, Debug)]
enum Op {
    SetParam(&'static Param, f32),
    GetParam(&'static Param),
    SetOutput(Output),
    GetOutput,
    SetFeature(Feature, bool),
    GetFeatures,
    SetSwitch(&'static Switch, f32),
    GetSwitch(&'static Switch),
}

#[derive(Clone, Copy)]
struct InFlight {
    op: Op,
    deadline: Instant,
    /// A protocol report arrived since the send: the device is alive, so a
    /// missing completion is a rejected command, not a hang.
    answered: bool,
}

pub(crate) struct Worker {
    dev: Option<Device>,
    pub(crate) state: Arc<Mutex<State>>,
    rx: Receiver<Command>,
    on_change: Box<dyn Fn() + Send>,
    queue: VecDeque<Op>,
    inflight: Option<InFlight>,
    /// Reply deadline per op; `REPLY_DEADLINE` outside tests.
    pub(crate) deadline: Duration,
    opener: Box<dyn FnMut() -> Result<Device, Error> + Send>,
    reconnect_delay: Duration,
    last_attempt: Option<Instant>,
    /// `SETTLE` outside tests.
    settle: Duration,
    opened_at: Option<Instant>,
    /// Sends wait while the X7 records; `last_error` says so meanwhile.
    paused_until: Option<Instant>,
    /// `CAPTURE_RECHECK` outside tests.
    pub(crate) recheck: Duration,
    /// The identity of the enumeration that hung. The marker file does the
    /// same across processes; this holds even if the marker could not be
    /// written, so a reopen never reaches the same device again.
    hung: Option<Option<String>>,
}

impl Worker {
    /// A worker over an already-open device, starting connected.
    #[cfg(test)]
    pub(crate) fn with_device(
        dev: Device,
        rx: Receiver<Command>,
        on_change: Box<dyn Fn() + Send>,
    ) -> Worker {
        let state = Arc::new(Mutex::new(State {
            link: Link::Ready,
            ..State::default()
        }));
        Worker {
            dev: Some(dev),
            state,
            rx,
            on_change,
            queue: VecDeque::new(),
            inflight: None,
            deadline: REPLY_DEADLINE,
            opener: Box::new(|| Err(Error::NoDevice)),
            reconnect_delay: Duration::from_secs(2),
            last_attempt: None,
            settle: Duration::ZERO,
            opened_at: None,
            paused_until: None,
            recheck: CAPTURE_RECHECK,
            hung: None,
        }
    }

    /// A worker with no device yet; `step` opens one through `opener`.
    pub(crate) fn disconnected(
        opener: Box<dyn FnMut() -> Result<Device, Error> + Send>,
        reconnect_delay: Duration,
        settle: Duration,
        rx: Receiver<Command>,
        on_change: Box<dyn Fn() + Send>,
    ) -> Worker {
        Worker {
            dev: None,
            state: Arc::new(Mutex::new(State::default())),
            rx,
            on_change,
            queue: VecDeque::new(),
            inflight: None,
            deadline: REPLY_DEADLINE,
            opener,
            reconnect_delay,
            last_attempt: None,
            settle,
            opened_at: None,
            paused_until: None,
            recheck: CAPTURE_RECHECK,
            hung: None,
        }
    }

    /// Try to open the device if enough time has passed since the last try.
    fn reconnect(&mut self) {
        if let Some(t) = self.last_attempt
            && t.elapsed() < self.reconnect_delay
        {
            std::thread::sleep(RECV_SLICE);
            return;
        }
        self.last_attempt = Some(Instant::now());
        match (self.opener)() {
            Ok(dev) => {
                if self.hung.as_ref() == Some(&dev.identity()) {
                    // Same enumeration as the one that hung: not power-cycled.
                    drop(dev);
                    self.update(|s| s.link = Link::Unresponsive);
                    return;
                }
                self.hung = None;
                self.dev = Some(dev);
                self.opened_at = Some(Instant::now());
                self.paused_until = None;
                self.update(|s| {
                    s.link = Link::Connecting;
                    s.last_error = None;
                });
                // Commands accepted while disconnected wait behind the refresh.
                let mut pending = std::mem::take(&mut self.queue);
                self.enqueue(Command::Refresh);
                self.queue.append(&mut pending);
                self.changed();
            }
            Err(e) => {
                let link = if matches!(e, Error::Unresponsive(_)) {
                    Link::Unresponsive
                } else {
                    Link::Absent
                };
                let text = e.to_string();
                let is_new = self.update(|s| {
                    let new = s.link != link || s.last_error.as_deref() != Some(&text);
                    s.link = link;
                    s.last_error = Some(text.clone());
                    new
                });
                if is_new {
                    self.changed();
                }
            }
        }
    }

    fn update<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }

    fn changed(&self) {
        (self.on_change)();
    }

    /// Drop the device and forget its values. `link` says why: `Absent` for
    /// an I/O error (the node is gone), `Unresponsive` for a hang, which the
    /// next `reconnect` reopens only after a power cycle.
    fn disconnect(&mut self, link: Link, why: String) {
        self.dev = None;
        self.queue.clear();
        self.inflight = None;
        self.opened_at = None;
        self.paused_until = None;
        // Try again at once: a vanished node reports "no device" now, and a
        // hung one reports its marker. Opening sends nothing.
        self.last_attempt = None;
        self.update(|s| {
            s.link = link;
            s.output = None;
            s.features = 0;
            s.params.clear();
            s.switches.clear();
            s.jack = None;
            s.last_error = Some(why);
        });
        self.changed();
    }

    /// Drain the command channel into the queue. False once the channel
    /// is closed (the `Session` was dropped).
    fn pump(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(cmd) => self.enqueue(cmd),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn enqueue(&mut self, cmd: Command) {
        match cmd {
            Command::SetParam(p, v) => match p.validate(&v) {
                Ok(()) => self.push_coalesced(Op::SetParam(p, p.to_raw(&v))),
                Err(e) => {
                    self.update(|s| s.last_error = Some(e.to_string()));
                    self.changed();
                }
            },
            Command::SetOutput(o) => self.push_coalesced(Op::SetOutput(o)),
            Command::SetFeature(f, on) => self.push_coalesced(Op::SetFeature(f, on)),
            Command::SetSwitch(sw, v) => match sw.validate(&v) {
                Ok(()) => self.push_coalesced(Op::SetSwitch(sw, sw.to_raw(&v))),
                Err(e) => {
                    self.update(|s| s.last_error = Some(e.to_string()));
                    self.changed();
                }
            },
            Command::Refresh => {
                self.queue.push_back(Op::GetOutput);
                self.queue.push_back(Op::GetFeatures);
                for p in settings::PARAMS {
                    self.queue.push_back(Op::GetParam(p));
                }
                for sw in settings::SWITCHES {
                    self.queue.push_back(Op::GetSwitch(sw));
                }
            }
        }
    }

    fn set_queued_for(&self, p: &Param) -> bool {
        self.queue
            .iter()
            .any(|q| matches!(q, Op::SetParam(x, _) if std::ptr::eq(*x, p)))
    }

    /// Replace a queued op with the same key instead of appending.
    fn push_coalesced(&mut self, op: Op) {
        let same = |q: &Op| match (q, &op) {
            (Op::SetParam(a, _), Op::SetParam(b, _)) => std::ptr::eq(*a, *b),
            (Op::SetOutput(_), Op::SetOutput(_)) => true,
            (Op::SetFeature(a, _), Op::SetFeature(b, _)) => a == b,
            (Op::SetSwitch(a, _), Op::SetSwitch(b, _)) => std::ptr::eq(*a, *b),
            _ => false,
        };
        if let Some(slot) = self.queue.iter_mut().find(|q| same(q)) {
            *slot = op;
        } else {
            self.queue.push_back(op);
        }
    }

    fn bytes_for(op: Op) -> Vec<u8> {
        match op {
            Op::SetParam(p, raw) => protocol::encode_set(p.hid_group, p.set_id, raw).to_vec(),
            Op::GetParam(p) => protocol::encode_get(p.hid_group, p.get_id).to_vec(),
            Op::SetOutput(o) => protocol::encode_output_set(o),
            Op::GetOutput => protocol::encode_output_get().to_vec(),
            Op::SetFeature(f, on) => protocol::encode_feature_set(f, on).to_vec(),
            Op::GetFeatures => protocol::encode_feature_get().to_vec(),
            Op::SetSwitch(sw, raw) => sw.wire.set_bytes(raw),
            Op::GetSwitch(sw) => sw.wire.get_bytes(),
        }
    }

    fn dispatch(&mut self, op: Op) {
        let Some(dev) = self.dev.as_mut() else { return };
        match dev.send(&Self::bytes_for(op)) {
            Ok(()) => {
                self.inflight = Some(InFlight {
                    op,
                    deadline: Instant::now() + self.deadline,
                    answered: false,
                });
                if self.paused_until.take().is_some() {
                    // The recording ended; its message is stale.
                    self.update(|s| s.last_error = None);
                    self.changed();
                }
            }
            Err(Error::CaptureOpen(why)) => {
                // Not a fault: wait for the recording to end, keep the op.
                self.queue.push_front(op);
                let first = self.paused_until.is_none();
                self.paused_until = Some(Instant::now() + self.recheck);
                if first {
                    let text = Error::CaptureOpen(why).to_string();
                    self.update(|s| s.last_error = Some(text));
                    self.changed();
                }
            }
            Err(Error::NotVerified) => {
                // A write before any answered read: read first, then the write.
                self.queue.push_front(op);
                self.dispatch(Op::GetOutput);
            }
            Err(e @ Error::Unresponsive(_)) => {
                self.hung = Some(dev.identity());
                self.disconnect(Link::Unresponsive, e.to_string());
            }
            Err(e) => self.disconnect(Link::Absent, e.to_string()),
        }
    }

    fn send_next(&mut self) {
        if self.inflight.is_some() {
            return;
        }
        if self.opened_at.is_some_and(|t| t.elapsed() < self.settle) {
            return;
        }
        if self.paused_until.is_some_and(|t| Instant::now() < t) {
            return;
        }
        if let Some(op) = self.queue.pop_front() {
            self.dispatch(op);
        }
    }

    /// An op without its completion by the deadline. If nothing at all came
    /// back the device hung: stop for good (no barrier, no retry, no
    /// reopen of this enumeration). If the device answered something else,
    /// it is alive and rejected the command: report it and go on.
    fn check_deadline(&mut self) {
        let Some(InFlight {
            op,
            deadline,
            answered,
        }) = self.inflight
        else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }
        self.inflight = None;
        if answered {
            self.update(|s| s.last_error = Some(format!("the X7 did not apply {op:?}")));
            self.changed();
            return;
        }
        let bytes = crate::log::hex(&Self::bytes_for(op));
        let reason = format!(
            "no reply to {bytes} within {} ms",
            self.deadline.as_millis()
        );
        let text = match self.dev.as_mut() {
            Some(dev) => {
                self.hung = Some(dev.identity());
                dev.trip(&reason).to_string()
            }
            None => reason,
        };
        self.disconnect(Link::Unresponsive, text);
    }

    /// One iteration: pump commands, send if idle, read one report, check
    /// the deadline. Returns false when the command channel is closed.
    pub(crate) fn step(&mut self) -> bool {
        if !self.pump() {
            return false;
        }
        if self.dev.is_none() {
            self.reconnect();
            return true;
        }
        self.send_next();
        let received = match self.dev.as_mut() {
            Some(dev) => dev.recv(RECV_SLICE),
            None => return true, // send_next disconnected
        };
        match received {
            Ok(Some(report)) => {
                self.apply(&report);
                // Send the next op now rather than after another recv slice.
                self.send_next();
            }
            Ok(None) => {}
            Err(e) => self.disconnect(Link::Absent, e.to_string()),
        }
        self.settle();
        self.check_deadline();
        true
    }

    /// The first refresh (and anything queued behind it) has completed:
    /// the state now mirrors the device.
    fn settle(&mut self) {
        if self.dev.is_some()
            && self.inflight.is_none()
            && self.queue.is_empty()
            && self.update(|s| s.link == Link::Connecting)
        {
            self.update(|s| s.link = Link::Ready);
            self.changed();
        }
    }

    /// Every report becomes state. Task 3 adds in-flight completion.
    fn apply(&mut self, raw: &protocol::InputReport) {
        if raw[0] == 0
            && raw[1..].iter().any(|&b| b != 0)
            && let Some(f) = self.inflight.as_mut()
        {
            f.answered = true;
        }
        let report = protocol::decode_report(raw);
        let changed = self.update(|s| match &report {
            Report::ParamPush { group, id, value } => {
                s.params.insert((*group, *id), *value);
                true
            }
            Report::FeatureChanged { feature, on, mask } => {
                // The echo of a host-sent set carries zeros where a device
                // push carries the mask, so update the bit and let a real
                // mask override it.
                // An id past 32 would overflow the shift; such a report
                // changes no bit and only a real mask can update the state.
                let bit = 1u32
                    .checked_shl(u32::from(*feature).wrapping_sub(1))
                    .unwrap_or(0);
                let updated = if *on {
                    s.features | bit
                } else {
                    s.features & !bit
                };
                s.features = if *mask != 0 { *mask } else { updated };
                true
            }
            Report::Features(mask) => {
                s.features = *mask;
                true
            }
            Report::OutputChanged(o) => {
                s.output = Some(*o);
                true
            }
            Report::Jack(present) => {
                s.jack = Some(*present);
                true
            }
            Report::Toggle { id, on } => {
                // Only the read form carries state; the set echo is handled
                // by `complete`.
                match settings::SWITCHES
                    .iter()
                    .find(|sw| matches!(sw.wire, Wire::Toggle { read, .. } if read == *id))
                {
                    Some(sw) => {
                        s.switches.insert(sw.wire, *on as u8 as f32);
                        true
                    }
                    None => false,
                }
            }
            Report::DeviceValue { cmd, raw } => {
                let wire = if *cmd == protocol::DEVICE_STANDBY {
                    Wire::AutoStandby
                } else {
                    Wire::OutputTarget
                };
                s.switches.insert(wire, *raw);
                true
            }
            _ => false,
        });
        self.complete(&report, raw);
        if changed {
            self.changed();
        }
    }

    /// Does this report finish the in-flight op? If so, what follows it.
    fn complete(&mut self, report: &Report, bytes: &protocol::InputReport) {
        let Some(InFlight { op, .. }) = self.inflight else {
            return;
        };
        let follow_up = match (op, report) {
            // A newer set for the same row is queued (mid-drag): reading the
            // old value back would only make the slider jump.
            (Op::SetParam(p, _), Report::SetAck) if self.set_queued_for(p) => None,
            (Op::SetParam(p, _), Report::SetAck) => Some(Op::GetParam(p)),
            (Op::GetParam(p), Report::ParamPush { group, id, .. })
                if *group == p.hid_group && *id == p.get_id =>
            {
                None
            }
            (Op::SetOutput(o), Report::Unknown(_)) if protocol::is_output_set_echo(bytes, o) => {
                Some(Op::GetOutput)
            }
            (Op::GetOutput, Report::OutputChanged(_)) => None,
            (
                Op::SetFeature(f, on),
                Report::FeatureChanged {
                    feature, on: got, ..
                },
            ) if *feature == f as u8 && *got == on => None,
            (Op::GetFeatures, Report::Features(_)) => None,
            (Op::SetSwitch(sw, value), _) if sw.wire.is_set_done(bytes, value) => {
                Some(Op::GetSwitch(sw))
            }
            (Op::GetSwitch(sw), _) if sw.wire.is_get_reply(bytes) => None,
            _ => return,
        };
        self.inflight = None;
        if let Some(next) = follow_up {
            self.queue.push_front(next);
        }
        if self.update(|s| {
            s.last_error
                .as_deref()
                .is_some_and(|e| e.starts_with("the X7 did not apply"))
        }) {
            // The device applies commands again; a rejection from earlier is stale.
            self.update(|s| s.last_error = None);
            self.changed();
        }
    }
}

/// A device session for long-lived clients. See the module docs.
pub struct Session {
    state: Arc<Mutex<State>>,
    tx: std::sync::mpsc::Sender<Command>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Session {
    /// Spawn the device thread. Never fails: with no X7 present the link
    /// stays `Link::Absent` and the thread retries every two seconds.
    /// `on_change` runs on the device thread after every state change and
    /// must not block.
    pub fn start(on_change: Box<dyn Fn() + Send>) -> Session {
        Self::spawn(
            Box::new(Device::open),
            Duration::from_secs(2),
            SETTLE,
            on_change,
        )
    }

    /// `start` with a custom opener: `fake::device` for a client without
    /// the X7, or one that always fails to stand in for an absent device.
    pub fn start_with_opener(
        opener: Box<dyn FnMut() -> Result<Device, Error> + Send>,
        reconnect_delay: Duration,
        on_change: Box<dyn Fn() + Send>,
    ) -> Session {
        Self::spawn(opener, reconnect_delay, Duration::ZERO, on_change)
    }

    fn spawn(
        opener: Box<dyn FnMut() -> Result<Device, Error> + Send>,
        reconnect_delay: Duration,
        settle: Duration,
        on_change: Box<dyn Fn() + Send>,
    ) -> Session {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut worker = Worker::disconnected(opener, reconnect_delay, settle, rx, on_change);
        let state = worker.state.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag = stop.clone();
        let thread =
            std::thread::Builder::new()
                .name("x7-session".into())
                .spawn(move || {
                    while !stop_flag.load(std::sync::atomic::Ordering::Relaxed) && worker.step() {}
                })
                .expect("spawn session thread");
        Session {
            state,
            tx,
            stop,
            thread: Mutex::new(Some(thread)),
        }
    }

    /// A clone of the current snapshot.
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    /// Stop the thread and join it. Safe through an `Arc`; a second call is
    /// a no-op. Dropping the last `Session` without calling it also ends the
    /// thread, because the command channel closes.
    pub fn shutdown(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{Sender, channel};

    pub(super) struct Rig {
        pub worker: Worker,
        pub x7: UnixStream,
        pub tx: Sender<Command>,
        pub changes: Arc<AtomicUsize>,
    }

    pub(super) fn rig() -> Rig {
        let (dev, x7) = crate::transport::fake_pair();
        let dev = dev.assume_verified();
        let (tx, rx) = channel();
        let changes = Arc::new(AtomicUsize::new(0));
        let c = changes.clone();
        let worker = Worker::with_device(
            dev,
            rx,
            Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        Rig {
            worker,
            x7,
            tx,
            changes,
        }
    }

    /// A rig whose worker has not opened the device yet; the first `step`
    /// opens it through the opener.
    pub(super) fn cold_rig() -> Rig {
        let (dev, x7) = crate::transport::fake_pair();
        let dev = Mutex::new(Some(dev));
        let (tx, rx) = channel();
        let changes = Arc::new(AtomicUsize::new(0));
        let c = changes.clone();
        let worker = Worker::disconnected(
            Box::new(move || Ok(dev.lock().unwrap().take().expect("opened once"))),
            Duration::from_millis(1),
            Duration::ZERO,
            rx,
            Box::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        Rig {
            worker,
            x7,
            tx,
            changes,
        }
    }

    pub(super) fn report(bytes: &[u8]) -> [u8; 16] {
        let mut r = [0u8; 16];
        r[..bytes.len()].copy_from_slice(bytes);
        r
    }

    impl Rig {
        /// The device sends one report; the worker takes one step.
        pub fn device_sends(&mut self, bytes: &[u8]) {
            self.x7.write_all(&report(bytes)).unwrap();
            self.worker.step();
        }
        /// What the worker last sent, without the report-id byte, trimmed.
        pub fn sent(&mut self) -> Vec<u8> {
            let mut buf = [0u8; 65];
            self.x7.read_exact(&mut buf).unwrap();
            let end = buf.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            buf[1..end.max(1)].to_vec()
        }
        pub fn state(&self) -> State {
            self.worker.state.lock().unwrap().clone()
        }
    }

    #[test]
    fn starts_ready_with_empty_state() {
        let r = rig();
        let s = r.state();
        assert_eq!(s.link, Link::Ready);
        assert_eq!(s.output, None);
        assert_eq!(s.jack, None);
        assert!(s.params.is_empty());
    }

    #[test]
    fn a_fresh_connection_is_connecting_until_the_refresh_settles() {
        let mut r = cold_rig();
        assert_eq!(r.state().link, Link::Absent);
        r.worker.step(); // opens the device, queues the refresh
        assert_eq!(r.state().link, Link::Connecting);
        r.worker.step(); // sends GetOutput
        // Answer every refresh op; the next one leaves in the same step.
        for _ in 0..(2 + settings::PARAMS.len() + settings::SWITCHES.len()) {
            assert_eq!(r.state().link, Link::Connecting);
            let sent = r.sent();
            let reply: Vec<u8> = match sent[..2] {
                [0x23, 0x4d] => vec![0x00, 0x23, 0x4d, 0x00, 0x01],
                [0x23, 0x24] => vec![0x00, 0x23, 0x24, 0x00, 0x03],
                // `sent()` trims trailing zeros, so an id of 0 is absent.
                [0x26, 0x01] => {
                    let id = sent.get(3).copied().unwrap_or(0);
                    vec![0x00, 0x26, 0x01, 0x00, sent[2], id, 0, 0, 0, 0]
                }
                [0x23, id] => vec![0x00, 0x23, id, 0x00, 0x00],
                [0x5a, 0x2c] => vec![0x00, 0x5a, 0x2c, 0x05, 0x01, 0x02],
                [0x5a, 0x39] => vec![0x00, 0x5a, 0x39, 0x05, 0x01],
                _ => panic!("unexpected op {sent:02x?}"),
            };
            r.device_sends(&reply);
        }
        let s = r.state();
        assert_eq!(s.link, Link::Ready);
        assert_eq!(s.output, Some(Output::Headphones));
        assert_eq!(s.params.len(), settings::PARAMS.len());
        assert_eq!(s.switches.len(), settings::SWITCHES.len());
    }

    #[test]
    fn a_connecting_device_that_vanishes_is_absent() {
        let dev = Mutex::new(Some(crate::transport::gone_device()));
        let (_tx, rx) = channel();
        let mut worker = Worker::disconnected(
            Box::new(move || dev.lock().unwrap().take().ok_or(Error::NoDevice)),
            Duration::from_secs(60),
            Duration::ZERO,
            rx,
            Box::new(|| {}),
        );
        worker.step();
        assert_eq!(worker.state.lock().unwrap().link, Link::Connecting);
        worker.step(); // the send fails with ENODEV: unplugged, not hung
        assert_eq!(worker.state.lock().unwrap().link, Link::Absent);
    }

    #[test]
    fn jack_report_updates_state_and_notifies() {
        let mut r = rig();
        r.device_sends(&[0x00, 0x23, 0x12, 0x00, 0x04]);
        assert_eq!(r.state().jack, Some(true));
        assert_eq!(r.changes.load(Ordering::SeqCst), 1);
        r.device_sends(&[0x00, 0x23, 0x12]);
        assert_eq!(r.state().jack, Some(false));
    }

    #[test]
    fn button_press_updates_mask() {
        let mut r = rig();
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]);
        let s = r.state();
        assert_eq!(s.features, 0x0001_0000);
        assert!(!s.feature(Feature::Sbx));
        assert!(s.feature(Feature::CrystalVoice));
    }

    #[test]
    fn mask_read_and_output_and_param_replies_update_state() {
        let mut r = rig();
        r.device_sends(&[0x00, 0x23, 0x24, 0x00, 0x03]);
        assert_eq!(r.state().features, 3);
        r.device_sends(&[0x00, 0x23, 0x4d, 0x00, 0x02]);
        assert_eq!(r.state().output, Some(Output::Speakers));
        r.device_sends(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x08, 0x3f, 0x26, 0x66, 0x66]);
        let p = settings::find("sbx", "crystalizer-level").unwrap();
        assert_eq!(r.state().param(p), Some(Value::Float(0.65)));
        assert_eq!(r.changes.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn knob_and_unknown_reports_change_nothing() {
        let mut r = rig();
        r.device_sends(&[0x01]);
        r.device_sends(&[]);
        r.device_sends(&[0x00, 0x23, 0x4c, 0x00, 0x07]);
        assert_eq!(r.changes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn peer_gone_marks_the_device_absent() {
        let Rig {
            mut worker,
            x7,
            tx: _tx,
            changes,
        } = rig();
        drop(x7);
        worker.step();
        let s = worker.state.lock().unwrap().clone();
        assert_eq!(s.link, Link::Absent);
        assert!(s.last_error.is_some());
        assert_eq!(changes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn step_reports_closed_channel() {
        let Rig {
            mut worker,
            x7: _x7,
            tx,
            changes: _,
        } = rig();
        drop(tx);
        assert!(!worker.step());
    }

    fn crystalizer() -> &'static Param {
        settings::find("sbx", "crystalizer-level").unwrap()
    }

    #[test]
    fn set_param_sends_set_then_get_after_ack_then_state_from_reply() {
        let mut r = rig();
        r.tx.send(Command::SetParam(crystalizer(), Value::Float(0.25)))
            .unwrap();
        r.worker.step();
        assert_eq!(
            r.sent(),
            vec![
                0x20, 0x00, 0x16, 0x0a, 0xd5, 0x02, 0x08, 0x10, 0x20, 0x96, 0x3e, 0x80
            ]
        );
        // Nothing in the state yet: we never write our own value.
        assert_eq!(r.state().param(crystalizer()), None);
        r.device_sends(&[0x00, 0x20]);
        r.worker.step();
        assert_eq!(r.sent(), vec![0x26, 0x01, 0x96, 0x08]);
        r.device_sends(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x08, 0x3e, 0x80]);
        assert_eq!(r.state().param(crystalizer()), Some(Value::Float(0.25)));
    }

    #[test]
    fn set_output_waits_for_echo_then_reads_back() {
        let mut r = rig();
        r.tx.send(Command::SetOutput(Output::Speakers)).unwrap();
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x4e, 0x00, 0x00, 0x00, 0x80]);
        // The device pushes 23 4d before the echo on a speaker switch.
        r.device_sends(&[0x00, 0x23, 0x4d, 0x00, 0x02]);
        assert_eq!(r.state().output, Some(Output::Speakers));
        r.device_sends(&[0x00, 0x23, 0x4e, 0x00, 0x00, 0x00, 0x00, 0x80]);
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x4d]);
        r.device_sends(&[0x00, 0x23, 0x4d, 0x00, 0x02]);
        assert!(r.worker.inflight.is_none());
    }

    #[test]
    fn set_output_ignores_an_echo_with_the_other_value() {
        let mut r = rig();
        r.tx.send(Command::SetOutput(Output::Speakers)).unwrap();
        r.worker.step();
        let _ = r.sent();
        // Echo of a headphone switch (another client, or a stale report).
        r.device_sends(&[0x00, 0x23, 0x4e, 0x00, 0x01]);
        assert!(matches!(
            r.worker.inflight,
            Some(InFlight {
                op: Op::SetOutput(Output::Speakers),
                ..
            })
        ));
        assert!(r.worker.queue.is_empty());
    }

    #[test]
    fn set_feature_completes_only_on_its_own_echo() {
        let mut r = rig();
        r.tx.send(Command::SetFeature(Feature::Sbx, false)).unwrap();
        r.worker.step();
        // `sent()` trims trailing zeros, so `23 23 01 00` reads back as `23 23 01`.
        assert_eq!(r.sent(), vec![0x23, 0x23, 0x01]);
        // Hardware button toggles Scout meanwhile: state updates, set stays in flight.
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00]);
        assert_eq!(r.state().features, 3);
        assert!(r.worker.inflight.is_some());
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00]);
        assert_eq!(r.state().features, 2);
        assert!(r.worker.inflight.is_none());
    }

    #[test]
    fn refresh_queues_output_mask_and_every_row() {
        let mut r = rig();
        r.tx.send(Command::Refresh).unwrap();
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x4d]);
        assert_eq!(
            r.worker.queue.len(),
            1 + settings::PARAMS.len() + settings::SWITCHES.len()
        );
    }

    #[test]
    fn queued_sets_for_the_same_row_coalesce() {
        let mut r = rig();
        r.tx.send(Command::SetParam(crystalizer(), Value::Float(0.1)))
            .unwrap();
        r.worker.step(); // 0.1 goes in flight
        for v in [0.2, 0.3, 0.4] {
            r.tx.send(Command::SetParam(crystalizer(), Value::Float(v)))
                .unwrap();
        }
        r.worker.step();
        assert_eq!(r.worker.queue.len(), 1);
        let _ = r.sent();
        // Ack 0.1: no read-back while a newer set waits; the newest queued
        // value goes straight out: 0.4 = 0x3ecccccd.
        r.device_sends(&[0x00, 0x20]);
        assert_eq!(
            r.sent(),
            vec![
                0x20, 0x00, 0x16, 0x0a, 0xd5, 0x02, 0x08, 0x10, 0x20, 0x96, 0x3e, 0xcc, 0xcc, 0xcd
            ]
        );
    }

    #[test]
    fn invalid_set_param_is_reported_not_sent() {
        let mut r = rig();
        r.tx.send(Command::SetParam(crystalizer(), Value::Float(7.0)))
            .unwrap();
        r.worker.step();
        assert!(r.worker.queue.is_empty());
        assert!(r.worker.inflight.is_none());
        assert!(r.state().last_error.unwrap().contains("outside"));
    }

    fn wait_past_deadline(r: &mut Rig) {
        std::thread::sleep(r.worker.deadline + Duration::from_millis(5));
        r.worker.step();
    }

    /// Nothing more reached the device end of the socket.
    fn nothing_sent(x7: &mut UnixStream) -> bool {
        x7.set_nonblocking(true).unwrap();
        let mut buf = [0u8; 65];
        let quiet = match x7.read(&mut buf) {
            Ok(0) => true, // the worker closed its end
            Ok(_) => false,
            Err(e) => e.kind() == std::io::ErrorKind::WouldBlock,
        };
        x7.set_nonblocking(false).unwrap();
        quiet
    }

    #[test]
    fn a_missing_reply_stops_the_session_without_any_retry() {
        let mut r = rig();
        r.worker.deadline = Duration::from_millis(20);
        r.tx.send(Command::Refresh).unwrap();
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x4d]);
        wait_past_deadline(&mut r);
        let s = r.state();
        assert_eq!(s.link, Link::Unresponsive);
        assert!(s.last_error.unwrap().contains("Power-cycle the X7"));
        assert!(r.worker.dev.is_none());
        assert!(
            r.worker.queue.is_empty(),
            "the rest of the refresh is dropped"
        );
        for _ in 0..5 {
            r.worker.step();
        }
        assert!(nothing_sent(&mut r.x7), "no barrier, no retry");
    }

    #[test]
    fn a_hung_device_stays_unresponsive_until_it_reenumerates() {
        let (dev, mut x7) = crate::transport::fake_pair();
        let dev = Mutex::new(Some(dev));
        let attempts = Arc::new(AtomicUsize::new(0));
        let a = attempts.clone();
        let (_tx, rx) = channel();
        let mut worker = Worker::disconnected(
            Box::new(move || {
                if a.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(Error::Unresponsive("marked hung".into()))
                } else {
                    Ok(dev.lock().unwrap().take().expect("opened once"))
                }
            }),
            Duration::ZERO,
            Duration::ZERO,
            rx,
            Box::new(|| {}),
        );
        worker.step();
        assert_eq!(worker.state.lock().unwrap().link, Link::Unresponsive);
        worker.step();
        assert_eq!(worker.state.lock().unwrap().link, Link::Unresponsive);
        // Power cycled: the opener finds a new enumeration.
        worker.step();
        assert_eq!(worker.state.lock().unwrap().link, Link::Connecting);
        worker.step();
        let mut buf = [0u8; 65];
        x7.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[1..3], &[0x23, 0x4d], "starts over with a read");
    }

    #[test]
    fn the_same_enumeration_is_never_reopened_after_a_hang() {
        // The opener hands out fresh handles to the same (unknown) device,
        // as Device::open would when the marker could not be written.
        let ends = Arc::new(Mutex::new(Vec::new()));
        let e = ends.clone();
        let (_tx, rx) = channel();
        let mut worker = Worker::disconnected(
            Box::new(move || {
                let (dev, x7) = crate::transport::fake_pair();
                e.lock().unwrap().push(x7);
                Ok(dev)
            }),
            Duration::ZERO,
            Duration::ZERO,
            rx,
            Box::new(|| {}),
        );
        worker.deadline = Duration::from_millis(20);
        worker.step(); // opens, queues the refresh
        worker.step(); // GetOutput sent
        std::thread::sleep(Duration::from_millis(25));
        worker.step(); // no reply: hung
        assert_eq!(worker.state.lock().unwrap().link, Link::Unresponsive);
        for _ in 0..5 {
            worker.step();
        }
        assert_eq!(worker.state.lock().unwrap().link, Link::Unresponsive);
        let mut ends = ends.lock().unwrap();
        let mut buf = [0u8; 65];
        ends[0].read_exact(&mut buf).unwrap(); // the one GetOutput
        for x7 in ends.iter_mut() {
            assert!(nothing_sent(x7), "nothing more on any handle");
        }
    }

    #[test]
    fn a_command_answered_otherwise_is_rejected_not_a_hang() {
        let mut r = rig();
        r.worker.deadline = Duration::from_millis(20);
        r.tx.send(Command::SetOutput(Output::Speakers)).unwrap();
        r.worker.step();
        let _ = r.sent();
        // The device answers, but with the other output: it is alive.
        r.device_sends(&[0x00, 0x23, 0x4e, 0x00, 0x01]);
        wait_past_deadline(&mut r);
        let s = r.state();
        assert_eq!(s.link, Link::Ready);
        assert!(s.last_error.unwrap().contains("did not apply"));
        assert!(r.worker.dev.is_some());
        assert!(!r.worker.dev.as_ref().unwrap().is_tripped());
        // The next command goes out normally and clears the message.
        r.tx.send(Command::SetFeature(Feature::Sbx, true)).unwrap();
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x23, 0x01, 0x01]);
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x01, 0x01]);
        assert_eq!(r.state().last_error, None);
    }

    #[test]
    fn the_first_command_waits_for_the_settle_time() {
        let mut r = cold_rig();
        r.worker.settle = Duration::from_millis(60);
        r.worker.step(); // opens
        r.worker.step();
        assert!(nothing_sent(&mut r.x7), "quiet while the firmware boots");
        std::thread::sleep(Duration::from_millis(70));
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x4d]);
    }

    #[test]
    fn sends_pause_while_the_x7_records_and_resume_after() {
        let dir = std::env::temp_dir().join(format!("x7-session-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sub = dir.join("card1/pcm0c/sub0");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("status"), "state: RUNNING\n").unwrap();
        let (dev, mut x7) = crate::transport::fake_pair();
        let dev = dev.assume_verified().with_safety(
            crate::safety::Capture::Card(dir.join("card1")),
            crate::safety::DeviceId {
                bus: 1,
                dev: 2,
                boot: "b".into(),
            },
            crate::safety::Marker::at(dir.join("hang")),
        );
        let (tx, rx) = channel();
        let mut worker = Worker::with_device(dev, rx, Box::new(|| {}));
        worker.recheck = Duration::from_millis(10);
        tx.send(Command::SetFeature(Feature::Sbx, true)).unwrap();
        worker.step();
        assert!(nothing_sent(&mut x7));
        let s = worker.state.lock().unwrap().clone();
        assert_eq!(s.link, Link::Ready, "a recording is no fault");
        assert!(s.last_error.unwrap().contains("recording"));
        std::fs::write(sub.join("status"), "closed\n").unwrap();
        std::thread::sleep(Duration::from_millis(15));
        worker.step();
        let mut buf = [0u8; 65];
        x7.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[1..5], &[0x23, 0x23, 0x01, 0x01]);
        assert_eq!(worker.state.lock().unwrap().last_error, None);
    }

    #[test]
    fn a_write_before_any_answered_read_reads_first() {
        let (dev, mut x7) = crate::transport::fake_pair();
        let (tx, rx) = channel();
        let mut worker = Worker::with_device(dev, rx, Box::new(|| {}));
        tx.send(Command::SetFeature(Feature::Sbx, true)).unwrap();
        worker.step();
        let mut buf = [0u8; 65];
        x7.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[1..3], &[0x23, 0x4d], "read first");
        x7.write_all(&report(&[0x00, 0x23, 0x4d, 0x00, 0x01]))
            .unwrap();
        worker.step();
        x7.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[1..5], &[0x23, 0x23, 0x01, 0x01], "then the write");
    }

    #[test]
    fn session_reconnects_through_the_opener() {
        use std::sync::atomic::AtomicBool;
        let (dev, x7) = crate::transport::fake_pair();
        let dev = Mutex::new(Some(dev));
        let attempts = Arc::new(AtomicUsize::new(0));
        let a = attempts.clone();
        let opener = move || {
            let n = a.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(Error::NoDevice)
            } else {
                Ok(dev.lock().unwrap().take().expect("opened once"))
            }
        };
        let changed = Arc::new(AtomicBool::new(false));
        let c = changed.clone();
        let session = Session::start_with_opener(
            Box::new(opener),
            Duration::from_millis(5),
            Box::new(move || c.store(true, Ordering::SeqCst)),
        );
        // Wait for the third attempt to connect and the refresh to start.
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.state().link == Link::Absent && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        // Open, refresh in flight, nothing answered yet.
        assert_eq!(session.state().link, Link::Connecting);
        assert!(attempts.load(Ordering::SeqCst) >= 3);
        assert!(changed.load(Ordering::SeqCst));
        // Refresh began: the first op is the output read.
        let mut x7 = x7;
        let mut buf = [0u8; 65];
        x7.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[1..3], &[0x23, 0x4d]);
        session.shutdown();
        session.shutdown(); // second call is a no-op
    }

    #[test]
    fn session_shutdown_through_shared_ownership() {
        let (dev, _x7) = crate::transport::fake_pair();
        let dev = Mutex::new(Some(dev));
        let session = Arc::new(Session::start_with_opener(
            Box::new(move || Ok(dev.lock().unwrap().take().unwrap())),
            Duration::from_millis(5),
            Box::new(|| {}),
        ));
        let other = session.clone();
        session.shutdown();
        assert!(session.thread.lock().unwrap().is_none());
        drop(other);
    }

    #[test]
    fn next_op_is_sent_in_the_step_that_completes_the_previous_one() {
        let mut r = rig();
        r.tx.send(Command::Refresh).unwrap();
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x4d]);
        r.device_sends(&[0x00, 0x23, 0x4d, 0x00, 0x01]);
        // No extra step: GetFeatures is already in flight.
        assert!(matches!(
            r.worker.inflight,
            Some(InFlight {
                op: Op::GetFeatures,
                ..
            })
        ));
    }

    #[test]
    fn follow_up_get_is_skipped_when_a_newer_set_for_the_row_is_queued() {
        let mut r = rig();
        r.tx.send(Command::SetParam(crystalizer(), Value::Float(0.1)))
            .unwrap();
        r.worker.step(); // 0.1 in flight
        let _ = r.sent();
        r.tx.send(Command::SetParam(crystalizer(), Value::Float(0.2)))
            .unwrap();
        r.worker.step(); // 0.2 queued
        r.device_sends(&[0x00, 0x20]); // ack 0.1
        let get_queued = r.worker.queue.iter().any(|o| matches!(o, Op::GetParam(_)));
        let get_in_flight = matches!(
            r.worker.inflight,
            Some(InFlight {
                op: Op::GetParam(_),
                ..
            })
        );
        assert!(
            !get_queued && !get_in_flight,
            "stale read-back would jump the slider"
        );
    }

    #[test]
    fn disconnect_forgets_the_device_values() {
        let Rig {
            mut worker,
            mut x7,
            tx: _tx,
            changes: _,
        } = rig();
        x7.write_all(&report(&[0x00, 0x23, 0x4d, 0x00, 0x02]))
            .unwrap();
        worker.step();
        x7.write_all(&report(&[0x00, 0x23, 0x24, 0x00, 0x03]))
            .unwrap();
        worker.step();
        x7.write_all(&report(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x08, 0x3f, 0x00]))
            .unwrap();
        worker.step();
        x7.write_all(&report(&[0x00, 0x23, 0x12, 0x00, 0x04]))
            .unwrap();
        worker.step();
        assert_eq!(worker.state.lock().unwrap().output, Some(Output::Speakers));
        drop(x7);
        worker.step();
        let s = worker.state.lock().unwrap().clone();
        assert_eq!(s.output, None);
        assert_eq!(s.features, 0);
        assert!(s.params.is_empty());
        // The refresh never reads the jack, so a stale value would outlive
        // the reconnect.
        assert_eq!(s.jack, None);
    }

    #[test]
    fn a_validation_error_survives_the_next_completed_op() {
        let mut r = rig();
        r.tx.send(Command::SetParam(crystalizer(), Value::Float(7.0)))
            .unwrap();
        r.tx.send(Command::Refresh).unwrap();
        r.worker.step(); // rejected set recorded, GetOutput sent
        let _ = r.sent();
        r.device_sends(&[0x00, 0x23, 0x4d, 0x00, 0x01]); // GetOutput completes
        assert!(
            r.state().last_error.unwrap().contains("outside"),
            "only stale timeout text clears on completion"
        );
    }

    #[test]
    fn refresh_runs_ahead_of_commands_accepted_while_disconnected() {
        let (dev, mut x7) = crate::transport::fake_pair();
        let dev = Mutex::new(Some(dev));
        let (tx, rx) = channel();
        let mut worker = Worker::disconnected(
            Box::new(move || Ok(dev.lock().unwrap().take().unwrap())),
            Duration::from_millis(1),
            Duration::ZERO,
            rx,
            Box::new(|| {}),
        );
        tx.send(Command::SetOutput(Output::Speakers)).unwrap();
        worker.step(); // queues the set, then opens the device
        worker.step(); // sends the first op
        let mut buf = [0u8; 65];
        x7.read_exact(&mut buf).unwrap();
        assert_eq!(&buf[1..3], &[0x23, 0x4d], "GetOutput goes before the set");
    }

    fn gain() -> &'static Switch {
        settings::find_switch("headphones", "gain").unwrap()
    }

    #[test]
    fn set_switch_sends_toggle_then_reads_after_the_echo() {
        let mut r = rig();
        r.tx.send(Command::SetSwitch(gain(), Value::Enum(1)))
            .unwrap();
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x45, 0x01]);
        assert_eq!(r.state().switch(gain()), None);
        // The device answers with the read form first, then the echo.
        r.device_sends(&[0x00, 0x23, 0x44, 0x00, 0x01]);
        assert_eq!(r.state().switch(gain()), Some(Value::Enum(1)));
        assert!(
            r.worker.inflight.is_some(),
            "read form does not complete the set"
        );
        r.device_sends(&[0x00, 0x23, 0x45, 0x00, 0x01]);
        r.worker.step();
        assert_eq!(r.sent(), vec![0x23, 0x44]);
        r.device_sends(&[0x00, 0x23, 0x44, 0x00, 0x01]);
        assert!(r.worker.inflight.is_none());
    }

    #[test]
    fn set_echo_alone_writes_no_switch_state() {
        let mut r = rig();
        r.device_sends(&[0x00, 0x23, 0x45, 0x00, 0x01]);
        assert_eq!(r.state().switch(gain()), None);
        assert_eq!(r.changes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn set_output_target_completes_on_its_ack_then_reads() {
        let target = settings::find_switch("speakers", "output-target").unwrap();
        let mut r = rig();
        r.tx.send(Command::SetSwitch(target, Value::Enum(2)))
            .unwrap();
        r.worker.step();
        assert_eq!(r.sent(), vec![0x5a, 0x2c, 0x05, 0x00, 0x03]);
        // Pushes of 1a/15/1e arrive first, then an ack for another command.
        r.device_sends(&[0x00, 0x26, 0x01, 0x00, 0x96, 0x1a]);
        r.device_sends(&[0x00, 0x5a, 0x02, 0x02, 0x39]);
        assert!(r.worker.inflight.is_some());
        r.device_sends(&[0x00, 0x5a, 0x02, 0x0a, 0x2c, 0x00, 0x00, 0x03]);
        r.worker.step();
        assert_eq!(r.sent(), vec![0x5a, 0x2c, 0x01, 0x01]);
        r.device_sends(&[0x00, 0x5a, 0x2c, 0x05, 0x01, 0x03]);
        assert_eq!(r.state().switch(target), Some(Value::Enum(2)));
        assert!(r.worker.inflight.is_none());
    }

    #[test]
    fn standby_reply_mask_reads_as_on() {
        let standby = settings::find_switch("speakers", "auto-standby").unwrap();
        let mut r = rig();
        r.device_sends(&[0x00, 0x5a, 0x39, 0x05, 0x01, 0x00, 0x08]);
        assert_eq!(r.state().switch(standby), Some(Value::Bool(true)));
        r.device_sends(&[0x00, 0x5a, 0x39, 0x05, 0x01]);
        assert_eq!(r.state().switch(standby), Some(Value::Bool(false)));
    }

    #[test]
    fn invalid_set_switch_is_reported_not_sent() {
        let mut r = rig();
        r.tx.send(Command::SetSwitch(gain(), Value::Enum(5)))
            .unwrap();
        r.worker.step();
        assert!(r.worker.queue.is_empty());
        assert!(r.worker.inflight.is_none());
        assert!(r.state().last_error.unwrap().contains("gain"));
    }

    #[test]
    fn queued_sets_for_the_same_switch_coalesce() {
        let mut r = rig();
        r.tx.send(Command::SetSwitch(gain(), Value::Enum(1)))
            .unwrap();
        r.worker.step(); // in flight
        let _ = r.sent();
        r.tx.send(Command::SetSwitch(gain(), Value::Enum(0)))
            .unwrap();
        r.tx.send(Command::SetSwitch(gain(), Value::Enum(1)))
            .unwrap();
        r.worker.step();
        assert_eq!(r.worker.queue.len(), 1);
    }

    #[test]
    fn refresh_ends_with_the_switch_reads() {
        let mut r = rig();
        r.tx.send(Command::Refresh).unwrap();
        r.worker.step();
        let _ = r.sent();
        assert_eq!(
            r.worker.queue.len(),
            1 + settings::PARAMS.len() + settings::SWITCHES.len()
        );
        let tail: Vec<Op> = r
            .worker
            .queue
            .iter()
            .rev()
            .take(settings::SWITCHES.len())
            .copied()
            .collect();
        assert!(tail.iter().all(|o| matches!(o, Op::GetSwitch(_))));
    }

    #[test]
    fn set_echo_without_a_mask_updates_only_its_feature_bit() {
        let mut r = rig();
        r.device_sends(&[0x00, 0x23, 0x24, 0x00, 0x01, 0x00, 0x01, 0x00]); // sbx + voice on
        r.tx.send(Command::SetFeature(Feature::Sbx, false)).unwrap();
        r.worker.step();
        let _ = r.sent();
        // The echo of a host-sent set carries fid and value but zeros where
        // a device push carries the mask (round 2, window check).
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x00]);
        let s = r.state();
        assert!(!s.feature(Feature::Sbx));
        assert!(s.feature(Feature::CrystalVoice), "other bits are kept");
        assert!(r.worker.inflight.is_none());
        r.tx.send(Command::SetFeature(Feature::Sbx, true)).unwrap();
        r.worker.step();
        let _ = r.sent();
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0x01, 0x01]);
        assert_eq!(r.state().features, 0x0001_0001);
    }

    #[test]
    fn feature_echo_with_an_out_of_range_id_does_not_panic() {
        let mut r = rig();
        r.device_sends(&[0x00, 0x23, 0x24, 0x00, 0x01]); // sbx on
        // An id the device has never sent; the shift must not overflow.
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0x40, 0x01]);
        assert_eq!(r.state().features, 1, "unknown ids change no bit");
        r.device_sends(&[0x00, 0x23, 0x23, 0x00, 0xff, 0x01, 0x03]);
        assert_eq!(r.state().features, 3, "a real mask still wins");
    }
}
