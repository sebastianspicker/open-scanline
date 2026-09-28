//! Platen design tokens and their mapping onto egui.
//!
//! Every color, size, and duration used by the views comes from here. The
//! workspace is paper on a scanner bed: neutral surfaces, ink text, and one
//! accent, the vermilion scan lamp, reserved for the scan line, keyboard focus,
//! and problems that block a scan.

use egui::{CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle};

pub(in crate::inbound::gui::view) mod color {
    use egui::Color32;

    /// Workspace surface.
    pub(in crate::inbound::gui::view) const PAPER: Color32 = Color32::from_rgb(250, 250, 247);
    /// Header and action bar: the frame around the glass.
    pub(in crate::inbound::gui::view) const PLATEN: Color32 = Color32::from_rgb(239, 238, 233);
    /// Editable fields, menus, and unfilled sheet slots.
    pub(in crate::inbound::gui::view) const FIELD: Color32 = Color32::WHITE;
    /// Text and the primary action.
    pub(in crate::inbound::gui::view) const INK: Color32 = Color32::from_rgb(23, 25, 27);
    /// Secondary text: notes, labels of read-only values (6.6:1 on paper).
    pub(in crate::inbound::gui::view) const GRAPHITE: Color32 = Color32::from_rgb(86, 91, 96);
    /// Dividers and sheet outlines that carry no meaning on their own.
    pub(in crate::inbound::gui::view) const RULE: Color32 = Color32::from_rgb(220, 219, 213);
    /// Edges of interactive controls (3.4:1 against fields).
    pub(in crate::inbound::gui::view) const EDGE: Color32 = Color32::from_rgb(140, 139, 132);
    /// Hovered and pressed control fills.
    pub(in crate::inbound::gui::view) const HOVER: Color32 = Color32::from_rgb(244, 243, 238);
    pub(in crate::inbound::gui::view) const PRESSED: Color32 = Color32::from_rgb(232, 231, 225);
    /// Selected items and text selection.
    pub(in crate::inbound::gui::view) const SELECTED: Color32 = Color32::from_rgb(234, 231, 223);
    /// The scan lamp (4.9:1 on paper, usable for short text).
    pub(in crate::inbound::gui::view) const LAMP: Color32 = Color32::from_rgb(200, 57, 31);
    /// Lamp text on tinted or selected surfaces (5.5:1 on `SELECTED`).
    pub(in crate::inbound::gui::view) const LAMP_INK: Color32 = Color32::from_rgb(166, 46, 26);
    /// Background of a blocking problem.
    pub(in crate::inbound::gui::view) const LAMP_WASH: Color32 = Color32::from_rgb(250, 236, 231);
}

/// Font sizes in points.
pub(in crate::inbound::gui::view) mod size {
    pub(in crate::inbound::gui::view) const DISPLAY: f32 = 34.0;
    pub(in crate::inbound::gui::view) const DISPLAY_NARROW: f32 = 27.0;
    pub(in crate::inbound::gui::view) const LEAD: f32 = 17.0;
    pub(in crate::inbound::gui::view) const HEADING: f32 = 19.0;
    pub(in crate::inbound::gui::view) const BODY: f32 = 16.0;
    pub(in crate::inbound::gui::view) const LABEL: f32 = 14.0;
    pub(in crate::inbound::gui::view) const SMALL: f32 = 13.5;
    pub(in crate::inbound::gui::view) const MONO: f32 = 14.5;
    pub(in crate::inbound::gui::view) const MONO_SMALL: f32 = 12.5;
    /// Height of text fields, selects, and buttons.
    pub(in crate::inbound::gui::view) const CONTROL: f32 = 40.0;
    /// Height of the primary action.
    pub(in crate::inbound::gui::view) const ACTION: f32 = 46.0;
}

/// Spacing scale in points (8-point rhythm with a 4-point half step).
pub(in crate::inbound::gui::view) mod space {
    pub(in crate::inbound::gui::view) const XS: f32 = 4.0;
    pub(in crate::inbound::gui::view) const S: f32 = 8.0;
    pub(in crate::inbound::gui::view) const M: f32 = 12.0;
    pub(in crate::inbound::gui::view) const L: f32 = 16.0;
    pub(in crate::inbound::gui::view) const XL: f32 = 24.0;
    pub(in crate::inbound::gui::view) const XXL: f32 = 32.0;
    pub(in crate::inbound::gui::view) const XXXL: f32 = 48.0;
    /// Widest the job sheet grows.
    pub(in crate::inbound::gui::view) const SHEET: f32 = 1120.0;
    /// Below this width the two columns stack.
    pub(in crate::inbound::gui::view) const STACK: f32 = 760.0;
}

pub(in crate::inbound::gui::view) mod motion {
    /// One pass of the scan line over the current sheet slot.
    pub(in crate::inbound::gui::view) const SWEEP_SECONDS: f64 = 1.6;
}

pub(in crate::inbound::gui::view) const HAIRLINE: f32 = 1.0;
pub(in crate::inbound::gui::view) const FOCUS: f32 = 2.0;
pub(in crate::inbound::gui::view) const RADIUS: u8 = 2;

const BOLD: &str = "atkinson-bold";

