//! PropertyManager: the open sketch, the feature editor while a feature is
//! being created or edited, otherwise the selected entity's properties.

use crate::document::{
    Command, EndKind, FeatureId, FeatureKind, FeatureStatus, PlaneKind, ProfileKind, Reference, Selection, SketchPlane,
    fmt_mm, plane_ref_label,
};
use crate::drag::Dimension;
use crate::icons::{self, Icon};
use crate::sketch::{ConstraintKind, EntityKind, fmt_value, solve};
use crate::sketcher;
use crate::state::{AppAction, AppState, DragStep, EditorParams, ExitInto, RelationFilter, SketchSel, Tool};
use crate::theme;
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, pos2, vec2};
use soliddraft_kernel_api::{HoleDepth, HoleHead, HoleTip, Vec3};
use soliddraft_model::{DatumPlane, PatternSpec, PlaneRef};

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if state.sketch.is_some() {
        sketch_panel(ui, state);
        crate::sketch_values::popup(ui, state);
    } else if state.editor.is_some() {
        editor(ui, state);
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

/// OK and Cancel in the header; returns (ok, cancel).
fn ok_cancel(ui: &mut egui::Ui, head: Rect, ok_tip: &str, cancel_tip: &str) -> (bool, bool) {
    let ok = small_button(
        ui,
        Rect::from_center_size(pos2(head.right() - 42.0, head.center().y), vec2(22.0, 22.0)),
        Icon::Check,
        ok_tip,
        true,
    );
    let cancel = small_button(
        ui,
        Rect::from_center_size(pos2(head.right() - 16.0, head.center().y), vec2(22.0, 22.0)),
        Icon::Close,
        cancel_tip,
        false,
    );
    (ok.clicked(), cancel.clicked())
}

// ---------------------------------------------------------------------------
// The feature form: a header, cards of sections, and the numbers in one table.

/// The width of a form table's caption column.
const LABEL_WIDTH: f32 = 96.0;
/// The height of one row of a form table.
const ROW_HEIGHT: f32 = 24.0;
/// The room a value cell keeps at its right for the unit.
const UNIT_WIDTH: f32 = 30.0;
/// The space between the cards of a form.
const CARD_GAP: f32 = 6.0;
/// The height of a pick's chip, inside its margin.
const CHIP_HEIGHT: f32 = 18.0;
/// How many picks a list shows before it scrolls.
const LIST_ROWS: usize = 4;

/// The feature editor's header: the feature's icon on an accent tile, then its title.
fn feature_header(ui: &mut egui::Ui, icon: Icon, title: &str) -> Rect {
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(head, CornerRadius::ZERO, theme::panel_header());
    let tile = Rect::from_center_size(pos2(head.left() + 22.0, head.center().y), vec2(24.0, 24.0));
    painter.rect_filled(tile, CornerRadius::same(7), theme::accent());
    icons::paint(painter, icon, tile.shrink(5.0), theme::on_accent());
    painter.text(
        pos2(tile.right() + 10.0, head.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::font_semibold(13.5),
        theme::ink(),
    );
    painter.hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, theme::line()));
    head
}

/// OK and Cancel at the right of the feature header; returns (ok, cancel). Without
/// `ok`, only Cancel shows.
fn header_buttons(ui: &mut egui::Ui, head: Rect, ok: bool) -> (bool, bool) {
    let size = vec2(24.0, 24.0);
    let cancel = small_button(
        ui,
        Rect::from_center_size(pos2(head.right() - 20.0, head.center().y), size),
        Icon::Close,
        "Cancel (Esc)",
        false,
    );
    let ok = ok
        && small_button(
            ui,
            Rect::from_center_size(pos2(head.right() - 49.0, head.center().y), size),
            Icon::Check,
            "OK (Enter)",
            true,
        )
        .clicked();
    (ok, cancel.clicked())
}

/// A card of the form: a titled section on the raised surface. The title keeps its
/// exact text, so automation and the acceptance tests can find the section by it.
fn section<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    card(ui, title, None, add)
}

/// A card for options the kernel cannot build yet: its title carries a Coming soon
/// badge, and everything in it is drawn disabled, with `tip` saying what it waits for.
fn soon_section(ui: &mut egui::Ui, title: &str, tip: &str, add: impl FnOnce(&mut egui::Ui)) {
    card(ui, title, Some(tip), |ui| {
        ui.add_enabled_ui(false, add).response.on_disabled_hover_text(tip);
    });
}

fn card<R>(ui: &mut egui::Ui, title: &str, soon: Option<&str>, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
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
                if let Some(tip) = soon {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        soon_badge(ui).on_hover_text(tip);
                    });
                }
            });
            ui.add_space(2.0);
            add(ui)
        })
        .inner;
    ui.add_space(CARD_GAP);
    inner
}

/// A small muted "Coming soon" pill.
fn soon_badge(ui: &mut egui::Ui) -> egui::Response {
    egui::Frame::new()
        .fill(theme::hover())
        .corner_radius(CornerRadius::same(5))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| ui.label(egui::RichText::new(COMING_SOON).size(10.5).color(theme::muted())))
        .response
}

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

