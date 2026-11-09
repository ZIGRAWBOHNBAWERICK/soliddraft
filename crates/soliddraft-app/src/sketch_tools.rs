//! Click and copy operations for the sketcher's Modify tools through [`Operation`].
//! Pointer placement and power-trim gestures live in `sketcher.rs`; the panel also
//! accepts numeric values for the copies. Every
//! tool previews what it will do before it does it, and a refused operation changes
//! nothing. The MCP `sketch_geometry` verb runs the same operations headlessly.

pub(crate) mod patterns;

use crate::document::Command;
use crate::sketch::{
    ConstraintKind, EntityId, P2, solve,
    tools::{ChamferSize, MirrorAxis, OffsetCap, Operation, ToolError},
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
    if s.tool == Tool::Offset
        && let Some(group) = s.modify.offset_group
    {
        return Some(Operation::EditOffset { group, distance: s.modify.distance });
    }
    let mut entities = selected_in_order(s);
    if matches!(s.tool, Tool::LinearPattern | Tool::CircularPattern) {
        return patterns::operation(s, entities);
    }
    if s.tool == Tool::Mirror {
        entities.retain(|id| Some(*id) != s.modify.mirror_axis);
    }
    if entities.is_empty() {
        return None;
    }
    let m = &s.modify;
    Some(match s.tool {
        Tool::Offset => Operation::Offset { entities, distance: m.distance, options: m.offset },
        Tool::Mirror if m.keep_symmetry => Operation::MirrorLinked {
            entities,
            axis: m.mirror_axis.map_or(MirrorAxis::Points { start: m.axis_start, end: m.axis_end }, |entity| {
                MirrorAxis::Entity { entity }
            }),
        },
        Tool::Mirror => Operation::Mirror { entities, axis_start: m.axis_start, axis_end: m.axis_end },
        Tool::LinearPattern => Operation::LinearPattern { entities, step: m.step, count: m.count },
        Tool::CircularPattern => {
            Operation::CircularPattern { entities, center: m.center, angle_degrees: m.angle, count: m.count }
        }
        Tool::Move => Operation::Move { entities, displacement: m.step },
        Tool::Copy => Operation::Copy { entities, base: m.base, displacement: m.step },
        Tool::Rotate => Operation::Rotate { entities, center: m.center, angle_degrees: m.angle, copy: m.copy },
        Tool::Scale => Operation::Scale { entities, base: m.base, factor: m.factor },
        _ => return None,
    })
}

/// What `operation` would add to the sketch, tried on a copy. The refusal is the tool's
/// own words.
pub fn try_operation(s: &SketchSession, operation: Operation) -> ToolPreview {
    if matches!(operation, Operation::EditPattern { .. }) {
        return patterns::preview(s, operation);
    }
    let junction = match &operation {
        Operation::Fillet { a, b, keep_a, keep_b, radius } => {
            Some(s.sketch.fillet_preview(*a, *b, *keep_a, *keep_b, *radius))
        }
        Operation::Chamfer { a, b, keep_a, keep_b, size } => {
            Some(s.sketch.chamfer_preview(*a, *b, *keep_a, *keep_b, *size))
        }
        _ => None,
    };
    if let Some(result) = junction {
        return match result {
            Ok(p) => ToolPreview {
                added: p.kept.into_iter().chain([p.bridge]).collect(),
                removed: p.removed,
                operation: Some(operation),
                ..Default::default()
            },
            Err(ToolError(why)) => ToolPreview { refused: Some(why), ..Default::default() },
        };
    }
    if let Operation::Split { entity, at, second } = &operation {
        return match s.sketch.split_preview(*entity, *at, *second) {
            Ok(p) => ToolPreview { added: p.pieces.to_vec(), operation: Some(operation), ..Default::default() },
            Err(ToolError(why)) => ToolPreview { refused: Some(why), ..Default::default() },
        };
    }
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
        Ok(ids) if ids.is_empty() && copy == s.sketch && matches!(operation, Operation::MirrorLinked { .. }) => {
            ToolPreview {
                refused: Some("The selection is already symmetric about this axis".into()),
                ..Default::default()
            }
        }
        Ok(mut ids) => {
            let replaced: Vec<_> = s
                .sketch
                .drawn()
                .filter(|old| {
                    copy.entity(old.id).is_some_and(|new| old.kind != new.kind || old.construction != new.construction)
                })
                .map(|e| e.id)
                .collect();
            for id in &replaced {
                if !ids.contains(id) {
                    ids.push(*id);
                }
            }
            let removed = match &operation {
                Operation::Move { entities, .. }
                | Operation::Rotate { entities, copy: false, .. }
                | Operation::Scale { entities, .. } => {
                    entities.iter().filter_map(|id| s.sketch.entity(*id).map(|e| e.kind)).collect()
                }
                _ => Vec::new(),
            };
            let entities: Vec<_> = ids.iter().filter_map(|id| copy.entity(*id)).collect();
            ToolPreview {
                added: entities.iter().map(|e| e.kind.clone()).collect(),
                construction: entities.iter().map(|e| e.construction).collect(),
                replaced,
                removed,
                operation: Some(operation),
                ..Default::default()
            }
        }
        Err(ToolError(why)) => ToolPreview { refused: Some(why), ..Default::default() },
    }
}

