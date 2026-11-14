//! Sketch and selection details, plus controls shared with the options bar.

use crate::document::{
    Command, FeatureId, FeatureKind, FeatureStatus, Reference, Selection, SketchPlane, fmt_mm, plane_ref_label,
};
use crate::drag::Dimension;
use crate::form_bar::value_cell;
use crate::icons::{self, Icon};
use crate::sketch::{ConstraintKind, EntityKind, fmt_value, solve};
use crate::sketcher;
use crate::state::{AppAction, AppState, DragStep, RelationFilter, SketchSel, Tool};
use crate::theme;
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, pos2, vec2};
use soliddraft_kernel_api::FaceId;
use soliddraft_materials::{Appearance, AppearanceId, Srgb};
use soliddraft_model::{AppearanceTarget, DatumPlane, PlaneRef};

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    settle_appearance(ui, state);
    if state.sketch.is_some() {
        sketch_panel(ui, state);
    } else {
        selection(ui, state);
    }
}

fn header(ui: &mut egui::Ui, icon: Icon, title: &str) -> Rect {
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    icons::paint(
        ui.painter(),
        icon,
        Rect::from_center_size(pos2(head.left() + 20.0, head.center().y), vec2(16.0, 16.0)),
        theme::ink(),
    );
    ui.painter().text(
        pos2(head.left() + 34.0, head.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::font_semibold(12.5),
        theme::ink(),
    );
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, theme::line()));
    head
}

fn small_button(ui: &mut egui::Ui, rect: Rect, icon: Icon, tip: &str, primary: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(("pm_btn", tip)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
    let (bg, fg) = if primary {
        (if resp.hovered() { theme::accent().gamma_multiply(0.9) } else { theme::accent() }, theme::on_accent())
    } else {
        (if resp.hovered() { theme::line() } else { theme::hover() }, theme::ink())
    };
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    icons::paint(ui.painter(), icon, rect.shrink(4.0), fg);
    resp.on_hover_text(tip)
}

// ---------------------------------------------------------------------------
// Shared form controls.

/// The width of a form table's caption column.
const LABEL_WIDTH: f32 = 96.0;
/// The height of one row of a form table.
const ROW_HEIGHT: f32 = 24.0;
/// The height of a pick's chip, inside its margin.
const CHIP_HEIGHT: f32 = 18.0;
/// How many picks a list shows before it scrolls.
const LIST_ROWS: usize = 4;

/// One row of a form table: the caption in its column, and the cell the value goes in.
fn row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui, Rect) -> R) -> R {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let caption = Rect::from_min_max(rect.min, pos2(rect.left() + LABEL_WIDTH - 6.0, rect.bottom()));
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(caption).layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| ui.add(egui::Label::new(egui::RichText::new(label).size(12.0).color(theme::ink())).truncate()),
    );
    let cell =
        Rect::from_min_max(pos2(rect.left() + LABEL_WIDTH, rect.top() + 1.0), pos2(rect.right(), rect.bottom() - 1.0));
    add(ui, cell)
}

/// A row whose cell holds a number widget, named "<label> value" for automation.
fn number_row(ui: &mut egui::Ui, label: &str, unit: &str, widget: impl egui::Widget) -> egui::Response {
    number_row_named(ui, label, label, unit, widget)
}

/// A number row whose widget is named "<name> value" apart from its caption.
fn number_row_named(
    ui: &mut egui::Ui,
    caption: &str,
    name: &str,
    unit: &str,
    widget: impl egui::Widget,
) -> egui::Response {
    let response = row(ui, caption, |ui, cell| value_cell(ui, cell, unit, |ui, inner| ui.put(inner, widget)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, format!("{name} value")));
    response
}

/// A row whose cell holds a choice, filling it.
fn choice_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui, f32)) {
    row(ui, label, |ui, cell| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(cell), |ui| {
            ui.spacing_mut().interact_size.y = cell.height();
            add(ui, cell.width());
        });
    });
}

pub fn mm_field(ui: &mut egui::Ui, label: &str, value: &mut f64) -> bool {
    unit_field(ui, label, value, "mm")
}

fn unit_field(ui: &mut egui::Ui, label: &str, value: &mut f64, unit: &str) -> bool {
    unit_value(ui, label, value, unit, true).changed()
}

/// A labelled number field with its unit. Without `update_while_editing`, a typed value
/// counts only when Enter or a click elsewhere ends the typing.
fn unit_value(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    unit: &str,
    update_while_editing: bool,
) -> egui::Response {
    number_row(
        ui,
        label,
        unit,
        egui::DragValue::new(value)
            .speed(0.5)
            .range(0.01..=100_000.0)
            .fixed_decimals(2)
            .update_while_editing(update_while_editing),
    )
}

pub(crate) fn size_input(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f64,
    refused: &mut Option<String>,
    cell: Rect,
    id: egui::Id,
) {
    let what = label.to_lowercase();
    // The typed text outlives the frame; it restarts from the size when it no longer
    // stands for it.
    let kept = ui.data(|d| d.get_temp::<String>(id));
    let mut text = kept
        .filter(|t| refused.is_some() || crate::fillet_collect::parse_size(t, &what) == Ok(*value))
        .unwrap_or_else(|| crate::document::fmt_mm(*value));
    let r = value_cell(ui, cell, "mm", |ui, inner| {
        ui.put(
            inner,
            egui::TextEdit::singleline(&mut text)
                .font(theme::font(12.5))
                .frame(egui::Frame::NONE)
                .horizontal_align(egui::Align::Center)
                .vertical_align(egui::Align::Center),
        )
    });
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, format!("{label} value")));
    if r.changed() {
        match crate::fillet_collect::parse_size(&text, &what) {
            Ok(v) => {
                *value = v;
                *refused = None;
            }
            Err(why) => *refused = Some(why),
        }
    }
    ui.data_mut(|d| d.insert_temp(id, text));
}

/// The selected feature's dimension. Dragging it previews the part off the interface
/// thread and applies the value where the drag ends as one undo step; a typed value
/// applies on Enter.
fn dimension_field(ui: &mut egui::Ui, state: &mut AppState, id: FeatureId, kind: &FeatureKind) {
    let Some(dimension) = Dimension::of(kind) else { return };
    let Some(applied) = dimension.get(kind) else { return };
    let mut value = match &state.drag_preview {
        Some(d) if d.feature() == id => d.value(),
        _ => applied,
    };
    ui.add_space(8.0);
    let r = unit_value(ui, dimension.label(), &mut value, "mm", false);
    if r.drag_started() {
        state.push(AppAction::DragDimension(DragStep::Start { feature: id, dimension }));
    }
    if r.changed() {
        if r.dragged() || r.drag_started() {
            state.push(AppAction::DragDimension(DragStep::Move(value)));
        } else if value != applied
            && let Some(command) = crate::drag::edit_command(id, dimension.with(kind, value))
        {
            state.push(AppAction::Command(command));
        }
    }
    if r.drag_stopped() {
        state.push(AppAction::DragDimension(DragStep::Finish));
    }
    if let Some(drag) = &state.drag_preview
        && drag.feature() == id
    {
        let (message, failed) = drag.label();
        ui.label(egui::RichText::new(crate::geometry_message::for_user(&message)).size(11.0).color(if failed {
            theme::error()
        } else {
            theme::muted()
        }));
    } else if let Some(message) = state.doc.display_failure() {
        ui.label(egui::RichText::new(crate::geometry_message::for_user(message)).size(11.0).color(theme::error()));
    } else if let Some((message, failed)) = state.doc.proof_message() {
        ui.label(egui::RichText::new(crate::geometry_message::for_user(message)).size(11.0).color(if failed {
            theme::error()
        } else {
            theme::muted()
        }));
    }
}

