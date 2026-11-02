//! The look: a named palette in light and dark, one builder for egui's
//! style, the embedded fonts, and the desktop's theme preference.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Vec2,
};

/// Content margin on every side of every zone.
pub const MARGIN: f32 = 12.0;
/// Height of one grid row.
pub const ROW_H: f32 = 26.0;
/// Label column in the page grids.
pub const LABEL_W: f32 = 130.0;
/// Readout column: wide enough for "500 Hz" in the mono face.
pub const READOUT_W: f32 = 52.0;
/// Corner radius of every widget.
pub const RADIUS: u8 = 3;

pub const INTER: &str = "Inter";
pub const CASCADIA: &str = "CascadiaMono";
const INTER_TTF: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
const CASCADIA_TTF: &[u8] = include_bytes!("../fonts/CascadiaMono-Regular.ttf");

/// The named colours a theme supplies. A theme file (Omarchy's colors.toml)
/// maps its names onto these; nothing else in the crate names a colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Window and page area.
    pub background: Color32,
    /// Header, rail, footer.
    pub surface: Color32,
    /// Active rail entry, widget fill.
    pub raised: Color32,
    /// Rules and outlines.
    pub line: Color32,
    pub text: Color32,
    /// Sublines, axis labels, band names.
    pub text_weak: Color32,
    /// The one warm colour.
    pub accent: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    /// Errors.
    pub warning: Color32,
}

pub const DARK: Palette = Palette {
    background: Color32::from_rgb(0x15, 0x16, 0x18),
    surface: Color32::from_rgb(0x1d, 0x1e, 0x21),
    raised: Color32::from_rgb(0x27, 0x29, 0x2d),
    line: Color32::from_rgb(0x34, 0x36, 0x3b),
    text: Color32::from_rgb(0xe6, 0xe3, 0xdc),
    text_weak: Color32::from_rgb(0x8d, 0x8c, 0x87),
    accent: Color32::from_rgb(0xd4, 0xa0, 0x4c),
    on_accent: Color32::from_rgb(0x1a, 0x14, 0x08),
    warning: Color32::from_rgb(0xd9, 0x82, 0x6a),
};

pub const LIGHT: Palette = Palette {
    background: Color32::from_rgb(0xf2, 0xf1, 0xee),
    surface: Color32::from_rgb(0xfa, 0xf9, 0xf7),
    raised: Color32::from_rgb(0xe8, 0xe6, 0xe1),
    line: Color32::from_rgb(0xd4, 0xd1, 0xcb),
    text: Color32::from_rgb(0x1f, 0x1e, 0x1c),
    text_weak: Color32::from_rgb(0x6f, 0x6d, 0x68),
    accent: Color32::from_rgb(0xb0, 0x7d, 0x2a),
    on_accent: Color32::from_rgb(0xff, 0xfb, 0xf2),
    warning: Color32::from_rgb(0xb4, 0x46, 0x2c),
};

/// The built-in palettes, as (light, dark). An Omarchy theme replaces
/// one of them through `apply_omarchy`; see `current`.
pub fn palettes() -> (Palette, Palette) {
    (LIGHT, DARK)
}

/// A palette read from an Omarchy theme and the egui slot it belongs in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Omarchy {
    pub palette: Palette,
    pub dark: bool,
}

/// Parse an Omarchy `colors.toml`. Every mapped key must be present and a
/// `#rrggbb` string; the error names the first key that is not. `mode =
/// "light"` is the light slot, anything else dark. Keys we do not map are
/// ignored.
pub fn parse_omarchy(text: &str) -> Result<Omarchy, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let color = |key: &str| -> Result<Color32, String> {
        let s = table
            .get(key)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("missing {key}"))?;
        hex(s).ok_or_else(|| format!("{key} is not #rrggbb: {s:?}"))
    };
    let background = color("background")?;
    let foreground = color("foreground")?;
    let accent = color("accent")?;
    let palette = Palette {
        background,
        surface: color("dark_background")?,
        raised: color("lighter_background")?,
        line: color("selection")?,
        text: foreground,
        text_weak: color("dark_foreground")?,
        accent,
        on_accent: on_accent(accent, background, foreground),
        warning: color("red")?,
    };
    let dark = table.get("mode").and_then(|v| v.as_str()) != Some("light");
    Ok(Omarchy { palette, dark })
}

