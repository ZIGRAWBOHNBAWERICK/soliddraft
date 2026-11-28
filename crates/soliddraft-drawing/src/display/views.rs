//! What derived views add to a sheet: hatching of cut faces, the boundaries of details
//! and broken-out sections, the marks they leave on their parents (a section's cutting
//! line, a detail's circle, an auxiliary view's arrow) and the location labels that
//! point from one to the other when they are on different sheets.

use kurbo::{Affine, BezPath, Vec2};
use soliddraft_kernel_api::Point2;

use super::{Context, HatchRecord, Item, Owner, PathItem, Role, TextItem, VAlign, add, arrow, line, polyline, sub};
use crate::Drawing;
use crate::ids::ViewId;
use crate::library::{self, Align, HatchPattern, TemplateFile, Weight};
use crate::project::{clip_path, section_arrow, view_bounds};
use crate::view::{Boundary, View, ViewKind};

/// Where a derived view and the mark it leaves on its parent are, so each can name the
/// other's sheet and zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocationLabel {
    /// The derived view: a section, detail or auxiliary view.
    pub view: ViewId,
    /// Its parent.
    pub parent: ViewId,
    /// Index of the derived view's sheet.
    pub view_sheet: usize,
    /// The zone the derived view's centre is in, such as "C4".
    pub view_zone: String,
    /// Index of the parent's sheet.
    pub parent_sheet: usize,
    /// The zone the mark on the parent is in: the middle of a cutting line, a detail's
    /// centre or an auxiliary view's arrow.
    pub parent_zone: String,
}

impl LocationLabel {
    /// The text under the mark on the parent, such as "SEE SHEET 2, ZONE C4", or
    /// `None` when both are on one sheet.
    pub fn on_parent(&self) -> Option<String> {
        (self.view_sheet != self.parent_sheet)
            .then(|| format!("SEE SHEET {}, ZONE {}", self.view_sheet + 1, self.view_zone))
    }

    /// The text under the derived view, such as "FROM SHEET 1, ZONE B3", or `None` when
    /// both are on one sheet.
    pub fn on_view(&self) -> Option<String> {
        (self.view_sheet != self.parent_sheet)
            .then(|| format!("FROM SHEET {}, ZONE {}", self.parent_sheet + 1, self.parent_zone))
    }
}

/// The zone of a sheet point under a template's zone grid: rows lettered from the top,
/// columns numbered from the left, as the border labels them, such as "B3".
pub fn zone_at(format: &TemplateFile, p: Point2) -> String {
    let [l, b, r, t] = format.frame();
    let cols = format.zones.columns.max(1);
    let rows = format.zones.rows.max(1);
    let col = (((p.u - l) / ((r - l) / f64::from(cols))).floor() as i64).clamp(0, i64::from(cols) - 1);
    let row = (((t - p.v) / ((t - b) / f64::from(rows))).floor() as i64).clamp(0, i64::from(rows) - 1);
    format!("{}{}", char::from(b'A' + (row % 26) as u8), col + 1)
}

/// Every derived view's location label.
pub fn location_labels(drawing: &Drawing) -> Vec<LocationLabel> {
    let mut out = Vec::new();
    for (vs, sheet) in drawing.sheets.iter().enumerate() {
        for view in &sheet.views {
            if let Some(l) = location_of(drawing, vs, view) {
                out.push(l);
            }
        }
    }
    out
}

fn location_of(drawing: &Drawing, view_sheet: usize, view: &View) -> Option<LocationLabel> {
    if !matches!(view.kind, ViewKind::Section { .. } | ViewKind::Detail { .. } | ViewKind::Auxiliary { .. }) {
        return None;
    }
    let parent_id = view.parent()?;
    let parent_sheet = crate::project::sheet_of(drawing, parent_id)?;
    let parent = drawing.view(parent_id)?;
    let format = |i: usize| library::template(drawing.sheets[i].template.as_str());
    let [x0, y0, x1, y1] = view_bounds(view);
    let view_zone = zone_at(format(view_sheet), Point2::new((x0 + x1) / 2.0, (y0 + y1) / 2.0));
    let mark = mark_point(parent, view)?;
    let parent_zone = zone_at(format(parent_sheet), mark);
    Some(LocationLabel { view: view.id, parent: parent_id, view_sheet, view_zone, parent_sheet, parent_zone })
}