/// What the viewport waits for: an accent-edged note.
fn pick_prompt(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::accent_soft())
        .stroke(Stroke::new(1.0, theme::accent().gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), Icon::Point, r, theme::accent());
                ui.label(egui::RichText::new(text).size(11.5).color(theme::accent()));
            });
        });
}

/// A pick the form holds: its icon and name on a chip, with a remove button at its
/// right when `remove` is offered. Returns whether the remove button was clicked.
fn picked_chip(ui: &mut egui::Ui, icon: Icon, label: &str, remove: bool) -> bool {
    let mut removed = false;
    egui::Frame::new()
        .fill(theme::accent_soft())
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin { left: 8, right: 3, top: 2, bottom: 2 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                icons::paint(ui.painter(), icon, r, theme::accent());
                // A long name is cut short with an ellipsis; the chip never grows past the card.
                let room = (ui.available_width() - if remove { 26.0 } else { 0.0 }).max(0.0);
                ui.allocate_ui_with_layout(
                    vec2(room, CHIP_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(room);
                        ui.add(
                            egui::Label::new(egui::RichText::new(label).size(12.0).color(theme::selection_ink()))
                                .truncate(),
                        )
                        .on_hover_text(label);
                    },
                );
                if remove {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(CHIP_HEIGHT, CHIP_HEIGHT), Sense::hover());
                        let tip = format!("Remove {label}");
                        let r = ui.interact(rect, ui.id().with(("remove", label)), Sense::click());
                        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &tip));
                        if r.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(4), theme::hover());
                        }
                        icons::paint(ui.painter(), Icon::Close, rect.shrink(4.5), theme::muted());
                        removed = r.on_hover_text("Remove").clicked();
                    });
                }
            });
        });
    removed
}

/// The picks a form holds, as chips, or the prompt for the first one. Returns the index
/// whose remove button was clicked.
pub(crate) fn pick_list<'a>(
    ui: &mut egui::Ui,
    prompt: &str,
    icon: Icon,
    picks: impl Iterator<Item = &'a str>,
) -> Option<usize> {
    let picks: Vec<&str> = picks.collect();
    if picks.is_empty() {
        pick_prompt(ui, prompt);
        return None;
    }
    let chips = |ui: &mut egui::Ui| {
        let mut remove = None;
        for (i, label) in picks.iter().enumerate() {
            if picked_chip(ui, icon, label, true) {
                remove = Some(i);
            }
        }
        remove
    };
    if picks.len() <= LIST_ROWS {
        return chips(ui);
    }
    // A long list scrolls in a box of its own, as CAD selection boxes do, so the
    // parameters under it stay in view. Half a chip shows below the box's edge.
    let pitch = CHIP_HEIGHT.max(ui.spacing().interact_size.y) + 4.0 + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt(("picks", prompt))
        .max_height(pitch * (LIST_ROWS as f32 + 0.5))
        .auto_shrink([false, true])
        .show(ui, chips)
        .inner
}

/// A row of mutually exclusive choices as one segmented control. Returns the index
/// clicked, if any. Each segment reports itself selected or not, like a selectable label.
fn segmented(ui: &mut egui::Ui, id: &str, options: &[&str], selected: usize) -> Option<usize> {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(7),
        theme::field_bg(),
        Stroke::new(1.0, theme::line()),
        egui::StrokeKind::Inside,
    );
    let each = rect.width() / options.len() as f32;
    let mut clicked = None;
    for (i, text) in options.iter().enumerate() {
        let segment =
            Rect::from_min_size(pos2(rect.left() + each * i as f32, rect.top()), vec2(each, rect.height())).shrink(2.0);
        let r = ui.interact(segment, ui.id().with((id, i)), Sense::click());
        let on = i == selected;
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, on, *text));
        let (fill, ink) = if on {
            (theme::accent(), theme::on_accent())
        } else if r.hovered() {
            (theme::hover(), theme::ink())
        } else {
            (egui::Color32::TRANSPARENT, theme::ink())
        };
        ui.painter().rect_filled(segment, CornerRadius::same(5), fill);
        let font = if on { theme::font_semibold(12.0) } else { theme::font(12.0) };
        ui.painter().text(segment.center(), Align2::CENTER_CENTER, *text, font, ink);
        if r.clicked() {
            clicked = Some(i);
        }
    }
    ui.add_space(4.0);
    clicked
}

pub(crate) fn coord(ui: &mut egui::Ui, id: &str, label: &str, value: &mut f64) {
    ui.label(egui::RichText::new(label).size(11.0).color(theme::muted()));
    let r = ui.add_sized(
        vec2(64.0, 22.0),
        egui::DragValue::new(value).speed(0.5).range(-100_000.0..=100_000.0).fixed_decimals(2),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, format!("{id} value")));
}

/// A reference plane choice; a picked face shows by its label.
pub(crate) fn plane_combo(
    ui: &mut egui::Ui,
    id: &str,
    plane: &mut PlaneRef,
    choices: &[(soliddraft_model::FeatureId, String)],
) {
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width() - 4.0)
        .selected_text(match plane {
            PlaneRef::Feature { feature } => choices
                .iter()
                .find(|(id, _)| id == feature)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| "Missing plane".into()),
            _ => plane_ref_label(plane),
        })
        .show_ui(ui, |ui| {
            for (feature, name) in choices {
                ui.selectable_value(plane, PlaneRef::Feature { feature: *feature }, name);
            }
            for datum in [DatumPlane::Top, DatumPlane::Front, DatumPlane::Right] {
                let chosen = matches!(plane, PlaneRef::Datum { datum: d, .. } if *d == datum);
                if ui.selectable_label(chosen, datum.label()).clicked() {
                    *plane = PlaneRef::Datum { datum, offset: 0.0 };
                }
            }
        });
}

fn section_gap(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    theme::hline(ui);
    ui.add_space(6.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(theme::muted()));
}

