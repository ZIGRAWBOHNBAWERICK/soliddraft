//! The dimensions that build the selected body, drawn where they act, the way CAD
//! applications show a feature's dimensions on selection: every dimension of the
//! sketches the body's features consume, on their planes, and each feature's own values
//! (depth, angle, radius, count and so on) at the feature. Double-clicking one edits it:
//! a sketch dimension in its sketch's value editor, an extrude's sizes and depth in a
//! field right there, and other feature values in the feature's editor.

use super::camera::Camera;
use crate::dimension_edit::{DimensionAction, DimensionEdit, FeatureField};
use crate::document::{Document, EndKind, Feature, FeatureId, FeatureKind, FeatureStatus, ProfileKind, fmt_mm};
use crate::sketch::{ConstraintId, ConstraintKind, DistanceKind, P2, Sketch};
use crate::sketcher::{arrow, paint_closed_dimensions, value_pill};
use crate::state::AppAction;
use crate::theme;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, vec2};
use glam::Vec3;
use model::view::{AnnotationKey, AnnotationValue};
use soliddraft_kernel_api::{HoleDepth, HoleHead, HoleTip, Plane, Point2};
use soliddraft_model as model;
use std::collections::HashMap;

/// What double-clicking a shown dimension edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DimensionTarget {
    /// A dimension of a sketch, edited in the sketch.
    Sketch { sketch: FeatureId, constraint: ConstraintId },
    /// A value of a feature, edited in the feature's editor unless the shown dimension
    /// names its [`FeatureField`], which types it in place.
    Feature(FeatureId),
}

/// One dimension on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct ShownDimension {
    pub target: DimensionTarget,
    /// Stable identity used by placement history and the part's view data.
    pub key: AnnotationKey,
    /// The label's anchor in model space. Text is always painted facing the viewer.
    pub position: Vec3,
    /// The value's rectangle, which takes the pointer.
    pub rect: Rect,
    /// What the hover tip says: the feature and the value's name.
    pub tip: String,
    /// The value as painted.
    pub text: String,
    /// Which of the feature's values this is, when the viewport can take it.
    pub field: Option<FeatureField>,
    /// The ends of a depth's dimension line on the model: at the profile and at the depth
    /// reached.
    pub witness: Option<[Vec3; 2]>,
}

impl ShownDimension {
    /// What a double-click on the value does.
    pub fn edit_action(&self) -> AppAction {
        match (self.target, self.field) {
            (DimensionTarget::Feature(feature), Some(field)) => {
                AppAction::Dimension(DimensionAction::Open { feature, field })
            }
            (target, _) => AppAction::EditDimension(target),
        }
    }

    pub fn is_field(&self, feature: FeatureId, field: FeatureField) -> bool {
        self.target == DimensionTarget::Feature(feature) && self.field == Some(field)
    }

    /// Names the painted value for assistive tools and automation.
    pub fn expose(&self, ui: &egui::Ui, index: usize) {
        let label = format!("{} {}", self.tip, self.text);
        ui.interact(self.rect, ui.id().with(("dimension", index)), Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone()));
    }
}

/// The field of a built-in profile that a dimension of its sketch shows.
fn profile_field(kind: &ConstraintKind) -> Option<FeatureField> {
    match *kind {
        ConstraintKind::Distance { kind: DistanceKind::Horizontal, .. } => Some(FeatureField::Width),
        ConstraintKind::Distance { kind: DistanceKind::Vertical, .. } => Some(FeatureField::Height),
        ConstraintKind::Radius { diameter: true, .. } => Some(FeatureField::Diameter),
        _ => None,
    }
}