/// Where a derived view's mark sits on its parent, in sheet millimetres.
fn mark_point(parent: &View, view: &View) -> Option<Point2> {
    let o = parent.position;
    match &view.kind {
        ViewKind::Section { line, .. } => {
            let length = |[a, b]: [Point2; 2]| (a.u - b.u).hypot(a.v - b.v);
            let [a, b] = trace_segments(line, parent.geometry.breaks.first().copied())
                .max_by(|a, b| length(*a).total_cmp(&length(*b)))?;
            Some(Point2::new(o.u + (a.u + b.u) / 2.0, o.v + (a.v + b.v) / 2.0))
        }
        ViewKind::Detail { center, .. } => Some(Point2::new(o.u + center.u, o.v + center.v)),
        _ => Some(o),
    }
}

fn trace_segments(line: &[Point2], gap: Option<crate::BreakGap>) -> impl Iterator<Item = [Point2; 2]> {
    line.windows(2)
        .flat_map(move |s| gap.map_or([[s[0], s[1]], [s[1], s[1]]], |g| g.trace(s[0], s[1])))
        .filter(|[a, b]| a != b)
}

/// The location label under a derived view whose parent is on another sheet.
pub(super) fn location_from(drawing: &Drawing, view: &View) -> Option<String> {
    let sheet = crate::project::sheet_of(drawing, view.id)?;
    location_of(drawing, sheet, view)?.on_view()
}

/// The hatch pattern of a view's cut faces: its section's, or for a detail, its
/// parent's.
fn pattern_of(drawing: &Drawing, view: &View) -> &'static HatchPattern {
    let lib = library::library();
    let mut v = view;
    for _ in 0..16 {
        match &v.kind {
            ViewKind::Section { hatch: Some(id), .. } => {
                if let Some(p) = lib.hatch.iter().find(|p| &p.id == id) {
                    return p;
                }
                break;
            }
            ViewKind::Detail { parent, .. } => match drawing.view(*parent) {
                Some(p) => v = p,
                None => break,
            },
            _ => break,
        }
    }
    &lib.hatch[0]
}

/// Hatch lines for every cut region of a view, clipped to its boundary, and a record of
/// each region for the DXF writer.
pub(super) fn hatch_items(
    drawing: &Drawing,
    view: &View,
    _ctx: &Context<'_>,
    items: &mut Vec<Item>,
    records: &mut Vec<HatchRecord>,
) {
    if view.geometry.hatch.is_empty() {
        return;
    }
    let pattern = pattern_of(drawing, view);
    let o = view.position;
    let shift = |p: &Point2| Point2::new(p.u + o.u, p.v + o.v);
    for poly in &view.geometry.hatch {
        let mut loops: Vec<Vec<Point2>> = vec![poly.outer.iter().map(shift).collect()];
        loops.extend(poly.holes.iter().map(|h| h.iter().map(shift).collect()));
        let clip = poly.clip.as_ref().map(|c| c.mapped(o.u, o.v, 1.0));
        let mut path = BezPath::new();
        if pattern.spacing > 0.0 {
            let mut angles = vec![pattern.angle];
            if pattern.cross {
                angles.push(pattern.angle + 90.0);
            }
            for angle in angles {
                let mut offsets = vec![0.0];
                if pattern.double && pattern.double_gap > 0.0 {
                    offsets.push(pattern.double_gap);
                }
                for offset in offsets {
                    for (a, b) in hatch_lines(&loops, angle, pattern.spacing, offset) {
                        path.move_to((a.u, a.v));
                        path.line_to((b.u, b.v));
                    }
                }
            }
        }
        if let Some(c) = &clip {
            path = clip_path(&path, &|q| c.contains(Point2::new(q.x, q.y)));
        }
        if !path.elements().is_empty() {
            let dash = (!pattern.dash.is_empty()).then(|| pattern.dash.clone());
            items.push(Item::Path(PathItem {
                path,
                width: pattern.width,
                dash,
                role: Role::Hatch,
                owner: Owner::View(view.id),
                fill: false,
                accuracy: poly.approximate.then_some(super::Accuracy::Approximate),
            }));
        }
        let loops = match &clip {
            Some(c) => {
                let window = c.polygon(96);
                clip_regions(&loops, &window, true)
            }
            None => vec![loops],
        };
        for loops in loops.into_iter().filter(|loops| !loops.is_empty()) {
            records.push(HatchRecord {
                owner: Owner::View(view.id),
                loops,
                angle: pattern.angle,
                spacing: pattern.spacing,
                approximate: poly.approximate || matches!(clip, Some(Boundary::Circle { .. })),
            });
        }
    }
}

