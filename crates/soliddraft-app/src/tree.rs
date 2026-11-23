//! FeatureManager-style tree: origin group, then one row per feature with its
//! absorbed sketch nested under it, and the rollback bar. Rows select,
//! double-click shows a solid feature's dimensions or edits the rest, F2 or a slow
//! second click renames, drag reorders, right-click offers the rest. The eye at the end
//! of a feature, plane or axis row shows or hides it (see
//! [`tree_visibility`](crate::tree_visibility)). A mark before the eye says whether the
//! feature is proved, pending or refused, and an under-defined sketch reads `(-)`.

use crate::document::{
    AxisKind, Command, Feature, FeatureId, FeatureKind, FeatureStatus, PlaneKind, Reference, Selection, SketchPlane,
    ViewItem,
};
use crate::icons::{self, Icon};
use crate::state::{AppAction, AppState, EditorParams, NoteKind, Rename};
use crate::theme;
use crate::tree_visibility;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use std::collections::BTreeMap;

/// A row the tree shows, flattened from the feature list.
struct RowItem {
    id: FeatureId,
    /// Index in the document's feature list; a nested sketch carries its consumer's.
    index: usize,
    /// Where the row's block starts in the feature list: its absorbed sketch, which may
    /// come well before the feature, or the feature itself.
    first: usize,
    /// The row's feature and the sketch it absorbs; a nested sketch row carries the same
    /// pair. A drop on its own pair moves nothing.
    pair: (FeatureId, Option<FeatureId>),
    indent: u8,
    chevron: Option<bool>,
    icon: Icon,
    name: String,
    sub: String,
    status: FeatureStatus,
    suppressed: bool,
    via_assistant: bool,
    under_defined: bool,
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    // Header: part name and rebuild state.
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let title = if state.doc.dirty { format!("{} •", state.doc.name) } else { state.doc.name.clone() };
    ui.painter().text(
        pos2(head.left() + 12.0, head.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::font_semibold(12.5),
        theme::ink(),
    );
    let errors = state.doc.error_count();
    let repairs = state.doc.repair_count();
    let (status, color) = if let Some((message, kind)) = state.edit_status() {
        (message.to_owned(), if matches!(kind, NoteKind::Error) { theme::error() } else { theme::muted() })
    } else if errors == 0 && repairs == 0 {
        ("Rebuilt · OK".to_owned(), theme::ok())
    } else if errors > 0 {
        (format!("{errors} error{}", if errors == 1 { "" } else { "s" }), theme::error())
    } else {
        (format!("{repairs} to repair"), theme::warn())
    };
    ui.painter().text(
        pos2(head.right() - 12.0, head.center().y),
        Align2::RIGHT_CENTER,
        status,
        theme::font_medium(11.0),
        color,
    );
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, theme::line()));

    let mut actions = Vec::new();
    egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        let selected_feature = state.selected_feature();
        // A feature asking for its sketch also takes a plane to draw a new one on.
        let picking_plane = matches!(
            state.editor.as_ref().map(|e| &e.params),
            Some(EditorParams::PickPlane | EditorParams::PickSketch(_))
        );
        let picking_axis = matches!(
            state.editor.as_ref().map(|e| &e.params),
            Some(EditorParams::Revolve(_) | EditorParams::Pattern(_))
        );

        // Origin group.
        let open = state.origin_open || picking_plane || picking_axis;
        if row(
            ui,
            RowSpec {
                indent: 0,
                chevron: Some(open),
                icon: Icon::Origin,
                name: "Origin, planes and axes",
                ..RowSpec::plain()
            },
        )
        .clicked()
        {
            state.origin_open = !state.origin_open;
        }
        if open {
            for plane in [PlaneKind::Front, PlaneKind::Top, PlaneKind::Right] {
                let name = state.doc.model().datum_name(plane.to_model()).to_owned();
                let reference = Selection::Reference(Reference::Plane(plane));
                let item = ViewItem::Plane(plane);
                let r = row(
                    ui,
                    RowSpec {
                        indent: 1,
                        icon: Icon::Plane,
                        name: &name,
                        shown: Some(tree_visibility::shown(state, item)),
                        selected: state.doc.selection == Some(reference),
                        ..RowSpec::plain()
                    },
                );
                tree_visibility::eye(ui, state, r.rect, item, &name);
                let r = r.on_hover_text(if picking_plane {
                    "Click to sketch on this plane"
                } else {
                    "Double-click to sketch on this plane"
                });
                if r.double_clicked() && !state.in_sketch() {
                    actions.push(AppAction::StartSketchOn(SketchPlane::Reference { plane, offset: 0.0 }));
                } else if r.clicked() {
                    actions.push(AppAction::Select(Some(reference)));
                }
            }
            for axis in AxisKind::ALL {
                let reference = Selection::Reference(Reference::Axis(axis));
                let item = ViewItem::Axis(axis);
                let r = row(
                    ui,
                    RowSpec {
                        indent: 1,
                        icon: Icon::Axis,
                        icon_color: Some(theme::axis_color(axis)),
                        name: axis.label(),
                        shown: Some(tree_visibility::shown(state, item)),
                        selected: state.doc.selection == Some(reference),
                        ..RowSpec::plain()
                    },
                );
                tree_visibility::eye(ui, state, r.rect, item, axis.label());
                let r = if picking_axis { r.on_hover_text("Click to use this axis") } else { r };
                if r.clicked() {
                    actions.push(AppAction::Select(Some(reference)));
                }
            }
            let reference = Selection::Reference(Reference::Origin);
            let r = row(
                ui,
                RowSpec {
                    indent: 1,
                    icon: Icon::Point,
                    name: "Origin",
                    dim: true,
                    selected: state.doc.selection == Some(reference),
                    ..RowSpec::plain()
                },
            );
            if r.clicked() {
                actions.push(AppAction::Select(Some(reference)));
            }
        }
        let material = state.doc.material();
        let can_assign = crate::commands::get("material").is_some_and(|command| command.is_enabled(state));
        let material_row = row(
            ui,
            RowSpec {
                indent: 0,
                icon: Icon::Material,
                name: "Material",
                sub: material.map_or("Unassigned", |m| m.name.as_str()),
                dim: !can_assign,
                ..RowSpec::plain()
            },
        );
        if can_assign && material_row.clicked() {
            actions.push(AppAction::ShowMaterial);
        }
        let origins: BTreeMap<u32, FeatureId> = state
            .doc
            .features
            .iter()
            .filter(|f| f.body.is_some() && f.status == FeatureStatus::Ok && !f.suppressed)
            .rev()
            .map(|f| (f.body_key, f.id))
            .collect();
        let (surfaces, solids): (Vec<_>, Vec<_>) =
            state.doc.display_bodies().into_iter().map(|(key, _)| key).partition(|key| state.doc.body_is_surface(*key));
        body_folder(ui, state, "Solid Bodies", &solids, &origins, &mut actions);
        if !surfaces.is_empty() {
            body_folder(ui, state, "Surface Bodies", &surfaces, &origins, &mut actions);
        }

        // Flatten features: absorbed sketches sit under their consumer.
        let absorbed: Vec<FeatureId> = state.doc.features.iter().filter_map(|f| f.kind.sketch()).collect();
        let mut items: Vec<RowItem> = Vec::new();
        for (i, f) in state.doc.features.iter().enumerate() {
            if f.is_sketch() && absorbed.contains(&f.id) {
                continue;
            }
            // An extrusion's Draft option shows on the extrusion's row.
            if state.doc.draft_owner(f.id).is_some() {
                continue;
            }
            let draft_status =
                state.doc.linked_draft(f.id).and_then(|d| state.doc.feature(d)).map(|d| d.status.clone());
            let icon = feature_icon(&f.kind);
            let child = f.kind.sketch().and_then(|s| state.doc.feature(s));
            let first =
                child.and_then(|c| state.doc.features.iter().position(|x| x.id == c.id)).map_or(i, |at| at.min(i));
            let (sub, status) = match (&state.drag_preview, draft_status) {
                (Some(drag), _) if drag.feature() == f.id => {
                    let (message, failed) = drag.label();
                    let status = if failed { FeatureStatus::Error(message.clone()) } else { FeatureStatus::Pending };
                    (message, status)
                }
                _ if f.status == FeatureStatus::Pending => (format!("{} · working", f.summary()), f.status.clone()),
                _ if matches!(f.status, FeatureStatus::Error(_)) => {
                    (format!("Failed · {}", f.summary()), f.status.clone())
                }
                (_, Some(status @ FeatureStatus::Error(_))) => (format!("Failed · {}", f.summary()), status),
                _ => (f.summary(), f.status.clone()),
            };
            let sub = if let FeatureKind::Combine { operation, tool } = &f.kind {
                let op = crate::multibody::operation_name(*operation);
                let target = state.doc.body_name(f.body_key);
                let tool = state.doc.body_name(*tool);
                let label = format!("{op}: {target} + {tool}");
                match status {
                    FeatureStatus::Ok => format!("{label} · consumed {tool}"),
                    FeatureStatus::Error(_) => format!("Failed · {label}"),
                    FeatureStatus::Pending => format!("{label} · working"),
                    _ => label,
                }
            } else if origins.get(&f.body_key) == Some(&f.id) {
                format!("Creates {} · {sub}", state.doc.body_name(f.body_key))
            } else if !f.is_sketch() && f.body.is_some() && state.doc.model().next_body() > 1 {
                format!("{} · {sub}", state.doc.body_name(f.body_key))
            } else {
                sub
            };
            items.push(RowItem {
                id: f.id,
                index: i,
                first,
                pair: (f.id, child.map(|c| c.id)),
                indent: 0,
                chevron: child.map(|_| state.expanded.contains(&f.id)),
                icon,
                name: f.name.clone(),
                sub,
                status,
                suppressed: f.suppressed,
                via_assistant: f.via_assistant,
                under_defined: under_defined(state, f),
            });
            if let Some(c) = child
                && state.expanded.contains(&f.id)
            {
                items.push(RowItem {
                    id: c.id,
                    index: i,
                    first,
                    pair: (f.id, Some(c.id)),
                    indent: 1,
                    chevron: None,
                    icon: Icon::Sketch,
                    name: c.name.clone(),
                    sub: c.summary(),
                    status: c.status.clone(),
                    suppressed: c.suppressed,
                    via_assistant: false,
                    under_defined: under_defined(state, c),
                });
            }
        }
        if selected_feature.is_none() {
            state.tree_followed = None;
        }
        if items.is_empty() {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("No features yet").family(theme::medium()).color(theme::muted()));
                ui.label(
                    egui::RichText::new("Sketch on a plane, or press E to extrude a built-in profile.")
                        .size(11.0)
                        .color(theme::muted()),
                );
            });
        }

        let rollback = state.doc.rollback;
        let feature_count = state.doc.features.len();
        let pointer_released = ui.input(|i| i.pointer.primary_released());
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());
        let dragging = state.drag_feature;
        // The row the dragged feature was dropped on: where its block starts, where its
        // feature is, and that feature's pair.
        let mut drop_target: Option<(usize, usize, (FeatureId, Option<FeatureId>))> = None;
        let mut bar_target: Option<usize> = None;
        let mut rename_commit: Option<(FeatureId, String)> = None;
        let mut rename_cancel = false;
        armed_rename(ui, state);
        let mut last_bottom = ui.cursor().top();
        let mut row_centres: Vec<(usize, f32)> = Vec::new();
        let mut bar_drawn = false;
        let bar_at = rollback.unwrap_or(feature_count);
        // While a sketch is open, the features after it wait for it, so they read dimmed.
        let editing = state.sketch.as_ref().map(|s| s.feature);
        let edit_index = editing.and_then(|id| state.doc.features.iter().position(|f| f.id == id));

        for item in &items {
            // The rollback bar sits before the first feature past it.
            if !bar_drawn && item.indent == 0 && item.index >= bar_at {
                rollback_bar(ui, state, bar_at, feature_count, &mut actions, &mut bar_target, pointer_released);
                bar_drawn = true;
            }
            let inactive = item.suppressed
                || matches!(item.status, FeatureStatus::Inactive)
                || (editing != Some(item.id) && edit_index.is_some_and(|at| item.index > at));
            let error = match &item.status {
                FeatureStatus::Error(e) => Some(crate::geometry_message::for_user(e)),
                _ => None,
            };
            let repair = match &item.status {
                FeatureStatus::NeedsRepair(items) => Some(format!(
                    "{} reference{} missing: {}. Select the feature and choose Repair.",
                    items.len(),
                    if items.len() == 1 { "" } else { "s" },
                    items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>().join(", ")
                )),
                _ => None,
            };
            let renaming = state.rename.as_ref().filter(|r| r.feature == item.id).cloned();
            let resp = if let Some(r) = renaming {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), theme::ROW_HEIGHT), Sense::hover());
                let mut text = r.text.clone();
                let edit = ui.put(
                    Rect::from_min_max(
                        pos2(rect.left() + 44.0 + item.indent as f32 * 20.0, rect.top() + 1.0),
                        pos2(rect.right() - 6.0, rect.bottom() - 1.0),
                    ),
                    egui::TextEdit::singleline(&mut text).font(theme::font(12.5)),
                );
                edit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Feature name"));
                if r.focus {
                    edit.request_focus();
                }
                if let Some(rn) = state.rename.as_mut() {
                    rn.text = text.clone();
                    rn.focus = false;
                }
                icons::paint(
                    ui.painter(),
                    item.icon,
                    Rect::from_center_size(
                        pos2(rect.left() + 32.0 + item.indent as f32 * 20.0, rect.center().y),
                        vec2(15.0, 15.0),
                    ),
                    theme::ink(),
                );
                if ui.input(|i| i.key_pressed(egui::Key::Enter))
                    || (edit.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape)))
                {
                    rename_commit = Some((item.id, text));
                } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    rename_cancel = true;
                }
                resp
            } else {
                let shown = tree_visibility::shown(state, ViewItem::Feature(item.id));
                let resp = row(
                    ui,
                    RowSpec {
                        indent: item.indent,
                        chevron: item.chevron,
                        icon: item.icon,
                        icon_color: None,
                        name: &item.name,
                        sub: &item.sub,
                        selected: selected_feature == Some(item.id),
                        dim: inactive,
                        shown: Some(shown),
                        error: error.as_deref(),
                        warn: repair.as_deref(),
                        via_assistant: item.via_assistant,
                        suppressed: item.suppressed,
                        under_defined: item.under_defined,
                        badge: true,
                        drop_above: dragging.is_some_and(|d| d != item.id)
                            && pointer_pos.is_some_and(|p| {
                                let r = Rect::from_min_size(pos2(0.0, last_bottom), vec2(10_000.0, theme::ROW_HEIGHT));
                                r.contains(pos2(r.center().x, p.y))
                            }),
                    },
                );
                tree_visibility::eye(ui, state, resp.rect, ViewItem::Feature(item.id), &item.name);
                if let Some(feature) = state.doc.feature(item.id) {
                    status_mark(ui, state, feature, resp.rect);
                }
                resp.on_hover_text(&item.sub)
            };
            last_bottom = resp.rect.bottom();
            if item.indent == 0 {
                row_centres.push((item.index, resp.rect.center().y));
            }
            // A newly selected feature scrolls into view, wherever the selection came from.
            if selected_feature == Some(item.id) && state.tree_followed != Some(item.id) {
                state.tree_followed = Some(item.id);
                resp.scroll_to_me(None);
            }
            if dragging.is_some_and(|d| d != item.id) && resp.hovered() && pointer_released {
                drop_target = Some((item.first, item.index, item.pair));
            }
            // A nested sketch drags too: it may move away from its feature, as far up as
            // its own inputs allow.
            if resp.drag_started_by(egui::PointerButton::Primary) {
                state.drag_feature = Some(item.id);
            }
            if resp.clicked() {
                let chevron_zone = resp.rect.left() + 20.0;
                if item.chevron.is_some() && resp.interact_pointer_pos().is_some_and(|p| p.x < chevron_zone) {
                    if state.expanded.contains(&item.id) {
                        state.expanded.remove(&item.id);
                    } else {
                        state.expanded.insert(item.id);
                    }
                } else if state.doc.selection == Some(Selection::Feature(item.id))
                    && state.rename.is_none()
                    && !ui.input(|i| i.modifiers.any())
                {
                    // A second click on the selected row starts a rename, like a file
                    // manager, but only a slow one: it waits out the double-click time
                    // below, so the first click of a double-click renames nothing.
                    state.rename_armed = Some((item.id, ui.input(|i| i.time)));
                } else {
                    actions.push(AppAction::Select(Some(Selection::Feature(item.id))));
                }
            }
            if resp.double_clicked() {
                state.rename = None;
                state.rename_armed = None;
                actions.push(crate::dimension_edit::row_double_click(state, item.id));
            }
            let id = item.id;

            if !state.in_sketch() {
                crate::context_menu::show(
                    state,
                    &resp,
                    crate::context_menu::Target::Feature,
                    Some(Selection::Feature(id)),
                );
            }
        }
        if !bar_drawn && !items.is_empty() {
            rollback_bar(ui, state, bar_at, feature_count, &mut actions, &mut bar_target, pointer_released);
        }

        // Finish a drag.
        if let Some((id, text)) = rename_commit {
            state.rename = None;
            if state.doc.feature(id).is_some_and(|f| f.name != text) && !text.trim().is_empty() {
                actions.push(AppAction::Command(Command::Rename { id, name: text }));
            }
        }
        if rename_cancel {
            state.rename = None;
        }
        if let Some(d) = dragging {
            if let Some(p) = pointer_pos {
                let painter =
                    ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("tree_drag")));
                let name = state.doc.feature(d).map(|f| f.name.clone()).unwrap_or_default();
                let galley = painter.layout_no_wrap(name, theme::font_medium(12.0), theme::ink());
                let r = Rect::from_min_size(p + vec2(12.0, -10.0), galley.size() + vec2(16.0, 8.0));
                painter.rect(
                    r,
                    CornerRadius::same(6),
                    theme::panel(),
                    Stroke::new(1.0, theme::accent()),
                    StrokeKind::Inside,
                );
                painter.galley(r.min + vec2(8.0, 4.0), galley, theme::ink());
            }
            if pointer_released {
                state.drag_feature = None;
                // Dropped on a row, the feature takes that row's place: above the row's block
                // when it came from below the row's feature, below the feature otherwise. A
                // drop on its own feature or sketch moves nothing; the command judges every
                // other order, keeping each sketch ahead of its consumer and refusing one
                // that breaks an input.
                if let Some((first, last, (feature, sketch))) = drop_target
                    && d != feature
                    && Some(d) != sketch
                    && let Some(from) = state.doc.features.iter().position(|f| f.id == d)
                {
                    let index = if from > last { first } else { last };
                    actions.push(AppAction::Command(Command::Reorder { id: d, index }));
                }
            }
        }
        if state.drag_rollback {
            if let Some(p) = pointer_pos {
                let painter = ui.painter();
                let y = p.y.clamp(ui.min_rect().top(), ui.min_rect().bottom());
                painter.hline(ui.min_rect().x_range().shrink(10.0), y, Stroke::new(2.0, theme::accent()));
            }
            if pointer_released {
                state.drag_rollback = false;
                if let Some(p) = pointer_pos {
                    // Build every feature whose row centre is above the pointer.
                    bar_target =
                        Some(row_centres.iter().filter(|(_, cy)| *cy < p.y).map(|(i, _)| i + 1).max().unwrap_or(0));
                }
            }
        }
        if let Some(t) = bar_target {
            actions.push(AppAction::Command(Command::Rollback(if t >= feature_count { None } else { Some(t) })));
        }
        ui.add_space(6.0);
    });
    for a in actions {
        state.push(a);
    }
}

