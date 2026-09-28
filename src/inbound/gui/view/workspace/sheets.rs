//! The sheet rail: one slot per requested side, filled as pages are saved.
//! The scan line sweeping the current slot is the only continuous motion in
//! the product.

use super::*;
use crate::inbound::gui::app::report::JobReport;
use egui::{pos2, vec2, Rect, Stroke};

const SLOT: egui::Vec2 = vec2(34.0, 48.0);
const GAP: f32 = 8.0;
const PAIR_GAP: f32 = 20.0;
/// Above this many sides the rail becomes a proportional bar.
const MAX_SLOTS: u32 = 24;

#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Saved,
    Current,
    Waiting,
    Unused,
}

pub(super) fn rail(ui: &mut egui::Ui, report: &JobReport) {
    let label = format!(
        "{} of up to {} sides saved",
        report.sides_complete, report.side_limit
    );
    let response = if report.side_limit > MAX_SLOTS {
        bar(ui, report)
    } else {
        slots(ui, report)
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, true, &label)
    });
    ui.add_space(space::S);
    note(ui, caption(report));
}

fn caption(report: &JobReport) -> String {
    if report.duplex && report.side_limit.is_multiple_of(2) {
        format!("Front and back of up to {} sheets", report.side_limit / 2)
    } else if report.side_limit == 1 {
        "One image".into()
    } else {
        format!("Up to {} sides", report.side_limit)
    }
}

fn slot_state(report: &JobReport, index: u32) -> Slot {
    let running = report.terminal.is_none();
    if index < report.sides_complete {
        Slot::Saved
    } else if running && index == report.sides_complete {
        Slot::Current
    } else if running {
        Slot::Waiting
    } else {
        Slot::Unused
    }
}

fn slots(ui: &mut egui::Ui, report: &JobReport) -> egui::Response {
    let width = ui.available_width();
    let positions = layout(report, width);
    let height = positions.last().map_or(SLOT.y, |rect| rect.bottom());
    let (area, response) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(area.expand(2.0));
    let phase = sweep_phase(ui);
    for (index, rect) in positions.into_iter().enumerate() {
        let rect = rect.translate(area.min.to_vec2());
        let index = index as u32;
        paint_slot(&painter, rect, index + 1, slot_state(report, index), phase);
    }
    if report.terminal.is_none() {
        ui.ctx().request_repaint();
    }
    response
}

/// Slot rectangles relative to the rail origin. Duplex pairs sit closer
/// together so the rail reads as sheets, not a row of identical boxes.
fn layout(report: &JobReport, width: f32) -> Vec<Rect> {
    let paired = report.duplex && report.side_limit.is_multiple_of(2);
    let mut rects = Vec::with_capacity(report.side_limit as usize);
    let mut cursor = pos2(0.0, 0.0);
    for index in 0..report.side_limit {
        if cursor.x + SLOT.x > width && cursor.x > 0.0 {
            cursor = pos2(0.0, cursor.y + SLOT.y + space::M + 14.0);
        }
        rects.push(Rect::from_min_size(cursor, SLOT));
        let gap = if paired && index % 2 == 1 {
            PAIR_GAP
        } else {
            GAP
        };
        cursor.x += SLOT.x + gap;
    }
    rects
}

fn sweep_phase(ui: &egui::Ui) -> f32 {
    let time = ui.input(|input| input.time);
    ((time % theme::motion::SWEEP_SECONDS) / theme::motion::SWEEP_SECONDS) as f32
}

fn paint_slot(painter: &egui::Painter, rect: Rect, number: u32, state: Slot, phase: f32) {
    let (fill, edge, ink) = match state {
        Slot::Saved => (color::FIELD, color::INK, color::INK),
        Slot::Current => (color::FIELD, color::INK, color::INK),
        Slot::Waiting => (color::PAPER, color::EDGE, color::GRAPHITE),
        Slot::Unused => (color::PAPER, color::RULE, color::GRAPHITE),
    };
    painter.rect_filled(rect, 0.0, fill);
    if state == Slot::Current {
        scan_line(painter, rect, phase);
    }
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(theme::HAIRLINE, edge),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center_top() + vec2(0.0, 8.0),
        egui::Align2::CENTER_TOP,
        format!("{number}"),
        theme::mono(size::MONO_SMALL),
        ink,
    );
    if state == Slot::Saved {
        check(
            painter,
            rect.center_bottom() - vec2(0.0, 13.0),
            10.0,
            color::INK,
        );
    }
}

/// The captured part of the sheet is tinted and the lamp line marks the edge.
fn scan_line(painter: &egui::Painter, rect: Rect, phase: f32) {
    let y = rect.top() + rect.height() * phase;
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.right(), y)),
        0.0,
        color::PRESSED,
    );
    painter.hline(
        rect.left() - 3.0..=rect.right() + 3.0,
        y,
        Stroke::new(theme::FOCUS, color::LAMP),
    );
}

fn bar(ui: &mut egui::Ui, report: &JobReport) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 14.0), egui::Sense::hover());
    let painter = ui.painter();
    let fraction = report.sides_complete as f32 / report.side_limit.max(1) as f32;
    painter.rect_filled(rect, 0.0, color::FIELD);
    let done = Rect::from_min_size(
        rect.min,
        vec2(rect.width() * fraction.min(1.0), rect.height()),
    );
    painter.rect_filled(done, 0.0, color::INK);
    if report.terminal.is_none() {
        painter.vline(
            done.right(),
            rect.y_range().expand(3.0),
            Stroke::new(theme::FOCUS, color::LAMP),
        );
    }
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(theme::HAIRLINE, color::EDGE),
        egui::StrokeKind::Inside,
    );
    response
}