/// A field parameter: one value, or a linear grade between two anchors on a part axis.
/// `unit` names the values, such as "mm"; the anchors are always millimetres.
pub(crate) fn grade_options(
    ui: &mut egui::Ui,
    label: &str,
    grade: &mut soliddraft_kernel_api::Grade,
    unit: &str,
    speed: f64,
) {
    use soliddraft_kernel_api::{Grade, GradeAxis, LinearGrade};
    ui.label(egui::RichText::new(label).size(12.0).color(theme::ink()));
    let linear = matches!(grade, Grade::Linear(_));
    if let Some(choice) = segmented(ui, label, &["Constant", "Linear along an axis"], linear as usize) {
        match (choice, *grade) {
            (0, Grade::Linear(l)) => *grade = Grade::Constant(l.start),
            (1, Grade::Constant(v)) => {
                *grade = Grade::Linear(LinearGrade { axis: GradeAxis::Z, from: 0.0, to: 10.0, start: v, end: v })
            }
            _ => {}
        }
    }
    let value = |ui: &mut egui::Ui, name: &str, v: &mut f64, unit: &str| {
        let response = row(ui, name, |ui, cell| {
            value_cell(ui, cell, unit, |ui, inner| ui.put(inner, egui::DragValue::new(v).speed(speed)))
        });
        let info = if name.is_empty() { format!("{label} value") } else { format!("{label} {name} value") };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, info.clone()));
    };
    match grade {
        Grade::Constant(_) => {}
        Grade::Linear(l) => {
            choice_row(ui, "Axis", |ui, _| {
                ui.horizontal_centered(|ui| {
                    for axis in GradeAxis::ALL {
                        ui.selectable_value(&mut l.axis, axis, axis.name());
                    }
                });
            });
            value(ui, "from", &mut l.from, "mm");
            value(ui, "to", &mut l.to, "mm");
            value(ui, "start", &mut l.start, unit);
            value(ui, "end", &mut l.end, unit);
            hint(ui, "Start at the from coordinate, end at the to coordinate, held beyond them. Part coordinates.");
        }
    }
}

pub(crate) fn pattern_sources(
    doc: &crate::document::Document,
    target: usize,
) -> Vec<(soliddraft_model::FeatureId, String)> {
    doc.features
        .iter()
        .take(target)
        .filter(|f| f.kind.to_model().can_pattern_feature())
        .map(|f| (soliddraft_model::FeatureId(f.id as u32), f.name.clone()))
        .collect()
}

pub(crate) fn pattern_source(
    ui: &mut egui::Ui,
    pattern: &mut soliddraft_model::PatternFeature,
    choices: &[(soliddraft_model::FeatureId, String)],
) {
    let name = pattern
        .source
        .and_then(|id| choices.iter().find(|(i, _)| *i == id).map(|(_, n)| n.as_str()))
        .unwrap_or("Whole body");
    let response = egui::ComboBox::from_id_salt("pattern_source").selected_text(name).show_ui(ui, |ui| {
        ui.selectable_value(&mut pattern.source, None, "Whole body");
        for (id, name) in choices {
            ui.selectable_value(&mut pattern.source, Some(*id), name);
        }
    });
    response.response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Pattern source");
        info.current_text_value = Some(name.into());
        info
    });
}

pub(crate) fn thicken_side(ui: &mut egui::Ui, side: &mut soliddraft_kernel_api::ThickenSide) {
    use soliddraft_kernel_api::ThickenSide;
    let name = match side {
        ThickenSide::Front => "Front",
        ThickenSide::Back => "Back",
    };
    let response = egui::ComboBox::from_id_salt("thicken_side").selected_text(name).show_ui(ui, |ui| {
        for (value, name) in [(ThickenSide::Front, "Front"), (ThickenSide::Back, "Back")] {
            ui.selectable_value(side, value, name);
        }
    });
    response.response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Thicken side");
        info.current_text_value = Some(name.into());
        info
    });
}

