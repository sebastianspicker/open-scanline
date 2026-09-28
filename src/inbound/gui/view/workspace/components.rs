//! Workspace building blocks. Each one draws from the Platen tokens only.

use super::theme::{self, color, size, space};
use egui::{RichText, Stroke};

/// Status readout above the title, like the display on the scanner itself.
pub(super) fn stage(ui: &mut egui::Ui, text: &str, live: bool) {
    ui.horizontal(|ui| {
        if live {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().line_segment(
                [rect.left_center(), rect.right_center()],
                Stroke::new(theme::FOCUS, color::LAMP),
            );
        }
        ui.label(
            RichText::new(text.to_uppercase())
                .font(theme::mono(size::MONO_SMALL))
                .extra_letter_spacing(1.2)
                .color(if live {
                    color::LAMP_INK
                } else {
                    color::GRAPHITE
                }),
        );
    });
    ui.add_space(space::XS);
}

pub(super) fn title(ui: &mut egui::Ui, text: &str, empty: &str) {
    let display = if ui.available_width() < 600.0 {
        size::DISPLAY_NARROW
    } else {
        size::DISPLAY
    };
    let (text, tone) = if text.trim().is_empty() {
        (empty, color::GRAPHITE)
    } else {
        (text, color::INK)
    };
    ui.add(egui::Label::new(RichText::new(text).font(theme::bold(display)).color(tone)).wrap());
}

pub(super) fn lead(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add_space(space::XS);
    ui.add(
        egui::Label::new(
            RichText::new(text.into())
                .font(theme::body(size::LEAD))
                .color(color::GRAPHITE),
        )
        .wrap(),
    );
}

/// Numbered section heading under an ink rule.
pub(super) fn section(ui: &mut egui::Ui, number: &str, text: &str) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(theme::HAIRLINE, color::INK),
    );
    ui.add_space(space::XS);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(number)
                .font(theme::mono(size::MONO_SMALL))
                .color(color::GRAPHITE),
        );
        ui.label(RichText::new(text).font(theme::bold(size::HEADING)));
    });
    ui.add_space(space::M);
}

pub(super) fn label_text(text: &str) -> RichText {
    RichText::new(text).font(theme::bold(size::LABEL))
}

pub(super) fn note(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(
        egui::Label::new(
            RichText::new(text.into())
                .font(theme::body(size::SMALL))
                .color(color::GRAPHITE),
        )
        .wrap(),
    );
}

/// An inline problem that blocks a scan: lamp bar and lamp text.
pub(super) fn problem(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = space::S;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 18.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, color::LAMP);
        ui.add(
            egui::Label::new(
                RichText::new(text.into())
                    .font(theme::body(size::SMALL))
                    .color(color::LAMP_INK),
            )
            .wrap(),
        );
    });
}

/// A job-level problem: lamp wash, bold headline, and the raw message.
pub(super) fn banner(ui: &mut egui::Ui, headline: &str, detail: &str) {
    egui::Frame::new()
        .fill(color::LAMP_WASH)
        .inner_margin(egui::Margin::symmetric(space::L as i8, space::M as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let top = ui.min_rect().left_top();
            ui.label(
                RichText::new(headline)
                    .font(theme::bold(size::BODY))
                    .color(color::LAMP_INK),
            );
            ui.add(egui::Label::new(RichText::new(detail).color(color::INK)).wrap());
            let bar = egui::Rect::from_min_max(
                top - egui::vec2(space::L, space::M),
                egui::pos2(top.x - space::L + 3.0, ui.min_rect().bottom() + space::M),
            );
            ui.painter().rect_filled(bar, 0.0, color::LAMP);
        });
}

pub(super) fn primary(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        RichText::new(label)
            .font(theme::bold(size::BODY))
            .color(color::PAPER),
    )
    .fill(color::INK)
    .stroke(Stroke::NONE)
    .min_size(egui::vec2(188.0, size::ACTION))
}

pub(super) fn secondary(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label)).min_size(egui::vec2(0.0, size::CONTROL))
}

/// Unframed text action for navigation-like commands.
pub(super) fn quiet(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label)).frame(false)
}

