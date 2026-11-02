//! The window's own controls, drawn with egui primitives from the palette.
//! Each takes a `&mut` value and returns a `Response` whose `changed()`
//! the window turns into a session command. Nothing here touches the
//! session.

use std::ops::RangeInclusive;

use eframe::egui::{self, Align, Color32, CornerRadius, Response, Sense, Stroke, Vec2};
use x7::Output;

use crate::theme::{Palette, RADIUS, ROW_H};

/// Height of a vertical EQ slider.
pub const BAND_H: f32 = 110.0;
/// Band names and axis marks: smaller than `Small` so "250" fits a 23 px
/// column with a gap on each side.
pub const BAND_LABEL_SIZE: f32 = 10.5;
/// Fixed width in the EQ strip beside the eleven columns: the 30 px axis,
/// its 6 px gap and the 8 px gap after the rule.
pub const STRIP_FIXED: f32 = 44.0;
const SWITCH_SIZE: Vec2 = Vec2::new(34.0, 18.0);
const KNOB: f32 = 14.0;

/// Width of one EQ column so eleven of them fit beside the fixed spacing.
pub fn band_slot(available: f32) -> f32 {
    ((available - STRIP_FIXED) / 11.0).floor().clamp(22.0, 34.0)
}

/// Two-segment control for the output. `changed()` when a different
/// segment was clicked. `None` (output not read yet) fills neither.
pub fn segmented(ui: &mut egui::Ui, p: &Palette, output: &mut Option<Output>) -> Response {
    let mut changed = false;
    let inner = egui::Frame::new()
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(RADIUS + 1))
        .inner_margin(2.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut first: Option<Response> = None;
            for (label, o) in [
                ("Headphones", Output::Headphones),
                ("Speakers", Output::Speakers),
            ] {
                let on = *output == Some(o);
                let text = egui::RichText::new(label).color(if on { p.on_accent } else { p.text });
                let b = egui::Button::new(text)
                    .fill(if on { p.accent } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(CornerRadius::same(RADIUS))
                    .min_size(Vec2::new(96.0, 22.0));
                let r = ui.add(b);
                if r.clicked() && !on {
                    *output = Some(o);
                    changed = true;
                }
                first = Some(match first {
                    Some(f) => f | r,
                    None => r,
                });
            }
            first.expect("two segments")
        });
    let mut r = inner.inner;
    if changed {
        r.mark_changed();
    }
    r
}

/// A 34 by 18 track with a square knob that slides right when on, and the
/// label to its right. `changed()` on click.
pub fn switch(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: &str) -> Response {
    let (track, mut r) = ui.allocate_exact_size(SWITCH_SIZE, Sense::click());
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let t = ui.ctx().animate_bool_with_time(r.id, *on, 0.12);
    let enabled = ui.is_enabled();
    let fade = |c: Color32| if enabled { c } else { ui.visuals().gray_out(c) };
    let fill = if *on { p.accent } else { p.raised };
    let outline = if *on { p.accent } else { p.line };
    let painter = ui.painter();
    painter.rect(
        track,
        CornerRadius::same(RADIUS),
        fade(fill),
        Stroke::new(1.0, fade(outline)),
        egui::StrokeKind::Inside,
    );
    let x0 = track.left() + 2.0;
    let x1 = track.right() - 2.0 - KNOB;
    let x = x0 + (x1 - x0) * t;
    let knob = egui::Rect::from_min_size(egui::pos2(x, track.top() + 2.0), Vec2::splat(KNOB));
    let knob_fill = if *on { p.on_accent } else { p.text };
    painter.rect_filled(knob, CornerRadius::same(2), fade(knob_fill));
    if !label.is_empty() {
        ui.add_space(6.0);
        let l = ui.add(egui::Label::new(label).sense(Sense::click()));
        if l.clicked() {
            *on = !*on;
            r.mark_changed();
        }
        r |= l;
    }
    r
}

