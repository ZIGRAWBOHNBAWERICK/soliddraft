//! Box selection in a sketch, following SolidWorks' Box Selection and Cross Selection
//! help pages (2025). Dragging from left to right draws a solid box that selects only
//! what lies completely inside it. Dragging from right to left draws a dotted box that
//! also selects what crosses its boundary. Both take sketch entities and dimensions.
//! Shift adds everything in the box; Ctrl inverts the selection within the box; either
//! way the selection outside the box stays as it was.

use super::Frame;
use crate::sketch::{ConstraintId, Sketch};
use crate::state::SketchSel;
use crate::theme;
use egui::{CornerRadius, Modifiers, Pos2, Rect, Stroke, StrokeKind};

/// Which items a selection box takes, from the direction it was dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxKind {
    /// Dragged left to right: only items completely inside.
    Window,
    /// Dragged right to left: items inside and items crossing the boundary.
    Crossing,
}

/// A selection box being dragged in the sketch, in screen points.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectionBox {
    pub start: Pos2,
    pub end: Pos2,
    /// How the box combines with the selection, from the modifiers at the press.
    pub combine: Combine,
    /// What the box would select if released now.
    pub picks: Vec<SketchSel>,
}

impl SelectionBox {
    pub fn new(start: Pos2, combine: Combine) -> Self {
        Self { start, end: start, combine, picks: Vec::new() }
    }

    pub fn kind(&self) -> BoxKind {
        if self.end.x >= self.start.x { BoxKind::Window } else { BoxKind::Crossing }
    }

    pub fn rect(&self) -> Rect {
        Rect::from_two_pos(self.start, self.end)
    }
}

/// How newly picked items join the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    /// The picks become the selection.
    Replace,
    /// The picks join the selection; nothing leaves it.
    Add,
    /// Each pick leaves the selection if it was in it and joins it otherwise.
    Toggle,
}

impl Combine {
    /// Shift adds and Ctrl (Cmd on a Mac) toggles, as in SolidWorks and a file manager.
    pub fn of(modifiers: Modifiers) -> Self {
        if modifiers.shift {
            Combine::Add
        } else if modifiers.ctrl || modifiers.command {
            Combine::Toggle
        } else {
            Combine::Replace
        }
    }

    /// Changes `selection` by `picks`, keeping the selection order of what stays.
    pub fn apply(self, selection: &mut Vec<SketchSel>, picks: &[SketchSel]) {
        match self {
            Combine::Replace => *selection = picks.to_vec(),
            Combine::Add => {
                for p in picks {
                    if !selection.contains(p) {
                        selection.push(*p);
                    }
                }
            }
            Combine::Toggle => {
                for p in picks {
                    match selection.iter().position(|s| s == p) {
                        Some(i) => {
                            selection.remove(i);
                        }
                        None => selection.push(*p),
                    }
                }
            }
        }
    }
}

/// The sketch entities and dimensions that `rect` selects as a box of `kind`. `labels`
/// are the dimension labels on screen; a dimension is picked by its label.
pub fn targets(
    frame: &Frame<'_>,
    sketch: &Sketch,
    labels: &[(ConstraintId, Rect)],
    rect: Rect,
    kind: BoxKind,
) -> Vec<SketchSel> {
    let mut picks = Vec::new();
    for e in sketch.drawn() {
        let paths: Vec<Vec<Pos2>> = e
            .polylines(64)
            .into_iter()
            .map(|path| path.into_iter().filter_map(|p| frame.screen(p)).collect())
            .filter(|path: &Vec<Pos2>| !path.is_empty())
            .collect();
        if paths.is_empty() {
            continue;
        }
        let taken = match kind {
            BoxKind::Window => paths.iter().flatten().all(|p| rect.contains(*p)),
            BoxKind::Crossing => paths.iter().any(|path| touches(rect, path)),
        };
        if taken {
            picks.push(SketchSel::Entity(e.id));
        }
    }
    for (id, label) in labels {
        let taken = match kind {
            BoxKind::Window => rect.contains_rect(*label),
            BoxKind::Crossing => rect.intersects(*label),
        };
        if taken {
            picks.push(SketchSel::Constraint(*id));
        }
    }
    picks
}

/// Whether a polyline meets the rectangle anywhere.
fn touches(rect: Rect, path: &[Pos2]) -> bool {
    match path {
        [] => false,
        [p] => rect.contains(*p),
        _ => path.windows(2).any(|w| segment_meets(rect, w[0], w[1])),
    }
}

/// Whether the segment from `a` to `b` meets the rectangle (Liang-Barsky clipping).
fn segment_meets(rect: Rect, a: Pos2, b: Pos2) -> bool {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, q) in [(-d.x, a.x - rect.min.x), (d.x, rect.max.x - a.x), (-d.y, a.y - rect.min.y), (d.y, rect.max.y - a.y)]
    {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return false;
                }
                t1 = t1.min(r);
            }
        }
    }
    true
}

/// Draws the box: solid for a window, dotted for a crossing box.
pub fn paint(painter: &egui::Painter, b: &SelectionBox) {
    let rect = b.rect();
    let accent = theme::accent();
    painter.rect_filled(rect, CornerRadius::ZERO, accent.gamma_multiply(0.08));
    let stroke = Stroke::new(1.0, accent);
    match b.kind() {
        BoxKind::Window => {
            painter.rect_stroke(rect, CornerRadius::ZERO, stroke, StrokeKind::Inside);
        }
        BoxKind::Crossing => {
            let outline = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
            painter.extend(egui::Shape::dashed_line(&outline, stroke, 4.0, 3.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn the_drag_direction_picks_the_box_kind() {
        let mut b = SelectionBox::new(pos2(10.0, 10.0), Combine::Replace);
        b.end = pos2(50.0, 0.0);
        assert_eq!(b.kind(), BoxKind::Window);
        b.end = pos2(0.0, 40.0);
        assert_eq!(b.kind(), BoxKind::Crossing);
        assert_eq!(b.rect(), Rect::from_min_max(pos2(0.0, 10.0), pos2(10.0, 40.0)));
    }

    #[test]
    fn a_segment_meets_a_box_it_crosses_without_an_end_inside() {
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0));
        assert!(segment_meets(rect, pos2(-5.0, 5.0), pos2(15.0, 5.0)));
        assert!(segment_meets(rect, pos2(5.0, 5.0), pos2(6.0, 6.0)));
        assert!(!segment_meets(rect, pos2(-5.0, -5.0), pos2(-1.0, 20.0)));
        assert!(!segment_meets(rect, pos2(11.0, 0.0), pos2(20.0, 9.0)));
        assert!(!segment_meets(rect, pos2(-5.0, 12.0), pos2(15.0, 12.0)));
    }

    #[test]
    fn modifiers_choose_how_the_box_combines() {
        let (a, b, c) = (SketchSel::Entity(1), SketchSel::Entity(2), SketchSel::Constraint(3));
        assert_eq!(Combine::of(Modifiers::NONE), Combine::Replace);
        assert_eq!(Combine::of(Modifiers::SHIFT), Combine::Add);
        assert_eq!(Combine::of(Modifiers::CTRL), Combine::Toggle);
        let mut selection = vec![a, b];
        Combine::Add.apply(&mut selection, &[b, c]);
        assert_eq!(selection, vec![a, b, c]);
        Combine::Toggle.apply(&mut selection, &[b, SketchSel::Entity(4)]);
        assert_eq!(selection, vec![a, c, SketchSel::Entity(4)]);
        Combine::Replace.apply(&mut selection, &[]);
        assert!(selection.is_empty());
    }
}