fn sketch_panel(ui: &mut egui::Ui, state: &mut AppState) {
    let plane_label = state
        .sketch
        .as_ref()
        .and_then(|s| state.doc.feature(s.feature))
        .and_then(|f| f.sketch())
        .map(|s| s.plane.label())
        .unwrap_or_default();

    let mut actions = Vec::new();
    let has_body = state.doc.model().body().is_some();
    let links =
        state.sketch.as_ref().map(|s| crate::sketch_values::links(state.doc.model(), s.feature)).unwrap_or_default();
    let Some(s) = state.sketch.as_mut() else { return };
    let profiles = crate::sketch::profile::profiles(&s.sketch, s.plane);
    let reset_scroll = ui.data_mut(|data| {
        let id = ui.id().with("sketch_active_tool");
        let previous = data.get_temp::<Tool>(id);
        data.insert_temp(id, s.tool);
        previous != Some(s.tool) && s.tool.modifies()
    });
    let scroll = egui::ScrollArea::vertical().id_salt("pm").auto_shrink([false, false]);
    let scroll = if reset_scroll { scroll.vertical_scroll_offset(0.0) } else { scroll };
    scroll.show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
            // Status.
            let mut reveal_relations = false;
            let (color, label) = match &s.sketch.status {
                crate::sketch::SolveStatus::Empty => (theme::muted(), "Empty".to_owned()),
                crate::sketch::SolveStatus::UnderDefined => (theme::accent(), "Under defined".to_owned()),
                crate::sketch::SolveStatus::FullyDefined => (theme::ok(), "Fully defined".to_owned()),
                crate::sketch::SolveStatus::OverDefined { conflicting } => {
                    (theme::error(), format!("Over defined ({} conflicting)", conflicting.len()))
                }
                crate::sketch::SolveStatus::Failed(e) => (theme::error(), format!("Solver failed: {e}")),
            };
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, color);
                ui.label(egui::RichText::new(&label).family(theme::medium()).size(12.0).color(color));
            });
            let (dims, rels) = s.sketch.counts();
            hint(
                ui,
                &format!(
                    "On {plane_label} · {} entities, {rels} relations, {dims} dimensions",
                    s.sketch.drawn().count()
                ),
            );
            if let crate::sketch::SolveStatus::OverDefined { conflicting } = &s.sketch.status {
                ui.add_space(4.0);
                let names: Vec<String> =
                    conflicting.iter().filter_map(|id| s.sketch.constraint(*id)).map(|c| c.name().to_owned()).collect();
                hint(ui, &format!("Conflicting: {}. Delete one of them.", names.join(", ")));
                if ui.small_button("Inspect conflicts").clicked() {
                    s.relations_filter = RelationFilter::Conflicting;
                    reveal_relations = true;
                }
            }
            section_gap(ui);

            crate::sketch_diagnostics::panel(ui, s, &mut actions, &profiles, has_body);

            if s.tool == Tool::Arc {
                ui.checkbox(&mut s.arc_clockwise, "Clockwise arc")
                    .on_hover_text("Off draws counterclockwise; either direction supports major arcs");
                section_gap(ui);
            }

            // Keep the active tool's controls ahead of the growing tool palette.
            if s.tool.modifies() {
                crate::sketch_tools::panel(ui, s, &mut actions);
                section_gap(ui);
            }

            // Tools.
            theme::section_label(ui, "Tools");
            let tools = [
                (Tool::Select, Icon::Views, "Select"),
                (Tool::Line, Icon::Line, "Line (L)"),
                (Tool::Centreline, Icon::Cad("construction"), "Centreline"),
                (Tool::Rectangle, Icon::Rectangle, "Rectangle (R)"),
                (Tool::CentreRectangle, Icon::Rectangle, "Centre rectangle"),
                (Tool::ThreePointCornerRectangle, Icon::Rectangle, "3-point corner rectangle"),
                (Tool::ThreePointCentreRectangle, Icon::Rectangle, "3-point centre rectangle"),
                (Tool::Parallelogram, Icon::Rectangle, "Parallelogram"),
                (Tool::Circle, Icon::Circle, "Circle (C)"),
                (Tool::PerimeterCircle, Icon::Circle, "Perimeter circle"),
                (Tool::Ellipse, Icon::Circle, "Ellipse"),
                (Tool::PartialEllipse, Icon::Arc, "Partial ellipse"),
                (Tool::FitSpline,Icon::Arc,"Fit-point spline"),
                (Tool::ControlSpline,Icon::Arc,"Control-point spline"),
                (Tool::Text,Icon::Text,"Sketch text"),
                (Tool::Arc, Icon::Arc, "Arc (A)"),
                (Tool::ThreePointArc, Icon::Arc, "Three-point arc"),
                (Tool::TangentArc, Icon::Arc, "Tangent arc"),
                (Tool::Slot, Icon::Slot, "Slot (S)"),
                (Tool::CentrepointSlot, Icon::Slot, "Centrepoint slot"),
                (Tool::ThreePointArcSlot, Icon::Slot, "Three-point arc slot"),
                (Tool::CentrepointArcSlot, Icon::Slot, "Centrepoint arc slot"),
                (Tool::Point, Icon::Point, "Point (P)"),
                (Tool::Dimension, Icon::SmartDim, "Smart dimension (D)"),
                (Tool::Polygon, Icon::Polygon, "Polygon"),
                (Tool::Trim, Icon::Trim, "Trim"),
                (Tool::Fillet, Icon::Fillet, "Sketch fillet"),
                (Tool::Chamfer, Icon::Chamfer, "Sketch chamfer"),
                (Tool::Split, Icon::Trim, "Split entities"),
                (Tool::Extend, Icon::Extend, "Extend"),
                (Tool::Offset, Icon::Offset, "Offset"),
                (Tool::Mirror, Icon::Mirror, "Mirror entities"),
                (Tool::LinearPattern, Icon::LinearPattern, "Linear pattern"),
                (Tool::CircularPattern, Icon::CircularPattern, "Circular pattern"),
                (Tool::Move, Icon::Views, "Move entities"),
                (Tool::Copy, Icon::LinearPattern, "Copy entities"),
                (Tool::Rotate, Icon::CircularPattern, "Rotate entities"),
                (Tool::Scale, Icon::Rectangle, "Scale entities"),
            ];
            ui.horizontal_wrapped(|ui| {
                // Keep Modify controls above the command bar as the palette gains tools.
                ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
                for (tool, icon, tip) in tools {
                    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            true,
                            s.tool == tool || (tool == Tool::Trim && s.tool.trims()),
                            tip,
                        )
                    });
                    let on = s.tool == tool || (tool == Tool::Trim && s.tool.trims());
                    if on {
                        ui.painter().rect_filled(r, CornerRadius::same(6), theme::selection_bg());
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), theme::hover());
                    }
                    icons::paint(
                        ui.painter(),
                        icon,
                        r.shrink(6.0),
                        if on { theme::selection_ink() } else { theme::ink() },
                    );
                    if resp.on_hover_text(tip).clicked() {
                        s.set_tool(tool);
                    }
                }
                if ui
                    .add(egui::Button::new("Sketch diagnostics").min_size(vec2(0.0, 26.0)))
                    .on_hover_text("Locate profile faults and invalid relation references")
                    .clicked()
                {
                    actions.push(AppAction::SketchDiagnostics(true));
                }
                if s.selection.iter().any(|selection|matches!(selection,SketchSel::Constraint(id) if s.sketch.constraint(*id).is_some_and(|c|c.is_dimension()))) {
                    let eligible = crate::sketch_values::selected(s);
                    if ui
                        .add_enabled(eligible.is_ok(), egui::Button::new("Link values"))
                        .on_hover_text(
                            eligible.err().unwrap_or_else(|| "Share a named value between selected dimensions".into()),
                        )
                        .clicked()
                    {
                        actions.push(AppAction::SketchValues(crate::sketch_values::Action::Open));
                    }
                }
            });
            if s.tool == Tool::Dimension {
                ui.checkbox(&mut s.reference_dimensions, "Reference dimensions")
                    .on_hover_text("Measure geometry without adding a solver equation");
            }
            section_gap(ui);

            crate::sketch_tools::patterns::editors(ui,s);
            crate::sketcher::text::panel(ui, s, &mut actions);

            ui.collapsing("Sketch display", |ui| {
                let mut shading = s.show_regions;
                if ui.checkbox(&mut shading, "Contour shading").changed() {
                    actions.push(AppAction::SketchShading(shading));
                }
                let mut dimensions = s.show_dimensions;
                if ui.checkbox(&mut dimensions, "Sketch dimensions").changed() {
                    actions.push(AppAction::SketchDimensions(dimensions));
                }
                ui.checkbox(&mut s.grid_snap, "Snap to grid")
                    .on_hover_text("Use the current minor grid spacing, even when the grid is hidden. Geometry snaps take priority. Quick snap overrides this for one placement.");
                hint(ui, "Grid spacing and display: Preferences > Viewport > Grid and planes.");
            });

            // Selection.
            let selection = s.selection.clone();
            let mut remove_constraint = None;
            let mut delete_entities = false;
            let mut toggle_construction = false;
            let mut value_change: Option<(crate::sketch::ConstraintId, f64)> = None;
            let mut reference_change = None;
            if selection.is_empty() {
                theme::section_label(ui, "Selection");
                hint(ui, "Click an entity, a point or a relation. Ctrl-click adds to the selection.");
            } else {
                theme::section_label(ui, &format!("Selection ({})", selection.len()));
                for sel in &selection {
                    match sel {
                        SketchSel::Entity(id) => {
                            if let Some(e) = s.sketch.entity(*id) {
                                let detail = match e.kind {
                                    EntityKind::Text {ref text} => format!("{} · {}",text.content,text.font.label()),
                                    EntityKind::Spline {ref points,..} => format!("{} points",points.len()),
                                    EntityKind::Line { a, b } => format!("{} mm long", fmt_value(a.dist(b))),
                                    EntityKind::Circle { radius, .. } => format!("Ø{} mm", fmt_value(radius * 2.0)),
                                    EntityKind::Arc { .. } => format!("R{} mm", fmt_value(e.radius().unwrap_or(0.0))),
                                    EntityKind::Ellipse { center, major, minor } | EntityKind::EllipticArc { center, major, minor, .. } => format!("Semi-axes {} × {} mm", fmt_value(center.dist(major)), fmt_value(center.dist(minor))),
                                    EntityKind::Point { p } => format!("{}, {}", fmt_value(p.u), fmt_value(p.v)),
                                };
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("{} {}", e.kind_name(), e.id)).size(12.0));
                                    ui.label(egui::RichText::new(detail).size(11.5).color(theme::muted()));
                                    if e.construction {
                                        ui.label(egui::RichText::new("construction").size(10.5).color(theme::muted()));
                                    }
                                });
                            }
                        }
                        SketchSel::Point(p) => {
                            if let Some(q) = s.sketch.point(*p) {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Point of {} {} at {}, {}",
                                        s.sketch.entity(p.entity).map(|e| e.kind_name()).unwrap_or("?"),
                                        p.entity,
                                        fmt_value(q.u),
                                        fmt_value(q.v)
                                    ))
                                    .size(12.0),
                                );
                            }
                        }
                        SketchSel::Constraint(c) => {
                            if let Some(con) = s.sketch.constraint(*c).cloned() {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(con.name()).size(12.0));
                                    if con.is_reference() {
                                        let value = con
                                            .display_value(&s.sketch)
                                            .map(fmt_value)
                                            .unwrap_or_else(|| "unavailable".into());
                                        let unit = if matches!(con.geometry_kind(), ConstraintKind::Angle { .. }) {
                                            "°"
                                        } else {
                                            "mm"
                                        };
                                        ui.label(format!("({value} {unit})"));
                                    } else if let Some(name) = links.get(c) {
                                        if ui
                                            .small_button(format!("{} = {name}", fmt_value(con.value().unwrap_or(0.0))))
                                            .on_hover_text("Edit the shared value for every linked member")
                                            .clicked()
                                        {
                                            s.selection = vec![SketchSel::Constraint(*c)];
                                            actions.push(AppAction::SketchValues(crate::sketch_values::Action::Open));
                                        }
                                    } else if con.is_dimension() {
                                        let mut v = con.value().unwrap_or(0.0);
                                        let unit =
                                            if matches!(con.kind, ConstraintKind::Angle { .. }) { "°" } else { "mm" };
                                        let mut field = egui::DragValue::new(&mut v)
                                            .speed(0.5)
                                            .fixed_decimals(2)
                                            .suffix(format!(" {unit}"));
                                        // A length is a size: a drag past zero stops at the smallest one.
                                        if !matches!(con.kind, ConstraintKind::Angle { .. }) {
                                            field = field.range(0.01..=100_000.0);
                                        }
                                        let r = ui.add(field);
                                        if r.changed() {
                                            value_change = Some((con.id, v));
                                        }
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if ui
                                            .add(egui::Button::new("×").small().frame(false))
                                            .on_hover_text("Delete relation")
                                            .clicked()
                                        {
                                            remove_constraint = Some(con.id);
                                        }
                                    });
                                });
                                if con.is_dimension() {
                                    let mut reference = con.is_reference();
                                    if ui.checkbox(&mut reference, "Reference dimension").changed() {
                                        reference_change = Some((con.id, reference));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let entities = s.selected_entities();
            if !entities.is_empty() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let all_construction = entities.iter().all(|e| s.sketch.entity(*e).is_some_and(|e| e.construction));
                    if ui
                        .selectable_label(all_construction, "For construction")
                        .on_hover_text("Construction geometry guides other entities and never becomes a profile (X)")
                        .clicked()
                    {
                        toggle_construction = true;
                    }
                    if ui.button(egui::RichText::new("Delete").color(theme::error())).clicked() {
                        delete_entities = true;
                    }
                });
            }
            // Inspect owners without making them geometry-deletion targets.
            let inspected: std::collections::HashSet<_> = selection
                .iter()
                .flat_map(|sel| match sel {
                    SketchSel::Entity(id) => vec![*id],
                    SketchSel::Point(p) => vec![p.entity],
                    SketchSel::Constraint(id) => s.sketch.constraint(*id).map(|c| c.entities()).unwrap_or_default(),
                })
                .collect();
            ui.add_space(6.0);
            if reveal_relations {
                ui.scroll_to_cursor(Some(egui::Align::TOP));
            }
            crate::sketch_tools::offset_inspector(ui, s);
            crate::sketch_tools::polygon_inspector(ui, s, &mut actions);
            let relation_top = ui.next_widget_position();
            theme::section_label(ui, "Existing relations");
            let previous_filter = s.relations_filter;
            egui::ComboBox::from_label("Show").selected_text(s.relations_filter.label()).show_ui(ui, |ui| {
                for scope in [
                    RelationFilter::Selected,
                    RelationFilter::All,
                    RelationFilter::Conflicting,
                    RelationFilter::Dangling,
                ] {
                    ui.selectable_value(&mut s.relations_filter, scope, scope.label());
                }
            });
            if previous_filter != s.relations_filter {
                ui.scroll_to_rect(Rect::from_min_size(relation_top, vec2(ui.available_width(), 90.0)), None);
            }
            let by_id: std::collections::HashMap<_, _> = s.sketch.entities.iter().map(|e| (e.id, e)).collect();
            let conflicts: std::collections::HashSet<_> = match &s.sketch.status {
                crate::sketch::SolveStatus::OverDefined { conflicting } => conflicting.iter().copied().collect(),
                _ => Default::default(),
            };
            let conflicting = |id| conflicts.contains(&id);
            let dangling = |c: &crate::sketch::Constraint| {
                c.entities().iter().any(|id| !by_id.contains_key(id))
                    || c.points().iter().any(|p| by_id.get(&p.entity).and_then(|e| e.point(p.index)).is_none())
            };
            let rels: Vec<_> = s
                .sketch
                .constraints
                .iter()
                .filter(|c| match s.relations_filter {
                    RelationFilter::Selected => c.entities().iter().any(|id| inspected.contains(id)),
                    RelationFilter::All => true,
                    RelationFilter::Conflicting => conflicting(c.id),
                    RelationFilter::Dangling => dangling(c),
                })
                .map(|c| (c.clone(), conflicting(c.id), dangling(c)))
                .collect();
            hint(ui, &format!("{} of {} shown", rels.len(), s.sketch.constraints.len()));
            if rels.is_empty() {
                hint(
                    ui,
                    match s.relations_filter {
                        RelationFilter::Selected if selection.is_empty() => {
                            "Select geometry or a relation, or choose All."
                        }
                        RelationFilter::Selected => "No relations on this selection.",
                        RelationFilter::All => "No stored relations in this sketch.",
                        RelationFilter::Conflicting => "No conflicting relations.",
                        RelationFilter::Dangling => "No dangling references.",
                    },
                );
            }
            for (c, conflict, broken) in rels {
                ui.push_id(c.id, |ui| {
                    ui.horizontal(|ui| {
                        let text = if c.is_reference() {
                            let value = c.display_value(&s.sketch).map(fmt_value).unwrap_or_else(|| "?".into());
                            format!("{} ({value})", c.name())
                        } else {
                            match c.value() {
                                Some(v) => format!("{} {}", c.name(), fmt_value(v)),
                                None => c.name().to_owned(),
                            }
                        };
                        let mut text = egui::RichText::new(text);
                        if conflict || broken {
                            text = text.color(theme::error());
                        }
                        let owners = c
                            .entities()
                            .iter()
                            .map(|id| {
                                by_id
                                    .get(id)
                                    .map(|e| format!("{} {}", e.kind_name(), e.id))
                                    .unwrap_or_else(|| format!("missing entity {id}"))
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        let status = if broken {
                            "Missing reference. "
                        } else if conflict {
                            "Conflicting. "
                        } else {
                            ""
                        };
                        if ui
                            .selectable_label(s.selection.contains(&SketchSel::Constraint(c.id)), text)
                            .on_hover_text(format!("{status}Relation {} on {owners}", c.id))
                            .clicked()
                        {
                            s.selection = vec![SketchSel::Constraint(c.id)];
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(egui::Button::new("×").small().frame(false))
                                .on_hover_text("Delete relation")
                                .clicked()
                            {
                                remove_constraint = Some(c.id);
                            }
                        });
                    })
                });
            }
            // Relations that can be added.
            let applicable = sketcher::applicable_relation_sets(&s.sketch, &selection);
            if !applicable.is_empty() {
                ui.add_space(6.0);
                theme::section_label(ui, "Add relation");
                let mut add = None;
                ui.horizontal_wrapped(|ui| {
                    for (name, kinds) in applicable {
                        let applied = kinds.iter().all(|kind| s.sketch.constraints.iter().any(|c| c.kind == *kind));
                        if ui.add_enabled(!applied, egui::Button::new(name)).on_disabled_hover_text("Already applied").clicked() {
                            add = Some(kinds);
                        }
                    }
                });
                if let Some(kind) = add {
                    actions.push(sketcher::add_relations(s, &kind));
                }
            }
            let mut changed = false;
            if let Some((id, reference)) = reference_change {
                match s.sketch.set_dimension_reference(id, reference) {
                    Ok(()) => changed = true,
                    Err(error) => s.say(error),
                }
            }
            if let Some((id, v)) = value_change
                && let Some(c) = s.sketch.constraint_mut(id)
            {
                c.set_value(v);
                changed = true;
            }
            if let Some(id) = remove_constraint {
                s.sketch.remove_constraint(id);
                s.selection.retain(|x| *x != SketchSel::Constraint(id));
                changed = true;
            }
            if toggle_construction {
                let all = entities.iter().all(|e| s.sketch.entity(*e).is_some_and(|e| e.construction));
                for e in &entities {
                    if let Some(ent) = s.sketch.entity_mut(*e) {
                        ent.construction = !all;
                    }
                }
                changed = true;
            }
            if delete_entities {
                for e in &entities {
                    s.sketch.remove_entity(*e);
                }
                s.selection.clear();
                changed = true;
            }
            if changed {
                solve::solve(&mut s.sketch, None, true);
                actions.push(AppAction::Command(Command::EditSketch { id: s.feature, sketch: s.sketch.clone() }));
            }
            ui.add_space(8.0);
            hint(ui, "Blue entities can still move; black ones are fully defined. Enter exits the sketch.");
        });
    });
    for a in actions {
        state.push(a);
    }
}