/// `#rrggbb`, either case. Nothing else.
fn hex(s: &str) -> Option<Color32> {
    let digits = s.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(digits, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// Relative luminance per WCAG: sRGB linearised, then weighted.
fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio, 1 (none) to 21 (black on white).
fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// No theme names text on its accent: the page background or the text
/// colour, whichever contrasts more with the accent fill.
fn on_accent(accent: Color32, background: Color32, foreground: Color32) -> Color32 {
    if contrast(accent, background) >= contrast(accent, foreground) {
        background
    } else {
        foreground
    }
}

/// The applied Omarchy theme's colours, inside an Omarchy session only.
/// Omarchy exports `OMARCHY_PATH` into every session process and its own
/// shell reads it to know where it runs; without it the theme file is a
/// leftover in a home shared with another desktop and the built-ins apply.
/// `$HOME` only, the way `omarchy-theme-set` spells it.
pub fn omarchy_path() -> Option<std::path::PathBuf> {
    omarchy_path_in(std::env::var_os("OMARCHY_PATH"), std::env::var_os("HOME"))
}

fn omarchy_path_in(
    omarchy: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<std::path::PathBuf> {
    omarchy?;
    Some(std::path::PathBuf::from(home?).join(".local/state/omarchy/current/theme/colors.toml"))
}

/// What the window last applied from the theme file, keyed by the file's
/// modification time. `None` means the file was absent, or never seen.
#[derive(Default)]
pub struct Watch {
    seen: Option<std::time::SystemTime>,
}

impl Watch {
    /// Stat the file. `Some(Some(text))`: new or changed since the last
    /// call, with its content. `Some(None)`: it has gone. `None`: no
    /// change. A file that cannot be read after a successful stat counts
    /// as gone, so its return is noticed.
    pub fn poll(&mut self, path: &std::path::Path) -> Option<Option<String>> {
        let now = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if now == self.seen {
            return None;
        }
        self.seen = now;
        if now.is_none() {
            return Some(None);
        }
        match std::fs::read_to_string(path) {
            Ok(text) => Some(Some(text)),
            Err(_) => {
                self.seen = None;
                Some(None)
            }
        }
    }
}

/// The palette for the theme the context is showing right now: the one
/// `apply_omarchy` stored for that slot, else the built-in.
pub fn current(ctx: &egui::Context) -> Palette {
    let theme = ctx.theme();
    ctx.data(|d| d.get_temp::<Palette>(palette_id(theme)))
        .unwrap_or_else(|| {
            let (light, dark) = palettes();
            match theme {
                egui::Theme::Light => light,
                egui::Theme::Dark => dark,
            }
        })
}

/// egui's style from a palette: 1 px strokes, 3 px corners, no hover
/// growth, thin slider rails with a rectangular handle.
pub fn style(p: &Palette, dark: bool) -> egui::Style {
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.background;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.raised;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.text_weak);
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent);
    v.hyperlink_color = p.accent;
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(4);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.45 };
    let r = CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    for (ws, fill, stroke, fg) in [
        (&mut w.noninteractive, p.surface, p.line, p.text_weak),
        (&mut w.inactive, p.raised, p.line, p.text),
        (&mut w.hovered, p.raised, p.accent, p.text),
        (&mut w.open, p.raised, p.accent, p.text),
    ] {
        ws.bg_fill = fill;
        ws.weak_bg_fill = fill;
        ws.bg_stroke = Stroke::new(1.0, stroke);
        ws.fg_stroke = Stroke::new(1.0, fg);
        ws.corner_radius = r;
        ws.expansion = 0.0;
    }
    w.active.bg_fill = p.accent;
    w.active.weak_bg_fill = p.raised;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.on_accent);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    let mut s = egui::Style {
        visuals: v,
        text_styles: [
            (TextStyle::Heading, FontId::proportional(15.0)),
            (TextStyle::Body, FontId::proportional(13.5)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into(),
        ..Default::default()
    };
    s.spacing.item_spacing = Vec2::new(8.0, 6.0);
    s.spacing.button_padding = Vec2::new(8.0, 3.0);
    s.spacing.slider_rail_height = 3.0;
    s.spacing.interact_size = Vec2::new(40.0, 20.0);
    s.spacing.combo_width = 120.0;
    s.spacing.menu_margin = egui::Margin::same(6);
    s
}

/// Font definitions with Inter and Cascadia Mono first in their families;
/// egui's own fonts stay behind them as fallbacks.
pub fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    defs.font_data
        .insert(INTER.to_owned(), Arc::new(FontData::from_static(INTER_TTF)));
    defs.font_data.insert(
        CASCADIA.to_owned(),
        Arc::new(FontData::from_static(CASCADIA_TTF)),
    );
    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, INTER.to_owned());
    defs.families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, CASCADIA.to_owned());
    defs
}

