//! The sweep property page: a chain of lines and arcs, and the edits that keep it valid.

use soliddraft_kernel_api::{ArcTurn, PathSegment, Sweep, SweepOrientation};

use crate::theme;

const STEP_MM: f64 = 10.0;

/// A label an accessibility tree can tell apart, keeping "sweep end Z" for the first piece.
fn field(index: usize, what: &str) -> String {
    if index == 0 { format!("sweep {what}") } else { format!("sweep {} {what}", index + 1) }
}

fn coords(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    value: &mut [f64; 3],
    coord: impl Fn(&mut egui::Ui, &str, &str, &mut f64),
) {
    theme::section_label(ui, title);
    ui.horizontal(|ui| {
        for (i, label) in ["X", "Y", "Z"].iter().enumerate() {
            coord(ui, &format!("{id} {label}"), label, &mut value[i]);
        }
    });
}

/// Keeps a typed coordinate exact: display math must not add binary noise to a saved
/// value, because the path's joins and arcs are compared exactly.
fn clean(value: [f64; 3]) -> [f64; 3] {
    value.map(|v| (v * 1e6).round() / 1e6 + 0.0)
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 0.0).then(|| v.map(|x| x / length))
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// A direction square to `tangent`, along a model axis where one is.
fn square_to(tangent: [f64; 3]) -> [f64; 3] {
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let square = |axis: &[f64; 3]| axis.iter().zip(&tangent).map(|(a, t)| a * t).sum::<f64>() == 0.0;
    axes.into_iter().find(square).unwrap_or_else(|| {
        let least = (0..3).min_by(|a, b| tangent[*a].abs().total_cmp(&tangent[*b].abs())).unwrap_or(0);
        clean(cross(tangent, axes[least]))
    })
}

/// Appends a line carrying on in the direction the path ends in.
pub fn add_line(path: &mut Sweep) {
    let direction = path.end_tangent().and_then(unit).unwrap_or([0.0, 0.0, 1.0]);
    let end = path.end();
    let to = clean(std::array::from_fn(|i| end[i] + direction[i] * STEP_MM));
    path.segments.push(PathSegment::Line { id: path.next_segment_id(), to, corner_axis: None });
    keep_valid(path);
}

/// Appends a quarter arc tangent to the path's end. Its centre is `radius` from the path,
/// so a profile within that distance of the path stays on one side of the arc's axis.
pub fn add_arc(path: &mut Sweep, radius: f64) {
    let direction = path.end_tangent().and_then(unit).unwrap_or([0.0, 0.0, 1.0]);
    let axis = square_to(direction);
    let toward = cross(direction, unit(axis).unwrap_or(axis));
    let end = path.end();
    let centre = clean(std::array::from_fn(|i| end[i] - toward[i] * radius));
    path.segments.push(PathSegment::Arc {
        id: path.next_segment_id(),
        centre,
        axis,
        turn: ArcTurn::Quarter,
        corner_axis: None,
    });
    // Only a profile that turns can follow a circle.
    path.orientation = SweepOrientation::FollowPath;
    keep_valid(path);
}

/// Removes a piece. A path keeps at least one.
pub fn remove(path: &mut Sweep, index: usize) {
    if path.segments.len() > 1 && index < path.segments.len() {
        path.segments.remove(index);
        keep_valid(path);
    }
}

/// A turn axis belongs exactly at the joins where a turning profile doubles back.
pub fn keep_valid(path: &mut Sweep) {
    let turning = path.orientation == SweepOrientation::FollowPath;
    for index in 0..path.segments.len() {
        if turning && path.reverses_at(index) {
            if path.segments[index].corner_axis().is_none() {
                let axis = path.start_tangent(index).map(square_to);
                path.segments[index].set_corner_axis(axis);
            }
        } else {
            path.segments[index].set_corner_axis(None);
        }
    }
}