fn selection(ui: &mut egui::Ui, state: &mut AppState) {
    crate::measure::panel::show(ui, state);
    let Some(sel) = state.doc.selection else {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
            icons::paint(ui.painter(), Icon::Views, r, theme::line().gamma_multiply(1.4));
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Nothing selected").family(theme::medium()).color(theme::muted()));
            ui.label(egui::RichText::new("Click a face, edge, vertex or a tree row.").size(11.0).color(theme::muted()));
        });
        return;
    };
    let Some(info) = state.doc.describe(sel) else { return };
    let feature = info.feature.and_then(|id| state.doc.feature(id)).cloned();
    let feature = feature.as_ref();
    let icon = match (sel, feature.map(|f| &f.kind)) {
        (Selection::Feature(_), Some(crate::document::FeatureKind::ReferencePlane(_))) => Icon::Plane,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Sketch(_))) => Icon::Sketch,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Cut(_))) => Icon::Cut,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Fillet(_))) => Icon::Fillet,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Chamfer(_))) => Icon::Chamfer,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Hole(_))) => Icon::Hole,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Pattern(_))) => Icon::LinearPattern,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Mirror(_))) => Icon::Mirror,
        (Selection::Feature(_), Some(crate::document::FeatureKind::Draft(_))) => Icon::Draft,
        (
            Selection::Feature(_),
            Some(
                crate::document::FeatureKind::Shell(_)
                | crate::document::FeatureKind::Offset(_)
                | crate::document::FeatureKind::Thicken(_),
            ),
        ) => Icon::Shell,
        (Selection::Feature(_), Some(crate::document::FeatureKind::ImportStep(_))) => Icon::Import,
        (
            Selection::Feature(_),
            Some(crate::document::FeatureKind::Lattice(_) | crate::document::FeatureKind::Gyroid(_)),
        ) => Icon::Cad("lattice"),
        (Selection::Feature(_), _) => Icon::Extrude,
        (Selection::Body, _) => Icon::Extrude,
        (Selection::Face(_), _) => Icon::Plane,
        (Selection::Edge(_), _) => Icon::Axis,
        (Selection::Vertex(_), _) => Icon::Point,
        (Selection::Reference(Reference::Origin), _) => Icon::Point,
        (Selection::Reference(Reference::Plane(_)), _) => Icon::Plane,
        (Selection::Reference(Reference::Axis(_)), _) => Icon::Axis,
    };
    let head = header(ui, icon, &info.title);
    let clear = small_button(
        ui,
        Rect::from_center_size(pos2(head.right() - 16.0, head.center().y), vec2(22.0, 22.0)),
        Icon::Close,
        "Clear selection (Esc)",
        false,
    );
    if clear.clicked() {
        state.push(AppAction::Select(None));
    }
    let status = feature.map(|f| f.status.clone());
    let suppressed = feature.is_some_and(|f| f.suppressed);
    let is_sketch = feature.is_some_and(|f| f.is_sketch());
    let is_planar_face = matches!(sel, Selection::Face(f) if state.doc.named_face(f).is_some_and(|nf| nf.surface == soliddraft_kernel_api::SurfaceKind::Plane));
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        let kind = match sel {
            Selection::Body => "Body",
            Selection::Feature(_) => "Feature",
            Selection::Face(_) => "Face",
            Selection::Edge(_) => "Edge",
            Selection::Vertex(_) => "Vertex",
            Selection::Reference(_) => "Reference",
        };
        theme::section_label(ui, kind);
        egui::Grid::new("props").num_columns(2).spacing([12.0, 5.0]).show(ui, |ui| {
            for (k, v) in &info.details {
                ui.label(egui::RichText::new(k).size(12.0).color(theme::muted()));
                let color = match k.as_str() {
                    "Error" => theme::error(),
                    "Needs repair" => theme::warn(),
                    _ => theme::ink(),
                };
                ui.add(egui::Label::new(egui::RichText::new(v).size(12.0).color(color)).wrap());
                ui.end_row();
            }
        });
        if sel == Selection::Body {
            ui.add_space(8.0);
            material_panel(ui, state);
            appearance_panel(ui, state, AppearanceTarget::Part, None);
        }
        if let Selection::Face(face) = sel
            && let Some(face_ref) = state.doc.face_ref(face)
        {
            ui.add_space(8.0);
            appearance_panel(ui, state, AppearanceTarget::Face { face: face_ref }, Some(face));
        }
        if let (Selection::Feature(id), Some(f)) = (sel, feature)
            && f.status != FeatureStatus::Inactive
        {
            dimension_field(ui, state, id, &f.kind);
            if let Some(message) = state.mass.message(&state.doc, id) {
                hint(ui, &crate::geometry_message::for_user(message));
            }
        }
        if let Some(FeatureStatus::NeedsRepair(items)) = &status
            && let Some(id) = info.feature
        {
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(theme::warn_soft())
                .stroke(Stroke::new(1.0, theme::warn().gamma_multiply(0.6)))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                        icons::paint(ui.painter(), Icon::Warning, r, theme::warn());
                        ui.label(
                            egui::RichText::new(format!(
                                "{} reference{} could not be found after the rebuild. The feature was skipped.",
                                items.len(),
                                if items.len() == 1 { "" } else { "s" }
                            ))
                            .size(11.5)
                            .color(theme::ink()),
                        );
                    });
                    ui.add_space(4.0);
                    if ui.add(egui::Button::new("Repair…").fill(theme::warn()).stroke(Stroke::NONE)).clicked() {
                        state.push(AppAction::StartRepair(id));
                    }
                });
        }
        ui.add_space(10.0);
        theme::hline(ui);
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| match sel {
            Selection::Feature(id) => {
                if state.doc.feature(id).is_some_and(|f| matches!(f.kind, crate::document::FeatureKind::ImportStep(_)))
                    && ui.button("Replace file...").clicked()
                {
                    state.push(AppAction::ReplaceImport { feature: id, path: None });
                }
                if is_sketch {
                    if ui.button("Edit sketch").clicked() {
                        state.push(AppAction::EditSketch(id));
                    }
                } else if ui.button("Edit feature").clicked() {
                    state.push(AppAction::EditFeature(id));
                }
                if ui.button(if suppressed { "Unsuppress" } else { "Suppress" }).clicked() {
                    state.push(AppAction::Command(Command::Suppress { id, suppressed: !suppressed }));
                }
                if ui.button(egui::RichText::new("Delete").color(theme::error())).clicked() {
                    state.push(AppAction::Command(Command::DeleteFeature(id)));
                }
            }
            Selection::Face(f) => {
                if is_planar_face
                    && ui.button("Sketch on face").clicked()
                    && let Some(r) = state.doc.face_ref(f)
                {
                    state.push(AppAction::StartSketchOn(SketchPlane::Face { face: r }));
                }
                if is_planar_face && ui.button("Normal to").clicked() {
                    state.push(AppAction::NormalTo);
                }
            }
            Selection::Edge(_) => {
                for id in ["fillet", "chamfer"] {
                    let command = crate::commands::get(id).expect("registered finishing command");
                    if ui
                        .add_enabled(command.is_enabled(state), egui::Button::new(command.label))
                        .on_disabled_hover_text(command.tip(state))
                        .clicked()
                        && let Some(action) = command.action_now(state)
                    {
                        state.push(action);
                    }
                }
            }
            Selection::Vertex(_) | Selection::Reference(_) => {}
            Selection::Body => {
                ui.label(
                    egui::RichText::new(
                        "The viewport shows every dimension that builds the body. Double-click one to edit it.",
                    )
                    .size(11.0)
                    .color(theme::muted()),
                );
            }
        });
        let _ = fmt_mm(0.0);
    });
}