/// A horizontal slider over 0..100 in whole steps, writing back `v / 100`.
/// The caller places the readout.
pub fn level(ui: &mut egui::Ui, _p: &Palette, v: &mut f32, width: f32) -> Response {
    let mut pct = (*v * 100.0).round();
    ui.spacing_mut().slider_width = width;
    let r = ui.add(
        egui::Slider::new(&mut pct, 0.0..=100.0)
            .show_value(false)
            .step_by(1.0),
    );
    if r.changed() {
        *v = pct / 100.0;
    }
    r
}

/// A horizontal whole-step slider over a range, no built-in value.
pub fn ranged(
    ui: &mut egui::Ui,
    _p: &Palette,
    v: &mut f32,
    range: RangeInclusive<f32>,
    width: f32,
) -> Response {
    ui.spacing_mut().slider_width = width;
    ui.add(egui::Slider::new(v, range).show_value(false).step_by(1.0))
}

/// One EQ column: a fixed three-character readout above, a vertical
/// slider, the band label below.
pub fn band(
    ui: &mut egui::Ui,
    p: &Palette,
    v: &mut f32,
    range: RangeInclusive<f32>,
    label: &str,
    width: f32,
) -> Response {
    let size = Vec2::new(width, BAND_H + 40.0);
    ui.allocate_ui_with_layout(size, egui::Layout::top_down(Align::Center), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(
            egui::RichText::new(crate::labels::readout(crate::labels::Unit::Decibel, *v))
                .monospace()
                .small(),
        );
        ui.spacing_mut().slider_width = BAND_H;
        let r = ui.add(
            egui::Slider::new(v, range)
                .vertical()
                .show_value(false)
                .step_by(1.0)
                .trailing_fill(false),
        );
        ui.label(
            egui::RichText::new(label)
                .size(BAND_LABEL_SIZE)
                .color(p.text_weak),
        );
        r
    })
    .inner
}

/// A 7 px circle in the given colour, vertically centred on the row.
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(10.0, ROW_H.min(18.0)), Sense::hover());
    ui.painter().circle_filled(r.center(), 3.5, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;
    use eframe::egui;

    /// Run one frame with the widget inside a central panel.
    fn frame(add: impl FnOnce(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let mut add = Some(add);
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(add) = add.take() {
                egui::CentralPanel::default().show(ui, add);
            }
        });
        out.drop_without_applying_deltas();
    }

    #[test]
    fn every_widget_renders_in_a_headless_frame() {
        let p = theme::DARK;
        frame(|ui| {
            let mut o = Some(x7::Output::Headphones);
            segmented(ui, &p, &mut o);
            let mut unknown: Option<x7::Output> = None;
            segmented(ui, &p, &mut unknown);
            assert!(
                unknown.is_none(),
                "an untouched unknown output stays unknown"
            );
            let mut on = true;
            switch(ui, &p, &mut on, "Surround");
            let mut lv = 0.5;
            level(ui, &p, &mut lv, 200.0);
            let mut hz = 80.0;
            ranged(ui, &p, &mut hz, 10.0..=500.0, 200.0);
            let mut db = 2.0;
            band(ui, &p, &mut db, -12.0..=12.0, "1k", 30.0);
            status_dot(ui, p.accent);
        });
    }

    #[test]
    fn level_writes_back_a_fraction() {
        let p = theme::DARK;
        frame(|ui| {
            let mut v = 0.67;
            level(ui, &p, &mut v, 200.0);
            assert!(
                (v - 0.67).abs() < 1e-6,
                "untouched value survives the round trip"
            );
        });
    }

    #[test]
    fn band_slot_never_goes_below_22() {
        assert_eq!(band_slot(0.0), 22.0);
        assert_eq!(band_slot(-50.0), 22.0);
        assert_eq!(band_slot(416.0), 33.0, "11 * 33 + 44 = 407 fits 416");
        assert_eq!(band_slot(300.0), 23.0);
        assert_eq!(band_slot(500.0), 34.0, "capped");
    }
}
