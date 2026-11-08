//! Click and copy operations for the sketcher's Modify tools through [`Operation`].
//! Pointer placement and power-trim gestures live in `sketcher.rs`; the panel also
//! accepts numeric values for the copies. Every
//! tool previews what it will do before it does it, and a refused operation changes
//! nothing. The MCP `sketch_geometry` verb runs the same operations headlessly.

use crate::document::Command;
use crate::sketch::{
    EntityId, P2, solve,
    tools::{Operation, ToolError},
};
use crate::state::{AppAction, SketchSel, SketchSession, Tool, ToolPreview};
use crate::theme;
use serde_json::Value;

fn point(ui: &mut egui::Ui, label: &str, value: &mut P2) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(11.5));
        ui.add(egui::DragValue::new(&mut value.u).speed(0.5).prefix("u "));
        ui.add(egui::DragValue::new(&mut value.v).speed(0.5).prefix("v "));
    });
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(theme::muted()));
}

/// The entities the selection names, in the order they were picked. A picked point
/// stands for its entity.
pub fn selected_in_order(s: &SketchSession) -> Vec<EntityId> {
    let mut selected = Vec::<EntityId>::new();
    for pick in &s.selection {
        let id = match pick {
            SketchSel::Entity(id) => *id,
            SketchSel::Point(p) => p.entity,
            SketchSel::Constraint(_) => continue,
        };
        if !selected.contains(&id) {
            selected.push(id);
        }
    }
    selected
}

/// The operation a copy tool would run on the selection with the current values.
pub fn operation_for(s: &SketchSession) -> Option<Operation> {
    let entities = selected_in_order(s);
    if entities.is_empty() {
        return None;
    }
    let m = &s.modify;
    Some(match s.tool {
        Tool::Offset => Operation::Offset { entities, distance: m.distance },
        Tool::Mirror => Operation::Mirror { entities, axis_start: m.axis_start, axis_end: m.axis_end },
        Tool::LinearPattern => Operation::LinearPattern { entities, step: m.step, count: m.count },
        Tool::CircularPattern => {
            Operation::CircularPattern { entities, center: m.center, angle_degrees: m.angle, count: m.count }
        }
        _ => return None,
    })
}

/// What `operation` would add to the sketch, tried on a copy. The refusal is the tool's
/// own words.
pub fn try_operation(s: &SketchSession, operation: Operation) -> ToolPreview {
    if let Operation::TrimCorner { a, b, keep_a, keep_b, options } = &operation {
        return match s.sketch.corner_preview(*a, *b, *keep_a, *keep_b, *options) {
            Ok(p) => ToolPreview {
                added: p.kept.to_vec(),
                removed: p.removed,
                operation: Some(operation),
                ..Default::default()
            },
            Err(ToolError(why)) => ToolPreview { refused: Some(why), ..Default::default() },
        };
    }
    if let Operation::TrimBounded { entity, a, b, outside, options } = &operation {
        return match s.sketch.trim_bounded_preview(*entity, *a, *b, *outside, *options) {
            Ok(p) => {
                ToolPreview { added: p.kept, removed: p.removed, operation: Some(operation), ..Default::default() }
            }
            Err(ToolError(why)) => ToolPreview { refused: Some(why), ..Default::default() },
        };
    }
    let mut copy = s.sketch.clone();
    match copy.apply_operation(&operation) {
        Ok(ids) => ToolPreview {
            added: ids.iter().filter_map(|id| copy.entity(*id).map(|e| e.kind)).collect(),
            operation: Some(operation),
            ..Default::default()
        },
        Err(ToolError(why)) => ToolPreview { refused: Some(why), ..Default::default() },
    }
}

/// The preview of a copy tool for the current selection and values.
pub fn selection_preview(s: &SketchSession) -> Option<ToolPreview> {
    if !s.tool.selects() || s.tool == Tool::Select {
        return None;
    }
    Some(match operation_for(s) {
        Some(operation) => try_operation(s, operation),
        None => ToolPreview { refused: Some("Select the entities first".into()), ..Default::default() },
    })
}