fn section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let inner = egui::Frame::new()
        .fill(theme::chip())
        .stroke(Stroke::new(1.0, theme::line().gamma_multiply(0.4)))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(vec2(3.0, 13.0), Sense::hover());
                ui.painter().rect_filled(bar, CornerRadius::same(2), theme::accent());
                ui.label(egui::RichText::new(title).family(theme::semibold()).size(11.5).color(theme::ink()));
            });
            ui.add_space(2.0);
            add(ui)
        })
        .inner;
    ui.add_space(6.0);
    inner
}

fn material_panel(ui: &mut egui::Ui, state: &mut AppState) {
    static CATALOG: std::sync::OnceLock<soliddraft_materials::Catalog> = std::sync::OnceLock::new();
    let catalog = CATALOG.get_or_init(soliddraft_materials::Catalog::built_in);
    let current = state.doc.material();
    let mut selected = state.doc.material_assignment().cloned();
    let before = selected.clone();
    let mut error = None;
    section(ui, "Material", |ui| {
        egui::ComboBox::from_label("Part material")
            .selected_text(current.map_or("Unassigned", |m| m.name.as_str()))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut selected,
                    None,
                    if state.doc.model().active_configuration().is_some() {
                        "Use inherited material"
                    } else {
                        "Unassigned"
                    },
                );
                for (_, material) in catalog.materials() {
                    ui.selectable_value(&mut selected, Some(material.id.clone()), &material.name);
                }
            });
        if let Some(material) = current {
            match material.density_kg_m3 {
                Some(density) => {
                    ui.label(format!("Density: {density} kg/m³"));
                }
                None => {
                    ui.label("Density: unknown");
                }
            }
        }
        if let Some(index) = state.doc.display_index() {
            let id = state.doc.features[index].id;
            if ui.button("Calculate mass").clicked() {
                error = state.mass.request(&state.doc, id, ui.ctx()).err();
            }
            if state.doc.proof_state().current_proved_display()
                && let Some(body) = state.doc.display_body()
                && let Some(properties) = state.doc.kernel().cached_mass_properties(body)
            {
                ui.label(format!("Volume: {:.3} mm³", properties.volume));
                match soliddraft_materials::mass::of_body(&properties, state.doc.material()) {
                    Ok(mass) => {
                        ui.label(format!("Mass: {:.6} kg", mass.mass_kg));
                    }
                    Err(why) => {
                        ui.label(format!("Mass: {why}"));
                    }
                }
            }
            if let Some(message) = state.mass.message(&state.doc, id) {
                hint(ui, &crate::geometry_message::for_user(message));
            }
        }
    });
    if let Some(error) = error {
        state.note(error, crate::state::NoteKind::Error);
    }
    if selected != before {
        let material = selected.as_ref().and_then(|id| catalog.material(id)).map(|(_, m)| m.clone());
        let appearance = material.as_ref().and_then(|m| catalog.default_appearance_of(m)).cloned();
        if let Err(error) = state.doc.set_material(material, appearance) {
            state.note(error, crate::state::NoteKind::Error);
        }
    }
}

