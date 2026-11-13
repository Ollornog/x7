//! The control window: a pinned header (output, SBX, Scout), a left rail
//! of pages, the page area and a status footer. Reads the session
//! snapshot every frame; sends commands. Words, units and widgets come
//! from `labels`, `theme` and `widgets`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Stroke, Vec2};
use x7::profile::{self, Profile};
use x7::settings::{self, Param, Switch};
use x7::{Command, Feature, Kind, Link, Session, State, Value};

use crate::labels::{self, Unit};
use crate::theme::{self, FocusEdge, LABEL_W, MARGIN, Palette, READOUT_W, ROW_H};
use crate::wake::Wake;
use crate::widgets;

// 116 and not the spec's 104: Inter is wider than the prototype font and
// "CrystalVoice" wrapped at 104.
const RAIL_W: f32 = 116.0;
/// Width egui's horizontal slider takes beyond `slider_width`.
const SLIDER_SLACK: f32 = 8.0;
/// How often a visible window stats the Omarchy theme file.
const THEME_POLL: Duration = Duration::from_secs(1);

/// The rail's entries, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Sbx,
    Equalizer,
    Headphones,
    Speakers,
    Voice,
    Profiles,
}

impl Page {
    const ALL: [Page; 6] = [
        Page::Sbx,
        Page::Equalizer,
        Page::Headphones,
        Page::Speakers,
        Page::Voice,
        Page::Profiles,
    ];

    /// The CLI group behind the page; Profiles has none.
    fn group(self) -> Option<&'static str> {
        match self {
            Page::Sbx => Some("sbx"),
            Page::Equalizer => Some("eq"),
            Page::Headphones => Some("headphones"),
            Page::Speakers => Some("speakers"),
            Page::Voice => Some("voice"),
            Page::Profiles => None,
        }
    }

    fn title(self) -> &'static str {
        match self.group() {
            Some(g) => labels::page(g),
            None => "Profiles",
        }
    }
}

/// A row from either settings table.
#[derive(Clone, Copy)]
enum Target {
    Param(&'static Param),
    Switch(&'static Switch),
}

impl Target {
    fn parts(self, s: &State) -> (&'static str, &'static str, Kind, Option<Value>) {
        match self {
            Target::Param(p) => (p.group, p.name, p.kind, s.param(p)),
            Target::Switch(w) => (w.group, w.name, w.kind, s.switch(w)),
        }
    }

    fn command(self, v: Value) -> Command {
        match self {
            Target::Param(p) => Command::SetParam(p, v),
            Target::Switch(w) => Command::SetSwitch(w, v),
        }
    }
}

fn param(group: &str, name: &str) -> &'static Param {
    settings::find(group, name).expect("settings row")
}

fn switch_row(group: &str, name: &str) -> &'static Switch {
    settings::find_switch(group, name).expect("switch row")
}

/// The full-range rows are `Choice` on=0.0 / off=1.0, shown as a switch.
/// A device value matching neither name gives None: shown as a number, no
/// control, never sent.
fn full_range_state(value: Option<Value>) -> Option<bool> {
    match value {
        Some(Value::Enum(0)) => Some(true),
        Some(Value::Enum(1)) => Some(false),
        _ => None,
    }
}

const CUSTOM_PRESET: &str = "Custom";

/// Preset names are lowercase identifiers in the table (`new-age`); the
/// label capitalises each word, so a new preset row needs no GUI change.
fn preset_label(name: &str) -> String {
    name.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A setting the Windows control panel confirms before sending, because
/// both can damage equipment. The window asks with the same wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Confirm {
    HighGain,
    HighPower,
}

impl Confirm {
    fn title(self) -> &'static str {
        match self {
            Confirm::HighGain => "High gain",
            Confirm::HighPower => "High Power Amplification",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Confirm::HighGain => {
                "Setting to high gain may damage your headphones if their \
                 impedance does not match the specified level of 600 ohms."
            }
            Confirm::HighPower => {
                "Please ensure that you are using the upgraded high output power \
                 adapter before enabling this option. Enabling this feature \
                 without the upgraded power adapter may compromise the product's \
                 safety and reliability."
            }
        }
    }

    fn question(self) -> &'static str {
        match self {
            Confirm::HighGain => "Set to high gain?",
            Confirm::HighPower => "Enable the high power amplifier?",
        }
    }

    fn command(self) -> Command {
        match self {
            Confirm::HighGain => {
                let gain = switch_row("headphones", "gain");
                let Kind::Choice(choices) = gain.kind else {
                    unreachable!("gain is a Choice")
                };
                let high = choices
                    .iter()
                    .position(|(n, _)| *n == "high")
                    .expect("high");
                Command::SetSwitch(gain, Value::Enum(high))
            }
            Confirm::HighPower => Command::SetSwitch(
                switch_row("headphones", "high-power-amp"),
                Value::Bool(true),
            ),
        }
    }
}

/// Commands held back until every confirmation they need is accepted.
/// Sent as one batch on the last accept; dropped whole on a cancel, so a
/// profile never half-applies.
struct Pending {
    commands: Vec<Command>,
    /// Remaining questions, front first.
    asks: Vec<Confirm>,
    /// The profile name to mark as picked once the batch is sent.
    pick: Option<String>,
}

impl Pending {
    fn single(c: Confirm) -> Pending {
        Pending {
            commands: vec![c.command()],
            asks: vec![c],
            pick: None,
        }
    }

    fn current(&self) -> Confirm {
        self.asks[0]
    }

    /// Answer the current question with yes. The batch comes back once
    /// the last question is answered.
    fn accept(&mut self) -> Option<Vec<Command>> {
        self.asks.remove(0);
        if self.asks.is_empty() {
            Some(std::mem::take(&mut self.commands))
        } else {
            None
        }
    }
}

