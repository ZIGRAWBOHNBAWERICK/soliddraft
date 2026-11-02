//! The Modify tools of the sketcher: polygon, trim, extend, offset, mirror and the two
//! patterns, all through [`Operation`]. The pointer drives polygon, trim and extend in
//! `sketcher.rs`; the copy tools take the selection and the values typed here. Every
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

/// The operation a copy tool would run on the selection with the typed values. `None`
/// for the tools the pointer drives and for an empty selection.
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
    theme::section_label(ui, tool.label());
    match tool {
        Tool::Polygon => {
            hint(ui, "Click the centre, then a vertex. The vertex sets the size and the turn.");
            ui.add(egui::DragValue::new(&mut s.modify.sides).range(3..=1000).prefix("Sides "));
        }
        Tool::Trim => {
            hint(ui, "Point at the piece of a line or arc to remove, between its crossings, and click.");
        }
        Tool::Extend => {
            hint(ui, "Point at the end of a line or arc and click. It lengthens to the nearest curve in its way.");
        }
        Tool::Offset => {
            hint(ui, "Select a connected chain in walking order. Positive is left of the first entity's direction.");
            ui.add(egui::DragValue::new(&mut s.modify.distance).speed(0.25).prefix("Distance ").suffix(" mm"));
        }
        Tool::Mirror => {
            hint(ui, "Select the entities and give the axis. Copies keep construction status.");
            point(ui, "Axis start", &mut s.modify.axis_start);
            point(ui, "Axis end", &mut s.modify.axis_end);
        }
        Tool::LinearPattern => {
            hint(ui, "Select the entities and give the step between instances.");
            point(ui, "Step", &mut s.modify.step);
            ui.add(egui::DragValue::new(&mut s.modify.count).range(2..=1000).prefix("Instances "));
        }
        Tool::CircularPattern => {
            hint(ui, "Select the entities and give the centre and the angle between instances.");
            point(ui, "Centre", &mut s.modify.center);
            ui.add(egui::DragValue::new(&mut s.modify.angle).speed(1.0).suffix(" degrees"));
            ui.add(egui::DragValue::new(&mut s.modify.count).range(2..=1000).prefix("Instances "));
        }
        _ => {}
    }
    if tool.selects() {
        hint(ui, "Coordinates are u, v in the sketch plane, in mm. Copies are independent; counts include the seed.");
        let selected = selected_in_order(s).len();
        hint(ui, &format!("{selected} selected. Click an entity to select it; Ctrl-click adds to the selection."));
        let preview = s.preview.clone().or_else(|| selection_preview(s));
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
        any_of, boolean, constant, integer, non_empty_array, number, object, one_of,
    };
    let point = object(&[("u", number("Plane u, mm")), ("v", number("Plane v, mm"))]);
    let id = integer("An existing entity id, greater than 0");
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
        object(&[("type", constant("circle")), ("center", point.clone()), ("radius", number("Positive radius, mm"))]),
    ]);
    any_of(vec![
        object(&[("tool", constant("add")), ("kind", geometry), ("construction", boolean("Exclude from profiles"))]),
        object(&[
            ("tool", constant("polygon")),
            ("center", point.clone()),
            ("vertex", point.clone()),
            ("sides", integer("Sides from 3 to 1000")),
        ]),
        object(&[("tool", constant("trim")), ("entity", id.clone()), ("at", point.clone())]),
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
