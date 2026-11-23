//! D1 Clear Bench History pane: chronological feature list with working rollback
//! playback controls (`|<`, `<`, `▶`/`⏸`, `>`, `>|`) and scrubber slider wired to
//! `Command::Rollback`. For assemblies and drawings, shows a clear coming-soon state.

use crate::document::{Command, FeatureKind, FeatureStatus, Selection};
use crate::icons::{self, Icon};
use crate::state::{AppAction, AppState};
use crate::theme;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::time::Duration;

/// Playback step interval while Play is active.
const PLAY_STEP_SECS: f64 = 0.65;

/// Persistent playback state stored in egui temp memory so no extra `AppState` churn is needed.
#[derive(Clone, Copy, Default)]
struct Playback {
    playing: bool,
    last_step_time: f64,
}

fn feature_icon(kind: &FeatureKind) -> Icon {
    match kind {
        FeatureKind::Sketch(_) => Icon::Sketch,
        FeatureKind::Extrude(_) => Icon::Extrude,
        FeatureKind::Revolve(_) => Icon::Revolve,
        FeatureKind::Sweep(_) => Icon::Sweep,
        FeatureKind::Loft(_) => Icon::Loft,
        FeatureKind::Cut(_) => Icon::Cut,
        FeatureKind::Fillet(_) => Icon::Fillet,
        FeatureKind::Chamfer(_) => Icon::Chamfer,
        FeatureKind::Hole(_) => Icon::Hole,
        FeatureKind::Pattern(_) => Icon::LinearPattern,
        FeatureKind::Mirror(_) => Icon::Mirror,
        FeatureKind::ReferencePlane(_) => Icon::Plane,
        FeatureKind::Draft(_) => Icon::Draft,
        FeatureKind::Shell(_) => Icon::Shell,
        FeatureKind::Offset(_) | FeatureKind::Thicken(_) => Icon::Offset,
        FeatureKind::Lattice(_) | FeatureKind::Gyroid(_) => Icon::Extrude,
        FeatureKind::ImportStep(_) => Icon::Import,
        FeatureKind::Combine { .. } => Icon::Assembly,
        FeatureKind::Scan(_) => Icon::Import,
    }
}

/// Compact badge text shown in the History pane header (e.g. `4/4`).
pub fn badge(state: &AppState) -> String {
    if state.assembly.is_active() || state.drawing.is_active() {
        return "Soon".to_owned();
    }
    let total = state.doc.features.len();
    let at = state.doc.rollback.unwrap_or(total).min(total);
    format!("{at}/{total}")
}