/// A framed value cell: the widget fills it, the unit sits at its right. The frame
/// follows the widget: the accent while it has the keyboard, a stronger line on hover.
fn value_cell(
    ui: &mut egui::Ui,
    cell: Rect,
    unit: &str,
    add: impl FnOnce(&mut egui::Ui, Rect) -> egui::Response,
) -> egui::Response {
    let frame = ui.painter().add(egui::Shape::Noop);
    // Every value keeps the unit's room, with a unit or without, so the column stays one width.
    let inner = Rect::from_min_max(
        pos2(cell.left() + 2.0, cell.top() + 1.0),
        pos2(cell.right() - UNIT_WIDTH, cell.bottom() - 1.0),
    );
    let response = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
                w.bg_fill = egui::Color32::TRANSPARENT;
                w.weak_bg_fill = egui::Color32::TRANSPARENT;
                w.bg_stroke = Stroke::NONE;
                w.expansion = 0.0;
            }
            v.selection.stroke = Stroke::NONE;
            ui.spacing_mut().interact_size = inner.size();
            ui.spacing_mut().button_padding = vec2(6.0, 2.0);
            add(ui, inner)
        })
        .inner;
    let (fill, stroke) = if response.has_focus() {
        (theme::chip(), Stroke::new(1.5, theme::accent()))
    } else if response.hovered() {
        (theme::field_bg(), Stroke::new(1.0, theme::line_strong()))
    } else {
        (theme::field_bg(), Stroke::new(1.0, theme::line()))
    };
    ui.painter()
        .set(frame, egui::epaint::RectShape::new(cell, CornerRadius::same(6), fill, stroke, egui::StrokeKind::Inside));
    if !unit.is_empty() {
        ui.painter().text(
            pos2(cell.right() - 8.0, cell.center().y),
            Align2::RIGHT_CENTER,
            unit,
            theme::font(11.0),
            theme::muted(),
        );
    }
    response
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

/// A fillet radius or chamfer distance, typed as text. Each edit that reads as a
/// positive number of millimetres is the new size at once; anything else is refused
/// beside the field and keeps the text, so nothing is silently clamped to another size.
fn size_field(ui: &mut egui::Ui, label: &str, value: &mut f64, refused: &mut Option<String>) {
    let what = label.to_lowercase();
    let id = ui.make_persistent_id(("size", label));
    // The typed text outlives the frame; it restarts from the size when it no longer
    // stands for it.
    let kept = ui.data(|d| d.get_temp::<String>(id));
    let mut text = kept
        .filter(|t| refused.is_some() || crate::fillet_collect::parse_size(t, &what) == Ok(*value))
        .unwrap_or_else(|| crate::document::fmt_mm(*value));
    let r = row(ui, label, |ui, cell| {
        value_cell(ui, cell, "mm", |ui, inner| {
            ui.put(
                inner,
                egui::TextEdit::singleline(&mut text)
                    .font(theme::font(12.5))
                    .frame(egui::Frame::NONE)
                    .horizontal_align(egui::Align::Center)
                    .vertical_align(egui::Align::Center),
            )
        })
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
    if let Some(why) = refused {
        error_row(ui, why);
    }
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

/// The end condition of an extrude or cut. Through all is offered to cuts only, and
/// the up-to conditions need a body to stop against.
fn end_combo(ui: &mut egui::Ui, id: &str, end: &mut EndKind, cut: bool, up_to: bool) {
    choice_row(ui, "End", |ui, width| {
        egui::ComboBox::from_id_salt(id).width(width).selected_text(end.label()).show_ui(ui, |ui| {
            let mut options = vec![EndKind::Blind];
            if cut {
                options.push(EndKind::ThroughAll);
            }
            if up_to {
                options.push(EndKind::UpToNext);
                options.push(EndKind::UpToFace(None));
            }
            for option in options {
                let same = std::mem::discriminant(&option) == std::mem::discriminant(end);
                if ui.selectable_label(same, option.label()).clicked() && !same {
                    *end = option;
                }
            }
        });
    });
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
fn pick_list<'a>(ui: &mut egui::Ui, prompt: &str, icon: Icon, picks: impl Iterator<Item = &'a str>) -> Option<usize> {
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

fn picked_face(ui: &mut egui::Ui, face: Option<&crate::document::FaceRef>) {
    match face {
        Some(f) => {
            picked_chip(ui, Icon::Plane, &f.label, false);
        }
        None => pick_prompt(ui, "Click the face to stop at"),
    }
}

/// A count badge beside a section's picks.
fn count_note(ui: &mut egui::Ui, count: usize, one: &str, many: &str) {
    if count > 0 {
        ui.label(
            egui::RichText::new(format!("{count} {}", if count == 1 { one } else { many }))
                .size(11.0)
                .color(theme::muted()),
        );
    }
}

/// A row of mutually exclusive choices as one segmented control. Returns the index
/// clicked, if any. Each segment reports itself selected or not, like a selectable label.
fn segmented(ui: &mut egui::Ui, id: &str, options: &[&str], selected: usize) -> Option<usize> {
    segmented_gated(ui, id, options, selected, &[])
}

/// A segmented control in which some segments wait for the kernel: each `(index, tip)`
/// of `soon` is drawn muted, takes no click, and says on hover what it waits for.
fn segmented_gated(
    ui: &mut egui::Ui,
    id: &str,
    options: &[&str],
    selected: usize,
    soon: &[(usize, &str)],
) -> Option<usize> {
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
        let waits = soon.iter().find(|(at, _)| *at == i).map(|(_, tip)| *tip);
        let r =
            ui.interact(segment, ui.id().with((id, i)), if waits.is_some() { Sense::hover() } else { Sense::click() });
        let on = i == selected;
        let enabled = ui.is_enabled() && waits.is_none();
        r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, enabled, on, *text));
        let r = match waits {
            Some(tip) => r.on_hover_text(format!("{COMING_SOON}: {tip}")),
            None => r,
        };
        let (fill, ink) = if on {
            (theme::accent(), theme::on_accent())
        } else if waits.is_some() {
            (egui::Color32::TRANSPARENT, theme::muted().gamma_multiply(0.6))
        } else if r.hovered() {
            (theme::hover(), theme::ink())
        } else {
            (egui::Color32::TRANSPARENT, theme::ink())
        };
        ui.painter().rect_filled(segment, CornerRadius::same(5), fill);
        let font = if on { theme::font_semibold(12.0) } else { theme::font(12.0) };
        ui.painter().text(segment.center(), Align2::CENTER_CENTER, *text, font, ink);
        if r.clicked() && waits.is_none() {
            clicked = Some(i);
        }
    }
    ui.add_space(4.0);
    clicked
}

/// What the gated controls say they are.
const COMING_SOON: &str = "Coming soon";

/// Fillet options the form shows before Catallax builds them. Each stays disabled and
/// marked Coming soon until its Catallax construction is merged and pinned; switching
/// one on means dropping it from here once the document can carry it.
#[derive(Clone, Copy)]
enum FilletSoon {
    /// A start and an end radius per edge, varying linearly between them.
    VariableRadius,
    /// The stations where a run stops when it keeps its neighbours.
    Setbacks,
    /// Keeping every neighbouring face whole instead of rolling over it.
    KeepNeighbours,
    /// Rounded transitions between the fillets of a mitred corner.
    Corners,
}

impl FilletSoon {
    /// What the option waits for, shown on hover.
    fn tip(self) -> &'static str {
        match self {
            FilletSoon::VariableRadius => {
                "a start and end radius for each edge, varying linearly from the end you pick. Waits for \
                 Catallax to build variable-radius fillets."
            }
            FilletSoon::Setbacks => {
                "the stations where the fillet stops short of an edge's end, for Keep neighbours. Waits for \
                 Catallax to build fillets that keep their neighbours."
            }
            FilletSoon::KeepNeighbours => {
                "keep every neighbouring face whole, and stop at setback stations instead of rolling over. \
                 Waits for Catallax to build fillets that keep their neighbours."
            }
            FilletSoon::Corners => {
                "rounded transitions where the fillets of a corner meet. Waits for Catallax corner blends."
            }
        }
    }
}