pub(crate) struct BodyRename {
    document: soliddraft_document::DocumentId,
    key: u32,
    label: String,
    focus: bool,
}

pub(crate) fn rename_body(state: &mut AppState) {
    state.show_pane(crate::panels::Pane::Tree);
    state.body_rename = Some(BodyRename {
        document: state.doc.model().id(),
        key: state.doc.active_body,
        label: state.doc.body_name(state.doc.active_body),
        focus: true,
    });
}

fn body_folder(
    ui: &mut egui::Ui,
    state: &mut AppState,
    title: &str,
    bodies: &[u32],
    origins: &BTreeMap<u32, FeatureId>,
    actions: &mut Vec<AppAction>,
) {
    let id = ui.make_persistent_id((state.doc.model().id(), title));
    let open = ui.ctx().data(|data| data.get_temp::<bool>(id)).unwrap_or(true);
    let label = format!("{title} ({})", bodies.len());
    let header = row(ui, RowSpec { chevron: Some(open), icon: Icon::Tree, name: &label, ..RowSpec::plain() });
    #[cfg(target_arch = "wasm32")]
    crate::web::show_control(&label, header.rect);
    if header.clicked() {
        ui.ctx().data_mut(|data| data.insert_temp(id, !open));
    }
    // Rename reveals its body even if the folder was closed.
    if state.body_rename.as_ref().is_some_and(|edit| edit.document != state.doc.model().id()) {
        state.body_rename = None;
    }
    let renaming = state.body_rename.as_ref().is_some_and(|edit| bodies.contains(&edit.key));
    if !open && !renaming {
        return;
    }
    for &key in bodies {
        ui.push_id(key, |ui| {
            let label = state.doc.body_name(key);
            if let Some(edit) = state.body_rename.as_mut().filter(|edit| edit.key == key) {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), theme::ROW_HEIGHT), Sense::hover());
                let rect = Rect::from_min_max(rect.min + vec2(27.0, 1.0), rect.max - vec2(6.0, 1.0));
                let field = ui.put(rect, egui::TextEdit::singleline(&mut edit.label));
                field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Body name"));
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control("Body name", field.rect);
                if edit.focus {
                    field.request_focus();
                    field.scroll_to_me(None);
                    edit.focus = false;
                }
                let (cancel, enter) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Enter)));
                if cancel {
                    state.body_rename = None;
                } else if enter || field.lost_focus() {
                    let edit = state.body_rename.take().expect("the body name is being edited");
                    if edit.label.trim() != label {
                        state.doc.set_body_name(key, &edit.label);
                    }
                }
                return;
            }
            let item = ViewItem::Body(key);
            let origin = origins
                .get(&key)
                .and_then(|id| state.doc.feature(*id))
                .map(|feature| format!("Created by {}", feature.name));
            let response = row(
                ui,
                RowSpec {
                    indent: 1,
                    icon: Icon::Extrude,
                    name: &label,
                    sub: origin.as_deref().unwrap_or_default(),
                    shown: Some(tree_visibility::shown(state, item)),
                    selected: state.doc.selection == Some(Selection::Body) && state.doc.active_body == key,
                    ..RowSpec::plain()
                },
            );
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control(&label, response.rect);
            tree_visibility::eye(ui, state, response.rect, item, &label);
            if response.clicked() {
                actions.push(AppAction::SelectBody(key));
            }
            if !state.in_sketch() {
                if response.secondary_clicked() {
                    state.doc.select_body(key);
                }
                crate::context_menu::show(state, &response, crate::context_menu::Target::Body, Some(Selection::Body));
            }
        });
    }
}