/// Runs a tool on the sketch and records it as one undo step. New entities become the
/// selection, so a copy can be copied again; a refusal leaves the sketch alone and says why.
pub fn apply(s: &mut SketchSession, operation: &Operation, actions: &mut Vec<AppAction>) -> bool {
    let name = s.tool.label();
    match s.sketch.apply_operation(operation) {
        Ok(ids) => {
            if s.construction && matches!(operation, Operation::Polygon { .. }) {
                for id in &ids {
                    if let Some(e) = s.sketch.entity_mut(*id) {
                        e.construction = true;
                    }
                }
            }
            solve::solve(&mut s.sketch, None, true);
            actions.push(AppAction::Command(Command::EditSketch { id: s.feature, sketch: s.sketch.clone() }));
            if s.tool.selects() {
                s.selection = ids.into_iter().map(SketchSel::Entity).collect();
            }
            s.preview = None;
            s.pending.clear();
            s.drag = None;
            s.say(format!("{name} applied"));
            true
        }
        Err(error) => {
            s.say(error.to_string());
            false
        }
    }
}

/// The active Modify tool's section of the sketch properties: what to do with the pointer,
/// the values it needs, and Apply and Cancel for the tools that work on the selection.
pub fn panel(ui: &mut egui::Ui, s: &mut SketchSession, actions: &mut Vec<AppAction>) {
    let tool = s.tool;
    if !tool.modifies() {
        return;
    }
    if tool.trims() {
        ui.horizontal_wrapped(|ui| {
            if ui.selectable_label(s.tool == Tool::Trim, "Power / closest").clicked() {
                s.set_tool(Tool::Trim);
            }
            if ui.selectable_label(s.tool == Tool::TrimCorner, "Corner").clicked() {
                s.set_tool(Tool::TrimCorner);
            }
            if ui.selectable_label(s.tool == Tool::TrimInside, "Inside").clicked() {
                s.set_tool(Tool::TrimInside);
            }
            if ui.selectable_label(s.tool == Tool::TrimOutside, "Outside").clicked() {
                s.set_tool(Tool::TrimOutside);
            }
        });
    }
    let tool = s.tool;
    theme::section_label(ui, tool.label());
    match tool {
        Tool::Polygon => {
            hint(ui, "Click the centre, then a vertex. The vertex sets the size and the turn.");
            ui.add(egui::DragValue::new(&mut s.modify.sides).range(3..=1000).prefix("Sides "));
        }
        Tool::Trim => {
            hint(
                ui,
                "Click a piece to trim to its closest crossings. Drag from empty space to power trim, or grab a line or arc to extend it. Retrace a red marker to undo a cut. Esc cancels a drag.",
            );
        }
        Tool::TrimCorner => {
            hint(
                ui,
                if s.corner_first.is_none() {
                    "Pick the part of the first line or arc to keep."
                } else {
                    "Pick the part of the second line or arc to keep. Both ends will meet at the previewed corner."
                },
            );
            if let Some(why) = s.preview.as_ref().and_then(|p| p.refused.as_deref()) {
                ui.label(egui::RichText::new(why).size(11.0).color(theme::error()));
            }
        }
        Tool::TrimInside | Tool::TrimOutside => {
            hint(
                ui,
                match s.trim_bounds.len() {
                    0 => "Pick the first bounding line, arc or circle.",
                    1 => "Pick a different second bounding curve.",
                    _ => "Pick a line or arc crossing each bound once.",
                },
            );
            hint(
                ui,
                if tool == Tool::TrimInside {
                    "Remove inside. Ignore other crossings."
                } else {
                    "Keep inside. Ignore other crossings."
                },
            );
            if !s.trim_bounds.is_empty() && ui.button("Choose bounds again").clicked() {
                s.trim_bounds.clear();
                s.selection.clear();
                s.preview = None;
            }
            if let Some(why) = s.preview.as_ref().and_then(|p| p.refused.as_deref()) {
                ui.label(egui::RichText::new(why).size(11.0).color(theme::error()));
            }
        }
        Tool::Extend => {
            hint(ui, "Point at the end of a line or arc and click. It lengthens to the nearest curve in its way.");
        }
        Tool::Offset => {
            hint(
                ui,
                "Select a connected chain in walking order, then drag a selected curve to the side and distance you want. Apply adds the copy.",
            );
            ui.add(egui::DragValue::new(&mut s.modify.distance).speed(0.25).prefix("Distance ").suffix(" mm"));
        }
        Tool::Mirror => {
            hint(ui, "Select the entities, then click two points on the mirror axis. Apply adds the copies.");
            point(ui, "Axis start", &mut s.modify.axis_start);
            point(ui, "Axis end", &mut s.modify.axis_end);
        }
        Tool::LinearPattern => {
            hint(ui, "Select the entities, then click the start and end of one step. Apply adds the copies.");
            point(ui, "Step", &mut s.modify.step);
            ui.add(egui::DragValue::new(&mut s.modify.count).range(2..=1000).prefix("Instances "));
        }
        Tool::CircularPattern => {
            hint(ui, "Select the entities, then click the centre. Set the angle and count here, then Apply.");
            point(ui, "Centre", &mut s.modify.center);
            ui.add(egui::DragValue::new(&mut s.modify.angle).speed(1.0).suffix(" degrees"));
            ui.add(egui::DragValue::new(&mut s.modify.count).range(2..=1000).prefix("Instances "));
        }
        _ => {}
    }
    if tool.trims() {
        let keep =
            ui.checkbox(&mut s.trim_options.keep_as_construction, "Keep trimmed pieces as construction").changed();
        let ignore = ui.checkbox(&mut s.trim_options.ignore_construction, "Ignore construction targets").changed();
        if keep || ignore {
            s.cancel_trim();
            s.preview = None;
        }
    }
    if tool.selects() {
        hint(ui, "Coordinates are u, v in the sketch plane, in mm. Copies are independent; counts include the seed.");
        let selected = selected_in_order(s).len();
        hint(
            ui,
            &format!("{selected} selected. Ctrl-click adds or removes entities. Esc or Cancel returns to Select."),
        );
        let preview = selection_preview(s);
        if let Some(why) = preview.as_ref().and_then(|p| p.refused.as_deref()) {
            ui.label(egui::RichText::new(why).size(11.0).color(theme::error()));
        }
        let operation = preview.and_then(|p| p.operation);
        ui.horizontal(|ui| {
            let apply_now = ui
                .add_enabled(operation.is_some(), egui::Button::new("Apply"))
                .on_hover_text("Add the previewed copies as one undo step")
                .clicked();
            if apply_now && let Some(operation) = operation {
                apply(s, &operation, actions);
            }
            if ui.button("Cancel").on_hover_text("Back to Select (Esc)").clicked() {
                s.set_tool(Tool::Select);
            }
        });
    } else {
        hint(ui, "Esc or a right-click returns to Select.");
    }
}