/// The hatch segments inside `loops` (even-odd): lines at `angle` degrees, `spacing`
/// apart, placed on a grid fixed to the sheet so neighbouring regions line up, moved
/// across by `offset`.
pub fn hatch_lines(loops: &[Vec<Point2>], angle: f64, spacing: f64, offset: f64) -> Vec<(Point2, Point2)> {
    let (sin, cos) = angle.to_radians().sin_cos();
    // Into a frame where the lines run along X.
    let fwd = |p: &Point2| Point2::new(p.u * cos + p.v * sin, -p.u * sin + p.v * cos);
    let back = |p: Point2| Point2::new(p.u * cos - p.v * sin, p.u * sin + p.v * cos);
    let rings: Vec<Vec<Point2>> = loops.iter().map(|l| l.iter().map(fwd).collect()).collect();
    let (lo, hi) =
        rings.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.v), hi.max(p.v)));
    let mut out = Vec::new();
    if !lo.is_finite() || spacing <= 0.0 {
        return out;
    }
    let first = ((lo - offset) / spacing).ceil() as i64;
    let last = ((hi - offset) / spacing).floor() as i64;
    for k in first..=last {
        let y = k as f64 * spacing + offset;
        let mut xs: Vec<f64> = Vec::new();
        for ring in &rings {
            let n = ring.len();
            for i in 0..n {
                let (a, b) = (ring[i], ring[(i + 1) % n]);
                if (a.v > y) != (b.v > y) {
                    xs.push(a.u + (y - a.v) * (b.u - a.u) / (b.v - a.v));
                }
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            if pair[1] - pair[0] > 1e-9 {
                out.push((back(Point2::new(pair[0], y)), back(Point2::new(pair[1], y))));
            }
        }
    }
    out
}

pub(crate) fn clip_regions(subject: &[Vec<Point2>], clip: &[Point2], inside: bool) -> Vec<Vec<Vec<Point2>>> {
    use i_overlay::{
        core::{fill_rule::FillRule, overlay_rule::OverlayRule},
        float::single::SingleFloatOverlay,
    };
    let ring = |points: &[Point2]| points.iter().map(|p| [p.u, p.v]).collect::<Vec<_>>();
    let subject: Vec<_> = subject.iter().filter(|r| r.len() >= 3).map(|r| ring(r)).collect();
    let rule = if inside { OverlayRule::Intersect } else { OverlayRule::Difference };
    subject
        .overlay_as::<i64>(&ring(clip), rule, FillRule::EvenOdd)
        .into_iter()
        .map(|shape| shape.into_iter().map(|r| r.into_iter().map(|[x, y]| Point2::new(x, y)).collect()).collect())
        .collect()
}

/// A detail view's own boundary and the edges of broken-out sections, thin and
/// continuous.
pub(super) fn boundary_items(view: &View, ctx: &Context<'_>, items: &mut Vec<Item>) {
    let shift = Affine::translate((view.position.u, view.position.v));
    let mut push = |b: &Boundary| {
        let mut path = b.path();
        path.apply_affine(shift);
        items.push(Item::Path(PathItem {
            path,
            width: ctx.format.lines.thin,
            dash: None,
            role: Role::Boundary,
            owner: Owner::View(view.id),
            fill: false,
            accuracy: None,
        }));
    };
    if let Some(outline) = &view.geometry.outline {
        push(outline);
    }
    for b in &view.broken_out {
        push(&b.boundary);
    }
    let o = view.position;
    let at = |p: Point2| Point2::new(p.u + o.u, p.v + o.v);
    // A half section's symmetry line: a thin chain line, as any centre line.
    for [a, b] in view.geometry.symmetry_segments.iter().copied() {
        items.push(Item::Path(PathItem {
            path: line(at(a), at(b)),
            width: ctx.format.lines.centre,
            dash: Some(super::CENTRE_DASH.to_vec()),
            role: Role::Boundary,
            owner: Owner::View(view.id),
            fill: false,
            accuracy: None,
        }));
    }
    // Break lines: a thin line across the view on each side of a gap, with one zigzag
    // (ISO 128-30).
    let [x0, y0, x1, y1] = view.geometry.bbox;
    for g in view.geometry.breaks.iter().filter(|_| !matches!(view.kind, ViewKind::Detail { .. })) {
        let vertical = g.axis == crate::view::AlignAxis::Vertical;
        let (from, to) = if vertical { (x0 - 3.0, x1 + 3.0) } else { (y0 - 3.0, y1 + 3.0) };
        let mid = (from + to) / 2.0;
        let z = 2.0f64.min((to - from) / 8.0);
        for along in [g.at, g.at + g.gap] {
            let p = |across: f64, off: f64| {
                if vertical { at(Point2::new(across, along + off)) } else { at(Point2::new(along + off, across)) }
            };
            items.push(Item::Path(PathItem {
                path: polyline(&[
                    p(from, 0.0),
                    p(mid - z, 0.0),
                    p(mid - z / 3.0, z),
                    p(mid + z / 3.0, -z),
                    p(mid + z, 0.0),
                    p(to, 0.0),
                ]),
                width: ctx.format.lines.thin,
                dash: None,
                role: Role::Boundary,
                owner: Owner::View(view.id),
                fill: false,
                accuracy: None,
            }));
        }
    }
}