/// The rollback bar: everything below it is not built. Drag it between rows.
fn rollback_bar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    at: usize,
    count: usize,
    actions: &mut Vec<AppAction>,
    target: &mut Option<usize>,
    released: bool,
) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Rollback bar"));
    let active = at < count;
    let color = if resp.hovered() || resp.dragged() {
        theme::accent()
    } else if active {
        theme::warn()
    } else {
        theme::line().gamma_multiply(1.3)
    };
    let y = rect.center().y;
    ui.painter().hline(
        (rect.left() + 10.0)..=(rect.right() - 10.0),
        y,
        Stroke::new(if active { 2.5 } else { 2.0 }, color),
    );
    ui.painter().rect_filled(
        Rect::from_center_size(pos2(rect.center().x, y), vec2(26.0, 6.0)),
        CornerRadius::same(3),
        color,
    );
    if resp.has_focus() {
        ui.painter().rect_stroke(rect, CornerRadius::same(3), Stroke::new(2.0, theme::ink()), StrokeKind::Inside);
    }
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let resp = resp.on_hover_text(if active {
        "Rollback bar: features below are not built. Drag it down, or right-click a feature."
    } else {
        "Rollback bar: drag it up to build only the features above it."
    });
    if resp.drag_started() {
        state.drag_feature = None;
        state.drag_rollback = true;
    }
    if resp.double_clicked() && active {
        actions.push(AppAction::Command(Command::Rollback(None)));
    }
    let _ = (released, &target);
}

