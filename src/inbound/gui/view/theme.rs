use egui::{Color32, FontFamily, FontId, Stroke, TextStyle};

pub(in crate::inbound::gui) fn apply_theme(ctx: &egui::Context) {
    // Keep the approved light workspace independent of the system appearance.
    ctx.set_theme(egui::Theme::Light);
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Cantarell".into(),
        egui::FontData::from_static(include_bytes!(
            "../../../../assets/fonts/Cantarell-Regular.ttf"
        ))
        .into(),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Cantarell".into());
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_rgb(250, 250, 249);
    visuals.window_fill = visuals.panel_fill;
    visuals.override_text_color = Some(Color32::from_rgb(22, 31, 43));
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.faint_bg_color = Color32::from_rgb(239, 242, 244);
    visuals.selection.bg_fill = Color32::from_rgb(58, 103, 136);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.hyperlink_color = Color32::from_rgb(39, 87, 124);
    visuals.widgets.inactive.bg_fill = Color32::WHITE;
    visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(205, 211, 216));
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(237, 243, 247);
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, visuals.hyperlink_color);
    visuals.widgets.active.bg_stroke = Stroke::new(2.0, visuals.hyperlink_color);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(219, 223, 227));
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size = egui::vec2(40.0, 38.0);
    for (kind, size) in [
        (TextStyle::Body, 17.0),
        (TextStyle::Button, 17.0),
        (TextStyle::Small, 14.0),
        (TextStyle::Heading, 25.0),
    ] {
        style
            .text_styles
            .insert(kind, FontId::new(size, FontFamily::Proportional));
    }
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);
}

#[cfg(test)]
mod tests {
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
                assert_eq!(style.text_styles[&egui::TextStyle::Body].size, 17.0);
                assert_eq!(style.spacing.interact_size.y, 38.0);
                assert_eq!(
                    style.visuals.panel_fill,
                    egui::Color32::from_rgb(250, 250, 249)
                );
            },
        );
    }
}