fn label(pos: Point2, height: f64, text: String, halign: Align, valign: VAlign, role: Role, owner: Owner) -> Item {
    Item::Text(TextItem { pos, height, angle: 0.0, text, halign, valign, weight: Weight::Medium, role, owner })
}

/// The marks derived views leave on the views of sheet `sheet_index`: cutting lines,
/// detail circles and auxiliary arrows, each with its letter and, when the derived view
/// is on another sheet, where to find it. They belong to the derived view.
pub(super) fn parent_marks(drawing: &Drawing, sheet_index: usize, ctx: &Context<'_>, list: &mut super::DisplayList) {
    let Some(sheet) = drawing.sheets.get(sheet_index) else { return };
    let items = &mut list.items;
    let lines = &ctx.format.lines;
    let letter_height = ctx.style.text.height * 1.4;
    for (child_sheet, s) in drawing.sheets.iter().enumerate() {
        for child in &s.views {
            let Some(parent) = child.parent().and_then(|p| sheet.view(p)) else { continue };
            let owner = Owner::View(child.id);
            let first = items.len();
            let o = parent.position;
            let at = |p: &Point2| Point2::new(p.u + o.u, p.v + o.v);
            let see = location_of(drawing, child_sheet, child).and_then(|l| l.on_parent());
            match &child.kind {
                ViewKind::Section { line: cut, flip, label: letter, .. } if cut.len() >= 2 => {
                    let gap = parent.geometry.breaks.first().copied();
                    let flat: Vec<_> = cut.iter().map(|p| gap.map_or(*p, |g| g.unshift(*p))).collect();
                    let Some(arrow_dir) = section_arrow(&flat[..2], *flip) else { continue };
                    let pts: Vec<Point2> = cut.iter().map(at).collect();
                    let trace = |points: &[Point2]| {
                        if gap.is_none() {
                            return polyline(&points.iter().map(at).collect::<Vec<_>>());
                        }
                        let mut path = BezPath::new();
                        for [a, b] in trace_segments(points, gap) {
                            let (a, b) = (at(&a), at(&b));
                            path.move_to((a.u, a.v));
                            path.line_to((b.u, b.v));
                        }
                        path
                    };
                    let (first, last) = (pts[0], pts[pts.len() - 1]);
                    let leg = |a: Point2, b: Point2| {
                        let dir = sub(b, a);
                        let len = dir.length();
                        (len >= 1e-9).then(|| (dir / len, len))
                    };
                    let (Some((d0, len0)), Some((d1, len1))) =
                        (leg(flat[0], flat[1]), leg(flat[flat.len() - 2], flat[flat.len() - 1]))
                    else {
                        continue;
                    };
                    // A thin chain line, thick at its ends and where it changes direction
                    // (ISO 128-40, ISO 128-44).
                    items.push(Item::Path(PathItem {
                        path: trace(cut),
                        width: lines.centre,
                        dash: Some(super::CENTRE_DASH.to_vec()),
                        role: Role::CuttingPlane,
                        owner,
                        fill: false,
                        accuracy: None,
                    }));
                    for (i, bend) in flat.windows(3).enumerate() {
                        let vertex = cut[i + 1];
                        if gap.is_some_and(|g| g.contains(vertex)) {
                            continue;
                        }
                        let (Some((into, l0)), Some((out, l1))) = (leg(bend[0], bend[1]), leg(bend[1], bend[2])) else {
                            continue;
                        };
                        let thick = 6.0f64.min(l0 / 3.0).min(l1 / 3.0);
                        let path =
                            polyline(&[at(&add(vertex, into * -thick)), pts[i + 1], at(&add(vertex, out * thick))]);
                        let path = if let Some(g) = gap {
                            clip_path(&path, &|p| !g.contains(Point2::new(p.x - o.u, p.y - o.v)))
                        } else {
                            path
                        };
                        items.push(Item::Path(PathItem {
                            path,
                            width: lines.cutting,
                            dash: None,
                            role: Role::CuttingPlane,
                            owner,
                            fill: false,
                            accuracy: None,
                        }));
                    }
                    let a = Vec2::new(arrow_dir.u, arrow_dir.v);
                    let side = d0.cross(a).signum();
                    let a_last = Vec2::new(-d1.y, d1.x) * side;
                    for (end, inward, thick, a) in
                        [(first, d0, 6.0f64.min(len0 / 3.0), a), (last, d1 * -1.0, 6.0f64.min(len1 / 3.0), a_last)]
                    {
                        let path = line(end, add(end, inward * thick));
                        let path = if let Some(g) = gap {
                            clip_path(&path, &|p| !g.contains(Point2::new(p.x - o.u, p.y - o.v)))
                        } else {
                            path
                        };
                        items.push(Item::Path(PathItem {
                            path,
                            width: lines.cutting,
                            dash: None,
                            role: Role::CuttingPlane,
                            owner,
                            fill: false,
                            accuracy: None,
                        }));
                        // The arrow stands on the line's end and points the way the
                        // section looks; the letter sits beyond its head.
                        let tip = add(end, a * 9.0);
                        items.push(Item::Path(PathItem {
                            path: line(end, tip),
                            width: lines.dimension,
                            dash: None,
                            role: Role::CuttingPlane,
                            owner,
                            fill: false,
                            accuracy: None,
                        }));
                        items.push(arrow(tip, a, ctx.style, Role::CuttingPlane, owner));
                        let text_at = add(tip, a * (letter_height * 0.5 + 2.0));
                        items.push(label(
                            text_at,
                            letter_height,
                            letter.clone(),
                            Align::Centre,
                            VAlign::Centre,
                            Role::Text,
                            owner,
                        ));
                    }
                    if let Some(see) = &see {
                        let pos = add(add(last, a_last * (9.0 + letter_height + 4.0)), Vec2::new(0.0, 0.0));
                        items.push(label(
                            pos,
                            ctx.style.text.label_height,
                            see.clone(),
                            Align::Centre,
                            VAlign::Top,
                            Role::Text,
                            owner,
                        ));
                    }
                }
                ViewKind::Detail { center, radius, profile, label: letter, .. } => {
                    let boundary = if profile.len() >= 3 {
                        Boundary::Polygon { points: profile.iter().map(at).collect() }
                    } else {
                        Boundary::Circle { center: at(center), radius: *radius }
                    };
                    items.push(Item::Path(PathItem {
                        path: boundary.path(),
                        width: lines.centre,
                        dash: None,
                        role: Role::Boundary,
                        owner,
                        fill: false,
                        accuracy: None,
                    }));
                    let [_, _, x1, y1] = boundary.bounds();
                    let c = at(center);
                    let corner = Point2::new(
                        c.u + (x1 - c.u) * std::f64::consts::FRAC_1_SQRT_2 + 2.0,
                        c.v + (y1 - c.v) * std::f64::consts::FRAC_1_SQRT_2 + 2.0,
                    );
                    items.push(label(
                        corner,
                        letter_height,
                        letter.clone(),
                        Align::Left,
                        VAlign::Bottom,
                        Role::Text,
                        owner,
                    ));
                    if let Some(see) = &see {
                        let pos = Point2::new(corner.u, corner.v - 1.5);
                        items.push(label(
                            pos,
                            ctx.style.text.label_height,
                            see.clone(),
                            Align::Left,
                            VAlign::Top,
                            Role::Text,
                            owner,
                        ));
                    }
                }
                ViewKind::Auxiliary { edge, label: letter, .. } => {
                    // The arrow points at the middle of the reference edge as the parent
                    // shows it, from the side the auxiliary view looks from.
                    let same = |c: &crate::Curve2| {
                        c.source.same_projected_entity(edge)
                            || (c.source.section_leg == edge.section_leg
                                && c.source.feature == edge.feature
                                && c.source.role == edge.role
                                && c.source.ordinal == edge.ordinal)
                    };
                    let Some(curve) = parent.geometry.curves.iter().find(|c| same(c)) else { continue };
                    let pts = super::flatten(&curve.path, 0.01);
                    let Some(poly) = pts.first().filter(|p| p.len() >= 2) else { continue };
                    let (s0, s1) = (poly[0], poly[poly.len() - 1]);
                    let mid = Point2::new((s0.x + s1.x) / 2.0, (s0.y + s1.y) / 2.0);
                    let axis = Vec2::new(s1.x - s0.x, s1.y - s0.y);
                    if axis.length() < 1e-9 {
                        continue;
                    }
                    let axis = axis / axis.length();
                    let mut eye = Vec2::new(-axis.y, axis.x);
                    if mid.u * eye.x + mid.v * eye.y < 0.0 {
                        eye *= -1.0;
                    }
                    if let ViewKind::Auxiliary { flip: true, .. } = child.kind {
                        eye *= -1.0;
                    }
                    let tip = add(at(&mid), eye * 4.0);
                    let tail = add(tip, eye * 12.0);
                    items.push(Item::Path(PathItem {
                        path: line(tail, tip),
                        width: lines.dimension,
                        dash: None,
                        role: Role::CuttingPlane,
                        owner,
                        fill: false,
                        accuracy: None,
                    }));
                    items.push(arrow(tip, eye * -1.0, ctx.style, Role::CuttingPlane, owner));
                    if !letter.is_empty() {
                        let text_at = add(tail, eye * (letter_height * 0.5 + 1.5));
                        items.push(label(
                            text_at,
                            letter_height,
                            letter.clone(),
                            Align::Centre,
                            VAlign::Centre,
                            Role::Text,
                            owner,
                        ));
                    }
                    if let Some(see) = &see {
                        let pos = add(tail, eye * (letter_height + 3.0));
                        items.push(label(
                            pos,
                            ctx.style.text.label_height,
                            see.clone(),
                            Align::Centre,
                            VAlign::Top,
                            Role::Text,
                            owner,
                        ));
                    }
                }
                _ => {}
            }
            if parent.geometry.display_only && first != items.len() {
                list.display_only_items.push(first..items.len());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: f64, half: f64) -> Vec<Point2> {
        vec![
            Point2::new(c - half, c - half),
            Point2::new(c + half, c - half),
            Point2::new(c + half, c + half),
            Point2::new(c - half, c + half),
        ]
    }

    #[test]
    fn hatch_lines_fill_a_region_and_skip_its_holes() {
        // Horizontal lines 1 mm apart through a 10 mm square with a 4 mm hole.
        let lines = hatch_lines(&[square(0.0, 5.0), square(0.0, 2.0)], 0.0, 1.0, 0.5);
        assert_eq!(lines.iter().filter(|(a, _)| a.v.abs() > 2.0).count(), 6, "{lines:?}");
        assert_eq!(lines.iter().filter(|(a, _)| a.v.abs() < 2.0).count(), 8, "two pieces each across the hole");
        let length: f64 = lines.iter().map(|(a, b)| (b.u - a.u).hypot(b.v - a.v)).sum();
        assert!((length - (100.0 - 16.0)).abs() < 1e-9, "{length}");
        // At 45 degrees every segment runs at 45 degrees.
        for (a, b) in hatch_lines(&[square(0.0, 5.0)], 45.0, 3.0, 0.0) {
            assert!(((b.v - a.v) - (b.u - a.u)).abs() < 1e-9);
        }
    }

    #[test]
    fn polygons_clip_to_a_convex_window() {
        let regions = clip_regions(&[square(0.0, 5.0)], &square(5.0, 5.0), true);
        let clipped = &regions[0][0];
        let area: f64 = (0..clipped.len())
            .map(|i| {
                let (a, b) = (clipped[i], clipped[(i + 1) % clipped.len()]);
                a.u * b.v - b.u * a.v
            })
            .sum::<f64>()
            / 2.0;
        assert!((area.abs() - 25.0).abs() < 1e-9, "{clipped:?}");
    }

    #[test]
    fn zones_count_rows_from_the_top_and_columns_from_the_left() {
        let f = library::template("iso-a3");
        let [l, b, r, t] = f.frame();
        assert_eq!(zone_at(f, Point2::new(l + 1.0, t - 1.0)), "A1");
        let last = format!("{}{}", char::from(b'A' + (f.zones.rows - 1) as u8), f.zones.columns);
        assert_eq!(zone_at(f, Point2::new(r - 1.0, b + 1.0)), last);
    }
}
