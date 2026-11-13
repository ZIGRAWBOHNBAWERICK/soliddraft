//! Edit Suite history. Chips share the tree's document actions, with a horizontal rollback bar.

use crate::document::{Command, Feature, FeatureId, FeatureStatus, Selection};
use crate::icons::{self, Icon};
use crate::panels::Side;
use crate::state::{AppAction, AppState, Rename};
use crate::{theme, tree};
use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

pub const HEIGHT: f32 = 76.0;
const CHIP_HEIGHT: f32 = 40.0;

/// Separate gestures from the tree, which may be open in the same frame.
#[derive(Default)]
pub struct State {
    pub rename: Option<Rename>,
    dragging: Option<FeatureId>,
    rollback_drag: bool,
    followed: Option<FeatureId>,
}

pub fn panel(root: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::bottom("feature_timeline")
        .exact_size(HEIGHT)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::chrome()).inner_margin(egui::Margin::symmetric(10, 4)))
        .show(root, |ui| show(ui, state));
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let mut actions = Vec::new();
    let editing = state.sketch.as_ref().map(|s| s.feature);
    let edit_index = editing.and_then(|id| state.doc.features.iter().position(|f| f.id == id));
    let can_change = editing.is_none() && state.editor.is_none();
    if !can_change || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.timeline.dragging = None;
        state.timeline.rollback_drag = false;
    }
    let count = state.doc.features.len();
    let bar_at = state.doc.rollback.unwrap_or(count);
    let selected = state.selected_feature();
    if selected.is_none() {
        state.timeline.followed = None;
    }
    let released = ui.input(|i| i.pointer.primary_released());
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut centres = Vec::new();
    let mut gaps = Vec::new();
    let mut bounds = Rect::NOTHING;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width(62.0);
            ui.label(egui::RichText::new("History").font(theme::font_medium(11.0)).color(theme::muted()));
            let shown = state.tree_shown();
            let (rect, response) = ui.allocate_exact_size(vec2(54.0, 20.0), Sense::click());
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, shown, "Tree"));
            if response.hovered() || shown {
                ui.painter().rect_filled(rect, 4, if shown { theme::selection_bg() } else { theme::hover() });
            }
            let ink = if shown { theme::accent() } else { theme::ink() };
            icons::paint(
                ui.painter(),
                Icon::Tree,
                Rect::from_center_size(rect.left_center() + vec2(9.0, 0.0), vec2(13.0, 13.0)),
                ink,
            );
            ui.painter().text(
                rect.left_center() + vec2(21.0, 0.0),
                Align2::LEFT_CENTER,
                "Tree",
                theme::font(11.0),
                ink,
            );
            focus_ring(ui, rect, response.has_focus());
            if response.on_hover_text("Show or hide the Tree and Values pane").clicked() {
                actions.push(AppAction::TogglePanel(Side::Tree));
            }
        });
        egui::ScrollArea::horizontal().id_salt("timeline_scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.horizontal(|ui| {
                for index in 0..=count {
                    gaps.push(ui.cursor().left());
                    if index == bar_at {
                        playhead(ui, state, can_change, &mut actions);
                    }
                    let Some(feature) = state.doc.features.get(index) else { break };
                    let status = state
                        .sketch
                        .as_ref()
                        .filter(|s| s.feature == feature.id)
                        .map(|s| &s.sketch.status)
                        .or_else(|| feature.sketch().map(|s| &s.sketch.status));
                    let caption = if matches!(status, Some(crate::sketch::SolveStatus::UnderDefined)) {
                        format!("(-) {}", feature.name)
                    } else {
                        feature.name.clone()
                    };
                    let (id, name, summary, icon, is_sketch, suppressed, repair) = (
                        feature.id,
                        feature.name.clone(),
                        if state.doc.model().next_body() > 1 {
                            format!("Body {} · {}", feature.body_key + 1, feature.summary())
                        } else {
                            feature.summary()
                        },
                        tree::feature_icon(&feature.kind),
                        feature.is_sketch(),
                        feature.suppressed,
                        matches!(feature.status, FeatureStatus::NeedsRepair(_)),
                    );
                    let dim = suppressed || index >= bar_at || edit_index.is_some_and(|at| index > at);
                    let active = editing == Some(id) || selected == Some(id);
                    let width = ui
                        .painter()
                        .layout_no_wrap(caption.clone(), theme::font_medium(12.0), theme::ink())
                        .size()
                        .x
                        .max(ui.painter().layout_no_wrap(summary.clone(), theme::font(10.5), theme::muted()).size().x)
                        .clamp(72.0, 166.0)
                        + 48.0;
                    let (rect, response) = ui.allocate_exact_size(vec2(width, CHIP_HEIGHT), Sense::click_and_drag());
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, format!("{name} in history"))
                    });
                    centres.push(rect.center().x);
                    bounds = bounds.union(rect);
                    let fill = if active {
                        theme::selection_bg()
                    } else if response.hovered() {
                        theme::hover()
                    } else {
                        theme::chip()
                    };
                    ui.painter().rect(
                        rect,
                        6,
                        fill,
                        Stroke::new(1.0, if active { theme::accent() } else { theme::line_strong() }),
                        StrokeKind::Inside,
                    );
                    focus_ring(ui, rect, response.has_focus());
                    let ink = if dim { theme::muted() } else { theme::ink() };
                    icons::paint(
                        ui.painter(),
                        icon,
                        Rect::from_center_size(pos2(rect.left() + 17.0, rect.center().y), vec2(18.0, 18.0)),
                        ink,
                    );
                    proof_badge(ui, state, feature, rect);
                    let text = Rect::from_min_max(rect.min + vec2(32.0, 0.0), rect.max - vec2(12.0, 0.0));
                    let painter = ui.painter().with_clip_rect(text.intersect(ui.clip_rect()));
                    painter.text(
                        pos2(text.left(), rect.top() + 12.0),
                        Align2::LEFT_CENTER,
                        &caption,
                        theme::font_medium(12.0),
                        ink,
                    );
                    painter.text(
                        pos2(text.left(), rect.top() + 28.0),
                        Align2::LEFT_CENTER,
                        &summary,
                        theme::font(10.5),
                        theme::muted(),
                    );
                    if state.timeline.rename.as_ref().is_some_and(|r| r.feature == id) {
                        rename(ui, state, text, &mut actions);
                    } else {
                        if response.clicked() {
                            actions.push(AppAction::Select(Some(Selection::Feature(id))));
                        }
                        if response.double_clicked() && can_change {
                            actions.push(if is_sketch {
                                AppAction::EditSketch(id)
                            } else {
                                AppAction::EditFeature(id)
                            });
                        }
                        if response.drag_started() && can_change {
                            state.timeline.dragging = Some(id);
                            state.timeline.rollback_drag = false;
                        }
                    }
                    if editing.or(selected) == Some(id) && state.timeline.followed != Some(id) {
                        response.scroll_to_me(None);
                        state.timeline.followed = Some(id);
                    }
                    let response = response.on_hover_text(format!("{name}\n{summary}"));
                    response.context_menu(|ui| {
                        ui.add_enabled_ui(can_change, |ui| {
                            if ui.button("Edit").clicked() {
                                actions.push(if is_sketch {
                                    AppAction::EditSketch(id)
                                } else {
                                    AppAction::EditFeature(id)
                                });
                                ui.close();
                            }
                            if ui.add_enabled(repair, egui::Button::new("Repair")).clicked() {
                                actions.push(AppAction::StartRepair(id));
                                ui.close();
                            }
                            if ui.button("Rename").clicked() {
                                state.timeline.rename = Some(Rename { feature: id, text: name.clone(), focus: true });
                                ui.close();
                            }
                            if ui.button(if suppressed { "Unsuppress" } else { "Suppress" }).clicked() {
                                actions.push(AppAction::Command(Command::Suppress { id, suppressed: !suppressed }));
                                ui.close();
                            }
                            if ui.button("Roll back to here").clicked() {
                                actions.push(AppAction::Command(Command::Rollback(
                                    (index + 1 < count).then_some(index + 1),
                                )));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button(egui::RichText::new("Delete").color(theme::error())).clicked() {
                                actions.push(AppAction::Command(Command::DeleteFeature(id)));
                                ui.close();
                            }
                        });
                    });
                    if index + 1 < count {
                        let (link, _) = ui.allocate_exact_size(vec2(16.0, CHIP_HEIGHT), Sense::hover());
                        ui.painter().hline(link.x_range(), link.center().y, Stroke::new(1.0, theme::line_strong()));
                    }
                }
                if count == 0 {
                    ui.label(egui::RichText::new("No features yet").color(theme::muted()));
                }
            });
            if let Some(pointer) = pointer
                && (state.timeline.dragging.is_some() || state.timeline.rollback_drag)
            {
                let target = centres.iter().filter(|x| **x < pointer.x).count();
                if let Some(x) = gaps.get(target) {
                    ui.painter().vline(*x, bounds.y_range(), Stroke::new(2.0, theme::accent()));
                }
                if released && ui.clip_rect().contains(pointer) {
                    if state.timeline.rollback_drag {
                        actions.push(AppAction::Command(Command::Rollback((target < count).then_some(target))));
                    } else if let Some(id) = state.timeline.dragging
                        && let Some(from) = state.doc.features.iter().position(|f| f.id == id)
                    {
                        let index = target - usize::from(from < target);
                        if index != from {
                            actions.push(AppAction::Command(Command::Reorder { id, index }));
                        }
                    }
                }
            }
        });
    });
    if released {
        state.timeline.dragging = None;
        state.timeline.rollback_drag = false;
    }
    for action in actions {
        state.push(action);
    }
}

