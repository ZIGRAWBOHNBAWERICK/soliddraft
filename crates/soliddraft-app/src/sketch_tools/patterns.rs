//! Pattern parameters and access to retained definitions.
use super::*;
use crate::sketch::tools::{PatternDirection, PatternSpec};

pub(crate) fn spec(s: &SketchSession) -> PatternSpec {
    let m = &s.modify;
    if s.tool == Tool::CircularPattern {
        PatternSpec::Circular { center: m.center, angle_degrees: m.angle, count: m.count, total_angle: m.total_angle }
    } else {
        PatternSpec::Linear {
            step: m.step,
            count: m.count,
            second: m.two_directions.then_some(PatternDirection { step: m.second_step, count: m.second_count }),
        }
    }
}

pub(crate) fn operation(s: &SketchSession, entities: Vec<EntityId>) -> Option<Operation> {
    let spec = spec(s);
    if let Some(group) = s.modify.pattern_group {
        Some(Operation::EditPattern { group, spec, skipped: s.modify.skipped.clone() })
    } else if entities.is_empty() {
        None
    } else {
        Some(Operation::Pattern { entities, spec, skipped: s.modify.skipped.clone() })
    }
}

pub(crate) fn preview(s: &SketchSession, operation: Operation) -> ToolPreview {
    let mut copy = s.sketch.clone();
    match copy.apply_operation(&operation) {
        Ok(_) => {
            let replaced = s
                .sketch
                .drawn()
                .filter(|old| {
                    copy.entity(old.id).is_none_or(|new| old.kind != new.kind || old.construction != new.construction)
                })
                .map(|e| e.id)
                .collect();
            let changed: Vec<_> = copy
                .drawn()
                .filter(|new| {
                    s.sketch
                        .entity(new.id)
                        .is_none_or(|old| old.kind != new.kind || old.construction != new.construction)
                })
                .collect();
            ToolPreview {
                added: changed.iter().map(|e| e.kind).collect(),
                construction: changed.iter().map(|e| e.construction).collect(),
                replaced,
                operation: Some(operation),
                ..Default::default()
            }
        }
        Err(error) => ToolPreview { refused: Some(error.to_string()), ..Default::default() },
    }
}

pub(crate) fn editors(ui: &mut egui::Ui, s: &mut SketchSession) {
    if s.tool != Tool::Select || s.sketch.patterns.is_empty() {
        return;
    }
    let selected = selected_in_order(s);
    let rows: Vec<_> = s
        .sketch
        .patterns
        .iter()
        .filter(|p| {
            selected.is_empty()
                || p.seeds.iter().chain(p.instances.iter().flat_map(|i| &i.entities)).any(|id| selected.contains(id))
        })
        .cloned()
        .collect();
    if rows.is_empty() {
        return;
    }
    theme::section_label(ui, "Sketch patterns");
    for p in rows {
        let kind = if matches!(p.spec, PatternSpec::Circular { .. }) { "Circular" } else { "Linear" };
        hint(ui, &format!("{kind} pattern {} - {} generated, {} skipped", p.id, p.instances.len(), p.skipped.len()));
        if ui.button(format!("Edit pattern {}", p.id)).clicked() {
            s.set_tool(if matches!(p.spec, PatternSpec::Circular { .. }) {
                Tool::CircularPattern
            } else {
                Tool::LinearPattern
            });
            s.modify.pattern_group = Some(p.id);
            s.modify.skipped = p.skipped;
            match p.spec {
                PatternSpec::Linear { step, count, second } => {
                    s.modify.step = step;
                    s.modify.count = count;
                    s.modify.two_directions = second.is_some();
                    if let Some(second) = second {
                        s.modify.second_step = second.step;
                        s.modify.second_count = second.count;
                    }
                }
                PatternSpec::Circular { center, angle_degrees, count, total_angle } => {
                    s.modify.center = center;
                    s.modify.angle = angle_degrees;
                    s.modify.count = count;
                    s.modify.total_angle = total_angle;
                }
            }
            s.selection = p.seeds.into_iter().map(SketchSel::Entity).collect();
            s.say(format!("Editing pattern {}", p.id));
        }
    }
}

fn number(ui: &mut egui::Ui, name: &str, value: &mut f64) {
    let response = ui.add(egui::DragValue::new(value).speed(0.25).prefix(format!("{name} ")));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, name));
}
fn count(ui: &mut egui::Ui, name: &str, value: &mut u32) {
    let response = ui.add(egui::DragValue::new(value).range(1..=1000).prefix(format!("{name} ")));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, name));
}

pub(crate) fn panel(ui: &mut egui::Ui, s: &mut SketchSession) {
    if s.tool == Tool::LinearPattern {
        hint(ui, "Click two points to place the first step, or enter its components. Counts include the seed.");
        number(ui, "First step u", &mut s.modify.step.u);
        number(ui, "First step v", &mut s.modify.step.v);
        count(ui, "First count", &mut s.modify.count);
        ui.checkbox(&mut s.modify.two_directions, "Second direction");
        if s.modify.two_directions {
            number(ui, "Second step u", &mut s.modify.second_step.u);
            number(ui, "Second step v", &mut s.modify.second_step.v);
            count(ui, "Second count", &mut s.modify.second_count);
        }
    } else {
        hint(ui, "Click to place the centre. Counts include the seed; a full span omits its repeated endpoint.");
        number(ui, "Centre u", &mut s.modify.center.u);
        number(ui, "Centre v", &mut s.modify.center.v);
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut s.modify.total_angle, false, "Angle per step");
            ui.selectable_value(&mut s.modify.total_angle, true, "Total angle");
        });
        number(ui, "Pattern angle", &mut s.modify.angle);
        count(ui, "Pattern count", &mut s.modify.count);
    }
    if let Some(group) = s.modify.pattern_group {
        hint(ui, &format!("Editing pattern {group}. Its source selection stays fixed."));
    }
    let dimensions = spec(s);
    if let Ok(positions) = dimensions.positions() {
        s.modify.skipped.retain(|p| positions.contains(p) && *p != [0, 0]);
        ui.collapsing("Skip instances", |ui| {
            hint(ui, "Uncheck an instance to omit it. The first instance is the source.");
            for position in positions {
                let mut included = !s.modify.skipped.contains(&position);
                let label = if s.tool == Tool::CircularPattern {
                    format!("Instance {}", position[0] + 1)
                } else {
                    format!("Instance {}, {}", position[0] + 1, position[1] + 1)
                };
                if ui.add_enabled(position != [0, 0], egui::Checkbox::new(&mut included, label)).changed() {
                    if included {
                        s.modify.skipped.retain(|p| *p != position);
                    } else {
                        s.modify.skipped.push(position);
                    }
                }
            }
        });
    }
}