pub(in crate::inbound::gui::view) fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub(in crate::inbound::gui::view) fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub(in crate::inbound::gui::view) fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub(in crate::inbound::gui) fn apply_theme(ctx: &egui::Context) {
    // Documents are judged against paper, so the workspace stays light
    // regardless of the system appearance.
    ctx.set_theme(egui::Theme::Light);
    ctx.set_fonts(fonts());
    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();
    spacing(&mut style.spacing);
    style.animation_time = 0.0;
    for (kind, font) in [
        (TextStyle::Body, body(size::BODY)),
        (TextStyle::Button, body(size::BODY)),
        (TextStyle::Small, body(size::SMALL)),
        (TextStyle::Heading, bold(size::HEADING)),
        (TextStyle::Monospace, mono(size::MONO)),
    ] {
        style.text_styles.insert(kind, font);
    }
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}

fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "atkinson",
            &include_bytes!("../../../../assets/fonts/AtkinsonHyperlegibleNext-Regular.ttf")[..],
        ),
        (
            BOLD,
            &include_bytes!("../../../../assets/fonts/AtkinsonHyperlegibleNext-Bold.ttf")[..],
        ),
        (
            "atkinson-mono",
            &include_bytes!("../../../../assets/fonts/AtkinsonHyperlegibleMono-Regular.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    // egui's bundled fonts stay behind Atkinson as fallbacks for scripts and
    // symbols it does not cover (Cyrillic, CJK fallbacks, emoji).
    let fallbacks = fonts.families[&FontFamily::Proportional].clone();
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "atkinson".into());
    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.insert(0, "atkinson-mono".into());
    let mut bold_family = vec![BOLD.to_owned(), "atkinson".to_owned()];
    bold_family.extend(fallbacks);
    fonts
        .families
        .insert(FontFamily::Name(BOLD.into()), bold_family);
    fonts
}

fn visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::light();
    let radius = CornerRadius::same(RADIUS);
    visuals.override_text_color = None;
    visuals.panel_fill = color::PAPER;
    visuals.window_fill = color::FIELD;
    visuals.window_stroke = Stroke::new(HAIRLINE, color::EDGE);
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow::NONE;
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.extreme_bg_color = color::FIELD;
    visuals.text_edit_bg_color = Some(color::FIELD);
    visuals.faint_bg_color = color::PLATEN;
    visuals.code_bg_color = color::PLATEN;
    visuals.hyperlink_color = color::INK;
    visuals.weak_text_color = Some(color::GRAPHITE);
    visuals.error_fg_color = color::LAMP;
    visuals.warn_fg_color = color::LAMP_INK;
    visuals.selection.bg_fill = color::SELECTED;
    visuals.selection.stroke = Stroke::new(FOCUS, color::LAMP_INK);
    visuals.text_cursor.stroke = Stroke::new(FOCUS, color::INK);
    visuals.collapsing_header_frame = false;
    visuals.indent_has_left_vline = false;
    visuals.striped = false;
    widgets(&mut visuals.widgets, radius);
    visuals
}

fn widgets(widgets: &mut egui::style::Widgets, radius: CornerRadius) {
    let ink = Stroke::new(HAIRLINE, color::INK);
    widgets.noninteractive.bg_fill = color::PAPER;
    widgets.noninteractive.weak_bg_fill = color::PAPER;
    widgets.noninteractive.bg_stroke = Stroke::new(HAIRLINE, color::RULE);
    widgets.noninteractive.fg_stroke = ink;
    for (state, fill, edge) in [
        (
            &mut widgets.inactive,
            color::FIELD,
            Stroke::new(HAIRLINE, color::EDGE),
        ),
        (&mut widgets.hovered, color::HOVER, ink),
        // Pressed and keyboard-focused widgets share this state; the lamp
        // ring makes focus visible without relying on fill alone.
        (
            &mut widgets.active,
            color::PRESSED,
            Stroke::new(FOCUS, color::LAMP_INK),
        ),
        (&mut widgets.open, color::FIELD, ink),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = edge;
        state.fg_stroke = ink;
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    widgets.noninteractive.corner_radius = radius;
}

fn spacing(spacing: &mut egui::style::Spacing) {
    spacing.item_spacing = egui::vec2(space::M, space::S);
    spacing.button_padding = egui::vec2(space::L, 9.0);
    spacing.interact_size = egui::vec2(40.0, size::CONTROL);
    spacing.icon_width = 18.0;
    spacing.icon_width_inner = 10.0;
    spacing.icon_spacing = space::S;
    spacing.menu_margin = egui::Margin::same(6);
    spacing.combo_height = 320.0;
}

#[cfg(test)]
mod tests {
    use super::{color, size};

    #[test]
    fn dark_system_appearance_preserves_light_workspace_style() {
        let context = egui::Context::default();
        super::apply_theme(&context);
        let _ = context.run(
            egui::RawInput {
                system_theme: Some(egui::Theme::Dark),
                ..Default::default()
            },
            |ctx| {
                assert_eq!(ctx.theme(), egui::Theme::Light);
                let style = ctx.style();
                assert!(!style.visuals.dark_mode);
                assert_eq!(style.text_styles[&egui::TextStyle::Body].size, size::BODY);
                assert_eq!(style.spacing.interact_size.y, size::CONTROL);
                assert_eq!(style.visuals.panel_fill, color::PAPER);
            },
        );
    }

    #[test]
    fn keyboard_focus_is_drawn_in_the_lamp_color() {
        let visuals = super::visuals();
        assert_eq!(visuals.widgets.active.bg_stroke.color, color::LAMP_INK);
        assert!(visuals.widgets.active.bg_stroke.width >= 2.0);
        assert_eq!(visuals.selection.stroke.color, color::LAMP_INK);
    }
}