fn proof_badge(ui: &mut egui::Ui, state: &AppState, feature: &Feature, chip: Rect) {
    let id = feature.id;
    let (label, reason) = if let Some(drag) = state.drag_preview.as_ref().filter(|drag| drag.feature() == id) {
        let (reason, failed) = drag.label();
        let label = if failed {
            "Refused"
        } else if drag.proved_request().is_some() && !drag.pending() {
            "Proved"
        } else {
            "Pending"
        };
        (label, reason)
    } else {
        match &feature.status {
            FeatureStatus::Ok => ("Proved", "The current feature is proved.".to_owned()),
            FeatureStatus::AnsweredWithinBound(bound) => (crate::document::WITHIN_BOUND_LABEL, bound.description()),
            FeatureStatus::Pending => {
                ("Pending", "The current edit is waiting for proof. Export waits for proof.".to_owned())
            }
            FeatureStatus::Error(reason) => ("Refused", crate::geometry_message::for_user(reason)),
            FeatureStatus::NeedsRepair(_) => {
                ("Refused", "A reference is missing. Choose Repair to replace it.".to_owned())
            }
            FeatureStatus::Inactive => ("Inactive", "This feature is suppressed or past the rollback bar.".to_owned()),
        }
    };
    let color = match label {
        "Proved" => theme::ok(),
        "Pending" => theme::warn(),
        "Refused" => theme::error(),
        crate::document::WITHIN_BOUND_LABEL => theme::bounded(),
        _ => theme::muted(),
    };
    let centre = pos2(chip.right() - 7.0, chip.center().y);
    if label == crate::document::WITHIN_BOUND_LABEL {
        ui.painter().add(egui::Shape::convex_polygon(
            vec![centre + vec2(0.0, -4.0), centre + vec2(4.0, 0.0), centre + vec2(0.0, 4.0), centre + vec2(-4.0, 0.0)],
            color,
            Stroke::NONE,
        ));
    } else if label == "Refused" {
        for sign in [-1.0, 1.0] {
            ui.painter().line_segment(
                [centre + vec2(-3.0, -3.0 * sign), centre + vec2(3.0, 3.0 * sign)],
                Stroke::new(1.5, color),
            );
        }
    } else if label == "Proved" {
        ui.painter().circle_filled(centre, 3.0, color);
    } else {
        ui.painter().circle_stroke(centre, 3.0, Stroke::new(1.5, color));
    }
    let response =
        ui.interact(Rect::from_center_size(centre, vec2(12.0, 18.0)), ui.id().with(("proof", id)), Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{} proof: {label}", feature.name))
    });
    response.on_hover_text(format!("{label}: {reason}"));
}

