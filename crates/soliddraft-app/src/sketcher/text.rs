//! Text placement and buffered, atomic property edits.

use super::*;
use crate::sketch::{SketchText, TextFont};

pub(super) fn place(s: &mut SketchSession, position: P2, snaps: Vec<Snap>, actions: &mut Vec<AppAction>) {
    let mut text = s.text.clone();
    text.position = position;
    if let Err(error) = text.outlines() {
        s.say(error);
        return;
    }
    let before = s.sketch.clone();
    let id = s.sketch.add(EntityKind::Text { text }, s.construction);
    apply_snaps(&mut s.sketch, PointRef::new(id, 0), &snaps, None);
    solve::solve(&mut s.sketch, None, true);
    if matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }) {
        s.sketch = before;
        s.say("Constraints prevent this text placement");
        return;
    }
    s.set_tool(Tool::Select);
    s.selection = vec![SketchSel::Entity(id)];
    commit(s, actions);
}

fn fields(ui: &mut egui::Ui, text: &mut SketchText) {
    let label = ui.label("Content");
    ui.add(
        egui::TextEdit::singleline(&mut text.content)
            .char_limit(256)
            .hint_text("Sketch text")
            .id_salt("sketch_text_content"),
    )
    .labelled_by(label.id);
    egui::ComboBox::from_label("Font").selected_text(text.font.label()).show_ui(ui, |ui| {
        for font in [TextFont::InterRegular, TextFont::InterMedium] {
            ui.selectable_value(&mut text.font, font, font.label());
        }
    });
    ui.horizontal(|ui| {
        let label = ui.label("Height");
        ui.add(egui::DragValue::new(&mut text.height).speed(0.1).suffix(" mm"))
            .labelled_by(label.id)
            .on_hover_text("Capital letter height in millimetres");
    });
    ui.horizontal(|ui| {
        let label = ui.label("Baseline X");
        ui.add(egui::DragValue::new(&mut text.position.u).speed(0.1)).labelled_by(label.id);
        let label = ui.label("Y");
        ui.add(egui::DragValue::new(&mut text.position.v).speed(0.1)).labelled_by(label.id);
    });
    ui.horizontal(|ui| {
        let label = ui.label("Angle");
        let mut degrees = text.angle.to_degrees();
        if ui.add(egui::DragValue::new(&mut degrees).speed(1.0).suffix("°")).labelled_by(label.id).changed() {
            text.angle = degrees.to_radians();
        }
    });
}

pub(crate) fn panel(ui: &mut egui::Ui, s: &mut SketchSession, actions: &mut Vec<AppAction>) {
    if s.tool == Tool::Text {
        theme::section_label(ui, "Sketch text");
        fields(ui, &mut s.text);
        if let Err(error) = s.text.outlines() {
            ui.colored_label(theme::error(), error);
        }
        ui.label("Click in the sketch to place the baseline origin.");
        return;
    }
    let selected = match s.selection.as_slice() {
        [SketchSel::Entity(id)] => Some(*id),
        [SketchSel::Point(point)] => Some(point.entity),
        _ => None,
    };
    let selected = selected.and_then(|id| match &s.sketch.entity(id)?.kind {
        EntityKind::Text { text } => Some((id, text.clone())),
        _ => None,
    });
    let Some((id, saved)) = selected else {
        s.text_edit = None;
        return;
    };
    if s.text_edit.as_ref().is_none_or(|edit| edit.entity != id || edit.original != saved) {
        s.text_edit = Some(crate::state::TextEdit { entity: id, original: saved.clone(), draft: saved.clone() });
    }
    theme::section_label(ui, "Sketch text");
    let draft = &mut s.text_edit.as_mut().unwrap().draft;
    fields(ui, draft);
    let error = draft.outlines().err();
    if let Some(error) = &error {
        ui.colored_label(theme::error(), error);
    }
    let changed = *draft != saved;
    let apply = ui.add_enabled(changed && error.is_none(), egui::Button::new("Apply text")).clicked();
    let reset = ui.add_enabled(changed, egui::Button::new("Reset text")).clicked();
    if apply {
        let text = draft.clone();
        let before = s.sketch.clone();
        let position = text.position;
        s.sketch.entity_mut(id).unwrap().kind = EntityKind::Text { text };
        solve::solve(&mut s.sketch, None, true);
        if matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. })
            || s.sketch.point(PointRef::new(id, 0)).is_none_or(|actual| actual.dist(position) > 1e-6)
        {
            s.sketch = before;
            s.say("Constraints prevent this text edit");
        } else {
            commit(s, actions);
        }
    } else if reset {
        s.text_edit = None;
    }
}