fn coord(ui: &mut egui::Ui, id: &str, label: &str, value: &mut f64) {
    ui.label(egui::RichText::new(label).size(11.0).color(theme::muted()));
    let r = ui.add_sized(
        vec2(64.0, 22.0),
        egui::DragValue::new(value).speed(0.5).range(-100_000.0..=100_000.0).fixed_decimals(2),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, format!("{id} value")));
}

fn count_field(ui: &mut egui::Ui, count: &mut u32) {
    number_row(ui, "Copies", "", egui::DragValue::new(count).range(2..=1000));
}

/// An angle with standard choices and a free value.
fn standard_angle(ui: &mut egui::Ui, id: &str, label: &str, angle: &mut f64, choices: &[(f64, &str)]) {
    let text = choices.iter().find(|(v, _)| v == angle).map_or(format!("{}°", fmt_mm(*angle)), |(_, t)| (*t).into());
    choice_row(ui, label, |ui, width| {
        egui::ComboBox::from_id_salt(id).width(width).selected_text(text).show_ui(ui, |ui| {
            for (value, text) in choices {
                ui.selectable_value(angle, *value, *text);
            }
        });
    });
    if *angle != 0.0 {
        number_row_named(
            ui,
            "",
            &format!("{label} angle"),
            "°",
            egui::DragValue::new(angle).speed(0.5).range(1.0..=179.0),
        );
    }
}

/// One of the world axes as a direction.
fn axis_combo(ui: &mut egui::Ui, id: &str, label: &str, direction: &mut Vec3) {
    let axes = [(Vec3::X, "X"), (Vec3::Y, "Y"), (Vec3::Z, "Z"), (-Vec3::X, "−X"), (-Vec3::Y, "−Y"), (-Vec3::Z, "−Z")];
    let text = axes.iter().find(|(v, _)| v == direction).map_or("Custom", |(_, t)| t);
    choice_row(ui, label, |ui, width| {
        egui::ComboBox::from_id_salt(id).width(width).selected_text(text).show_ui(ui, |ui| {
            for (value, text) in axes {
                ui.selectable_value(direction, value, text);
            }
        });
    });
}

/// A reference plane choice; a picked face shows by its label.
fn plane_combo(ui: &mut egui::Ui, id: &str, plane: &mut PlaneRef, choices: &[(soliddraft_model::FeatureId, String)]) {
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

/// The closing note of a feature form: what the preview shows and how to finish.
fn form_note(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        ui.painter().circle_stroke(r.center(), 6.0, Stroke::new(1.0, theme::muted()));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, "i", theme::font_semibold(9.5), theme::muted());
        ui.add(egui::Label::new(egui::RichText::new(text).size(11.0).color(theme::muted())).wrap());
    });
}

/// A refusal, on its own tinted strip beside the control it is about.
fn error_row(ui: &mut egui::Ui, err: &str) {
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(theme::error_soft())
        .stroke(Stroke::new(1.0, theme::error().gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                icons::paint(ui.painter(), Icon::Warning, r, theme::error());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(crate::geometry_message::for_user(err)).size(11.5).color(theme::error()),
                    )
                    .wrap(),
                );
            });
        });
}

/// The fillet's setback and option cards, in the SolidWorks fillet panel's places. The
/// overflow choice is Catallax's: roll over, which it builds, or keep neighbours with
/// setback stations, which waits, so the stations' card is drawn disabled. A fillet
/// made by a tool with stations lists them there.
fn fillet_option_cards(ui: &mut egui::Ui, fillet: &mut crate::document::FilletParams) {
    use soliddraft_model::FilletOverflow;
    let tip = |o: FilletSoon| format!("{COMING_SOON}: {}", o.tip());
    soon_section(ui, "Setback parameters", &tip(FilletSoon::Setbacks), |ui| {
        if let Some(FilletOverflow::KeepNeighbours(stations)) = &fillet.overflow {
            // The distance and the end come first, so a narrow panel cuts only the face's name short.
            for s in stations {
                let edge = if fillet.edges.len() > 1 { format!("Edge {}: ", s.edge + 1) } else { String::new() };
                picked_chip(ui, Icon::Axis, &format!("{edge}{} mm from {}", fmt_mm(s.setback), s.at.label), false);
            }
        }
        pick_prompt(ui, "Click the end of an edge to stop short of");
        let mut none = String::new();
        let r = row(ui, "Distance", |ui, cell| {
            value_cell(ui, cell, "mm", |ui, inner| {
                ui.put(inner, egui::TextEdit::singleline(&mut none).hint_text("-").frame(egui::Frame::NONE))
            })
        });
        r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, false, "Setback distance value"));
    });
    section(ui, "Fillet options", |ui| {
        ui.label(egui::RichText::new("Overflow").size(12.0).color(theme::ink()));
        let keeps = matches!(fillet.overflow, Some(FilletOverflow::KeepNeighbours(_)));
        let options = ["Roll over", "Keep neighbours"];
        if segmented_gated(ui, "fillet_overflow", &options, keeps as usize, &[(1, FilletSoon::KeepNeighbours.tip())])
            == Some(0)
            && keeps
        {
            fillet.overflow = Some(FilletOverflow::RollOver);
        }
        ui.add_enabled(false, egui::Checkbox::new(&mut false, "Round corners"))
            .on_disabled_hover_text(tip(FilletSoon::Corners));
    });
}