/// Turns a click on the selected row into its rename once the double-click time has
/// passed, unless the user did anything else first. Another press or key, an open
/// editor or sketch, or a changed selection means the click was not a slow second click
/// to rename: the field would open later and take the keyboard from whatever followed.
fn armed_rename(ui: &egui::Ui, state: &mut AppState) {
    let Some((armed, since)) = state.rename_armed else { return };
    let now = ui.input(|i| i.time);
    let wait = ui.ctx().options(|o| o.input_options.max_double_click_delay);
    let acted = ui.input(|i| {
        i.pointer.any_pressed()
            || i.pointer.is_decidedly_dragging()
            || i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. } | egui::Event::Text(_)))
    });
    let name = state.doc.feature(armed).map(|f| f.name.clone());
    if acted
        || state.editor.is_some()
        || state.in_sketch()
        || state.rename.is_some()
        || state.doc.selection != Some(Selection::Feature(armed))
    {
        state.rename_armed = None;
    } else if now < since + wait {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(since + wait - now));
    } else {
        state.rename_armed = None;
        // A tree that was hidden meanwhile comes back with no rename it did not see asked.
        if let Some(text) = name.filter(|_| now < since + wait + STALE_RENAME) {
            state.rename = Some(Rename { feature: armed, text, focus: true });
        }
    }
}