/// Fonts and both styles, once, from the app constructor. egui then picks
/// the style by its theme preference.
pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    let (light, dark) = palettes();
    ctx.set_style_of(egui::Theme::Light, style(&light, false));
    ctx.set_style_of(egui::Theme::Dark, style(&dark, true));
}

/// Where `apply_omarchy` keeps the palette a slot's style was built from.
fn palette_id(theme: egui::Theme) -> egui::Id {
    egui::Id::new(("omarchy-palette", theme == egui::Theme::Dark))
}

/// Install an Omarchy theme: its style into the slot its `mode` names,
/// the built-in into the other, and egui's preference pinned to that
/// slot, because on Omarchy neither winit nor the portal knows which the
/// theme is. `None` restores both built-ins and the `System` preference.
pub fn apply_omarchy(ctx: &egui::Context, theme: Option<Omarchy>) {
    let (light, dark) = palettes();
    ctx.data_mut(|d| {
        d.remove::<Palette>(palette_id(egui::Theme::Light));
        d.remove::<Palette>(palette_id(egui::Theme::Dark));
    });
    ctx.set_style_of(egui::Theme::Light, style(&light, false));
    ctx.set_style_of(egui::Theme::Dark, style(&dark, true));
    match theme {
        Some(t) => {
            let slot = if t.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            };
            ctx.set_style_of(slot, style(&t.palette, t.dark));
            ctx.data_mut(|d| d.insert_temp(palette_id(slot), t.palette));
            ctx.set_theme(slot);
        }
        None => ctx.set_theme(egui::ThemePreference::System),
    }
    // A switch within one slot changes no theme, so egui would not ask
    // for a frame on its own; the window would keep the old text colours
    // until the next input or poll tick.
    ctx.request_repaint();
}

/// The portal's `color-scheme`: 1 dark, 2 light, 0 no preference.
pub fn decode_color_scheme(n: u32) -> Option<egui::Theme> {
    match n {
        1 => Some(egui::Theme::Dark),
        2 => Some(egui::Theme::Light),
        _ => None,
    }
}

/// `ReadOne` answers a `u32` inside one variant; the older `Read` wraps it
/// in two. Anything else is not a colour scheme.
fn scheme_number(v: &zbus::zvariant::Value<'_>) -> Option<u32> {
    match v {
        zbus::zvariant::Value::U32(n) => Some(*n),
        zbus::zvariant::Value::Value(inner) => scheme_number(inner),
        _ => None,
    }
}

/// The desktop's colour-scheme preference from the settings portal, or
/// None when there is no portal, no answer, or no preference. A blocking
/// round trip of a few milliseconds; call it from the main thread only on
/// startup and focus gain.
pub fn desktop_theme() -> Option<egui::Theme> {
    // zbus has no default method timeout: a portal being activated or a
    // stuck backend would otherwise block the window for the bus's 25 s.
    let conn = zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(PORTAL_TIMEOUT)
        .build()
        .ok()?;
    let read = |method: &str| -> Option<u32> {
        let reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Settings"),
                method,
                &("org.freedesktop.appearance", "color-scheme"),
            )
            .ok()?;
        let body = reply.body();
        let value: zbus::zvariant::Value<'_> = body.deserialize().ok()?;
        scheme_number(&value)
    };
    let n = read("ReadOne").or_else(|| read("Read"))?;
    decode_color_scheme(n)
}