/// Colours one click away, with the names their appearances take.
const SWATCHES: [(&str, [u8; 3]); 10] = [
    ("Red", [198, 40, 40]),
    ("Orange", [239, 108, 0]),
    ("Yellow", [249, 190, 37]),
    ("Green", [46, 125, 50]),
    ("Teal", [0, 131, 143]),
    ("Blue", [21, 101, 192]),
    ("Purple", [106, 27, 154]),
    ("Graphite", [55, 61, 69]),
    ("Silver", [176, 182, 190]),
    ("White", [244, 244, 244]),
];

/// How transparent "Transparent" makes an opaque surface, and the most the slider allows,
/// so a part never disappears altogether.
const FIRST_TRANSPARENCY: f32 = 0.6;
const MOST_TRANSPARENCY: f32 = 0.9;

/// `from` in another colour and opacity, keeping its finish. The id names everything
/// that differs from a plain opaque dielectric, so two different looks never share an
/// id in the part's copies.
fn painted(from: &Appearance, colour: Srgb, opacity: f32) -> Appearance {
    let opacity = (opacity.clamp(1.0 - MOST_TRANSPARENCY, 1.0) * 100.0).round() / 100.0;
    let transparency = ((1.0 - opacity) * 100.0).round() as u32;
    let hex = colour.to_string();
    let mut id = format!("custom-{}", hex.trim_start_matches('#'));
    if transparency > 0 {
        id.push_str(&format!("-t{transparency}"));
    }
    if from.metallic != 0.0 || from.roughness != 0.5 {
        id.push_str(&format!("-m{}-r{}", from.metallic, from.roughness));
    }
    if from.emissive != Srgb::BLACK {
        id.push_str(&format!("-e{}", from.emissive.to_string().trim_start_matches('#')));
    }
    let colour_name = SWATCHES
        .iter()
        .find(|(_, c)| Srgb::new(c[0], c[1], c[2]) == colour)
        .map_or(hex.clone(), |(n, _)| (*n).to_owned());
    let name = if transparency > 0 { format!("{colour_name}, {transparency}% transparent") } else { colour_name };
    Appearance { id: AppearanceId(id), name, base_color: colour, opacity, ..from.clone() }
}