/// The same operations used by the sketcher's Modify tools, exposed to command/MCP scenarios.
pub fn operation_schema() -> Value {
    use soliddraft_assistant::tools::schema::{
        any_of, boolean, constant, integer, non_empty_array, number, object, one_of, optional,
    };
    let point = object(&[("u", number("Plane u, mm")), ("v", number("Plane v, mm"))]);
    let trim_options = optional(object(&[
        ("keep_as_construction", optional(boolean("Keep removed pieces as guides, default false"))),
        (
            "ignore_construction",
            optional(boolean("Protect construction targets but retain them as boundaries, default false")),
        ),
    ]));
    let id = integer("An existing entity id, greater than 0");
    let point_ref =
        object(&[("entity", id.clone()), ("index", integer("Point index within its entity, starting at 0"))]);
    let relation = any_of(vec![
        object(&[("type", constant("collinear")), ("a", id.clone()), ("b", id.clone())]),
        object(&[("type", constant("coradial")), ("a", id.clone()), ("b", id.clone())]),
        object(&[
            ("type", constant("intersection")),
            ("point", point_ref.clone()),
            ("a", id.clone()),
            ("b", id.clone()),
        ]),
        object(&[
            ("type", constant("symmetric_points")),
            ("a", point_ref.clone()),
            ("b", point_ref),
            ("axis", id.clone()),
        ]),
        object(&[("type", constant("symmetric_entities")), ("a", id.clone()), ("b", id.clone()), ("axis", id.clone())]),
        object(&[("type", constant("fix_entity")), ("entity", id.clone())]),
    ]);
    let ids = non_empty_array(id.clone(), "Distinct entity ids, in selection order");
    let count = integer("Instances including the seed, from 2 to 1000");
    let geometry = any_of(vec![
        object(&[("type", constant("point")), ("p", point.clone())]),
        object(&[("type", constant("line")), ("a", point.clone()), ("b", point.clone())]),
        object(&[
            ("type", constant("arc")),
            ("center", point.clone()),
            ("start", point.clone()),
            ("end", point.clone()),
        ]),
        object(&[
            ("type", constant("ellipse")),
            ("center", point.clone()),
            ("major", point.clone()),
            ("minor", point.clone()),
        ]),
        object(&[
            ("type", constant("elliptic_arc")),
            ("center", point.clone()),
            ("major", point.clone()),
            ("minor", point.clone()),
            ("start", point.clone()),
            ("end", point.clone()),
        ]),
        object(&[("type", constant("circle")), ("center", point.clone()), ("radius", number("Positive radius, mm"))]),
    ]);
    any_of(vec![
        object(&[("tool", constant("relate")), ("relation", relation)]),
        object(&[("tool", constant("add")), ("kind", geometry), ("construction", boolean("Exclude from profiles"))]),
        object(&[
            ("tool", constant("snapped_line")),
            ("start", point.clone()),
            ("end", point.clone()),
            ("reach", number("Positive snap reach in mm")),
            (
                "start_filter",
                optional(one_of(
                    &[
                        "all",
                        "point",
                        "midpoint",
                        "intersection",
                        "on",
                        "axes",
                        "parallel",
                        "perpendicular",
                        "tangent",
                        "none",
                    ],
                    "Snap class for the first placement",
                )),
            ),
            (
                "end_filter",
                optional(one_of(
                    &[
                        "all",
                        "point",
                        "midpoint",
                        "intersection",
                        "on",
                        "axes",
                        "parallel",
                        "perpendicular",
                        "tangent",
                        "none",
                    ],
                    "Snap class for the second placement",
                )),
            ),
            ("construction", optional(boolean("Exclude from profiles"))),
        ]),
        object(&[
            ("tool", constant("polygon")),
            ("center", point.clone()),
            ("vertex", point.clone()),
            ("sides", integer("Sides from 3 to 1000")),
        ]),
        object(&[
            ("tool", constant("trim")),
            ("entity", id.clone()),
            ("at", point.clone()),
            ("options", trim_options.clone()),
        ]),
        object(&[
            ("tool", constant("trim_corner")),
            ("a", id.clone()),
            ("b", id.clone()),
            ("keep_a", point.clone()),
            ("keep_b", point.clone()),
            ("options", trim_options.clone()),
        ]),
        object(&[
            ("tool", constant("trim_bounded")),
            ("entity", id.clone()),
            ("a", id.clone()),
            ("b", id.clone()),
            ("outside", boolean("Remove outside portions when true, otherwise remove the inside interval")),
            ("options", trim_options),
        ]),
        object(&[
            ("tool", constant("extend")),
            ("entity", id.clone()),
            ("end", one_of(&["start", "end"], "Endpoint to extend")),
            ("boundary", id),
        ]),
        object(&[
            ("tool", constant("offset")),
            ("entities", ids.clone()),
            ("distance", number("Signed distance to the left of the chain, mm")),
        ]),
        object(&[
            ("tool", constant("mirror")),
            ("entities", ids.clone()),
            ("axis_start", point.clone()),
            ("axis_end", point.clone()),
        ]),
        object(&[
            ("tool", constant("linear_pattern")),
            ("entities", ids.clone()),
            ("step", point.clone()),
            ("count", count.clone()),
        ]),
        object(&[
            ("tool", constant("circular_pattern")),
            ("entities", ids),
            ("center", point),
            ("angle_degrees", number("Signed angle per instance, degrees")),
            ("count", count),
        ]),
    ])
}