/// The path controls. `radius` is the default for a new arc.
pub fn page(
    ui: &mut egui::Ui,
    path: &mut Sweep,
    radius: f64,
    coord: impl Fn(&mut egui::Ui, &str, &str, &mut f64) + Copy,
) {
    let arcs = path.segments.iter().any(|s| matches!(s, PathSegment::Arc { .. }));
    theme::section_label(ui, "Profile orientation");
    if arcs {
        ui.add_enabled(false, egui::RadioButton::new(false, "Keep direction · Coming soon"));
    } else {
        ui.radio_value(&mut path.orientation, SweepOrientation::Fixed, "Keep direction");
    }
    ui.radio_value(&mut path.orientation, SweepOrientation::FollowPath, "Turn with the path");
    coords(ui, "sweep start", "Path start", &mut path.start, coord);
    let mut removed = None;
    let many = path.segments.len() > 1;
    let ends = path.ends();
    let reversals: Vec<bool> = (0..path.segments.len()).map(|i| path.reverses_at(i)).collect();
    let turning = path.orientation == SweepOrientation::FollowPath;
    for (index, segment) in path.segments.iter_mut().enumerate() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let kind = if matches!(segment, PathSegment::Line { .. }) { "line" } else { "arc" };
            ui.label(egui::RichText::new(format!("Segment {}: {kind}", index + 1)).size(12.0));
            if many && ui.small_button(format!("Remove segment {}", index + 1)).clicked() {
                removed = Some(index);
            }
        });
        match segment {
            PathSegment::Line { to, .. } => coords(ui, &field(index, "end"), "End", to, coord),
            PathSegment::Arc { centre, axis, turn, .. } => {
                coords(ui, &field(index, "centre"), "Centre", centre, coord);
                coords(ui, &field(index, "axis"), "Axis", axis, coord);
                theme::section_label(ui, "Turn");
                ui.horizontal(|ui| {
                    ui.selectable_value(turn, ArcTurn::Quarter, "Quarter");
                    ui.selectable_value(turn, ArcTurn::Half, "Half");
                    ui.selectable_value(turn, ArcTurn::ThreeQuarter, "Three quarters");
                });
                let [x, y, z] = ends[index].map(crate::document::fmt_mm);
                ui.label(egui::RichText::new(format!("Ends at {x}, {y}, {z} mm")).size(11.0).color(theme::muted()));
            }
        }
        if turning && reversals[index] {
            let mut axis = segment.corner_axis().unwrap_or([1.0, 0.0, 0.0]);
            coords(ui, &field(index, "turn axis"), "Turn axis where the path doubles back", &mut axis, coord);
            segment.set_corner_axis(Some(axis));
        }
    }
    if let Some(index) = removed {
        remove(path, index);
    }
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Add line").clicked() {
            add_line(path);
        }
        if ui.button("Add arc").clicked() {
            add_arc(path, radius);
        }
        ui.add_enabled(false, egui::Button::new("Spline · Coming soon"));
        ui.add_enabled(false, egui::Button::new("Helix · Coming soon"));
    });
    keep_valid(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight() -> Sweep {
        Sweep::line([9.0, -2.0, 11.0], [9.0, -2.0, 21.0], "travel")
    }

    #[test]
    fn new_pieces_continue_the_path_and_keep_saved_identities() {
        let mut path = straight();
        add_line(&mut path);
        add_arc(&mut path, 5.0);
        add_line(&mut path);
        let ids: Vec<_> = path.segments.iter().map(PathSegment::id).collect();
        assert_eq!(ids, ["travel", "segment2", "segment3", "segment4"]);
        assert_eq!(path.ends()[1], [9.0, -2.0, 31.0]);
        let PathSegment::Arc { centre, axis, .. } = &path.segments[2] else { panic!("an arc") };
        assert_eq!((*centre, *axis), ([9.0, -7.0, 31.0], [1.0, 0.0, 0.0]));
        assert!(path.ends()[2].iter().zip(&[9.0, -7.0, 36.0]).all(|(a, b)| (a - b).abs() < 1e-9));
        assert_eq!(path.orientation, SweepOrientation::FollowPath);
        remove(&mut path, 1);
        let ids: Vec<_> = path.segments.iter().map(PathSegment::id).collect();
        assert_eq!(ids, ["travel", "segment3", "segment4"]);
    }

    #[test]
    fn the_last_piece_cannot_be_removed() {
        let mut path = straight();
        remove(&mut path, 0);
        assert_eq!(path, straight());
    }

    #[test]
    fn a_turn_axis_appears_at_a_reversal_only_while_the_profile_turns() {
        let mut path = straight();
        path.orientation = SweepOrientation::FollowPath;
        path.segments.push(PathSegment::Line { id: "back".into(), to: [9.0, -2.0, 15.0], corner_axis: None });
        keep_valid(&mut path);
        assert_eq!(path.segments[1].corner_axis(), Some([1.0, 0.0, 0.0]));
        path.segments[1].set_corner_axis(Some([0.0, 1.0, 0.0]));
        keep_valid(&mut path);
        assert_eq!(path.segments[1].corner_axis(), Some([0.0, 1.0, 0.0]), "a chosen axis stays");
        path.orientation = SweepOrientation::Fixed;
        keep_valid(&mut path);
        assert_eq!(path.segments[1].corner_axis(), None);
    }

    #[test]
    fn saved_coordinates_stay_clean_decimals() {
        let mut path = Sweep::line([0.1, 0.2, 0.0], [0.1, 0.2, 0.3], "travel");
        add_line(&mut path);
        assert_eq!(path.ends()[1], [0.1, 0.2, 10.3]);
        assert_eq!(clean([0.1 + 0.2, -0.0, 1.0]), [0.3, 0.0, 1.0]);
    }
}