/// Where winit reports no system theme (X11), egui shows its fallback
/// theme. Fill that from the portal; the preference itself stays
/// `System`, so a desktop that does report a theme is still followed.
pub fn apply_desktop_fallback(ctx: &egui::Context) {
    if let Some(theme) = desktop_theme() {
        ctx.options_mut(|o| o.fallback_theme = theme);
    }
}

/// Longest the window waits for the settings portal on a focus gain.
pub const PORTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

/// Detects the frame on which the window gained focus. Starts unfocused,
/// so the first frame after launch counts as a gain.
#[derive(Default)]
pub struct FocusEdge {
    was: bool,
}

impl FocusEdge {
    pub fn new() -> FocusEdge {
        FocusEdge::default()
    }

    pub fn gained(&mut self, focused: bool) -> bool {
        let gained = focused && !self.was;
        self.was = focused;
        gained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn styles_carry_their_mode() {
        assert!(style(&DARK, true).visuals.dark_mode);
        assert!(!style(&LIGHT, false).visuals.dark_mode);
    }

    #[test]
    fn active_widget_fill_is_the_accent() {
        let s = style(&DARK, true);
        assert_eq!(s.visuals.widgets.active.bg_fill, DARK.accent);
        assert_eq!(s.visuals.widgets.active.fg_stroke.color, DARK.on_accent);
        assert_eq!(s.visuals.widgets.inactive.bg_stroke.color, DARK.line);
    }

    #[test]
    fn install_puts_our_fonts_first() {
        let ctx = egui::Context::default();
        install(&ctx);
        let defs = fonts();
        assert_eq!(defs.families[&egui::FontFamily::Proportional][0], INTER);
        assert_eq!(defs.families[&egui::FontFamily::Monospace][0], CASCADIA);
        // Both themes were installed with our text sizes.
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let s = ctx.style_of(theme);
            assert_eq!(s.text_styles[&egui::TextStyle::Heading].size, 15.0);
            assert_eq!(s.text_styles[&egui::TextStyle::Monospace].size, 12.5);
        }
    }

    #[test]
    fn current_follows_the_context_theme() {
        let ctx = egui::Context::default();
        install(&ctx);
        ctx.set_theme(egui::Theme::Light);
        assert_eq!(current(&ctx), LIGHT);
        ctx.set_theme(egui::Theme::Dark);
        assert_eq!(current(&ctx), DARK);
    }

    #[test]
    fn decode_color_scheme_maps_only_one_and_two() {
        assert_eq!(decode_color_scheme(1), Some(egui::Theme::Dark));
        assert_eq!(decode_color_scheme(2), Some(egui::Theme::Light));
        assert_eq!(decode_color_scheme(0), None);
        assert_eq!(decode_color_scheme(7), None);
    }

    #[test]
    fn scheme_number_unwraps_nested_variants() {
        use zbus::zvariant::Value;
        assert_eq!(scheme_number(&Value::U32(2)), Some(2));
        let nested = Value::Value(Box::new(Value::U32(1)));
        assert_eq!(scheme_number(&nested), Some(1));
        assert_eq!(scheme_number(&Value::Str("dark".into())), None);
    }

    #[test]
    fn focus_edge_fires_once_per_gain() {
        let mut e = FocusEdge::new();
        assert!(e.gained(true), "first focused frame counts");
        assert!(!e.gained(true), "staying focused does not");
        assert!(!e.gained(false));
        assert!(e.gained(true), "a fresh gain counts again");
    }

    const NORD: &str = r##"
mode = "dark"

accent = "#81a1c1"
selection = "#434c5e"
muted = "#4c566a"

background = "#2e3440"
dark_background = "#222730"
darker_background = "#1a1e26"
lighter_background = "#3b4252"

foreground = "#d8dee9"
dark_foreground = "#667080"
light_foreground = "#e5e9f0"
bright_foreground = "#eceff4"

red = "#bf616a"
yellow = "#ebcb8b"
green = "#a3be8c"
"##;

    const LATTE: &str = r##"
mode = "light"
accent = "#1e66f5"
selection = "#ccd0da"
background = "#eff1f5"
dark_background = "#e3e4e8"
lighter_background = "#dce0e8"
foreground = "#4c4f69"
dark_foreground = "#9ca0b0"
red = "#d20f39"
"##;