/// Renders the History pane body.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if state.assembly.is_active() || state.drawing.is_active() {
        ui.add_space(10.0);
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.label(
                egui::RichText::new("History rollback — Coming soon")
                    .family(theme::semibold())
                    .size(12.0)
                    .color(theme::ink()),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Step-by-step history playback is currently available for Part documents.")
                    .size(11.5)
                    .color(theme::muted()),
            );
        });
        return;
    }

    let total = state.doc.features.len();
    let current = state.doc.rollback.unwrap_or(total).min(total);
    let can_edit = !state.in_sketch() && state.editor.is_none();

    let pb_id = egui::Id::new("d1_history_playback");
    let mut pb = ui.data(|d| d.get_temp::<Playback>(pb_id).unwrap_or_default());
    let now = ui.input(|i| i.time);

    if pb.playing {
        if !can_edit || total == 0 || current >= total {
            pb.playing = false;
        } else if now - pb.last_step_time >= PLAY_STEP_SECS {
            pb.last_step_time = now;
            let next = (current + 1).min(total);
            let target = if next >= total { None } else { Some(next) };
            state.push(AppAction::Command(Command::Rollback(target)));
            if next >= total {
                pb.playing = false;
            }
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(120));
        }
    }

    let mut rollback_to: Option<usize> = None;

    // 1. Playback toolbar (`|<  <  ▶/⏸  >  >|`)
    egui::Frame::new().fill(theme::panel_header()).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let avail = ui.available_width();
            let group_w = 5.0 * 28.0 + 4.0 * 4.0;
            ui.add_space(((avail - group_w) * 0.5).max(0.0));

            if step_btn(ui, StepIcon::First, "Roll to start", can_edit && current > 0, false).clicked() {
                pb.playing = false;
                rollback_to = Some(0);
            }
            if step_btn(ui, StepIcon::Prev, "Step back", can_edit && current > 0, false).clicked() {
                pb.playing = false;
                rollback_to = Some(current.saturating_sub(1));
            }
            let play_icon = if pb.playing { StepIcon::Pause } else { StepIcon::Play };
            let play_tip = if pb.playing { "Pause history playback" } else { "Play history forward" };
            let play_enabled = can_edit && total > 0;
            if step_btn(ui, play_icon, play_tip, play_enabled, pb.playing).clicked() {
                if pb.playing {
                    pb.playing = false;
                } else {
                    pb.playing = true;
                    pb.last_step_time = now;
                    if current >= total {
                        rollback_to = Some(0);
                    }
                }
            }
            if step_btn(ui, StepIcon::Next, "Step forward", can_edit && current < total, false).clicked() {
                pb.playing = false;
                rollback_to = Some((current + 1).min(total));
            }
            if step_btn(ui, StepIcon::Last, "Roll to end", can_edit && current < total, false).clicked() {
                pb.playing = false;
                rollback_to = Some(total);
            }
        });
    });
    theme::hline(ui);

    // 2. Scrubber footer height reserved so the list scrolls cleanly above it.
    let footer_h = 36.0;
    let list_h = (ui.available_height() - footer_h).max(40.0);

    let selected_feature = state.selected_feature();
    let mut select_feature = None;

    egui::ScrollArea::vertical().id_salt("d1_history_list").max_height(list_h).auto_shrink([false, false]).show(
        ui,
        |ui| {
            ui.add_space(3.0);
            if total == 0 {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("No history steps yet").size(11.5).color(theme::muted()));
                });
                return;
            }
            for (idx, feat) in state.doc.features.iter().enumerate() {
                let past = idx >= current;
                let active_step = idx + 1 == current;
                let selected = selected_feature == Some(feat.id);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Button,
                        true,
                        selected || active_step,
                        format!("History step {}: {}", idx + 1, feat.name),
                    )
                });
                let fill = if active_step {
                    theme::accent_soft()
                } else if selected || resp.hovered() {
                    theme::hover()
                } else {
                    Color32::TRANSPARENT
                };
                if fill != Color32::TRANSPARENT {
                    ui.painter().rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(4), fill);
                }
                let alpha = if past || feat.suppressed { 0.45 } else { 1.0 };
                let ink = if active_step { theme::accent() } else { theme::ink().gamma_multiply(alpha) };
                let muted = theme::muted().gamma_multiply(alpha);
                icons::paint(
                    ui.painter(),
                    feature_icon(&feat.kind),
                    Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(14.0, 14.0)),
                    ink,
                );
                let summary = match &feat.status {
                    FeatureStatus::Error(_) => "Error".to_owned(),
                    FeatureStatus::Inactive => "Rolled back".to_owned(),
                    _ => feat.summary(),
                };
                ui.painter().text(
                    pos2(rect.left() + 30.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    &feat.name,
                    if active_step { theme::font_semibold(12.0) } else { theme::font(12.0) },
                    ink,
                );
                ui.painter().text(
                    pos2(rect.right() - 10.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    summary,
                    theme::font(11.0),
                    muted,
                );
                let resp = resp.on_hover_text("Click to select · Double-click to roll back to this step");
                if resp.double_clicked() && can_edit {
                    pb.playing = false;
                    rollback_to = Some(idx + 1);
                } else if resp.clicked() {
                    select_feature = Some(feat.id);
                }
            }
            ui.add_space(4.0);
        },
    );

    // 3. Rollback scrubber slider at bottom of History pane
    ui.add_space(2.0);
    theme::hline(ui);
    egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 8 }).show(ui, |ui| {
        let mut step = current;
        ui.spacing_mut().slider_width = (ui.available_width() - 4.0).max(40.0);
        let slider =
            ui.add_enabled(can_edit && total > 0, egui::Slider::new(&mut step, 0..=total.max(1)).show_value(false));
        slider.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, can_edit && total > 0, "History rollback scrubber")
        });
        if slider.changed() && step != current {
            pb.playing = false;
            rollback_to = Some(step);
        }
    });

    ui.data_mut(|d| d.insert_temp(pb_id, pb));

    if let Some(id) = select_feature {
        state.push(AppAction::Select(Some(Selection::Feature(id))));
    }
    if let Some(target) = rollback_to {
        let rb = if target >= total { None } else { Some(target) };
        state.push(AppAction::Command(Command::Rollback(rb)));
    }
}