/// The preview of a copy tool for the current selection and values.
pub fn selection_preview(s: &SketchSession) -> Option<ToolPreview> {
    if !s.tool.selects() || s.tool == Tool::Select {
        return None;
    }
    if s.tool == Tool::Offset
        && s.modify.offset_group.is_some_and(|group| s.sketch.offset_group_distance(group) == Some(s.modify.distance))
    {
        return Some(ToolPreview::default());
    }
    if let Some(group) = s.modify.pattern_group {
        let mut skipped = s.modify.skipped.clone();
        skipped.sort_unstable();
        if s.sketch.patterns.iter().any(|p| p.id == group && p.spec == patterns::spec(s) && p.skipped == skipped) {
            return Some(ToolPreview::default());
        }
    }
    Some(match operation_for(s) {
        Some(operation) => try_operation(s, operation),
        None => ToolPreview { refused: Some("Select the entities first".into()), ..Default::default() },
    })
}

/// Runs a tool on the sketch and records it as one undo step. New entities become the
/// selection, so a copy can be copied again; a refusal leaves the sketch alone and says why.
pub fn apply(s: &mut SketchSession, operation: &Operation, actions: &mut Vec<AppAction>) -> bool {
    apply_with_analysis(s, operation, actions, true)
}

pub(crate) fn apply_with_analysis(
    s: &mut SketchSession,
    operation: &Operation,
    actions: &mut Vec<AppAction>,
    analyse: bool,
) -> bool {
    let name = s.tool.label();
    let old_relations: Vec<_> = s.sketch.constraints.iter().map(|c| c.id).collect();
    match s.sketch.apply_operation(operation) {
        Ok(ids) => {
            if s.construction && matches!(operation, Operation::Polygon { .. }) {
                for id in &ids {
                    if let Some(e) = s.sketch.entity_mut(*id) {
                        e.construction = true;
                    }
                }
            }
            solve::solve(&mut s.sketch, None, analyse);
            actions.push(AppAction::Command(Command::EditSketch { id: s.feature, sketch: s.sketch.clone() }));
            if s.tool.selects() {
                s.selection = ids.into_iter().map(SketchSel::Entity).collect();
            }
            s.preview = None;
            s.pending.clear();
            s.drag = None;
            let released = old_relations.iter().filter(|id| s.sketch.constraint(**id).is_none()).count();
            if matches!(operation, Operation::Fillet { .. } | Operation::Chamfer { .. } | Operation::Split { .. })
                && released > 0
            {
                let noun = if released == 1 { "relation" } else { "relations" };
                s.say(format!("{name} applied; released {released} {noun} from changed geometry"));
            } else {
                s.say(format!("{name} applied"));
            }
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
            if !s.trim_bounds.is_empty() && ui.button("Choose bounds again").clicked() {
                s.trim_bounds.clear();
                s.selection.clear();
                s.preview = None;
            }
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
            if let Some(why) = s.preview.as_ref().and_then(|p| p.refused.as_deref()) {
                ui.label(egui::RichText::new(why).size(11.0).color(theme::error()));
            }
        }
        Tool::Fillet => {
            hint(ui, "Pick each line or arc on the portion to keep. The preview inserts a tangent radius.");
            ui.add(egui::DragValue::new(&mut s.modify.fillet_radius).speed(0.25).prefix("Radius ").suffix(" mm"));
        }
        Tool::Chamfer => {
            hint(ui, "Pick each line on the portion to keep. Setbacks are measured from their corner.");
            let mut angle = matches!(s.modify.chamfer, ChamferSize::DistanceAngle { .. });
            if ui.checkbox(&mut angle, "Distance and angle").changed() {
                s.modify.chamfer = match s.modify.chamfer {
                    ChamferSize::Distances { first, .. } => {
                        ChamferSize::DistanceAngle { distance: first, angle_degrees: 45.0 }
                    }
                    ChamferSize::DistanceAngle { distance, .. } => {
                        ChamferSize::Distances { first: distance, second: distance }
                    }
                };
            }
            match &mut s.modify.chamfer {
                ChamferSize::Distances { first, second } => {
                    ui.add(egui::DragValue::new(first).speed(0.25).prefix("First setback ").suffix(" mm"));
                    ui.add(egui::DragValue::new(second).speed(0.25).prefix("Second setback ").suffix(" mm"));
                }
                ChamferSize::DistanceAngle { distance, angle_degrees } => {
                    ui.add(egui::DragValue::new(distance).speed(0.25).prefix("First setback ").suffix(" mm"));
                    ui.add(egui::DragValue::new(angle_degrees).speed(1.0).prefix("Angle ").suffix(" degrees"));
                }
            }
        }
        Tool::Split => {
            hint(
                ui,
                "Click inside a line or arc to split it. For a circle, pick two distinct points. No interval is removed.",
            );
        }
        Tool::Extend => {
            hint(ui, "Point at the end of a line or arc and click. It lengthens to the nearest curve in its way.");
        }
        Tool::Offset => {
            let editing = s.modify.offset_group.is_some();
            hint(
                ui,
                if editing {
                    "Change the linked distance, then Apply. The preview respects existing constraints. Esc cancels."
                } else {
                    "Pick a seed curve. Chain selection follows connected curves without branches. Drag to set the side and distance, or enter a distance."
                },
            );
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut s.modify.distance).speed(0.25).prefix("Distance ").suffix(" mm"));
                if ui.button("Reverse").clicked() {
                    s.modify.distance = -s.modify.distance;
                }
            });
            if !editing {
                ui.checkbox(&mut s.modify.offset.chain, "Chain selection");
                let both = ui.checkbox(&mut s.modify.offset.both_sides, "Both sides").changed();
                let old_cap = s.modify.offset.cap;
                egui::ComboBox::from_label("End caps")
                    .selected_text(match old_cap {
                        OffsetCap::None => "None",
                        OffsetCap::Lines => "Lines",
                        OffsetCap::Round => "Round",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut s.modify.offset.cap, OffsetCap::None, "None");
                        ui.selectable_value(&mut s.modify.offset.cap, OffsetCap::Lines, "Lines");
                        ui.selectable_value(&mut s.modify.offset.cap, OffsetCap::Round, "Round");
                    });
                if (both || old_cap != s.modify.offset.cap) && s.modify.offset.cap != OffsetCap::None {
                    s.modify.offset.make_base_construction = s.modify.offset.both_sides;
                }
                if s.modify.offset.cap == OffsetCap::None || s.modify.offset.both_sides {
                    ui.checkbox(&mut s.modify.offset.make_base_construction, "Make originals construction");
                }
                hint(ui, "Caps apply to open chains. One-sided caps use the originals as their other boundary.");
            }
        }
        Tool::Mirror => {
            hint(
                ui,
                "Select entities to reflect. Choose an axis line, or place two axis points. Apply adds the copies.",
            );
            let old_axis = s.modify.mirror_axis;
            egui::ComboBox::from_label("Mirror axis")
                .selected_text(s.modify.mirror_axis.map_or_else(|| "Two points".into(), |id| format!("Line {id}")))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.modify.mirror_axis, None, "Two points");
                    for e in s.sketch.drawn().filter(|e| e.is_line()) {
                        ui.selectable_value(&mut s.modify.mirror_axis, Some(e.id), format!("Line {}", e.id));
                    }
                });
            if s.modify.mirror_axis != old_axis {
                s.pending.clear();
                s.say(if s.modify.mirror_axis.is_some() {
                    "Mirror: select entities, then Apply"
                } else {
                    "Mirror: click the axis start"
                });
            }
            if let Some(entity) = s.modify.mirror_axis {
                if let Some(crate::sketch::EntityKind::Line { a, b }) = s.sketch.entity(entity).map(|e| e.kind) {
                    s.modify.axis_start = a;
                    s.modify.axis_end = b;
                }
            } else {
                point(ui, "Axis start", &mut s.modify.axis_start);
                point(ui, "Axis end", &mut s.modify.axis_end);
            }
            ui.checkbox(&mut s.modify.keep_symmetry, "Keep symmetry");
            if ui.add_enabled(s.modify.mirror_axis.is_some(), egui::Button::new("Start dynamic mirror")).clicked() {
                s.dynamic_mirror = s.modify.mirror_axis;
                s.set_tool(Tool::Line);
            }
            hint(ui, "Dynamic mirror draws reflected pairs about an existing line. Esc stops it.");
        }
        Tool::LinearPattern | Tool::CircularPattern => patterns::panel(ui, s),
        Tool::Move | Tool::Copy => {
            hint(
                ui,
                "Select entities, then click a base point and destination, or enter a displacement. Apply commits the preview.",
            );
            point(ui, "Base", &mut s.modify.base);
            point(ui, "Displacement", &mut s.modify.step);
        }
        Tool::Rotate => {
            hint(ui, "Select entities, click the centre, then enter the rotation angle and Apply.");
            point(ui, "Centre", &mut s.modify.center);
            ui.add(egui::DragValue::new(&mut s.modify.angle).speed(1.0).suffix(" degrees"));
            ui.checkbox(&mut s.modify.copy, "Copy entities");
        }
        Tool::Scale => {
            hint(ui, "Select entities, click the base point, then enter a positive scale factor and Apply.");
            point(ui, "Base", &mut s.modify.base);
            ui.add(egui::DragValue::new(&mut s.modify.factor).speed(0.05).prefix("Factor "));
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
        hint(
            ui,
            if tool == Tool::Offset {
                "Offsets retain their source links. Select a resulting curve or relation to edit the distance."
            } else if tool == Tool::Mirror && s.modify.keep_symmetry {
                "Mirrored entities keep symmetry about the chosen axis. Coordinates are u, v in mm."
            } else if matches!(tool, Tool::LinearPattern | Tool::CircularPattern) {
                "Instances follow their source geometry. Counts include the source."
            } else {
                "Coordinates are u, v in the sketch plane, in mm. Copies are independent; counts include the seed."
            },
        );
        if tool.transforms() {
            hint(
                ui,
                "In-place edits retain relations and driving dimensions. Incompatible transforms are refused. Copies retain construction state and join matching copied endpoints, without copying driving relations.",
            );
        }
        let selected = selected_in_order(s).len();
        hint(
            ui,
            &if s.modify.pattern_group.is_some() {
                format!("{selected} source entities. Esc or Cancel returns to Select.")
            } else {
                format!("{selected} selected. Ctrl-click adds or removes entities. Esc or Cancel returns to Select.")
            },
        );
        let preview = selection_preview(s);
        if let Some(why) = preview.as_ref().and_then(|p| p.refused.as_deref()) {
            ui.label(egui::RichText::new(why).size(11.0).color(theme::error()));
        }
        let operation = preview.and_then(|p| p.operation);
        ui.horizontal(|ui| {
            let apply_now = ui
                .add_enabled(operation.is_some(), egui::Button::new("Apply"))
                .on_hover_text("Apply the preview as one undo step")
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

/// Inspect a selected offset without turning every new Offset command into an edit.
pub fn offset_inspector(ui: &mut egui::Ui, s: &mut SketchSession) {
    if s.tool != Tool::Select {
        return;
    }
    let mut groups = Vec::new();
    for selection in &s.selection {
        for c in &s.sketch.constraints {
            let selected = match selection {
                SketchSel::Constraint(id) => c.id == *id,
                SketchSel::Entity(id) => c.entities().contains(id),
                SketchSel::Point(p) => c.entities().contains(&p.entity),
            };
            if selected
                && let ConstraintKind::Offset { group, .. } | ConstraintKind::OffsetCap { group, .. } = c.kind
                && !groups.contains(&group)
            {
                groups.push(group);
            }
        }
    }
    if let [group] = groups.as_slice()
        && let Some(distance) = s.sketch.offset_group_distance(*group)
    {
        theme::section_label(ui, "Linked offset");
        ui.label(format!("Distance {} mm", crate::sketch::fmt_value(distance)));
        if ui.button("Edit offset distance").clicked() {
            let group = *group;
            s.set_tool(Tool::Offset);
            s.modify.offset_group = Some(group);
            s.modify.distance = distance;
        }
    }
}

/// The same operations used by the sketcher's Modify tools, exposed to command/MCP scenarios.
pub fn operation_schema() -> Value {
    use soliddraft_assistant::tools::schema::{
        any_of, array, boolean, constant, integer, non_empty_array, number, object, one_of, optional,
    };
    let point = object(&[("u", number("Plane u, mm")), ("v", number("Plane v, mm"))]);
    let offset_options = optional(object(&[
        ("chain", optional(boolean("Follow the connected chain from the first seed, default false"))),
        ("both_sides", optional(boolean("Offset both sides, default false"))),
        ("cap", optional(one_of(&["none", "lines", "round"], "End caps on open chains, default none"))),
        ("make_base_construction", optional(boolean("Keep the originals as guides, default false"))),
    ]));
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
    let mut spline_points = non_empty_array(point.clone(), "At least two fit or control points in traversal order");
    spline_points["minItems"] = 2.into();
    let geometry = any_of(vec![
        object(&[
            ("type", constant("spline")),
            ("mode", one_of(&["fit", "control"], "Fit through points or use their control polygon")),
            ("points", spline_points),
        ]),
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
    let pattern_spec = any_of(vec![
        object(&[
            ("type", constant("linear")),
            ("step", point.clone()),
            ("count", integer("Positions in the first direction, including the source")),
            (
                "second",
                optional(object(&[("step", point.clone()), ("count", integer("Positions in the second direction"))])),
            ),
        ]),
        object(&[
            ("type", constant("circular")),
            ("center", point.clone()),
            ("angle_degrees", number("Angle per step or total span, degrees")),
            ("count", count.clone()),
            (
                "total_angle",
                optional(boolean("Distribute across the entire span; a full turn has no repeated endpoint")),
            ),
        ]),
    ]);
    let skipped = optional(array(
        non_empty_array(
            integer("Zero-based nonnegative position index"),
            "Exactly two indices [column, row]; circular patterns use row 0",
        ),
        "Distinct positions to skip; [0, 0] is the source and cannot be skipped; empty keeps all positions",
    ));
    let chamfer_size = any_of(vec![
        object(&[
            ("mode", constant("distances")),
            ("first", number("First positive setback, mm")),
            ("second", number("Second positive setback, mm")),
        ]),
        object(&[
            ("mode", constant("distance_angle")),
            ("distance", number("First positive setback, mm")),
            ("angle_degrees", number("Acute angle to the first line, degrees")),
        ]),
    ]);
    any_of(vec![
        object(&[("tool", constant("relate")), ("relation", relation)]),
        object(&[
            ("tool", constant("fillet")),
            ("a", id.clone()),
            ("b", id.clone()),
            ("keep_a", point.clone()),
            ("keep_b", point.clone()),
            ("radius", number("Positive radius, mm")),
        ]),
        object(&[
            ("tool", constant("chamfer")),
            ("a", id.clone()),
            ("b", id.clone()),
            ("keep_a", point.clone()),
            ("keep_b", point.clone()),
            ("size", chamfer_size),
        ]),
        object(&[
            ("tool", constant("split")),
            ("entity", id.clone()),
            ("at", point.clone()),
            ("second", optional(point.clone())),
        ]),
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
            ("options", offset_options),
        ]),
        object(&[
            ("tool", constant("edit_offset")),
            ("group", integer("An existing offset group constraint id")),
            ("distance", number("New signed offset distance, mm")),
        ]),
        object(&[
            ("tool", constant("mirror_linked")),
            ("entities", ids.clone()),
            (
                "axis",
                any_of(vec![
                    object(&[("type", constant("entity")), ("entity", integer("Existing axis line id"))]),
                    object(&[("type", constant("points")), ("start", point.clone()), ("end", point.clone())]),
                ]),
            ),
        ]),
        object(&[
            ("tool", constant("mirror")),
            ("entities", ids.clone()),
            ("axis_start", point.clone()),
            ("axis_end", point.clone()),
        ]),
        object(&[
            ("tool", constant("pattern")),
            ("entities", ids.clone()),
            ("spec", pattern_spec.clone()),
            ("skipped", skipped.clone()),
        ]),
        object(&[
            ("tool", constant("edit_pattern")),
            ("group", integer("Retained sketch pattern id")),
            ("spec", pattern_spec),
            ("skipped", skipped),
        ]),
        object(&[
            ("tool", constant("linear_pattern")),
            ("entities", ids.clone()),
            ("step", point.clone()),
            ("count", count.clone()),
        ]),
        object(&[
            ("tool", constant("circular_pattern")),
            ("entities", ids.clone()),
            ("center", point.clone()),
            ("angle_degrees", number("Signed angle per instance, degrees")),
            ("count", count),
        ]),
        object(&[("tool", constant("move")), ("entities", ids.clone()), ("displacement", point.clone())]),
        object(&[
            ("tool", constant("copy")),
            ("entities", ids.clone()),
            ("base", point.clone()),
            ("displacement", point.clone()),
        ]),
        object(&[
            ("tool", constant("rotate")),
            ("entities", ids.clone()),
            ("center", point.clone()),
            ("angle_degrees", number("Signed rotation, degrees")),
            ("copy", optional(boolean("Add independent copies, default false"))),
        ]),
        object(&[
            ("tool", constant("scale")),
            ("entities", ids),
            ("base", point),
            ("factor", number("Positive uniform scale factor, other than one")),
        ]),
    ])
}
