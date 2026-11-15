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

/// What the sketcher offers at the cursor, given what it inferred there.
pub fn offered(auto_relations: bool, raw: P2, inferred: (P2, Vec<Snap>)) -> (P2, Vec<Snap>) {
    if auto_relations { inferred } else { (raw, Vec::new()) }
}

/// Snap and construction controls, in the options bar beside the sketch's name, so they
/// never cover the sketch. With the Polygon tool active its sides and its inscribed or
/// circumscribed circle come first: they shape the next polygon.
pub fn bar(ui: &mut egui::Ui, s: &mut SketchSession) {
    if s.tool == Tool::Polygon {
        crate::sketch_tools::polygon_options(ui, s);
        ui.separator();
    }
    ui.checkbox(&mut s.auto_relations, "Auto relations");
    let centreline = s.tool == Tool::Centreline;
    let mut construction = s.draws_construction();
    ui.add_enabled(!centreline, egui::Checkbox::new(&mut construction, "Draw construction")).on_disabled_hover_text(
        "Centrelines always use construction geometry. Other tools keep their drawing setting.",
    );
    if !centreline {
        s.construction = construction;
    }
    ui.checkbox(&mut s.grid_snap, "Snap to grid").on_hover_text(
        "Use the current minor grid spacing. Geometry snaps and explicit Quick snap choices take priority.",
    );
    if s.tool.places_points() {
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
                    })
                    .inner
                    .on_disabled_hover_text(
                        "Place a start point first. Parallel, perpendicular and tangent snaps require a line endpoint.",
                    );
                }
            })
            .response
            .on_hover_text("Use this snap class for the next click. Escape or a tool change clears it.");
    }
}