    #[test]
    fn nord_maps_onto_the_palette_and_is_dark() {
        let t = parse_omarchy(NORD).unwrap();
        assert!(t.dark);
        let p = t.palette;
        assert_eq!(p.background, Color32::from_rgb(0x2e, 0x34, 0x40));
        assert_eq!(p.surface, Color32::from_rgb(0x22, 0x27, 0x30));
        assert_eq!(p.raised, Color32::from_rgb(0x3b, 0x42, 0x52));
        assert_eq!(p.line, Color32::from_rgb(0x43, 0x4c, 0x5e));
        assert_eq!(p.text, Color32::from_rgb(0xd8, 0xde, 0xe9));
        assert_eq!(p.text_weak, Color32::from_rgb(0x66, 0x70, 0x80));
        assert_eq!(p.accent, Color32::from_rgb(0x81, 0xa1, 0xc1));
        assert_eq!(p.warning, Color32::from_rgb(0xbf, 0x61, 0x6a));
        assert_eq!(p.on_accent, p.background, "dark text on a mid-blue accent");
    }

    #[test]
    fn latte_is_light() {
        let t = parse_omarchy(LATTE).unwrap();
        assert!(!t.dark);
        assert_eq!(t.palette.accent, Color32::from_rgb(0x1e, 0x66, 0xf5));
        assert_eq!(t.palette.on_accent, t.palette.background);
    }

    #[test]
    fn a_missing_mode_is_dark() {
        let text = NORD.replace("mode = \"dark\"\n", "");
        assert!(parse_omarchy(&text).unwrap().dark);
    }

    #[test]
    fn a_missing_key_fails_naming_it() {
        let text = NORD.replace("dark_foreground = \"#667080\"\n", "");
        let err = parse_omarchy(&text).unwrap_err();
        assert!(err.contains("dark_foreground"), "{err}");
    }

    #[test]
    fn a_bad_hex_fails_naming_the_key() {
        let text = NORD.replace("#434c5e", "434c5e");
        let err = parse_omarchy(&text).unwrap_err();
        assert!(err.contains("selection"), "{err}");
    }

    #[test]
    fn hex_accepts_uppercase_and_rejects_short_forms() {
        assert_eq!(hex("#89B4FA"), Some(Color32::from_rgb(0x89, 0xb4, 0xfa)));
        assert_eq!(hex("#fff"), None);
        assert_eq!(hex("fff"), None);
        assert_eq!(hex("#89b4fa00"), None);
    }

    #[test]
    fn on_accent_picks_the_better_contrast() {
        let bg = Color32::from_rgb(0x2e, 0x34, 0x40);
        let fg = Color32::from_rgb(0xd8, 0xde, 0xe9);
        // Nord's blue: the dark background wins (4.2 against 2.2).
        assert_eq!(on_accent(Color32::from_rgb(0x81, 0xa1, 0xc1), bg, fg), bg);
        // White's grey accent on a white page: the page wins (5.1 against 4.1).
        let white = Color32::from_rgb(0xff, 0xff, 0xff);
        let black = Color32::from_rgb(0x00, 0x00, 0x00);
        assert_eq!(
            on_accent(Color32::from_rgb(0x6e, 0x6e, 0x6e), white, black),
            white
        );
    }

    #[test]
    fn on_accent_takes_the_foreground_when_the_accent_is_dark() {
        let bg = Color32::from_rgb(0x10, 0x10, 0x10);
        let fg = Color32::from_rgb(0xe0, 0xe0, 0xe0);
        assert_eq!(on_accent(Color32::from_rgb(0x20, 0x20, 0x20), bg, fg), fg);
    }