fn gv(v: soliddraft_kernel_api::Vec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

fn line_color(hover: bool) -> Color32 {
    if hover { theme::accent() } else { theme::ink().gamma_multiply(0.7) }
}

/// Paints body dimensions and dimensions of every visible closed sketch.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &egui::Painter,
    camera: &Camera,
    rect: Rect,
    doc: &Document,
    pointer: Option<Pos2>,
    body: bool,
    visible_sketches: &[FeatureId],
    preview: Option<(AnnotationKey, Vec3)>,
) -> Vec<ShownDimension> {
    let end = doc.rollback.unwrap_or(doc.features.len()).min(doc.features.len());
    let built: Vec<&Feature> =
        doc.features[..end].iter().filter(|f| !f.suppressed && f.status != FeatureStatus::Inactive).collect();
    let anchors = if body { anchors(doc) } else { HashMap::new() };
    let mut shown = Vec::new();
    let authored_position = |key: AnnotationKey| {
        preview
            .filter(|(k, _)| *k == key)
            .map(|(_, p)| p)
            .or_else(|| doc.model().visibility().annotation_position(key).map(Vec3::from_array))
    };
    let position = |key: AnnotationKey, default: Vec3| authored_position(key).unwrap_or(default);
    let sketch_dims = |shown: &mut Vec<ShownDimension>,
                       f: &Feature,
                       plane: Plane,
                       sketch: &Sketch,
                       edit_sketch: bool,
                       authored_pass: Option<bool>| {
        let key = |constraint| AnnotationKey {
            feature: model::FeatureId(f.id as u32),
            value: AnnotationValue::Sketch(constraint),
        };
        for (constraint, r, world, text) in paint_closed_dimensions(
            painter,
            camera,
            rect,
            plane,
            sketch,
            |constraint| authored_position(key(constraint)),
            shown.iter().map(|d| d.rect),
            authored_pass,
            pointer,
        ) {
            let c = sketch.constraint(constraint);
            let target = if edit_sketch {
                DimensionTarget::Sketch { sketch: f.id, constraint }
            } else {
                DimensionTarget::Feature(f.id)
            };
            // A built-in profile's sizes are the extrude's own values.
            let field = c.and_then(|c| profile_field(&c.kind)).filter(|_| !edit_sketch);
            let name = field.map(FeatureField::label).or(c.map(|c| c.name())).unwrap_or("Dimension");
            shown.push(ShownDimension {
                target,
                key: key(constraint),
                position: world,
                rect: r,
                tip: format!("{} · {name}", f.name),
                text,
                field,
                witness: None,
            });
        }
    };
    // Reserve authored labels across every visible sketch before placing any defaults.
    let sketches = built.iter().filter_map(|f| {
        let FeatureKind::Sketch(s) = &f.kind else { return None };
        let plane = f.plane?;
        (visible_sketches.contains(&f.id) || (body && built.iter().any(|c| c.kind.sketch() == Some(f.id))))
            .then_some((*f, plane, &s.sketch))
    });
    for authored in [true, false] {
        for (f, plane, sketch) in sketches.clone() {
            sketch_dims(&mut shown, f, plane, sketch, true, Some(authored));
        }
    }
    for f in &built {
        let mut depth: Option<(Plane, &Sketch, f64)> = None;
        let builtin;
        match &f.kind {
            FeatureKind::Sketch(_) => {}
            FeatureKind::Extrude(p) if body => {
                let profile = match p.sketch {
                    None => {
                        let shape = match p.profile {
                            ProfileKind::Rectangle { width, height } => {
                                model::BuiltinShape::Rectangle { width, height }
                            }
                            ProfileKind::Circle { diameter } => model::BuiltinShape::Circle { diameter },
                        };
                        builtin = shape.sketch().ok();
                        // A built-in profile's sizes are the extrude's own values.
                        builtin.as_ref().map(|sk| {
                            sketch_dims(&mut shown, f, p.plane.to_kernel(), sk, false, None);
                            (p.plane.to_kernel(), sk)
                        })
                    }
                    Some(id) => doc.feature(id).and_then(|s| Some((s.plane?, &s.sketch()?.sketch))),
                };
                if p.end == EndKind::Blind
                    && let Some((plane, sk)) = profile
                {
                    depth = Some((plane, sk, if p.flip { -p.depth } else { p.depth }));
                }
            }
            FeatureKind::Cut(p) if body => {
                if p.end() == EndKind::Blind
                    && let Some(s) = doc.feature(p.sketch)
                    && let (Some(plane), Some(sk)) = (s.plane, s.sketch())
                {
                    // Cuts go against the sketch normal unless flipped.
                    depth = Some((plane, &sk.sketch, if p.flip { p.depth } else { -p.depth }));
                }
            }
            kind if body => {
                let values = values(kind);
                if values.is_empty() {
                    continue;
                }
                let Some(at) = anchors.get(&(f.id as u32)).copied().or_else(|| body_centre(doc)) else { continue };
                // Untouched multi-value labels share a view-plane stack. Equal camera
                // depth keeps its point spacing exact in perspective as well as orthographic.
                let (origin, step) = if values.len() > 1 {
                    let depth_scale = if camera.orthographic {
                        1.0
                    } else {
                        (at - camera.eye()).dot(camera.forward()) / camera.distance
                    };
                    let mm_per_point = camera.mm_per_point(rect.height()) * depth_scale;
                    let up = camera.up();
                    (at + (camera.right() * 48.0 + up * 32.0) * mm_per_point, -up * 28.0 * mm_per_point)
                } else {
                    (at + Vec3::new(12.0, 0.0, 12.0), Vec3::ZERO)
                };
                for (i, (name, text)) in values.into_iter().enumerate() {
                    let key = AnnotationKey { feature: model::FeatureId(f.id as u32), value: name };
                    let world = position(key, origin + step * i as f32);
                    let Some(r) = paint_leader(painter, camera, rect, at, world, &text, pointer) else { continue };
                    shown.push(ShownDimension {
                        target: DimensionTarget::Feature(f.id),
                        key,
                        position: world,
                        rect: r,
                        tip: f.name.clone(),
                        text,
                        field: None,
                        witness: None,
                    });
                }
            }
            _ => {}
        }
        let key = AnnotationKey { feature: model::FeatureId(f.id as u32), value: AnnotationValue::Depth };
        if let Some((plane, sk, value)) = depth
            && let Some((r, witness, world)) =
                paint_depth(painter, camera, rect, plane, sk, value, |default| position(key, default), pointer)
        {
            shown.push(ShownDimension {
                target: DimensionTarget::Feature(f.id),
                key,
                position: world,
                rect: r,
                tip: format!("{} · Depth", f.name),
                text: fmt_mm(value.abs()),
                // A cut's depth keeps the cut's editor for now.
                field: FeatureField::Depth.get(&f.kind).map(|_| FeatureField::Depth),
                witness: Some(witness),
            });
        }
    }
    for dimension in &shown {
        pill(painter, dimension.rect, dimension.text.clone(), pointer);
    }
    shown
}