fn focus_ring(ui: &egui::Ui, rect: Rect, focused: bool) {
    if focused {
        ui.painter().rect_stroke(rect.shrink(2.0), 4, Stroke::new(2.0, theme::ink()), StrokeKind::Inside);
    }
}

fn playhead(ui: &mut egui::Ui, state: &mut AppState, enabled: bool, actions: &mut Vec<AppAction>) {
    let (rect, response) = ui.allocate_exact_size(vec2(18.0, 46.0), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, enabled, "History rollback bar"));
    focus_ring(ui, rect, response.has_focus());
    let x = rect.center().x;
    let color = if enabled { theme::accent() } else { theme::muted() };
    ui.painter().vline(x, rect.y_range().shrink(4.0), Stroke::new(3.0, color));
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(x - 5.0, rect.top()), pos2(x + 5.0, rect.top()), pos2(x, rect.top() + 6.0)],
        color,
        Stroke::NONE,
    ));
    if enabled && response.drag_started() {
        state.timeline.rollback_drag = true;
        state.timeline.dragging = None;
    }
    if enabled && response.double_clicked() {
        actions.push(AppAction::Command(Command::Rollback(None)));
    }
    response
        .on_hover_text("Drag to roll back history. Double-click to restore all features.")
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
}

fn rename(ui: &mut egui::Ui, state: &mut AppState, rect: Rect, actions: &mut Vec<AppAction>) {
    let Some(rename) = state.timeline.rename.as_mut() else { return };
    let response =
        ui.put(rect.shrink2(vec2(0.0, 8.0)), egui::TextEdit::singleline(&mut rename.text).font(theme::font(12.0)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "History feature name"));
    if rename.focus {
        response.request_focus();
        rename.focus = false;
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.timeline.rename = None;
    } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) || response.lost_focus() {
        let rename = state.timeline.rename.take().unwrap();
        if !rename.text.trim().is_empty() && state.doc.feature(rename.feature).is_some_and(|f| f.name != rename.text) {
            actions.push(AppAction::Command(Command::Rename { id: rename.feature, name: rename.text }));
        }
    }
}