/// A field parameter: one value, or a linear grade between two anchors on a part axis.
/// `unit` names the values, such as "mm"; the anchors are always millimetres.
fn grade_field(ui: &mut egui::Ui, label: &str, grade: &mut soliddraft_kernel_api::Grade, unit: &str, speed: f64) {
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
        Grade::Constant(v) => value(ui, "", v, unit),
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

fn editor(ui: &mut egui::Ui, state: &mut AppState) {
    let (title, icon) = match state.editor.as_ref().map(|e| &e.params) {
        Some(EditorParams::Revolve(_)) => (state.editor.as_ref().unwrap().title(), Icon::Revolve),
        Some(EditorParams::Sweep(_)) => (state.editor.as_ref().unwrap().title(), Icon::Sweep),
        Some(EditorParams::Extrude(_)) => (state.editor.as_ref().unwrap().title(), Icon::Extrude),
        Some(EditorParams::Cut(_)) => (state.editor.as_ref().unwrap().title(), Icon::Cut),
        Some(EditorParams::Fillet(_)) => (state.editor.as_ref().unwrap().title(), Icon::Fillet),
        Some(EditorParams::Chamfer(_)) => (state.editor.as_ref().unwrap().title(), Icon::Chamfer),
        Some(EditorParams::Hole(_)) => (state.editor.as_ref().unwrap().title(), Icon::Hole),
        Some(EditorParams::Pattern(p)) => (
            state.editor.as_ref().unwrap().title(),
            if matches!(p.layout, PatternSpec::Linear { .. }) { Icon::LinearPattern } else { Icon::CircularPattern },
        ),
        Some(EditorParams::ReferencePlane(_)) => (state.editor.as_ref().unwrap().title(), Icon::Plane),
        Some(EditorParams::Mirror(_)) => (state.editor.as_ref().unwrap().title(), Icon::Mirror),
        Some(EditorParams::Draft(_)) => (state.editor.as_ref().unwrap().title(), Icon::Draft),
        Some(EditorParams::Lattice(_) | EditorParams::Gyroid(_)) => {
            (state.editor.as_ref().unwrap().title(), Icon::Cad("lattice"))
        }
        Some(EditorParams::Shell(_)) => (state.editor.as_ref().unwrap().title(), Icon::Shell),
        Some(EditorParams::PickPlane) => ("Sketch".to_owned(), Icon::Sketch),
        Some(EditorParams::Repair { .. }) => ("Repair references".to_owned(), Icon::Repair),
        None => return,
    };
    let head = feature_header(ui, icon, &title);
    let pick_plane = matches!(state.editor.as_ref().map(|e| &e.params), Some(EditorParams::PickPlane));
    let (ok, cancel) = header_buttons(ui, head, !pick_plane);
    if ok {
        state.push(AppAction::CommitEditor);
    }
    if cancel {
        state.push(AppAction::CancelEditor);
    }

    let sketch_names: Vec<(crate::document::FeatureId, String)> =
        state.doc.features.iter().filter(|f| f.is_sketch()).map(|f| (f.id, f.name.clone())).collect();
    let sketch_name = |id: crate::document::FeatureId, missing: &str| {
        sketch_names.iter().find(|(i, _)| *i == id).map_or_else(|| missing.to_owned(), |(_, n)| n.clone())
    };
    let target = state
        .editor
        .as_ref()
        .and_then(|e| e.target)
        .and_then(|id| state.doc.index_of(id))
        .unwrap_or(state.doc.features.len());
    let plane_choices: Vec<_> = state
        .doc
        .features
        .iter()
        .take(target)
        .filter(|f| matches!(f.kind, crate::document::FeatureKind::ReferencePlane(_)))
        .map(|f| (soliddraft_model::FeatureId(f.id as u32), f.name.clone()))
        .collect();
    let sweep_radius = match state.editor.as_ref().map(|e| &e.params) {
        Some(EditorParams::Sweep(s)) => state.doc.sweep_anchor(s.sketch.0 as u64).map_or(10.0, |(_, radius)| radius),
        _ => 0.0,
    };
    let mut actions = Vec::new();
    let Some(editor) = state.editor.as_mut() else { return };
    egui::ScrollArea::vertical().id_salt("pm").auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 8)).show(ui, |ui| match &mut editor.params {
            EditorParams::Sweep(s) => {
                section(ui, "Polygon profile", |ui| {
                    picked_chip(ui, Icon::Sketch, &sketch_name(s.sketch.0 as u64, "Missing sketch"), false);
                });
                section(ui, "Path", |ui| crate::sweep_panel::page(ui, &mut s.path, sweep_radius, coord));
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
                form_note(ui, "The profile stays where it was sketched, so start the path on it.");
            }
            EditorParams::Revolve(r) => {
                section(ui, "Meridian sketch", |ui| {
                    picked_chip(ui, Icon::Sketch, &sketch_name(r.sketch.0 as u64, "Missing sketch"), false);
                });
                section(ui, "Axis", |ui| {
                    for (caption, values, id) in
                        [("Point", &mut r.axis_origin, "origin"), ("Direction", &mut r.axis_direction, "direction")]
                    {
                        ui.label(egui::RichText::new(caption).size(12.0).color(theme::ink()));
                        ui.horizontal(|ui| {
                            for (i, label) in ["X", "Y", "Z"].iter().enumerate() {
                                coord(ui, &format!("revolve {id} {label}"), label, &mut values[i]);
                            }
                        });
                    }
                });
                section(ui, "Angle", |ui| {
                    number_row(ui, "Angle", "°", egui::DragValue::new(&mut r.angle_deg).speed(1.0).range(0.0..=360.0));
                });
            }
            EditorParams::Extrude(params) => {
                section(ui, "Profile", |ui| match params.sketch {
                    Some(id) => {
                        picked_chip(ui, Icon::Sketch, &sketch_name(id, "Sketch"), false);
                    }
                    None => {
                        let is_rect = matches!(params.profile, ProfileKind::Rectangle { .. });
                        match segmented(ui, "extrude_profile", &["Rectangle", "Circle"], (!is_rect) as usize) {
                            Some(0) if !is_rect => {
                                params.profile = ProfileKind::Rectangle { width: 60.0, height: 40.0 }
                            }
                            Some(1) if is_rect => params.profile = ProfileKind::Circle { diameter: 24.0 },
                            _ => {}
                        }
                        match &mut params.profile {
                            ProfileKind::Rectangle { width, height } => {
                                mm_field(ui, "Width", width);
                                mm_field(ui, "Height", height);
                            }
                            ProfileKind::Circle { diameter } => {
                                mm_field(ui, "Diameter", diameter);
                            }
                        }
                        choice_row(ui, "Plane", |ui, width| {
                            egui::ComboBox::from_id_salt("plane")
                                .width(width)
                                .selected_text(state.doc.model().datum_name(params.plane.to_model()))
                                .show_ui(ui, |ui| {
                                    for p in PlaneKind::ALL {
                                        ui.selectable_value(
                                            &mut params.plane,
                                            p,
                                            state.doc.model().datum_name(p.to_model()),
                                        );
                                    }
                                });
                        });
                    }
                });
                section(ui, "Direction 1", |ui| {
                    let mut end = params.end.clone();
                    end_combo(ui, "extrude_end", &mut end, false, params.sketch.is_some());
                    params.end = end;
                    match &params.end {
                        EndKind::Blind => {
                            mm_field(ui, "Depth", &mut params.depth);
                        }
                        EndKind::UpToFace(face) => picked_face(ui, face.as_ref()),
                        _ => {}
                    }
                    ui.checkbox(&mut params.flip, "Flip direction");
                });
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
                form_note(ui, "The ghost in the viewport previews the added material. Enter applies, Esc cancels.");
            }
            EditorParams::Cut(params) => {
                section(ui, "Profile", |ui| {
                    picked_chip(ui, Icon::Sketch, &sketch_name(params.sketch, "Sketch"), false);
                });
                section(ui, "Direction 1", |ui| {
                    let mut end = params.end();
                    end_combo(ui, "cut_end", &mut end, true, true);
                    if end != params.end() {
                        params.set_end(end);
                    }
                    match params.end() {
                        EndKind::Blind => {
                            mm_field(ui, "Depth", &mut params.depth);
                        }
                        EndKind::UpToFace(face) => picked_face(ui, face.as_ref()),
                        _ => {}
                    }
                    ui.checkbox(&mut params.flip, "Flip direction");
                });
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
                form_note(ui, "The material to remove shows in red; the body previews the result.");
            }
            EditorParams::Fillet(_) | EditorParams::Chamfer(_) => {
                let chamfer = matches!(editor.params, EditorParams::Chamfer(_));
                let edges = editor.edges().to_vec();
                let remove = section(ui, "Items to round", |ui| {
                    count_note(ui, edges.len(), "edge", "edges");
                    let remove = pick_list(
                        ui,
                        "Click edges or faces in the viewport",
                        Icon::Axis,
                        edges.iter().map(|e| e.label.as_str()),
                    );
                    if let Some(why) = &editor.feedback.pick {
                        error_row(ui, why);
                    }
                    remove
                });
                if let Some(i) = remove {
                    editor.remove_edge(i);
                }
                section(ui, if chamfer { "Chamfer parameters" } else { "Fillet parameters" }, |ui| {
                    let refused = &mut editor.feedback.size;
                    match &mut editor.params {
                        EditorParams::Fillet(f) => {
                            // A part saved with radius laws opens on Variable; Constant drops them.
                            let variable = !f.laws.is_empty();
                            let choice = segmented_gated(
                                ui,
                                "fillet_radius_kind",
                                &["Constant", "Variable"],
                                variable as usize,
                                &[(1, FilletSoon::VariableRadius.tip())],
                            );
                            if choice == Some(0) {
                                f.laws.clear();
                            }
                            size_field(ui, "Radius", &mut f.radius, refused);
                            if !f.laws.is_empty() {
                                for law in &f.laws {
                                    let soliddraft_kernel_api::RadiusLaw::Linear { start, end } = law.radius;
                                    let edge = if f.edges.len() > 1 {
                                        format!("Edge {}: ", law.edge + 1)
                                    } else {
                                        String::new()
                                    };
                                    let text = format!(
                                        "{edge}R{} to R{} mm from {}",
                                        fmt_mm(start),
                                        fmt_mm(end),
                                        law.from.label
                                    );
                                    picked_chip(ui, Icon::Axis, &text, false);
                                }
                                if f.laws.len() < f.edges.len() {
                                    hint(ui, "Edges without a law keep the radius above.");
                                }
                            }
                        }
                        EditorParams::Chamfer(c) => size_field(ui, "Distance", &mut c.distance, refused),
                        _ => {}
                    }
                    if let Some(err) = &editor.error {
                        error_row(ui, err);
                    }
                });
                if let EditorParams::Fillet(f) = &mut editor.params {
                    fillet_option_cards(ui, f);
                }
                form_note(
                    ui,
                    "A face adds every edge on its boundary. Edges stay picked across a rebuild: each one is \
                     named by the feature that made it.",
                );
            }
            EditorParams::Hole(h) => {
                section(ui, "Placement", |ui| {
                    match &h.face {
                        Some(face) => {
                            picked_chip(ui, Icon::Plane, &face.label, false);
                        }
                        None => pick_prompt(ui, "Click the planar face the holes start on"),
                    }
                    hint(ui, "Centres in the face's sketch frame, mm.");
                    let mut remove = None;
                    for (i, p) in h.points.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}", i + 1)).size(12.0).color(theme::muted()));
                            coord(ui, &format!("hole_u{i}"), "u", &mut p.u);
                            coord(ui, &format!("hole_v{i}"), "v", &mut p.v);
                            if ui.add(egui::Button::new("×").small().frame(false)).on_hover_text("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove.filter(|_| h.points.len() > 1) {
                        h.points.remove(i);
                    }
                    if ui.button("Add point").clicked() {
                        let next = h.points.last().copied().unwrap_or_default();
                        h.points.push(crate::sketch::P2::new(next.u + 10.0, next.v));
                    }
                });
                section(ui, "Hole", |ui| {
                    mm_field(ui, "Diameter", &mut h.diameter);
                    let through = matches!(h.depth, HoleDepth::Through);
                    match segmented(ui, "hole_depth", &["Through", "Blind"], (!through) as usize) {
                        Some(0) => h.depth = HoleDepth::Through,
                        Some(1) if through => h.depth = HoleDepth::Blind { depth: 10.0 },
                        _ => {}
                    }
                    if let HoleDepth::Blind { depth } = &mut h.depth {
                        mm_field(ui, "Depth", depth);
                        let mut angle = match h.tip {
                            HoleTip::Flat => 0.0,
                            HoleTip::Cone { angle_deg } => angle_deg,
                        };
                        standard_angle(
                            ui,
                            "hole_tip",
                            "Tip",
                            &mut angle,
                            &[(0.0, "Flat"), (118.0, "118°"), (90.0, "90°")],
                        );
                        h.tip = if angle == 0.0 { HoleTip::Flat } else { HoleTip::Cone { angle_deg: angle } };
                    }
                });
                section(ui, "Head", |ui| {
                    let kind = match h.head {
                        None => 0,
                        Some(HoleHead::Counterbore { .. }) => 1,
                        Some(HoleHead::Countersink { .. }) => 2,
                    };
                    match segmented(ui, "hole_head", &["None", "Counterbore", "Countersink"], kind) {
                        Some(0) => h.head = None,
                        Some(1) if kind != 1 => {
                            h.head = Some(HoleHead::Counterbore { diameter: h.diameter * 1.8, depth: h.diameter })
                        }
                        Some(2) if kind != 2 => {
                            h.head = Some(HoleHead::Countersink { diameter: h.diameter * 2.0, angle_deg: 90.0 })
                        }
                        _ => {}
                    }
                    match &mut h.head {
                        Some(HoleHead::Counterbore { diameter, depth }) => {
                            mm_field(ui, "Diameter", diameter);
                            mm_field(ui, "Depth", depth);
                        }
                        Some(HoleHead::Countersink { diameter, angle_deg }) => {
                            mm_field(ui, "Diameter", diameter);
                            standard_angle(
                                ui,
                                "hole_sink",
                                "Angle",
                                angle_deg,
                                &[(82.0, "82°"), (90.0, "90°"), (100.0, "100°"), (120.0, "120°")],
                            );
                        }
                        None => {}
                    }
                });
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
                form_note(ui, "Standard angles: 118° drill tips; 82°, 90° and 100° countersinks.");
            }
            EditorParams::Pattern(p) => {
                section(ui, "Layout", |ui| match &mut p.layout {
                    PatternSpec::Linear { direction, spacing, count } => {
                        axis_combo(ui, "pattern_dir", "Direction", direction);
                        mm_field(ui, "Spacing", spacing);
                        count_field(ui, count);
                    }
                    PatternSpec::Circular { axis_direction, count, angle, .. } => {
                        axis_combo(ui, "pattern_axis", "Axis", axis_direction);
                        count_field(ui, count);
                        unit_field(ui, "Angle", angle, "°");
                    }
                });
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
                form_note(ui, "Copies the part's body and unites the copies with it.");
            }
            EditorParams::ReferencePlane(source) => {
                section(ui, "Source plane", |ui| {
                    let mut plane = match source {
                        soliddraft_model::SketchPlane::Datum { datum, .. } => {
                            PlaneRef::Datum { datum: *datum, offset: 0.0 }
                        }
                        soliddraft_model::SketchPlane::Feature { feature, .. } => {
                            PlaneRef::Feature { feature: *feature }
                        }
                        soliddraft_model::SketchPlane::Face { face }
                        | soliddraft_model::SketchPlane::FaceOffset { face, .. } => {
                            PlaneRef::Face { face: face.clone() }
                        }
                    };
                    let mut offset = source.offset();
                    plane_combo(ui, "reference_plane_source", &mut plane, &plane_choices);
                    ui.add_space(4.0);
                    number_row(
                        ui,
                        "Offset",
                        "mm",
                        egui::DragValue::new(&mut offset).speed(0.5).range(-100_000.0..=100_000.0),
                    );
                    *source = match plane {
                        PlaneRef::Datum { datum, .. } => soliddraft_model::SketchPlane::Datum { datum, offset },
                        PlaneRef::Feature { feature } => soliddraft_model::SketchPlane::Feature { feature, offset },
                        PlaneRef::Face { face } => soliddraft_model::SketchPlane::FaceOffset { face, offset },
                    };
                });
                form_note(
                    ui,
                    "Choose a plane in the tree or click a planar face. A negative offset moves in the opposite \
                     direction.",
                );
            }
            EditorParams::Mirror(m) => {
                section(ui, "Mirror plane", |ui| {
                    plane_combo(ui, "mirror_plane", &mut m.plane, &plane_choices);
                    hint(ui, "Or click a planar face in the viewport.");
                    ui.checkbox(&mut m.keep_original, "Keep the original");
                });
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
            }
            EditorParams::Lattice(l) => {
                section(ui, "Cubic lattice", |ui| {
                    hint(ui, "Grid at the part origin. Unproved preview; export certifies the mesh.");
                    let v = l.cell_size_mm.get_or_insert(0.0);
                    number_row(ui, "Cell size", "mm", egui::DragValue::new(v).speed(0.1));
                    let radius = l.radius_mm.get_or_insert(soliddraft_kernel_api::Grade::Constant(0.0));
                    grade_field(ui, "Beam radius (mm), controls density", radius, "mm", 0.1);
                    for (label, v) in [("Accuracy", &mut l.accuracy_mm), ("Minimum detail", &mut l.min_detail_mm)] {
                        number_row(ui, label, "mm", egui::DragValue::new(v).speed(0.01));
                    }
                });
                l.cell = Some(soliddraft_kernel_api::LatticeCell::Cubic);
                ui.label("unproved preview");
                if let Some(error) = &editor.error {
                    error_row(ui, error);
                }
            }
            EditorParams::Gyroid(g) => {
                section(ui, "Network gyroid", |ui| {
                    hint(ui, "Fixed to the part origin. Unproved preview; export certifies the mesh.");
                    number_row(ui, "Period", "mm", egui::DragValue::new(&mut g.period_mm).speed(0.1));
                    grade_field(ui, "Network level (dimensionless)", &mut g.level, "", 0.01);
                    for (label, v) in [("Accuracy", &mut g.accuracy_mm), ("Minimum detail", &mut g.min_detail_mm)] {
                        number_row(ui, label, "mm", egui::DragValue::new(v).speed(0.01));
                    }
                });
                ui.label("unproved preview");
                if let Some(error) = &editor.error {
                    error_row(ui, error);
                }
            }
            EditorParams::Shell(s) => {
                section(ui, "Open faces", |ui| {
                    let mut every_face = s.open_faces.is_all();
                    if ui.checkbox(&mut every_face, "Open every face").changed() {
                        s.open_faces = if every_face {
                            soliddraft_model::ShellOpenings::All
                        } else {
                            soliddraft_model::ShellOpenings::Faces(Vec::new())
                        };
                    }
                    if let soliddraft_model::ShellOpenings::Faces(faces) = &mut s.open_faces {
                        let remove = pick_list(
                            ui,
                            "Click faces to open; leave empty for a closed shell",
                            Icon::Plane,
                            faces.iter().map(|f| f.label.as_str()),
                        );
                        if let Some(i) = remove {
                            faces.remove(i);
                        }
                    } else {
                        hint(ui, "Nothing of the part remains.");
                    }
                });
                section(ui, "Wall", |ui| {
                    // Keep the user's typed value: invalid walls must be reported, not
                    // silently clamped to the generic positive dimension-field range.
                    number_row(ui, "Wall thickness", "mm", egui::DragValue::new(&mut s.thickness).speed(0.1));
                });
                if let Some(error) = &editor.error {
                    error_row(ui, error);
                }
            }
            EditorParams::Draft(d) => {
                section(ui, "Faces to draft", |ui| {
                    count_note(ui, d.faces.len(), "face", "faces");
                    let remove = pick_list(
                        ui,
                        "Click faces in the viewport",
                        Icon::Plane,
                        d.faces.iter().map(|f| f.label.as_str()),
                    );
                    if let Some(i) = remove {
                        d.faces.remove(i);
                    }
                });
                section(ui, "Neutral plane", |ui| {
                    plane_combo(ui, "draft_neutral", &mut d.neutral, &plane_choices);
                    ui.checkbox(&mut d.picking_neutral, "Next face click sets the neutral plane");
                    unit_field(ui, "Angle", &mut d.angle, "°");
                    ui.checkbox(&mut d.reverse, "Pull against the neutral normal");
                });
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
                form_note(ui, "Draft tapers planar faces, and cylinders parallel to the pull.");
            }
            EditorParams::PickPlane => {
                section(ui, "Sketch plane", |ui| {
                    hint(ui, "Choose a reference plane, or click a planar face of the body.");
                    ui.add_space(4.0);
                    for (feature, name) in &plane_choices {
                        if ui.add_sized(vec2(ui.available_width(), 26.0), egui::Button::new(name)).clicked() {
                            actions.push(AppAction::StartSketchOn(SketchPlane::Feature {
                                feature: feature.0 as _,
                                offset: 0.0,
                            }));
                        }
                    }
                    for p in PlaneKind::ALL {
                        if ui
                            .add_sized(
                                vec2(ui.available_width(), 26.0),
                                egui::Button::new(state.doc.model().datum_name(p.to_model())),
                            )
                            .clicked()
                        {
                            actions.push(AppAction::StartSketchOn(SketchPlane::Reference { plane: p, offset: 0.0 }));
                        }
                    }
                });
            }
            EditorParams::Repair { items, chosen, active } => {
                section(ui, "Missing references", |ui| {
                    hint(ui, "Pick the replacement for each one in the viewport, then OK.");
                    ui.add_space(4.0);
                    for (i, item) in items.iter().enumerate() {
                        let is_active = *active == i;
                        let picked = chosen.get(i).and_then(|c| c.as_ref()).map(|c| c.label().to_owned());
                        let frame = egui::Frame::new()
                            .fill(if is_active { theme::accent_soft() } else { theme::panel() })
                            .stroke(Stroke::new(1.0, if is_active { theme::accent() } else { theme::line() }))
                            .corner_radius(CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(8, 5));
                        let resp = frame
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                                    let icon = match item.kind {
                                        crate::document::RefKind::Edge => Icon::Axis,
                                        crate::document::RefKind::Face | crate::document::RefKind::Plane => Icon::Plane,
                                    };
                                    icons::paint(ui.painter(), icon, r, theme::muted());
                                    ui.label(
                                        egui::RichText::new(&item.label)
                                            .size(12.0)
                                            .strikethrough()
                                            .color(theme::muted()),
                                    );
                                });
                                match &picked {
                                    Some(l) => {
                                        ui.label(egui::RichText::new(format!("→ {l}")).size(12.0).color(theme::ok()));
                                    }
                                    None => {
                                        ui.label(
                                            egui::RichText::new(if is_active {
                                                match item.kind {
                                                    crate::document::RefKind::Edge => "Click the replacement edge",
                                                    crate::document::RefKind::Face => "Click the replacement face",
                                                    crate::document::RefKind::Plane => "Choose a plane or planar face",
                                                }
                                            } else {
                                                "Waiting"
                                            })
                                            .size(11.5)
                                            .color(if is_active {
                                                theme::accent()
                                            } else {
                                                theme::muted()
                                            }),
                                        );
                                    }
                                }
                            })
                            .response;
                        if ui.interact(resp.rect, ui.id().with(("repair_item", i)), Sense::click()).clicked() {
                            *active = i;
                        }
                        ui.add_space(4.0);
                    }
                });
                if let Some(err) = &editor.error {
                    error_row(ui, err);
                }
            }
        });
    });
    for a in actions {
        state.push(a);
    }
}