pub(super) fn field(ui: &mut egui::Ui, label: &str, value: &mut String) -> egui::Response {
    let label = ui.label(label_text(label));
    ui.add(
        egui::TextEdit::singleline(value)
            .desired_width(ui.available_width())
            .min_size(egui::vec2(0.0, size::CONTROL))
            .margin(egui::Margin::symmetric(10, 10)),
    )
    .labelled_by(label.id)
}

pub(super) fn choice(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    value: &mut String,
    choices: &[(&str, &str)],
) {
    let label_response = ui.label(label_text(label));
    let selected = choices
        .iter()
        .find(|(key, _)| *key == value)
        .map_or(value.as_str(), |(_, label)| *label)
        .to_owned();
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .height(size::CONTROL * 8.0)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (key, label) in choices {
                ui.selectable_value(value, (*key).into(), *label);
            }
        })
        .response
        .labelled_by(label_response.id);
}

/// Two mutually exclusive options drawn as one control; the chosen half is inked.
pub(super) fn segmented(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut bool,
    options: [(&str, bool); 2],
) {
    let label_response = ui.label(label_text(label));
    let width = ui.available_width();
    let stacked = stacks(ui, &options, width);
    let (cell, rows) = if stacked {
        (egui::vec2(width, size::CONTROL), 2.0)
    } else {
        (egui::vec2(width / 2.0, size::CONTROL), 1.0)
    };
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, size::CONTROL * rows),
        egui::Sense::hover(),
    );
    let enabled = ui.is_enabled();
    for (index, (text, option)) in options.into_iter().enumerate() {
        let offset = if stacked {
            egui::vec2(0.0, index as f32 * cell.y)
        } else {
            egui::vec2(index as f32 * cell.x, 0.0)
        };
        let half = egui::Rect::from_min_size(rect.min + offset, cell);
        let id = ui.id().with((label, index));
        let response = ui
            .interact(half, id, egui::Sense::click())
            .labelled_by(label_response.id);
        let selected = *value == option;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, enabled, selected, text)
        });
        if response.clicked() {
            *value = option;
        }
        paint_segment(ui, &response, half, text, selected);
    }
    ui.painter().rect_stroke(
        rect,
        theme::RADIUS,
        Stroke::new(theme::HAIRLINE, color::EDGE),
        egui::StrokeKind::Inside,
    );
}

/// Narrow columns stack the options instead of clipping their labels.
fn stacks(ui: &egui::Ui, options: &[(&str, bool); 2], width: f32) -> bool {
    options.iter().any(|(text, _)| {
        let text_width = ui.fonts(|fonts| {
            fonts
                .layout_no_wrap((*text).into(), theme::bold(size::BODY), color::INK)
                .size()
                .x
        });
        text_width + 2.0 * space::S > width / 2.0
    })
}

fn paint_segment(
    ui: &egui::Ui,
    response: &egui::Response,
    rect: egui::Rect,
    text: &str,
    selected: bool,
) {
    let painter = ui.painter();
    let (fill, ink) = match (selected, response.hovered()) {
        (true, _) => (color::INK, color::PAPER),
        (false, true) => (color::HOVER, color::INK),
        (false, false) => (color::FIELD, color::INK),
    };
    let ink = if ui.is_enabled() {
        ink
    } else {
        ink.gamma_multiply(0.5)
    };
    painter.rect_filled(rect, theme::RADIUS, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        if selected {
            theme::bold(size::BODY)
        } else {
            theme::body(size::BODY)
        },
        ink,
    );
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.0),
            theme::RADIUS,
            Stroke::new(theme::FOCUS, color::LAMP),
            egui::StrokeKind::Inside,
        );
    }
}

/// Drawn check mark; avoids relying on a font glyph.
pub(super) fn check(painter: &egui::Painter, center: egui::Pos2, scale: f32, tone: egui::Color32) {
    let stroke = Stroke::new(1.6_f32, tone);
    let a = center + egui::vec2(-0.5, 0.0) * scale;
    let b = center + egui::vec2(-0.15, 0.35) * scale;
    let c = center + egui::vec2(0.5, -0.4) * scale;
    painter.line_segment([a, b], stroke);
    painter.line_segment([b, c], stroke);
}

/// Replace the home directory with `~` for display; the full path stays in the tooltip.
pub(super) fn display_path(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}