/// How long after the double-click time an armed rename may still open, in seconds.
const STALE_RENAME: f64 = 0.5;

pub(crate) struct RowSpec<'a> {
    pub(crate) indent: u8,
    pub(crate) chevron: Option<bool>,
    pub(crate) icon: Icon,
    /// Paints the icon in this colour instead of the row's ink.
    pub(crate) icon_color: Option<Color32>,
    pub(crate) name: &'a str,
    pub(crate) sub: &'a str,
    pub(crate) selected: bool,
    pub(crate) dim: bool,
    /// For an item with an eye: whether it shows. A hidden row is grey, and the row keeps
    /// its right end free for the eye.
    pub(crate) shown: Option<bool>,
    pub(crate) error: Option<&'a str>,
    pub(crate) warn: Option<&'a str>,
    pub(crate) via_assistant: bool,
    pub(crate) suppressed: bool,
    /// A sketch that is not fully defined: its name reads `(-) Name`, as in SolidWorks.
    pub(crate) under_defined: bool,
    /// Keeps room before the eye for the feature's status mark (see [`status_mark`]).
    pub(crate) badge: bool,
    pub(crate) drop_above: bool,
}

impl RowSpec<'static> {
    pub(crate) fn plain() -> Self {
        RowSpec {
            indent: 0,
            chevron: None,
            icon: Icon::Point,
            icon_color: None,
            name: "",
            sub: "",
            selected: false,
            dim: false,
            shown: None,
            error: None,
            warn: None,
            via_assistant: false,
            suppressed: false,
            under_defined: false,
            badge: false,
            drop_above: false,
        }
    }
}