/// The confirmations a batch needs: each dangerous setting it would
/// change from the device's current value, in command order.
fn asks_for(commands: &[Command], s: &State) -> Vec<Confirm> {
    let gain = switch_row("headphones", "gain");
    let hpa = switch_row("headphones", "high-power-amp");
    let mut asks = Vec::new();
    for c in commands {
        let Command::SetSwitch(w, v) = c else {
            continue;
        };
        let ask = if std::ptr::eq(*w, gain) && *v == Value::Enum(1) {
            Confirm::HighGain
        } else if std::ptr::eq(*w, hpa) && *v == Value::Bool(true) {
            Confirm::HighPower
        } else {
            continue;
        };
        if s.switch(w).as_ref() != Some(v) && !asks.contains(&ask) {
            asks.push(ask);
        }
    }
    asks
}

/// The word beside the status dot at the rail's foot.
fn status_word(link: Link, error: Option<&str>) -> &'static str {
    match (link, error) {
        (Link::Ready, None) => "Connected",
        (Link::Ready, Some(_)) => "Error",
        (Link::Connecting, _) => "Connecting",
        (Link::Absent, _) => "Not connected",
        (Link::Unresponsive, _) => "Not answering",
    }
}

/// Which page is shown, and whether the profile list must be re-read
/// from disk before the Profiles page draws.
struct Nav {
    page: Page,
    profiles_stale: bool,
}

impl Nav {
    /// The window was shown or refocused: a profile may have been saved
    /// from the CLI meanwhile.
    fn on_focus_gain(&mut self) {
        self.profiles_stale = true;
    }

    fn go(&mut self, page: Page) {
        if page == Page::Profiles && self.page != Page::Profiles {
            self.profiles_stale = true;
        }
        self.page = page;
    }
}

pub struct Controls {
    session: Arc<Session>,
    quit: Arc<AtomicBool>,
    /// Without a tray host, closing the window must quit or the app is
    /// unreachable. Owned by the tray, which flips it as the host comes and goes.
    tray_ok: Arc<AtomicBool>,
    /// Told on the close frame, so a show during the close opens a new window.
    wake: Arc<Wake>,
    /// Inside an Omarchy session: the window has no close button there, so
    /// a hint names the key that puts it back in the tray.
    on_omarchy: bool,
    /// Slider values being dragged, shown instead of the state until
    /// released, keyed by (group, name) so both tables share it.
    dragging: HashMap<(&'static str, &'static str), f32>,
    nav: Nav,
    focus: FocusEdge,
    /// The Omarchy theme file, polled on each focus gain and once a second
    /// while visible.
    omarchy: theme::Watch,
    last_poll: Option<Instant>,
    /// Label column of the page being drawn: `LABEL_W`, or wider when one
    /// of the page's switch labels needs it ("Direct Mode for S/PDIF in").
    label_w: f32,
    /// Control column of the page being drawn: sliders and combos take it.
    slider_w: f32,
    /// Profile UI state: the name typed for Save, the saved name picked for
    /// Load, and the last file error, which the session never sees.
    profile_name: String,
    profile_pick: Option<String>,
    profile_error: Option<String>,
    /// Saved names, read when the Profiles page is entered and after a save.
    profiles: Vec<String>,
    /// Commands waiting for the user's yes, see `Pending`.
    pending: Option<Pending>,
}

impl Controls {
    pub fn new(
        session: Arc<Session>,
        quit: Arc<AtomicBool>,
        tray_ok: Arc<AtomicBool>,
        wake: Arc<Wake>,
    ) -> Controls {
        Controls {
            session,
            quit,
            tray_ok,
            wake,
            on_omarchy: theme::omarchy_path().is_some(),
            dragging: HashMap::new(),
            nav: Nav {
                page: Page::Sbx,
                profiles_stale: false,
            },
            focus: FocusEdge::new(),
            omarchy: theme::Watch::default(),
            last_poll: None,
            label_w: LABEL_W,
            slider_w: 0.0,
            profile_name: String::new(),
            profile_pick: None,
            profile_error: None,
            profiles: Vec::new(),
            pending: None,
        }
    }

    // ---- drag handling, as before ------------------------------------------

    fn shown_float(&self, key: (&'static str, &'static str), f: f32) -> f32 {
        *self.dragging.get(&key).unwrap_or(&f)
    }

    fn after_slider(
        &mut self,
        key: (&'static str, &'static str),
        r: &egui::Response,
        v: f32,
        t: Target,
    ) {
        if r.changed() {
            self.dragging.insert(key, v);
            self.session.send(t.command(Value::Float(v)));
        }
        if !r.dragged() && !r.has_focus() {
            self.dragging.remove(&key);
        }
    }

    // ---- zones ------------------------------------------------------------------