fn sketch_panel(ui: &mut egui::Ui, state: &mut AppState) {
    let name = state
        .sketch
        .as_ref()
        .and_then(|s| state.doc.feature(s.feature))
        .map(|f| f.name.clone())
        .unwrap_or("Sketch".into());
    let plane_label = state
        .sketch
        .as_ref()
        .and_then(|s| state.doc.feature(s.feature))
        .and_then(|f| f.sketch())
        .map(|s| s.plane.label())
        .unwrap_or_default();
    let head = header(ui, Icon::Sketch, &name);
    let (ok, cancel) = ok_cancel(ui, head, "Exit sketch (Enter)", "Exit sketch and discard nothing (Esc)");
    if ok || cancel {
        state.push(AppAction::ExitSketch(None));
    }
    let mut actions = Vec::new();
    let has_body = state.doc.model().body().is_some();
    let links =
        state.sketch.as_ref().map(|s| crate::sketch_values::links(state.doc.model(), s.feature)).unwrap_or_default();
    let Some(s) = state.sketch.as_mut() else { return };
    let profiles = crate::sketch::profile::profiles(&s.sketch, s.plane);
    let has_profile = profiles.is_ok();
    egui::ScrollArea::vertical().id_salt("pm").auto_shrink([false, false]).show(ui, |ui| {
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

            // Exit into a feature.
            theme::section_label(ui, "Use the sketch");
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 6.0) / 2.0;
                if ui
                    .add_enabled(has_profile, egui::Button::new("Extrude").min_size(vec2(w, 24.0)))
                    .on_hover_text("Exit the sketch into a boss extrude")
                    .clicked()
                {
                    actions.push(AppAction::ExitSketch(Some(ExitInto::Extrude)));
                }
                if ui
                    .add_enabled(has_profile, egui::Button::new("Cut").min_size(vec2(w, 24.0)))
                    .on_hover_text("Exit the sketch into a cut")
                    .clicked()
                {
                    actions.push(AppAction::ExitSketch(Some(ExitInto::Cut)));
                }
            });
            if !has_profile {
                hint(ui, "Close a loop to extrude or cut.");
            }
            section_gap(ui);

            // Tools.
            theme::section_label(ui, "Tools");
            let tools = [
                (Tool::Select, Icon::Views, "Select"),
                (Tool::Line, Icon::Line, "Line (L)"),
                (Tool::Rectangle, Icon::Rectangle, "Rectangle (R)"),
                (Tool::Circle, Icon::Circle, "Circle (C)"),
                (Tool::Arc, Icon::Arc, "Arc (A)"),
                (Tool::Slot, Icon::Slot, "Slot (S)"),
                (Tool::Point, Icon::Point, "Point (P)"),
                (Tool::Dimension, Icon::SmartDim, "Smart dimension (D)"),
                (Tool::Polygon, Icon::Polygon, "Polygon"),
                (Tool::Trim, Icon::Trim, "Trim"),
                (Tool::Extend, Icon::Extend, "Extend"),
                (Tool::Offset, Icon::Offset, "Offset"),
                (Tool::Mirror, Icon::Mirror, "Mirror entities"),
                (Tool::LinearPattern, Icon::LinearPattern, "Linear pattern"),
                (Tool::CircularPattern, Icon::CircularPattern, "Circular pattern"),
            ];
            ui.horizontal_wrapped(|ui| {
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
            if s.tool.modifies() {
                section_gap(ui);
                crate::sketch_tools::panel(ui, s, &mut actions);
            }
            section_gap(ui);

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
                                    EntityKind::Line { a, b } => format!("{} mm long", fmt_value(a.dist(b))),
                                    EntityKind::Circle { radius, .. } => format!("Ø{} mm", fmt_value(radius * 2.0)),
                                    EntityKind::Arc { .. } => format!("R{} mm", fmt_value(e.radius().unwrap_or(0.0))),
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
                            if let Some(con) = s.sketch.constraint(*c).copied() {
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
                .map(|c| (*c, conflicting(c.id), dangling(c)))
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
            let applicable = sketcher::applicable_relations(&s.sketch, &selection);
            if !applicable.is_empty() {
                ui.add_space(6.0);
                theme::section_label(ui, "Add relation");
                let mut add = None;
                ui.horizontal_wrapped(|ui| {
                    for (name, kind) in applicable {
                        if ui.button(name).clicked() {
                            add = Some(kind);
                        }
                    }
                });
                if let Some(kind) = add {
                    actions.push(sketcher::add_relation(s, kind));
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
        (Selection::Feature(_), Some(crate::document::FeatureKind::Shell(_))) => Icon::Shell,
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