pub(crate) fn row(ui: &mut egui::Ui, spec: RowSpec<'_>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), theme::ROW_HEIGHT), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, spec.selected, spec.name));
    if let Some(shown) = spec.shown {
        ui.ctx().accesskit_node_builder(resp.id, |node| {
            node.set_state_description(if shown { "Shown" } else { "Hidden" });
        });
    }
    let hidden = spec.shown == Some(false);
    let dim = spec.dim || hidden;
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let painter = ui.painter();
    let inner = Rect::from_min_max(pos2(rect.left() + 4.0, rect.top()), pos2(rect.right() - 4.0, rect.bottom()));
    if spec.selected {
        painter.rect_filled(inner, CornerRadius::same(5), theme::selection_bg());
    } else if resp.hovered() {
        painter.rect_filled(inner, CornerRadius::same(5), theme::hover());
    }
    if spec.drop_above {
        painter.hline(inner.x_range(), inner.top() + 0.5, Stroke::new(2.0, theme::accent()));
    }
    // Keyboard focus is a ring in the ink colour, apart from the selection fill.
    if resp.has_focus() {
        painter.rect_stroke(inner, CornerRadius::same(5), Stroke::new(2.0, theme::ink()), StrokeKind::Inside);
    }
    let x0 = rect.left() + 10.0 + spec.indent as f32 * 20.0;
    let ink = if spec.selected {
        theme::selection_ink()
    } else if dim {
        theme::muted()
    } else {
        theme::ink()
    };
    if let Some(open) = spec.chevron {
        let icon = if open { Icon::ChevronDown } else { Icon::ChevronRight };
        icons::paint(
            painter,
            icon,
            Rect::from_center_size(pos2(x0 + 4.0, rect.center().y), vec2(11.0, 11.0)),
            theme::muted(),
        );
    }
    let icon_color = if spec.error.is_some() {
        theme::error()
    } else if spec.warn.is_some() {
        theme::warn()
    } else if hidden {
        ink
    } else {
        spec.icon_color.unwrap_or(ink)
    };
    icons::paint(
        painter,
        spec.icon,
        Rect::from_center_size(pos2(x0 + 22.0, rect.center().y), vec2(15.0, 15.0)),
        icon_color,
    );
    let font = if dim { theme::font(12.0) } else { theme::font(12.5) };
    let caption = if spec.under_defined { format!("(-) {}", spec.name) } else { spec.name.to_owned() };
    let name_rect = painter.text(pos2(x0 + 35.0, rect.center().y), Align2::LEFT_CENTER, caption, font, ink);
    let mut after = name_rect.right() + 6.0;
    if spec.suppressed {
        icons::paint(
            painter,
            Icon::Suppress,
            Rect::from_center_size(pos2(after + 6.0, rect.center().y), vec2(12.0, 12.0)),
            theme::muted(),
        );
        after += 16.0;
    }
    if spec.via_assistant {
        icons::paint(
            painter,
            Icon::Spark,
            Rect::from_center_size(pos2(after + 6.0, rect.center().y), vec2(11.0, 11.0)),
            theme::accent(),
        );
    }
    if !spec.sub.is_empty() {
        let eye = if spec.shown.is_some() { crate::tree_visibility::EYE_WIDTH } else { 0.0 };
        let badge = if spec.badge { BADGE_WIDTH } else { 0.0 };
        painter.text(
            pos2(inner.right() - 8.0 - eye - badge, rect.center().y),
            Align2::RIGHT_CENTER,
            spec.sub,
            theme::font(11.0),
            theme::muted(),
        );
    }
    if let Some(err) = spec.error {
        painter.circle_filled(pos2(x0 + 29.0, rect.top() + 6.0), 3.0, theme::error());
        return resp.on_hover_text(egui::RichText::new(err).color(theme::ink()));
    }
    if let Some(w) = spec.warn {
        icons::paint(
            painter,
            Icon::Warning,
            Rect::from_center_size(pos2(x0 + 30.0, rect.top() + 6.5), vec2(9.0, 9.0)),
            theme::warn(),
        );
        return resp.on_hover_text(egui::RichText::new(w).color(theme::ink()));
    }
    resp
}