#[derive(Clone, Copy)]
enum StepIcon {
    First,
    Prev,
    Play,
    Pause,
    Next,
    Last,
}

fn step_btn(ui: &mut egui::Ui, icon: StepIcon, tip: &str, enabled: bool, active: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 22.0), sense);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, tip));
    let (fill, ink) = if active {
        (theme::accent(), theme::on_accent())
    } else if enabled && resp.hovered() {
        (theme::hover(), theme::ink())
    } else {
        (theme::panel(), if enabled { theme::ink() } else { theme::muted().gamma_multiply(0.45) })
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        fill,
        Stroke::new(1.0, if active { theme::accent() } else { theme::line() }),
        StrokeKind::Inside,
    );
    let c = rect.center();
    let p = ui.painter();
    let s = Stroke::new(1.5, ink);
    match icon {
        StepIcon::First => {
            p.line_segment([pos2(c.x - 4.0, c.y - 4.0), pos2(c.x - 4.0, c.y + 4.0)], s);
            p.add(egui::Shape::convex_polygon(
                vec![pos2(c.x - 2.0, c.y), pos2(c.x + 4.0, c.y - 4.0), pos2(c.x + 4.0, c.y + 4.0)],
                ink,
                Stroke::NONE,
            ));
        }
        StepIcon::Prev => {
            p.line_segment([pos2(c.x + 2.0, c.y - 4.0), pos2(c.x - 2.5, c.y)], s);
            p.line_segment([pos2(c.x - 2.5, c.y), pos2(c.x + 2.0, c.y + 4.0)], s);
        }
        StepIcon::Play => {
            p.add(egui::Shape::convex_polygon(
                vec![pos2(c.x - 3.0, c.y - 4.5), pos2(c.x + 4.5, c.y), pos2(c.x - 3.0, c.y + 4.5)],
                ink,
                Stroke::NONE,
            ));
        }
        StepIcon::Pause => {
            p.rect_filled(Rect::from_center_size(pos2(c.x - 2.5, c.y), vec2(2.2, 8.0)), CornerRadius::same(1), ink);
            p.rect_filled(Rect::from_center_size(pos2(c.x + 2.5, c.y), vec2(2.2, 8.0)), CornerRadius::same(1), ink);
        }
        StepIcon::Next => {
            p.line_segment([pos2(c.x - 2.0, c.y - 4.0), pos2(c.x + 2.5, c.y)], s);
            p.line_segment([pos2(c.x + 2.5, c.y), pos2(c.x - 2.0, c.y + 4.0)], s);
        }
        StepIcon::Last => {
            p.add(egui::Shape::convex_polygon(
                vec![pos2(c.x - 4.0, c.y - 4.0), pos2(c.x + 2.0, c.y), pos2(c.x - 4.0, c.y + 4.0)],
                ink,
                Stroke::NONE,
            ));
            p.line_segment([pos2(c.x + 4.0, c.y - 4.0), pos2(c.x + 4.0, c.y + 4.0)], s);
        }
    }
    resp.on_hover_text(tip)
}