    fn header(&self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let ready = s.link == Link::Ready;
        ui.add_enabled_ui(ready, |ui| {
            ui.horizontal(|ui| {
                let mut out = s.output;
                if widgets::segmented(ui, p, &mut out).changed()
                    && let Some(o) = out
                {
                    self.session.send(Command::SetOutput(o));
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let mut scout = s.feature(Feature::Scout);
                    if widgets::switch(ui, p, &mut scout, "Scout").changed() {
                        self.session
                            .send(Command::SetFeature(Feature::Scout, scout));
                    }
                    ui.add_space(10.0);
                    let mut sbx = s.feature(Feature::Sbx);
                    if widgets::switch(ui, p, &mut sbx, "SBX").changed() {
                        self.session.send(Command::SetFeature(Feature::Sbx, sbx));
                    }
                });
            });
        });
    }

    fn rail(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let ready = s.link == Link::Ready;
        ui.add_enabled_ui(ready, |ui| {
            let entry = |ui: &mut egui::Ui, page: Page, active: bool| -> egui::Response {
                let text =
                    egui::RichText::new(page.title()).color(if active { p.accent } else { p.text });
                let b = egui::Button::new(text)
                    .fill(if active {
                        p.raised
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .min_size(Vec2::new(ui.available_width(), ROW_H));
                let r = ui.add(b);
                if active {
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(ui.max_rect().left() - 6.0, r.rect.top()),
                        Vec2::new(2.0, r.rect.height()),
                    );
                    ui.painter().rect_filled(bar, 0.0, p.accent);
                }
                r
            };
            let mut clicked = None;
            for page in Page::ALL.iter().copied().filter(|pg| *pg != Page::Profiles) {
                if entry(ui, page, self.nav.page == page).clicked() {
                    clicked = Some(page);
                }
            }
            ui.with_layout(egui::Layout::bottom_up(Align::Min), |ui| {
                // Bottom-up: the status row lands at the very foot, the
                // Profiles entry above it.
                Self::status_row(ui, s, p);
                ui.add_space(4.0);
                if entry(ui, Page::Profiles, self.nav.page == Page::Profiles).clicked() {
                    clicked = Some(Page::Profiles);
                }
            });
            if let Some(page) = clicked {
                self.nav.go(page);
            }
        });
    }

    /// Status dot and word at the rail's foot; an error while connected
    /// turns both to the warning colour and shows its text on hover.
    fn status_row(ui: &mut egui::Ui, s: &State, p: &Palette) {
        let error = if s.link == Link::Ready {
            s.last_error.as_deref()
        } else {
            None
        };
        let word = status_word(s.link, error);
        let color = match (s.link, error) {
            (Link::Ready, Some(_)) => p.warning,
            (Link::Ready, None) => p.accent,
            (Link::Connecting, _) => {
                let t = ui.input(|i| i.time) as f32;
                let a = 0.65 + 0.35 * (t * std::f32::consts::TAU).sin();
                ui.ctx().request_repaint_after(Duration::from_millis(40));
                p.accent.gamma_multiply(a)
            }
            (Link::Absent, _) => p.text_weak,
            (Link::Unresponsive, _) => p.warning,
        };
        let r = ui
            .horizontal(|ui| {
                ui.add_space(6.0);
                widgets::status_dot(ui, color);
                let text_color = if error.is_some() {
                    p.warning
                } else {
                    p.text_weak
                };
                ui.label(egui::RichText::new(word).small().color(text_color));
            })
            .response;
        if let Some(e) = error {
            r.on_hover_text(e);
        }
    }

    /// The centred block shown instead of a page while not Ready.
    fn state_block(&self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let (title, sub) = match s.link {
            Link::Connecting => (
                "Connecting",
                "The X7 answers a few seconds after it powers on",
            ),
            Link::Unresponsive => (
                "Not answering",
                "Unplug the X7's power supply for 10 seconds, then plug it back in. Nothing is sent to it until then",
            ),
            _ => ("Not connected", "Power on or plug in the Sound Blaster X7"),
        };
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.35);
            ui.heading(title);
            ui.label(egui::RichText::new(sub).color(p.text_weak));
            if matches!(s.link, Link::Absent | Link::Unresponsive)
                && let Some(e) = &s.last_error
            {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(e)
                        .small()
                        .monospace()
                        .color(p.text_weak),
                );
            }
        });
    }

    fn page_title(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette, page: Page) {
        ui.horizontal(|ui| {
            match page {
                Page::Sbx | Page::Voice => {
                    let feature = if page == Page::Sbx {
                        Feature::Sbx
                    } else {
                        Feature::CrystalVoice
                    };
                    let mut on = s.feature(feature);
                    if widgets::switch(ui, p, &mut on, "").changed() {
                        self.session.send(Command::SetFeature(feature, on));
                    }
                    ui.add_space(2.0);
                }
                Page::Equalizer => {
                    let enabled = param("eq", "enabled");
                    if let Some(Value::Bool(b)) = s.param(enabled) {
                        let mut on = b;
                        if widgets::switch(ui, p, &mut on, "").changed() {
                            self.session
                                .send(Command::SetParam(enabled, Value::Bool(on)));
                        }
                        ui.add_space(2.0);
                    }
                }
                _ => {}
            }
            ui.heading(page.title());
            if page == Page::Equalizer {
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    self.eq_preset_combo(ui, s);
                });
            }
        });
        let r = ui.available_rect_before_wrap();
        ui.painter()
            .hline(r.x_range(), r.top() + 2.0, Stroke::new(1.0, p.line));
        ui.add_space(8.0);
    }

    // ---- rows ---------------------------------------------------------------------

    /// A fixed-width grid cell.
    fn cell<R>(
        ui: &mut egui::Ui,
        w: f32,
        layout: egui::Layout,
        add: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        ui.allocate_ui_with_layout(Vec2::new(w, ROW_H), layout, add)
            .inner
    }

    fn label_cell(&self, ui: &mut egui::Ui, p: &Palette, text: &str, indent: bool) {
        Self::cell(
            ui,
            self.label_w,
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                if indent {
                    ui.add_space(16.0);
                    ui.label(egui::RichText::new(text).color(p.text_weak));
                } else {
                    ui.label(text);
                }
            },
        );
    }

    fn readout_cell(ui: &mut egui::Ui, text: &str) {
        Self::cell(
            ui,
            READOUT_W,
            egui::Layout::right_to_left(Align::Center),
            |ui| {
                ui.label(egui::RichText::new(text).monospace());
            },
        );
    }

    /// A `Bool` row on its own line: label cell holds the switch and the
    /// words. Returns the current value so children can dim on it.
    /// With a `guard`, switching on asks first (see `Confirm`); switching
    /// off never does.
    fn switch_line(
        &mut self,
        ui: &mut egui::Ui,
        s: &State,
        p: &Palette,
        t: Target,
        guard: Option<Confirm>,
    ) -> Option<bool> {
        let (group, name, _, current) = t.parts(s);
        let on = match current {
            Some(Value::Bool(b)) => b,
            _ => {
                self.label_cell(ui, p, labels::row(group, name), false);
                ui.label("…");
                ui.label("");
                ui.end_row();
                return None;
            }
        };
        let mut v = on;
        Self::cell(
            ui,
            self.label_w,
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                if widgets::switch(ui, p, &mut v, labels::row(group, name)).changed() {
                    match guard {
                        Some(c) if v => self.pending = Some(Pending::single(c)),
                        _ => self.session.send(t.command(Value::Bool(v))),
                    }
                }
            },
        );
        ui.label("");
        ui.label("");
        ui.end_row();
        Some(on)
    }

    /// A feature and its level on one line: switch and words, slider,
    /// percent readout. The slider is dimmed while the switch is off.
    fn feature_line(
        &mut self,
        ui: &mut egui::Ui,
        s: &State,
        p: &Palette,
        on_row: &'static Param,
        level_row: &'static Param,
        slider_w: f32,
    ) -> Option<bool> {
        let on = match s.param(on_row) {
            Some(Value::Bool(b)) => b,
            _ => {
                self.label_cell(ui, p, labels::row(on_row.group, on_row.name), false);
                ui.label("…");
                ui.label("");
                ui.end_row();
                return None;
            }
        };
        let mut v = on;
        Self::cell(
            ui,
            self.label_w,
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                if widgets::switch(ui, p, &mut v, labels::row(on_row.group, on_row.name)).changed()
                {
                    self.session.send(Command::SetParam(on_row, Value::Bool(v)));
                }
            },
        );
        self.level_cells(ui, s, p, level_row, on, slider_w);
        ui.end_row();
        Some(on)
    }

    /// Slider and readout cells for a level or ranged float row.
    fn level_cells(
        &mut self,
        ui: &mut egui::Ui,
        s: &State,
        p: &Palette,
        row: &'static Param,
        enabled: bool,
        slider_w: f32,
    ) {
        let key = (row.group, row.name);
        let Kind::Float { min, max } = row.kind else {
            ui.label("");
            ui.label("");
            return;
        };
        let unit = labels::unit(row).unwrap_or(Unit::Percent);
        match s.param(row) {
            Some(Value::Float(f)) => {
                let mut v = self.shown_float(key, f);
                let r = ui
                    .add_enabled_ui(enabled, |ui| match unit {
                        Unit::Percent => widgets::level(ui, p, &mut v, slider_w),
                        _ => widgets::ranged(ui, p, &mut v, min..=max, slider_w),
                    })
                    .inner;
                self.after_slider(key, &r, v, Target::Param(row));
                ui.add_enabled_ui(enabled, |ui| {
                    Self::readout_cell(ui, &labels::readout(unit, v))
                });
            }
            _ => {
                ui.label("…");
                ui.label("");
            }
        }
    }

    /// An indented float row under a parent switch.
    fn child_slider_line(
        &mut self,
        ui: &mut egui::Ui,
        s: &State,
        p: &Palette,
        row: &'static Param,
        enabled: bool,
        slider_w: f32,
    ) {
        self.label_cell(ui, p, labels::row(row.group, row.name), true);
        self.level_cells(ui, s, p, row, enabled, slider_w);
        ui.end_row();
    }

    /// A combo over a `Choice` or `Enum` row. Indented and dimmable so it
    /// also serves as a child row (Mode under Smart Volume): three radios
    /// would widen the control column past the window at 440 px.
    fn combo_line(
        &mut self,
        ui: &mut egui::Ui,
        s: &State,
        p: &Palette,
        t: Target,
        indent: bool,
        enabled: bool,
    ) {
        let (group, name, kind, current) = t.parts(s);
        self.label_cell(ui, p, labels::row(group, name), indent);
        let names: Vec<&'static str> = match kind {
            Kind::Enum(names) => names.to_vec(),
            Kind::Choice(choices) => choices.iter().map(|(n, _)| *n).collect(),
            _ => Vec::new(),
        };
        match current {
            Some(Value::Enum(i)) => {
                let mut sel = i;
                ui.add_enabled_ui(enabled, |ui| {
                    egui::ComboBox::from_id_salt((group, name))
                        .width(self.slider_w)
                        .truncate()
                        .selected_text(
                            names
                                .get(sel)
                                .map_or("?", |n| labels::value(group, name, n)),
                        )
                        .show_ui(ui, |ui| {
                            for (n, raw) in names.iter().enumerate() {
                                ui.selectable_value(&mut sel, n, labels::value(group, name, raw));
                            }
                        });
                });
                if sel != i {
                    self.session.send(t.command(Value::Enum(sel)));
                }
            }
            Some(other) => {
                ui.label(kind.format(&other));
            }
            None => {
                ui.label("…");
            }
        }
        ui.label("");
        ui.end_row();
    }

    /// Full-range rows: a `Choice` shown as a switch.
    fn full_range_line(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette, row: &'static Param) {
        match full_range_state(s.param(row)) {
            Some(on) => {
                let mut v = on;
                Self::cell(
                    ui,
                    self.label_w,
                    egui::Layout::left_to_right(Align::Center),
                    |ui| {
                        if widgets::switch(ui, p, &mut v, labels::row(row.group, row.name))
                            .changed()
                        {
                            self.session
                                .send(Command::SetParam(row, Value::Enum(if v { 0 } else { 1 })));
                        }
                    },
                );
                ui.label("");
            }
            None => {
                self.label_cell(ui, p, labels::row(row.group, row.name), false);
                match s.param(row) {
                    Some(v) => ui.label(row.format(&v)),
                    None => ui.label("…"),
                };
            }
        }
        ui.label("");
        ui.end_row();
    }

    /// A weak one-line note under a row, spanning the grid.
    fn note_line(&self, ui: &mut egui::Ui, p: &Palette, text: &str) {
        Self::cell(
            ui,
            self.label_w,
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                ui.add_space(16.0);
                ui.label(egui::RichText::new(text).small().color(p.text_weak));
            },
        );
        ui.label("");
        ui.label("");
        ui.end_row();
    }

    /// The page grid. The label column is `LABEL_W` or the widest of the
    /// page's switch labels plus the switch, so no label wraps or clips
    /// and the slider column takes what is left.
    fn grid<R>(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        switch_labels: &[&str],
        add: impl FnOnce(&mut Self, &mut egui::Ui, f32) -> R,
    ) -> R {
        let font = egui::TextStyle::Body.resolve(ui.style());
        let widest = switch_labels
            .iter()
            .map(|t| {
                ui.painter()
                    .layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
        // 34 px switch, 6 px gap, 4 px slack.
        self.label_w = LABEL_W.max(widest + 44.0);
        // egui's slider allocates a little more than its rail width, so the
        // readout column ran past the page edge by a few pixels ("80 Hz"
        // clipped on Speakers); hold that back from the slider.
        let slider_w =
            (ui.available_width() - self.label_w - READOUT_W - 2.0 * 10.0 - SLIDER_SLACK).max(60.0);
        self.slider_w = slider_w;
        egui::Grid::new(id)
            .num_columns(3)
            .spacing([10.0, 4.0])
            .show(ui, |ui| add(self, ui, slider_w))
            .inner
    }

    // ---- pages ----------------------------------------------------------------------

    fn sbx_page(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let f = |n| param("sbx", n);
        let names = [
            "surround",
            "crystalizer",
            "bass",
            "smart-volume",
            "dialog-plus",
        ]
        .map(|n| labels::row("sbx", n));
        self.grid(ui, "sbx", &names, |me, ui, w| {
            me.feature_line(ui, s, p, f("surround"), f("surround-level"), w);
            me.feature_line(ui, s, p, f("crystalizer"), f("crystalizer-level"), w);
            let bass = me.feature_line(ui, s, p, f("bass"), f("bass-level"), w);
            me.child_slider_line(ui, s, p, f("bass-crossover"), bass.unwrap_or(false), w);
            let smart = me.feature_line(ui, s, p, f("smart-volume"), f("smart-volume-level"), w);
            me.combo_line(
                ui,
                s,
                p,
                Target::Param(f("smart-volume-mode")),
                true,
                smart.unwrap_or(false),
            );
            me.feature_line(ui, s, p, f("dialog-plus"), f("dialog-plus-level"), w);
        });
    }

    fn voice_page(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let f = |n| param("voice", n);
        let names = [
            "noise-reduction",
            "echo-cancellation",
            "focus",
            "smart-volume",
        ]
        .map(|n| labels::row("voice", n));
        self.grid(ui, "voice", &names, |me, ui, w| {
            me.switch_line(ui, s, p, Target::Param(f("noise-reduction")), None);
            me.switch_line(ui, s, p, Target::Param(f("echo-cancellation")), None);
            me.feature_line(ui, s, p, f("focus"), f("focus-angle"), w);
            me.feature_line(ui, s, p, f("smart-volume"), f("smart-volume-level"), w);
        });
    }

    fn headphones_page(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        // Gain sits outside the grid: its two radios and the note need the
        // page width, and the grid's label column is wide on this page.
        let gain = switch_row("headphones", "gain");
        let Kind::Choice(choices) = gain.kind else {
            unreachable!("gain is a Choice")
        };
        ui.label(labels::row("headphones", "gain"));
        ui.indent("gain", |ui| match s.switch(gain) {
            Some(Value::Enum(i)) => {
                let mut sel = i;
                for (n, (name, _)) in choices.iter().enumerate() {
                    ui.radio_value(&mut sel, n, labels::value("headphones", "gain", name));
                }
                if sel != i {
                    if choices[sel].0 == "high" {
                        self.pending = Some(Pending::single(Confirm::HighGain));
                    } else {
                        self.session
                            .send(Command::SetSwitch(gain, Value::Enum(sel)));
                    }
                }
            }
            Some(other) => {
                ui.label(gain.format(&other));
            }
            None => {
                ui.label("…");
            }
        });
        ui.add_space(6.0);
        let names = ["direct-mode", "direct-mode-spdif", "high-power-amp"]
            .map(|n| labels::row("headphones", n));
        self.grid(ui, "headphones", &names, |me, ui, _w| {
            for name in ["direct-mode", "direct-mode-spdif", "high-power-amp"] {
                let guard = (name == "high-power-amp").then_some(Confirm::HighPower);
                me.switch_line(
                    ui,
                    s,
                    p,
                    Target::Switch(switch_row("headphones", name)),
                    guard,
                );
            }
        });
    }

    fn speakers_page(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let f = |n| param("speakers", n);
        let names = [
            "front-full-range",
            "rear-full-range",
            "bass-redirection",
            "subwoofer-gain",
            "auto-standby",
        ]
        .map(|n| labels::row("speakers", n));
        // The two combos have no readout, so they sit in a grid of their own
        // and take the control and readout columns together; "Line and
        // amplifier" does not fit the control column alone on this page.
        self.grid(ui, "speakers-combos", &names, |me, ui, w| {
            me.slider_w = w + 10.0 + READOUT_W;
            me.combo_line(ui, s, p, Target::Param(f("layout")), false, true);
            me.combo_line(
                ui,
                s,
                p,
                Target::Switch(switch_row("speakers", "output-target")),
                false,
                true,
            );
        });
        self.grid(ui, "speakers", &names, |me, ui, w| {
            me.full_range_line(ui, s, p, f("front-full-range"));
            me.full_range_line(ui, s, p, f("rear-full-range"));
            let redirect = me.switch_line(ui, s, p, Target::Param(f("bass-redirection")), None);
            me.child_slider_line(ui, s, p, f("crossover"), redirect.unwrap_or(false), w);
            me.switch_line(ui, s, p, Target::Param(f("subwoofer-gain")), None);
            me.switch_line(
                ui,
                s,
                p,
                Target::Switch(switch_row("speakers", "auto-standby")),
                None,
            );
            me.note_line(ui, p, "after about 20 minutes idle");
        });
    }

    // ---- Equalizer ------------------------------------------------------------

    fn eq_preset_combo(&mut self, ui: &mut egui::Ui, s: &State) {
        let preamp = param("eq", "preamp");
        let bands = settings::eq_bands();
        let current: Option<Vec<f32>> = bands
            .iter()
            .map(|p| s.param(p).map(|v| p.to_raw(&v)))
            .collect();
        let matching = current
            .and_then(|v| <[f32; 10]>::try_from(v).ok())
            .and_then(settings::eq_preset_matching);
        egui::ComboBox::from_id_salt("eq-preset")
            .selected_text(
                matching.map_or_else(|| CUSTOM_PRESET.to_owned(), |p| preset_label(p.name)),
            )
            .show_ui(ui, |ui| {
                for preset in settings::EQ_PRESETS {
                    let selected = matching.is_some_and(|m| std::ptr::eq(m, preset));
                    if ui
                        .selectable_label(selected, preset_label(preset.name))
                        .clicked()
                    {
                        self.session
                            .send(Command::SetParam(preamp, Value::Float(0.0)));
                        for (band, db) in bands.iter().zip(preset.bands) {
                            self.session.send(Command::SetParam(band, Value::Float(db)));
                        }
                    }
                }
            });
    }

    /// The strip: dB axis, ten bands, a rule, the preamp. Dimmed while the
    /// master is off; values stay editable.
    fn eq_page(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let preamp = param("eq", "preamp");
        let bands = settings::eq_bands();
        let enabled = matches!(s.param(param("eq", "enabled")), Some(Value::Bool(true)));
        let slot = widgets::band_slot(ui.available_width());
        // Dimmed, not disabled: the sliders stay draggable with the master off.
        ui.scope(|ui| {
            if !enabled {
                ui.visuals_mut().override_text_color = Some(p.text_weak);
                ui.visuals_mut().widgets.inactive.fg_stroke.color = p.text_weak;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // The axis: the three marks painted at the rail's top, middle
                // and bottom, "dB" in the band-name row.
                ui.allocate_ui_with_layout(
                    Vec2::new(30.0, widgets::BAND_H + 40.0),
                    egui::Layout::top_down(Align::Max),
                    |ui| {
                        ui.set_width(30.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(egui::RichText::new(" ").monospace().small());
                        let (rail, _) = ui.allocate_exact_size(
                            Vec2::new(30.0, widgets::BAND_H),
                            egui::Sense::hover(),
                        );
                        let font = egui::FontId::proportional(widgets::BAND_LABEL_SIZE);
                        let color = ui.visuals().text_color();
                        for (t, y) in [
                            ("+12", rail.top() + 6.0),
                            ("0", rail.center().y),
                            ("-12", rail.bottom() - 6.0),
                        ] {
                            ui.painter().text(
                                egui::pos2(rail.right() - 2.0, y),
                                egui::Align2::RIGHT_CENTER,
                                t,
                                font.clone(),
                                color,
                            );
                        }
                        ui.label(
                            egui::RichText::new("dB")
                                .size(widgets::BAND_LABEL_SIZE)
                                .color(p.text_weak),
                        );
                    },
                );
                ui.add_space(6.0);
                for band in bands {
                    self.band_column(ui, s, p, band, slot);
                }
                let x = ui.cursor().left() + 4.0;
                let top = ui.cursor().top();
                ui.painter().vline(
                    x,
                    top..=top + widgets::BAND_H + 36.0,
                    Stroke::new(1.0, p.line),
                );
                ui.add_space(8.0);
                self.band_column(ui, s, p, preamp, slot);
            });
        });
    }

    fn band_column(
        &mut self,
        ui: &mut egui::Ui,
        s: &State,
        p: &Palette,
        row: &'static Param,
        slot: f32,
    ) {
        let Kind::Float { min, max } = row.kind else {
            return;
        };
        let key = (row.group, row.name);
        match s.param(row) {
            Some(Value::Float(f)) => {
                let mut v = self.shown_float(key, f);
                let r = widgets::band(
                    ui,
                    p,
                    &mut v,
                    min..=max,
                    labels::row(row.group, row.name),
                    slot,
                );
                self.after_slider(key, &r, v, Target::Param(row));
            }
            _ => {
                ui.allocate_ui_with_layout(
                    Vec2::new(slot, widgets::BAND_H + 40.0),
                    egui::Layout::top_down(Align::Center),
                    |ui| {
                        ui.label("…");
                    },
                );
            }
        }
    }

    // ---- Profiles ---------------------------------------------------------------

    /// Saved names as rows; clicking one applies it. Save as below, then
    /// the last file error.
    fn profiles_page(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        if self.nav.profiles_stale {
            self.profiles = profile::list();
            self.nav.profiles_stale = false;
        }
        if self.profiles.is_empty() {
            ui.label(egui::RichText::new("No saved profiles yet").color(p.text_weak));
        }
        let mut load: Option<String> = None;
        for name in &self.profiles {
            let selected = self.profile_pick.as_deref() == Some(name.as_str());
            let text = egui::RichText::new(name).color(if selected { p.accent } else { p.text });
            let b = egui::Button::new(text)
                .fill(if selected {
                    p.raised
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::NONE)
                .min_size(Vec2::new(ui.available_width(), ROW_H));
            if ui.add(b).clicked() {
                load = Some(name.clone());
            }
        }
        if let Some(name) = load {
            self.profile_error =
                match profile::path_for(&name).and_then(|path| profile::load(&path)) {
                    Ok(prof) => {
                        let commands: Vec<Command> =
                            prof.entries().iter().map(|e| e.command()).collect();
                        let asks = asks_for(&commands, s);
                        if asks.is_empty() {
                            for c in commands {
                                self.session.send(c);
                            }
                            self.profile_pick = Some(name);
                        } else {
                            // High gain or high power in the file: ask first,
                            // and apply nothing until every answer is yes. The
                            // row is marked picked only once the batch is sent.
                            self.pending = Some(Pending {
                                commands,
                                asks,
                                pick: Some(name),
                            });
                        }
                        None
                    }
                    Err(e) => Some(e.to_string()),
                };
        }
        ui.add_space(10.0);
        let r = ui.available_rect_before_wrap();
        ui.painter()
            .hline(r.x_range(), r.top(), Stroke::new(1.0, p.line));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.profile_name)
                    .hint_text("Save as")
                    .desired_width(ui.available_width() - 70.0),
            );
            let name = self.profile_name.trim().to_owned();
            if ui
                .add_enabled(!name.is_empty(), egui::Button::new("Save"))
                .clicked()
            {
                self.profile_error = match profile::path_for(&name)
                    .and_then(|path| profile::save(&path, &Profile::from_state(s)))
                {
                    Ok(()) => {
                        self.profile_pick = Some(name);
                        self.profile_name.clear();
                        self.nav.profiles_stale = true;
                        None
                    }
                    Err(e) => Some(e.to_string()),
                };
            }
        });
        if let Some(e) = &self.profile_error {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(e).small().color(p.warning));
        }
    }

    fn page_area(&mut self, ui: &mut egui::Ui, s: &State, p: &Palette) {
        let page = self.nav.page;
        self.page_title(ui, s, p, page);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match page {
                Page::Sbx => self.sbx_page(ui, s, p),
                Page::Equalizer => self.eq_page(ui, s, p),
                Page::Headphones => self.headphones_page(ui, s, p),
                Page::Speakers => self.speakers_page(ui, s, p),
                Page::Voice => self.voice_page(ui, s, p),
                Page::Profiles => self.profiles_page(ui, s, p),
            });
    }
}