/// The room a row keeps for the proof mark, between its summary and its eye.
const BADGE_WIDTH: f32 = 12.0;

/// Whether `feature` is a sketch that is not fully defined, reading the open sketch
/// session when the feature is the one being edited.
fn under_defined(state: &AppState, feature: &Feature) -> bool {
    let status = state
        .sketch
        .as_ref()
        .filter(|s| s.feature == feature.id)
        .map(|s| &s.sketch.status)
        .or_else(|| feature.sketch().map(|s| &s.sketch.status));
    matches!(status, Some(crate::sketch::SolveStatus::UnderDefined))
}

/// The mark before a feature row's eye. An open amber ring says the feature is still
/// working, a red cross that it was refused, and a grey ring that it is inactive. A built
/// feature has no mark. Hover gives the reason.
fn status_mark(ui: &mut egui::Ui, state: &AppState, feature: &Feature, row: Rect) {
    let id = feature.id;
    let (label, reason) = if let Some(drag) = state.drag_preview.as_ref().filter(|drag| drag.feature() == id) {
        let (reason, failed) = drag.label();
        if failed {
            (REFUSED, crate::geometry_message::for_user(&reason))
        } else if drag.pending() {
            (WORKING, crate::geometry_message::for_user(&reason))
        } else {
            return;
        }
    } else if let Some(editor) = state.editor.as_ref().filter(|editor| editor.target == Some(id)) {
        match &editor.error {
            Some(reason) => (REFUSED, crate::geometry_message::for_user(reason)),
            None => (WORKING, "Finish or cancel this feature edit. Its preview cannot export.".into()),
        }
    } else if state.sketch.as_ref().is_some_and(|sketch| sketch.feature == id) {
        (WORKING, "Finish or cancel this sketch edit before exporting.".into())
    } else if let Some(form) = state.doc.combine_form.as_ref().filter(|form| form.editing == Some(id)) {
        match &form.error {
            Some(reason) => (REFUSED, crate::geometry_message::for_user(reason)),
            None => (WORKING, "Finish or cancel this feature edit. Its preview cannot export.".into()),
        }
    } else if let Some(soliddraft_model::FeatureStatus::Blocked(input)) =
        state.doc.model().status(soliddraft_model::FeatureId(id as u32))
        && !matches!(feature.status, FeatureStatus::Inactive)
    {
        let name = state.doc.feature(u64::from(input.0)).map_or("an earlier feature", |feature| feature.name.as_str());
        (WORKING, format!("Waiting for {name} to build. Repair that feature before exporting."))
    } else {
        match &feature.status {
            FeatureStatus::Ok | FeatureStatus::AnsweredWithinBound(_) => return,
            FeatureStatus::Pending => (WORKING, "The current edit is still building. Export waits for it.".to_owned()),
            FeatureStatus::Error(reason) => (REFUSED, crate::geometry_message::for_user(reason)),
            FeatureStatus::NeedsRepair(_) => {
                (REFUSED, "A reference is missing. Choose Repair to replace it.".to_owned())
            }
            FeatureStatus::Inactive => (INACTIVE, "This feature is suppressed or past the rollback bar.".to_owned()),
        }
    };
    let centre = pos2(row.right() - 4.0 - tree_visibility::EYE_WIDTH - BADGE_WIDTH / 2.0, row.center().y);
    if ui.is_rect_visible(Rect::from_center_size(centre, vec2(8.0, 8.0))) {
        let painter = ui.painter();
        match label {
            REFUSED => {
                for sign in [-1.0, 1.0] {
                    painter.line_segment(
                        [centre + vec2(-3.0, -3.0 * sign), centre + vec2(3.0, 3.0 * sign)],
                        Stroke::new(1.5, theme::error()),
                    );
                }
            }
            WORKING => {
                painter.circle_stroke(centre, 3.0, Stroke::new(1.5, theme::warn()));
            }
            _ => {
                painter.circle_stroke(centre, 3.0, Stroke::new(1.5, theme::muted()));
            }
        }
    }
    let response = ui.interact(
        Rect::from_center_size(centre, vec2(BADGE_WIDTH, theme::ROW_HEIGHT - 6.0)),
        ui.id().with(("status", id)),
        Sense::hover(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{} status: {label}", feature.name))
    });
    #[cfg(target_arch = "wasm32")]
    crate::web::show_mark(&feature.name, label, &reason);
    response.on_hover_text(format!("{label}: {reason}"));
}

