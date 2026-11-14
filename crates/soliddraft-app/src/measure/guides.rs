//! Auto relations around the sketcher's inference, and drawing new entities as
//! construction geometry.
//!
//! With auto relations on, the sketcher offers its usual inferences in their usual
//! order (points, intersections, midpoints, directions, curves). A click keeps the
//! relation it showed. Auto relations off stops ordinary inference; an explicit quick
//! snap overrides it for one placement. Existing and manual relations still solve.
//! These controls change interaction state without editing the document or its history.

use crate::sketch::P2;
use crate::state::{SketchSession, Snap, SnapFilter, Tool};
use egui::{Rect, pos2, vec2};

/// What the sketcher offers at the cursor, given what it inferred there.
pub fn offered(auto_relations: bool, raw: P2, inferred: (P2, Vec<Snap>)) -> (P2, Vec<Snap>) {
    if auto_relations { inferred } else { (raw, Vec::new()) }
}

/// Snap and construction controls, under the sketch status in the viewport's corner.
pub fn switches(ui: &mut egui::Ui, s: &mut SketchSession, rect: Rect) {
    let at = Rect::from_min_size(pos2(rect.left() + 12.0, rect.top() + 58.0), vec2(150.0, 22.0));

    let height = if s.tool.places_points() { 94.0 } else { 70.0 };
    let controls = Rect::from_min_size(at.min, vec2(at.width(), height)).expand(4.0);
    ui.painter().rect_filled(controls, 6, crate::theme::hud_bg());

    ui.put(at, egui::Checkbox::new(&mut s.auto_relations, "Auto relations"));
    let centreline = s.tool == Tool::Centreline;
    let mut construction = s.draws_construction();
    ui.add_enabled_ui(!centreline, |ui| {
        ui.put(at.translate(vec2(0.0, 24.0)), egui::Checkbox::new(&mut construction, "Draw construction"))
    })
    .inner
    .on_disabled_hover_text("Centrelines always use construction geometry. Other tools keep their drawing setting.");
    if !centreline {
        s.construction = construction;
    }
    ui.put(at.translate(vec2(0.0, 48.0)), egui::Checkbox::new(&mut s.grid_snap, "Snap to grid")).on_hover_text(
        "Use the current minor grid spacing. Geometry snaps and explicit Quick snap choices take priority.",
    );
    if s.tool.places_points() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(at.translate(vec2(0.0, 72.0))), |ui| {
            egui::ComboBox::from_id_salt("quick-snap")
                .height(280.0)
                .selected_text(s.quick_snap.map_or("Quick snap", SnapFilter::label))
                .show_ui(ui, |ui| {
                    for filter in SnapFilter::ALL {
                        let enabled = match filter {
                            SnapFilter::Axes => !s.pending.is_empty(),
                            SnapFilter::Parallel | SnapFilter::Perpendicular | SnapFilter::Tangent => {
                                s.tool.draws_line() && !s.pending.is_empty()
                            }
                            _ => true,
                        };
                        ui.add_enabled_ui(enabled, |ui| {
                            ui.selectable_value(&mut s.quick_snap, Some(filter), filter.label())
                        }).inner.on_disabled_hover_text("Place a start point first. Parallel, perpendicular and tangent snaps require a line endpoint.");
                    }
                })
                .response
                .on_hover_text("Use this snap class for the next click. Escape or a tool change clears it.");
        });
    }
}