    /// Every theme Omarchy ships parses. Runs only where Omarchy is
    /// installed; elsewhere (CI) it returns early.
    #[test]
    fn every_stock_theme_parses() {
        let Ok(dir) = std::fs::read_dir("/usr/share/omarchy/themes") else {
            return;
        };
        let mut seen = 0;
        for entry in dir.flatten() {
            let path = entry.path().join("colors.toml");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            parse_omarchy(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            seen += 1;
        }
        assert!(seen > 0, "an Omarchy with no themes");
    }
    #[test]
    fn omarchy_path_is_under_the_state_directory() {
        let p = omarchy_path_in(Some("/usr/share/omarchy".into()), Some("/home/x".into()))
            .expect("an Omarchy session with a home directory");
        assert_eq!(
            p,
            std::path::Path::new("/home/x/.local/state/omarchy/current/theme/colors.toml")
        );
    }

    #[test]
    fn omarchy_path_is_none_outside_an_omarchy_session() {
        assert_eq!(
            omarchy_path_in(None, Some("/home/x".into())),
            None,
            "a theme file left in a shared home is ignored under another desktop"
        );
        assert_eq!(
            omarchy_path_in(Some("/usr/share/omarchy".into()), None),
            None,
            "no home directory, no file"
        );
    }

    #[test]
    fn poll_reports_each_change_once() {
        use std::time::{Duration, SystemTime};
        let dir = std::env::temp_dir().join(format!("x7-theme-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("colors.toml");
        let mut w = Watch::default();

        assert_eq!(w.poll(&path), None, "absent from the start is not a change");

        std::fs::write(&path, "one").unwrap();
        assert_eq!(w.poll(&path), Some(Some("one".to_owned())));
        assert_eq!(w.poll(&path), None, "unchanged");

        // A rewrite within the same mtime tick would be invisible; give the
        // new file a later stamp the way a real theme set does.
        std::fs::write(&path, "two").unwrap();
        let later = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(w.poll(&path), Some(Some("two".to_owned())));
        assert_eq!(w.poll(&path), None);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(w.poll(&path), Some(None), "gone");
        assert_eq!(w.poll(&path), None, "still gone is not news");

        std::fs::write(&path, "three").unwrap();
        assert_eq!(w.poll(&path), Some(Some("three".to_owned())), "back again");

        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn apply_installs_the_theme_in_its_slot_and_pins_it() {
        let ctx = egui::Context::default();
        install(&ctx);
        let nord = parse_omarchy(NORD).unwrap();
        apply_omarchy(&ctx, Some(nord));
        assert_eq!(ctx.theme(), egui::Theme::Dark, "pinned to the file's mode");
        assert_eq!(current(&ctx), nord.palette);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            nord.palette.background
        );
        // The other slot keeps the built-in.
        ctx.set_theme(egui::Theme::Light);
        assert_eq!(current(&ctx), LIGHT);

        let latte = parse_omarchy(LATTE).unwrap();
        apply_omarchy(&ctx, Some(latte));
        assert_eq!(ctx.theme(), egui::Theme::Light);
        assert_eq!(current(&ctx), latte.palette);
        ctx.set_theme(egui::Theme::Dark);
        assert_eq!(current(&ctx), DARK, "Nord is gone from the dark slot");
    }

    #[test]
    fn apply_none_restores_the_built_ins_and_system() {
        let ctx = egui::Context::default();
        install(&ctx);
        apply_omarchy(&ctx, Some(parse_omarchy(NORD).unwrap()));
        apply_omarchy(&ctx, None);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::System
        );
        ctx.set_theme(egui::Theme::Dark);
        assert_eq!(current(&ctx), DARK);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            DARK.background
        );
        ctx.set_theme(egui::Theme::Light);
        assert_eq!(current(&ctx), LIGHT);
    }
    #[test]
    fn apply_requests_a_repaint_even_within_the_same_slot() {
        // A same-slot switch (one dark theme to another) flips no slot, so
        // egui itself would request nothing and the window would keep the
        // old text colours until the next input. The apply must ask.
        let ctx = egui::Context::default();
        install(&ctx);
        apply_omarchy(&ctx, Some(parse_omarchy(NORD).unwrap()));
        let frame = || {
            ctx.begin_pass(Default::default());
            ctx.end_pass().textures_delta.clear();
        };
        frame();
        let ours = |c: &egui::RepaintCause| c.file.ends_with("theme.rs");
        frame();
        assert!(
            !ctx.repaint_causes().iter().any(ours),
            "a plain frame asks nothing"
        );
        let osaka = parse_omarchy(&NORD.replace("#2e3440", "#111c18")).unwrap();
        apply_omarchy(&ctx, Some(osaka));
        frame();
        assert!(
            ctx.repaint_causes().iter().any(ours),
            "{:?}",
            ctx.repaint_causes()
        );
    }
}