impl eframe::App for Controls {
    // eframe 0.36: `ui` replaces `update` and runs only while the window is
    // shown; a hidden window costs nothing per frame.
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        // Wayland cannot hide a window, so closing destroys this one and the
        // tray builds the next. Without a tray host, closing is quitting.
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.tray_ok.load(Ordering::Relaxed) {
                self.quit.store(true, Ordering::Relaxed);
            }
            self.wake.closing();
        }
        // On X11 winit reports no theme; ask the portal on every focus gain.
        // Each window starts unfocused, so its first frame counts as one.
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let focus_gain = self.focus.gained(focused);
        if focus_gain {
            theme::apply_desktop_fallback(&ctx);
            self.nav.on_focus_gain();
        }
        // Inside an Omarchy session, the theme file on each focus gain (the
        // show frame included) and once a second while visible. A stat.
        // Applied, it pins egui's slot, so the portal's fallback above is
        // consulted only while there is no theme file. Other desktops skip
        // the poll and its repaint tick entirely.
        if let Some(path) = theme::omarchy_path() {
            let poll_due = focus_gain || self.last_poll.is_none_or(|t| t.elapsed() >= THEME_POLL);
            if poll_due {
                self.last_poll = Some(Instant::now());
                let applied = match self.omarchy.poll(&path) {
                    Some(Some(text)) => {
                        let parsed = theme::parse_omarchy(&text);
                        if let Err(e) = &parsed {
                            eprintln!("x7-gui: Omarchy theme ignored: {e}");
                        }
                        theme::apply_omarchy(&ctx, parsed.ok());
                        true
                    }
                    Some(None) => {
                        theme::apply_omarchy(&ctx, None);
                        true
                    }
                    None => false,
                };
                // `root` took its style before this frame began; the panels
                // below inherit it, so give it the style just installed or
                // this frame draws new fills under old text colours.
                if applied {
                    root.set_style(ctx.global_style());
                }
            }
            ctx.request_repaint_after(THEME_POLL);
        }
        let p = theme::current(&ctx);
        let s = self.session.state();
        if s.link != Link::Ready {
            self.dragging.clear();
        }
        let zone = |fill: Color32| {
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::same(MARGIN as i8))
        };

        egui::Panel::top("header")
            .frame(zone(p.surface))
            .show_separator_line(false)
            .show(root, |ui| self.header(ui, &s, &p));
        egui::Panel::left("rail")
            .exact_size(RAIL_W)
            .resizable(false)
            // The rail's entries run close to its edges (6 px) so the 2 px
            // accent bar sits on the edge and "CrystalVoice" fits in 104 px.
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .inner_margin(egui::Margin::symmetric(6, MARGIN as i8)),
            )
            .show_separator_line(false)
            .show(root, |ui| self.rail(ui, &s, &p));
        if self.on_omarchy && self.tray_ok.load(Ordering::Relaxed) {
            egui::Panel::bottom("hint")
                .frame(
                    egui::Frame::new()
                        .fill(p.background)
                        .inner_margin(egui::Margin::symmetric(MARGIN as i8, 4)),
                )
                .show_separator_line(false)
                .show(root, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("Super+W puts the window back in the tray")
                                .small()
                                .color(p.text_weak),
                        );
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(zone(p.background))
            .show(root, |ui| {
                if s.link == Link::Ready {
                    self.page_area(ui, &s, &p);
                } else {
                    self.state_block(ui, &s, &p);
                }
            });
        if let Some(c) = self.pending.as_ref().map(Pending::current) {
            let m = egui::Modal::new(egui::Id::new("confirm")).show(&ctx, |ui| {
                ui.set_width(320.0);
                ui.heading(c.title());
                ui.add_space(6.0);
                ui.label(c.text());
                ui.add_space(6.0);
                ui.label(c.question());
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Enable").clicked() {
                        let released = self
                            .pending
                            .as_mut()
                            .and_then(|pd| pd.accept().map(|cmds| (cmds, pd.pick.take())));
                        if let Some((commands, pick)) = released {
                            for c in commands {
                                self.session.send(c);
                            }
                            if let Some(name) = pick {
                                self.profile_pick = Some(name);
                            }
                            self.pending = None;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending = None;
                    }
                });
            });
            if m.should_close() {
                self.pending = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_range_state_of_an_unmatched_value_is_none() {
        assert_eq!(
            full_range_state(Some(Value::Enum(0))),
            Some(true),
            "index 0 is `on`"
        );
        assert_eq!(full_range_state(Some(Value::Enum(1))), Some(false));
        assert_eq!(
            full_range_state(Some(Value::Float(2.0))),
            None,
            "device value matches no name"
        );
        assert_eq!(full_range_state(None), None);
    }

    #[test]
    fn pages_in_rail_order_with_profiles_last() {
        assert_eq!(Page::ALL.len(), 6);
        assert_eq!(Page::ALL[0], Page::Sbx);
        assert_eq!(Page::ALL[5], Page::Profiles);
        assert_eq!(Page::Equalizer.group(), Some("eq"));
        assert_eq!(Page::Profiles.group(), None);
    }

    #[test]
    fn entering_the_profiles_page_marks_the_list_for_refresh() {
        let mut nav = Nav {
            page: Page::Sbx,
            profiles_stale: false,
        };
        nav.go(Page::Speakers);
        assert!(!nav.profiles_stale);
        nav.go(Page::Profiles);
        assert!(nav.profiles_stale, "the list is read on entry");
        nav.profiles_stale = false;
        nav.go(Page::Profiles);
        assert!(!nav.profiles_stale, "staying on the page does not re-read");
    }

    #[test]
    fn confirmations_send_the_dangerous_setting_only() {
        let gain = switch_row("headphones", "gain");
        let hpa = switch_row("headphones", "high-power-amp");
        match Confirm::HighGain.command() {
            Command::SetSwitch(w, v) => {
                assert!(std::ptr::eq(w, gain));
                assert_eq!(v, Value::Enum(1), "index of `high`");
            }
            other => panic!("{other:?}"),
        }
        match Confirm::HighPower.command() {
            Command::SetSwitch(w, v) => {
                assert!(std::ptr::eq(w, hpa));
                assert_eq!(v, Value::Bool(true));
            }
            other => panic!("{other:?}"),
        }
        assert!(Confirm::HighGain.text().contains("600 ohms"));
        assert!(Confirm::HighPower.text().contains("power adapter"));
    }

    #[test]
    fn a_profile_batch_asks_for_each_dangerous_setting_it_changes() {
        let gain = switch_row("headphones", "gain");
        let hpa = switch_row("headphones", "high-power-amp");
        let batch = vec![
            Command::SetSwitch(gain, Value::Enum(1)),
            Command::SetSwitch(hpa, Value::Bool(true)),
            Command::SetParam(param("sbx", "surround"), Value::Bool(true)),
        ];
        let mut s = State::default();
        assert_eq!(
            asks_for(&batch, &s),
            vec![Confirm::HighGain, Confirm::HighPower]
        );
        // Already at high gain: only the amplifier is a change.
        s.switches.insert(gain.wire, 1.0);
        assert_eq!(asks_for(&batch, &s), vec![Confirm::HighPower]);
        let safe = vec![
            Command::SetSwitch(gain, Value::Enum(0)),
            Command::SetSwitch(hpa, Value::Bool(false)),
        ];
        assert!(asks_for(&safe, &State::default()).is_empty());
    }

    #[test]
    fn a_pending_batch_is_released_only_by_the_last_accept() {
        let hpa = switch_row("headphones", "high-power-amp");
        let mut pending = Pending {
            commands: vec![Command::SetSwitch(hpa, Value::Bool(true))],
            asks: vec![Confirm::HighGain, Confirm::HighPower],
            pick: None,
        };
        assert_eq!(pending.current(), Confirm::HighGain);
        assert!(pending.accept().is_none(), "one more to answer");
        assert_eq!(pending.current(), Confirm::HighPower);
        let released = pending.accept().expect("last accept releases the batch");
        assert_eq!(released.len(), 1);
    }

    #[test]
    fn a_pending_batch_carries_the_profile_pick_until_it_is_sent() {
        let hpa = switch_row("headphones", "high-power-amp");
        let mut pending = Pending {
            commands: vec![Command::SetSwitch(hpa, Value::Bool(true))],
            asks: vec![Confirm::HighPower],
            pick: Some("danger".to_owned()),
        };
        assert!(Pending::single(Confirm::HighGain).pick.is_none());
        assert!(pending.accept().is_some());
        assert_eq!(pending.pick.take().as_deref(), Some("danger"));
    }

    #[test]
    fn a_focus_gain_marks_the_profile_list_stale() {
        let mut nav = Nav {
            page: Page::Profiles,
            profiles_stale: false,
        };
        nav.on_focus_gain();
        assert!(nav.profiles_stale);
    }

    #[test]
    fn preset_labels_derive_from_the_identifier() {
        assert_eq!(preset_label("flat"), "Flat");
        assert_eq!(preset_label("new-age"), "New Age");
        assert_eq!(preset_label("rock"), "Rock");
    }

    #[test]
    fn status_word_names_an_error_only_while_connected() {
        assert_eq!(status_word(Link::Ready, None), "Connected");
        assert_eq!(status_word(Link::Ready, Some("no reply")), "Error");
        assert_eq!(status_word(Link::Connecting, Some("timeout")), "Connecting");
        assert_eq!(status_word(Link::Absent, Some("gone")), "Not connected");
        assert_eq!(status_word(Link::Unresponsive, None), "Not answering");
    }
}