/// Applies a colour or transparency still being dragged once the pointer lets go, as one
/// undo step.
fn settle_appearance(ui: &egui::Ui, state: &mut AppState) {
    if ui.ctx().input(|i| i.pointer.any_down()) {
        return;
    }
    let Some((target, appearance)) = state.doc.appearance_preview() else { return };
    let (target, appearance) = (target.clone(), appearance.cloned());
    if let Err(error) = state.doc.set_appearance(target, appearance) {
        state.note(error, crate::state::NoteKind::Error);
    }
}

/// Colour and transparency of the part, or of one face. A swatch, the Transparent box
/// and the reset button apply at once; the colour picker and the slider show their
/// value while dragged and apply it when the pointer lets go.
fn appearance_panel(ui: &mut egui::Ui, state: &mut AppState, target: AppearanceTarget, face: Option<FaceId>) {
    let shown = state.doc.shown_appearance(face);
    let own = state.doc.has_own_appearance(&target);
    let dragging = ui.ctx().input(|i| i.pointer.any_down());
    let mut apply: Option<Option<Appearance>> = None;
    let mut preview: Option<Appearance> = None;
    let mut remove = false;
    let mut changed = |a: Appearance, from_pointer: bool| {
        if from_pointer && dragging {
            preview = Some(a);
        } else {
            apply = Some(Some(a));
        }
    };
    section(ui, "Appearance", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for (name, [r, g, b]) in SWATCHES {
                let colour = Srgb::new(r, g, b);
                let (rect, response) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                let current = shown.base_color == colour;
                let painter = ui.painter();
                painter.rect_filled(rect, CornerRadius::same(4), egui::Color32::from_rgb(r, g, b));
                let edge = if current {
                    Stroke::new(2.0, theme::accent())
                } else if response.hovered() {
                    Stroke::new(1.0, theme::ink())
                } else {
                    Stroke::new(1.0, theme::line())
                };
                painter.rect_stroke(rect, CornerRadius::same(4), edge, egui::StrokeKind::Inside);
                response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, name));
                if response.on_hover_text(name).clicked() {
                    changed(painted(&shown, colour, shown.opacity), false);
                }
            }
            let mut rgb = [shown.base_color.r, shown.base_color.g, shown.base_color.b];
            let picker = egui::color_picker::color_edit_button_srgb(ui, &mut rgb).on_hover_text("Any colour");
            picker.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, true, "Custom colour"));
            if picker.changed() {
                changed(painted(&shown, Srgb::new(rgb[0], rgb[1], rgb[2]), shown.opacity), true);
            }
        });
        ui.add_space(2.0);
        let mut transparent = shown.opacity < 1.0;
        if ui.checkbox(&mut transparent, "Transparent").changed() {
            let opacity = if transparent { 1.0 - FIRST_TRANSPARENCY } else { 1.0 };
            changed(painted(&shown, shown.base_color, opacity), false);
        }
        if transparent {
            let mut percent = ((1.0 - shown.opacity) * 100.0).round();
            let slider = ui.add(
                egui::Slider::new(&mut percent, 10.0..=MOST_TRANSPARENCY * 100.0)
                    .step_by(1.0)
                    .fixed_decimals(0)
                    .suffix("%")
                    .text("Transparency"),
            );
            if slider.changed() {
                changed(painted(&shown, shown.base_color, 1.0 - percent / 100.0), true);
            }
        }
        if own {
            let reset = match target {
                AppearanceTarget::Face { .. } => "Use the part's appearance",
                _ => "Remove colour",
            };
            remove = ui.button(reset).clicked();
        }
    });
    if remove {
        apply = Some(None);
    }
    if let Some(appearance) = apply {
        if let Err(error) = state.doc.set_appearance(target, appearance) {
            state.note(error, crate::state::NoteKind::Error);
        }
    } else if let Some(appearance) = preview {
        state.doc.preview_appearance(target, Some(appearance));
    }
}