/// The status mark's words.
const WORKING: &str = "Working";
const REFUSED: &str = "Refused";
const INACTIVE: &str = "Inactive";

/// The icon of a feature's kind, in its tree row.
pub(crate) fn feature_icon(kind: &FeatureKind) -> Icon {
    match kind {
        FeatureKind::ReferencePlane(_) => Icon::Plane,
        FeatureKind::Sketch(_) => Icon::Sketch,
        FeatureKind::Extrude(_) => Icon::Extrude,
        FeatureKind::Revolve(_) => Icon::Revolve,
        FeatureKind::Sweep(_) => Icon::Sweep,
        FeatureKind::Loft(_) => Icon::Loft,
        FeatureKind::Cut(_) => Icon::Cut,
        FeatureKind::Fillet(_) => Icon::Fillet,
        FeatureKind::Chamfer(_) => Icon::Chamfer,
        FeatureKind::Hole(_) => Icon::Hole,
        FeatureKind::Pattern(p) => match p.layout {
            soliddraft_model::PatternSpec::Linear { .. } => Icon::LinearPattern,
            soliddraft_model::PatternSpec::Circular { .. } => Icon::CircularPattern,
        },
        FeatureKind::Mirror(_) => Icon::Mirror,
        FeatureKind::Draft(_) => Icon::Draft,
        FeatureKind::Combine { .. } => Icon::Extrude,
        FeatureKind::Shell(_) | FeatureKind::Thicken(_) => Icon::Shell,
        FeatureKind::Offset(_) => Icon::Cad("srf_offset"),
        FeatureKind::ImportStep(_) | FeatureKind::Scan(_) => Icon::Import,
        FeatureKind::Lattice(_) | FeatureKind::Gyroid(_) => Icon::Cad("lattice"),
    }
}