/// The values a feature shows beside its faces.
fn values(kind: &FeatureKind) -> Vec<(AnnotationValue, String)> {
    use AnnotationValue::*;
    match kind {
        FeatureKind::Revolve(r) => vec![(Angle, format!("{}°", fmt_mm(r.angle_deg)))],
        FeatureKind::Sweep(_) | FeatureKind::Loft(_) => vec![],
        FeatureKind::Fillet(p) => vec![(Radius, format!("R{}", fmt_mm(p.radius)))],
        FeatureKind::Chamfer(p) => vec![(Distance, fmt_mm(p.distance))],
        FeatureKind::Draft(d) => vec![(Angle, format!("{}°", fmt_mm(d.angle)))],
        FeatureKind::Shell(s) => vec![(Thickness, fmt_mm(s.thickness))],
        FeatureKind::Thicken(s) => vec![(Thickness, fmt_mm(s.thickness))],
        FeatureKind::Offset(s) => vec![(Distance, fmt_mm(s.distance))],
        FeatureKind::Hole(h) => {
            let mut v = vec![(Diameter, format!("Ø{}", fmt_mm(h.diameter)))];
            if let HoleDepth::Blind { depth } = h.depth {
                v.push((Depth, format!("{} deep", fmt_mm(depth))));
                if let HoleTip::Cone { angle_deg } = h.tip {
                    v.push((Tip, format!("{}° tip", fmt_mm(angle_deg))));
                }
            }
            match h.head {
                Some(HoleHead::Counterbore { diameter, depth }) => {
                    v.push((Counterbore, format!("Ø{} × {} deep", fmt_mm(diameter), fmt_mm(depth))))
                }
                Some(HoleHead::Countersink { diameter, angle_deg }) => {
                    v.push((Countersink, format!("Ø{} × {}°", fmt_mm(diameter), fmt_mm(angle_deg))))
                }
                None => {}
            }
            v
        }
        FeatureKind::Pattern(p) => match p.layout {
            model::PatternSpec::Linear { spacing, count, .. } => {
                vec![(Count, format!("{count} ×")), (Spacing, fmt_mm(spacing))]
            }
            model::PatternSpec::Circular { count, angle, .. } => {
                vec![(Count, format!("{count} ×")), (Angle, format!("{}°", fmt_mm(angle)))]
            }
        },
        FeatureKind::ReferencePlane(_)
        | FeatureKind::Sketch(_)
        | FeatureKind::Extrude(_)
        | FeatureKind::Cut(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Lattice(_)
        | FeatureKind::Gyroid(_)
        | FeatureKind::ImportStep(_) => Vec::new(),
    }
}

/// A point on the faces each feature made: the mesh vertex nearest their centre, by
/// feature id.
fn anchors(doc: &Document) -> HashMap<u32, Vec3> {
    let Some(mesh) = doc.display_body().and_then(|b| doc.meshes.get(&b)) else { return HashMap::new() };
    let mut sums: HashMap<u32, (Vec3, f32)> = HashMap::new();
    for (p, f) in mesh.positions.iter().zip(&mesh.face_features).filter(|(_, f)| **f != 0) {
        let s = sums.entry(*f).or_insert((Vec3::ZERO, 0.0));
        s.0 += *p;
        s.1 += 1.0;
    }
    let mut best: HashMap<u32, (Vec3, f32)> = HashMap::new();
    for (p, f) in mesh.positions.iter().zip(&mesh.face_features) {
        let Some((sum, n)) = sums.get(f) else { continue };
        let d = p.distance_squared(*sum / *n);
        if best.get(f).is_none_or(|(_, bd)| d < *bd) {
            best.insert(*f, (*p, d));
        }
    }
    best.into_iter().map(|(f, (p, _))| (f, p)).collect()
}

fn body_centre(doc: &Document) -> Option<Vec3> {
    let (lo, hi) = doc.display_body().and_then(|b| doc.meshes.get(&b))?.bounds()?;
    Some((lo + hi) / 2.0)
}

/// Paints a laid-out value above the complete set of dimension lines.
fn pill(painter: &egui::Painter, rect: Rect, text: String, pointer: Option<Pos2>) {
    let hover = pointer.is_some_and(|p| rect.contains(p));
    let (bg, stroke) = if hover { (theme::hover(), theme::accent()) } else { (theme::hud_bg(), theme::line()) };
    value_pill(painter, rect.center(), text, theme::ink(), bg, Stroke::new(1.0, stroke));
}

/// A model-space leader with viewer-facing text at its far end.
#[allow(clippy::too_many_arguments)]
fn paint_leader(
    painter: &egui::Painter,
    camera: &Camera,
    rect: Rect,
    at: Vec3,
    label: Vec3,
    text: &str,
    pointer: Option<Pos2>,
) -> Option<Rect> {
    let (a, p) = (camera.project(at, rect)?, camera.project(label, rect)?);
    let size = painter.layout_no_wrap(text.to_owned(), theme::font_medium(11.5), theme::ink()).size();
    let r = Rect::from_center_size(p, size + vec2(12.0, 6.0));
    let hover = pointer.is_some_and(|p| r.contains(p));
    painter.line_segment([a, r.clamp(a)], Stroke::new(1.0, line_color(hover)));
    painter.circle_filled(a, 2.5, line_color(hover));
    Some(r)
}

/// A depth dimension whose extension lines follow the label's model-space offset.
#[allow(clippy::too_many_arguments)]
fn paint_depth(
    painter: &egui::Painter,
    camera: &Camera,
    rect: Rect,
    plane: Plane,
    sketch: &Sketch,
    depth: f64,
    position: impl FnOnce(Vec3) -> Vec3,
    pointer: Option<Pos2>,
) -> Option<(Rect, [Vec3; 2], Vec3)> {
    let dir = P2::new(-1.0, 1.0).normalized();
    let corner = sketch
        .drawn()
        .filter(|e| !e.construction)
        .flat_map(|e| e.polyline(32))
        .max_by(|a, b| a.dot(dir).total_cmp(&b.dot(dir)))?;
    let base = gv(plane.to_world(Point2::new(corner.u, corner.v)));
    let out = (gv(plane.x_dir) * dir.u as f32 + gv(plane.y_dir()) * dir.v as f32) * (0.1 * depth.abs() as f32 + 4.0);
    let top = base + gv(plane.normal) * depth as f32;
    let world = position((base + top) * 0.5 + out);
    let axis = (top - base).normalize_or_zero();
    let offset = world - (base + top) * 0.5;
    let offset = offset - axis * offset.dot(axis);
    let witness = [base + offset, top + offset];
    let [b, t, bo, to] = [base, top, witness[0], witness[1]].map(|p| camera.project(p, rect));
    let (b, t, bo, to) = (b?, t?, bo?, to?);
    let at = camera.project(world, rect)?;
    let text = fmt_mm(depth.abs());
    let size = painter.layout_no_wrap(text.clone(), theme::font_medium(11.5), theme::ink()).size();
    let color = line_color(pointer.is_some_and(|p| Rect::from_center_size(at, size + vec2(12.0, 6.0)).contains(p)));
    let ext = |from: Pos2, to: Pos2| {
        let d = (to - from).normalized();
        painter.line_segment([from + d * 4.0, to + d * 5.0], Stroke::new(0.8, color.gamma_multiply(0.8)));
    };
    ext(b, bo);
    ext(t, to);
    painter.line_segment([bo, to], Stroke::new(1.0, color));
    let mid = bo + (to - bo) * 0.5;
    if at.distance_sq(mid) > 0.01 {
        painter.line_segment([mid, at], Stroke::new(1.0, color));
    }
    let d = to - bo;
    if d.length() > 20.0 {
        arrow(painter, bo, -d, color);
        arrow(painter, to, d, color);
    }
    Some((Rect::from_center_size(at, size + vec2(12.0, 6.0)), witness, world))
}

/// The field where a value is typed, over its dimension in the viewport. It stays inside
/// `view`, on a short leader when that moves it off the dimension, and a refused value
/// says why right under it. Enter submits; Escape or a click elsewhere gives up.
pub fn value_editor(
    ui: &egui::Ui,
    painter: &egui::Painter,
    view: Rect,
    at: Rect,
    edit: &mut DimensionEdit,
    actions: &mut Vec<AppAction>,
) {
    /// About the field's size, to centre it on the dimension.
    const SIZE: egui::Vec2 = vec2(110.0, 30.0);
    let mut submitted = false;
    let area = egui::Area::new(egui::Id::new("dimension_value"))
        .order(egui::Order::Foreground)
        .fixed_pos(at.center() - SIZE / 2.0)
        .constrain_to(view.shrink(4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(theme::panel())
                .stroke(Stroke::new(1.0, theme::accent()))
                .corner_radius(CornerRadius::same(7))
                .shadow(ui.style().visuals.popup_shadow)
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let field = egui::TextEdit::singleline(&mut edit.text)
                            .id(egui::Id::new("dimension_value_text"))
                            .desired_width(60.0)
                            .font(theme::font_medium(12.5))
                            .show(ui);
                        let resp = field.response.response;
                        let name = format!("{} dimension value", edit.field.label());
                        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, name.clone()));
                        if edit.focus {
                            edit.focus = false;
                            resp.request_focus();
                            let mut state = field.state;
                            let all = egui::text::CCursorRange::select_all(&field.galley);
                            state.cursor.set_char_range(Some(all));
                            state.store(ui.ctx(), resp.id);
                        }
                        ui.label(egui::RichText::new("mm").size(11.0).italics().color(theme::muted()));
                        if resp.lost_focus() {
                            submitted = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if !submitted {
                                actions.push(AppAction::Dimension(DimensionAction::Cancel));
                            }
                        }
                    });
                    if let Some(error) = &edit.error {
                        ui.add(
                            egui::Label::new(egui::RichText::new(error).size(11.0).color(theme::error()))
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    }
                });
        });
    if submitted {
        match edit.typed() {
            Ok(Some(value)) => actions.push(AppAction::Dimension(DimensionAction::Commit(value))),
            Ok(None) => actions.push(AppAction::Dimension(DimensionAction::Cancel)),
            Err(message) => {
                edit.error = Some(message);
                edit.focus = true;
            }
        }
    }
    if !area.response.rect.contains(at.center()) {
        let end = area.response.rect.clamp(at.center());
        painter.line_segment([at.center(), end], Stroke::new(1.0, theme::accent()));
    }
}
