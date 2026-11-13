//! Field lists and controls for the feature editors in the Edit Suite options bar.

use crate::document::{EndKind, ProfileKind};
use crate::form_bar::Field;
use crate::icons::Icon;
use crate::state::EditorParams;
use soliddraft_kernel_api::{Grade, HoleDepth as DrillDepth, HoleHead, HoleTip};
use soliddraft_model::PatternSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Profile,
    Width,
    Height,
    Diameter,
    Plane,
    End,
    Depth,
    UpToFace,
    Flip,
    RevolveAxis,
    Angle,
    Edges,
    Radius,
    Distance,
    HoleFace,
    Positions,
    PositionList,
    HoleDepth,
    TipKind,
    TipAngle,
    HeadKind,
    HeadDiameter,
    HeadDepth,
    HeadAngle,
    Layout,
    Count,
    Spacing,
    Direction,
    AxisPoint,
    PlaneSource,
    Offset,
    KeepOriginal,
    Body,
    Cell,
    CellSize,
    BeamRadius,
    BeamGrade,
    Accuracy,
    MinimumDetail,
    Period,
    Level,
    LevelGrade,
    OpenFaces,
    Thickness,
    DraftFaces,
    NeutralPlane,
    Reverse,
    PathStart,
    Orientation,
    SweepPath,
    LoftProfiles,
    LoftShape,
    LoftProfileList,
    PlaneDatum(u8),
    PlaneFeature(u64),
    RepairWas,
    RepairReplacement,
    RepairNavigation,
    RepairList,
}

pub struct Spec {
    pub kind: &'static str,
    pub icon: Icon,
    pub confirm: bool,
    pub fields: Vec<Field<Key>>,
}

pub fn spec(params: &EditorParams, plane_choices: &[(soliddraft_model::FeatureId, String)]) -> Spec {
    use Key::*;
    let field = |id, label, group, width| Field::new(id, label, group, width);
    let (kind, icon, fields) = match params {
        EditorParams::Extrude(p) => {
            let mut fields = vec![field(Profile, "Profile", "Profile", 160.0)];
            if p.sketch.is_none() {
                match p.profile {
                    ProfileKind::Rectangle { .. } => fields
                        .extend([field(Width, "Width", "Profile", 130.0), field(Height, "Height", "Profile", 130.0)]),
                    ProfileKind::Circle { .. } => fields.push(field(Diameter, "Diameter", "Profile", 145.0)),
                }
                fields.push(field(Plane, "Plane", "Profile", 160.0));
            }
            fields.push(field(End, "End", "Direction", 170.0));
            match p.end {
                EndKind::Blind => fields.push(field(Depth, "Depth", "Direction", 140.0)),
                EndKind::UpToFace(_) => fields.push(field(UpToFace, "To", "Direction", 190.0)),
                _ => {}
            }
            fields.push(field(Flip, "", "Direction", 112.0));
            ("extrude", Icon::Extrude, fields)
        }
        EditorParams::Revolve(_) => (
            "revolve",
            Icon::Revolve,
            vec![
                field(Profile, "Profile", "Profile", 175.0),
                field(RevolveAxis, "Axis", "Axis", 220.0),
                field(Angle, "Angle", "Size", 145.0),
            ],
        ),
        EditorParams::Sweep(s) => (
            "sweep",
            Icon::Sweep,
            vec![
                field(Profile, "Profile", "Profile", 175.0),
                field(PathStart, "Path start", "Path", 210.0),
                field(Orientation, "Profile", "Path", 200.0),
                field(SweepPath, "", "Path", 280.0).sheet().count(s.path.segments.len()),
            ],
        ),
        EditorParams::Loft(l) => (
            "loft",
            Icon::Loft,
            vec![
                field(LoftProfiles, "Profiles", "Profiles", 170.0),
                field(LoftShape, "Sides", "Sides", 160.0),
                field(LoftProfileList, "", "Profiles in order", 280.0).sheet().count(l.profiles.len()),
            ],
        ),
        EditorParams::Cut(p) => {
            let mut fields = vec![field(Profile, "Profile", "Profile", 175.0), field(End, "End", "Direction", 170.0)];
            match p.end() {
                EndKind::Blind => fields.push(field(Depth, "Depth", "Direction", 140.0)),
                EndKind::UpToFace(_) => fields.push(field(UpToFace, "To", "Direction", 190.0)),
                _ => {}
            }
            fields.push(field(Flip, "", "Direction", 112.0));
            ("cut", Icon::Cut, fields)
        }
        EditorParams::Fillet(_) => (
            "fillet",
            Icon::Fillet,
            vec![field(Edges, "Edges", "Picks", 220.0), field(Radius, "Radius", "Size", 160.0)],
        ),
        EditorParams::Chamfer(_) => (
            "chamfer",
            Icon::Chamfer,
            vec![field(Edges, "Edges", "Picks", 220.0), field(Distance, "Distance", "Size", 170.0)],
        ),
        EditorParams::Hole(h) => {
            let mut fields = vec![
                field(HoleFace, "Face", "Placement", 190.0),
                field(Positions, "Positions", "Placement", 160.0),
                field(Diameter, "Diameter", "Hole", 150.0),
                field(HoleDepth, "Depth", "Hole", 170.0),
                field(PositionList, "", "Positions", 280.0).sheet().count(h.points.len()),
            ];
            if matches!(h.depth, DrillDepth::Blind { .. }) {
                fields.push(field(Depth, "Depth", "Blind depth", 150.0).sheet());
                fields.push(field(TipKind, "Tip", "Tip", 150.0).sheet());
                if matches!(h.tip, HoleTip::Cone { .. }) {
                    fields.push(field(TipAngle, "Tip angle", "Tip", 150.0).sheet());
                }
            }
            fields.push(field(HeadKind, "Head", "Head", 180.0).sheet());
            if h.head.is_some() {
                fields.push(field(HeadDiameter, "Head diameter", "Head", 180.0).sheet());
            }
            match h.head {
                Some(HoleHead::Counterbore { .. }) => {
                    fields.push(field(HeadDepth, "Head depth", "Head", 170.0).sheet())
                }
                Some(HoleHead::Countersink { .. }) => {
                    fields.push(field(HeadAngle, "Head angle", "Head", 170.0).sheet())
                }
                None => {}
            }
            ("hole", Icon::Hole, fields)
        }
        EditorParams::Pattern(p) => {
            let mut fields = vec![
                field(Body, "Body", "Seed", 170.0),
                field(Layout, "Layout", "Layout", 165.0),
                field(Count, "Copies", "Layout", 135.0),
            ];
            let icon = match &p.layout {
                PatternSpec::Linear { .. } => {
                    fields.push(field(Spacing, "Spacing", "Layout", 150.0));
                    Icon::LinearPattern
                }
                PatternSpec::Circular { .. } => {
                    fields.push(field(Angle, "Angle", "Layout", 145.0));
                    fields.push(field(AxisPoint, "Axis point", "Direction", 200.0).sheet());
                    Icon::CircularPattern
                }
            };
            fields.push(field(Direction, "Direction", "Direction", 180.0).sheet());
            ("pattern", icon, fields)
        }
        EditorParams::ReferencePlane(_) => (
            "plane",
            Icon::Plane,
            vec![field(PlaneSource, "From", "Source", 220.0), field(Offset, "Offset", "Offset", 160.0)],
        ),
        EditorParams::Mirror(_) => (
            "mirror",
            Icon::Mirror,
            vec![field(PlaneSource, "Plane", "Plane", 220.0), field(KeepOriginal, "", "Plane", 150.0)],
        ),
        EditorParams::Draft(_) => (
            "draft",
            Icon::Draft,
            vec![
                field(DraftFaces, "Faces", "Picks", 190.0),
                field(NeutralPlane, "Neutral plane", "Neutral plane", 240.0),
                field(Angle, "Angle", "Size", 150.0),
                field(Reverse, "", "Size", 120.0),
            ],
        ),
        EditorParams::Shell(_) => (
            "shell",
            Icon::Shell,
            vec![field(OpenFaces, "Open faces", "Picks", 210.0), field(Thickness, "Thickness", "Wall", 180.0)],
        ),
        EditorParams::Lattice(l) => (
            "lattice",
            Icon::Cad("lattice"),
            vec![
                field(Body, "Body", "Body", 150.0),
                field(Cell, "Cell", "Cell", 120.0),
                field(CellSize, "Cell size", "Cell", 160.0),
                field(BeamRadius, "Beam radius", "Cell", 180.0),
                field(BeamGrade, "", "Beam gradient", 280.0)
                    .sheet()
                    .count(if matches!(l.radius_mm, Some(Grade::Linear(_))) { 5 } else { 1 }),
                field(Accuracy, "Accuracy", "Accuracy", 170.0).sheet(),
                field(MinimumDetail, "Minimum detail", "Accuracy", 180.0).sheet(),
            ],
        ),
        EditorParams::Gyroid(g) => {
            (
                "gyroid",
                Icon::Cad("lattice"),
                vec![
                    field(Body, "Body", "Body", 150.0),
                    field(Period, "Period", "Network", 150.0),
                    field(Level, "Level", "Network", 155.0),
                    field(LevelGrade, "", "Level gradient", 280.0)
                        .sheet()
                        .count(if matches!(g.level, Grade::Linear(_)) { 5 } else { 1 }),
                    field(Accuracy, "Accuracy", "Accuracy", 170.0).sheet(),
                    field(MinimumDetail, "Minimum detail", "Accuracy", 180.0).sheet(),
                ],
            )
        }
        EditorParams::PickPlane => {
            let mut fields = vec![
                field(PlaneDatum(0), "", "Origin planes", 130.0),
                field(PlaneDatum(1), "", "Origin planes", 130.0),
                field(PlaneDatum(2), "", "Origin planes", 130.0),
            ];
            fields.extend(
                plane_choices.iter().map(|(id, name)| {
                    Field::new(PlaneFeature(id.0 as u64), name, "Construction planes", 180.0).sheet()
                }),
            );
            ("pick_plane", Icon::Sketch, fields)
        }
        EditorParams::Repair { items, .. } => (
            "repair",
            Icon::Repair,
            vec![
                field(RepairWas, "Was", "Current reference", 220.0),
                field(RepairReplacement, "Replace with", "Current reference", 250.0),
                field(RepairNavigation, "", "Current reference", 150.0),
                field(RepairList, "", "Missing references", 280.0).sheet().count(items.len()),
            ],
        ),
    };
    Spec { kind, icon, confirm: !matches!(params, EditorParams::PickPlane), fields }
}

struct Context {
    sketches: Vec<(crate::document::FeatureId, String)>,
    planes: Vec<(soliddraft_model::FeatureId, String)>,
    datums: Vec<(crate::document::PlaneKind, String)>,
    sweep_radius: f64,
    loft_choices: Vec<(soliddraft_model::LoftProfile, String)>,
}

pub fn show(ui: &mut egui::Ui, state: &mut crate::state::AppState) {
    use crate::{form_bar, state::AppAction};
    let Some(editor) = &state.editor else { return };
    let target = editor.target.and_then(|id| state.doc.index_of(id)).unwrap_or(state.doc.features.len());
    let context = Context {
        loft_choices: if matches!(editor.params, EditorParams::Loft(_)) {
            crate::loft_panel::choices(&state.doc, target)
        } else {
            Vec::new()
        },
        sketches: state.doc.features.iter().filter(|f| f.is_sketch()).map(|f| (f.id, f.name.clone())).collect(),
        planes: state
            .doc
            .features
            .iter()
            .take(target)
            .filter(|f| matches!(f.kind, crate::document::FeatureKind::ReferencePlane(_)))
            .map(|f| (soliddraft_model::FeatureId(f.id as u32), f.name.clone()))
            .collect(),
        datums: crate::document::PlaneKind::ALL
            .into_iter()
            .map(|p| (p, state.doc.model().datum_name(p.to_model()).to_owned()))
            .collect(),
        sweep_radius: match &editor.params {
            EditorParams::Sweep(s) => state.doc.sweep_anchor(s.sketch.0 as u64).map_or(10.0, |(_, radius)| radius),
            _ => 0.0,
        },
    };
    let spec = spec(&editor.params, &context.planes);
    let title = editor.title();
    let key = egui::Id::new(("feature_form", spec.kind, editor.target));
    let error = editor
        .feedback
        .size
        .as_ref()
        .or(editor.feedback.pick.as_ref())
        .or(editor.error.as_ref())
        .map(|s| crate::geometry_message::for_user(s));
    let mut actions = Vec::new();
    let editor = state.editor.as_mut().expect("open feature form");
    let response = form_bar::show(
        ui,
        &mut state.form_bar,
        form_bar::Form {
            key,
            title: &title,
            icon: spec.icon,
            fields: &spec.fields,
            can_confirm: spec.confirm,
            error: error.as_deref(),
        },
        |ui, field| render(ui, field, editor, &context, &mut actions),
    );
    if response.confirm {
        actions.push(AppAction::CommitEditor);
    }
    if response.cancel {
        actions.push(AppAction::CancelEditor);
    }
    for action in actions {
        state.push(action);
    }
}

fn number(ui: &mut egui::Ui, label: &str, unit: &str, widget: impl egui::Widget) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(36.0), 24.0), egui::Sense::hover());
    let response = crate::form_bar::value_cell(ui, rect, unit, |ui, inner| ui.put(inner, widget));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, format!("{label} value")));
}

fn mm(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    number(ui, label, "mm", egui::DragValue::new(value).speed(0.5).range(0.01..=100_000.0).fixed_decimals(2));
}

fn choice<T: Copy + PartialEq>(ui: &mut egui::Ui, name: &str, value: &mut T, choices: &[(T, &str)]) {
    let text = choices.iter().find(|(v, _)| v == value).map_or("Custom", |(_, s)| *s);
    egui::ComboBox::from_id_salt(name).width(ui.available_width().max(40.0)).selected_text(text).show_ui(ui, |ui| {
        for (v, label) in choices {
            ui.selectable_value(value, *v, *label);
        }
    });
}

fn readout(ui: &mut egui::Ui, text: impl Into<String>) {
    let text = text.into();
    ui.add(egui::Label::new(&text).truncate()).on_hover_text(text);
}

fn axis(ui: &mut egui::Ui, name: &str, value: &mut soliddraft_kernel_api::Vec3) {
    use soliddraft_kernel_api::Vec3;
    choice(
        ui,
        name,
        value,
        &[(Vec3::X, "X"), (Vec3::Y, "Y"), (Vec3::Z, "Z"), (-Vec3::X, "-X"), (-Vec3::Y, "-Y"), (-Vec3::Z, "-Z")],
    );
}

fn end(ui: &mut egui::Ui, value: &mut crate::document::EndKind, cut: bool, linked: bool) {
    use crate::document::EndKind;
    let text = match value {
        EndKind::Blind => "Blind",
        EndKind::ThroughAll => "Through all",
        EndKind::UpToFace(_) => "Up to face",
        EndKind::UpToNext => "Up to next",
    };
    egui::ComboBox::from_id_salt("end").width(ui.available_width()).selected_text(text).show_ui(ui, |ui| {
        ui.selectable_value(value, EndKind::Blind, "Blind");
        if cut {
            ui.selectable_value(value, EndKind::ThroughAll, "Through all");
        }
        if linked {
            let chosen = matches!(value, EndKind::UpToFace(_));
            if ui.selectable_label(chosen, "Up to face").clicked() && !chosen {
                *value = EndKind::UpToFace(None);
            }
        }
        if linked {
            ui.selectable_value(value, EndKind::UpToNext, "Up to next");
        }
    });
}

fn picked_edges(ui: &mut egui::Ui, editor: &mut crate::state::FeatureEditor) {
    let edges = editor.edges().to_vec();
    let text = if edges.is_empty() { "Click edges or faces".into() } else { format!("{} edges", edges.len()) };
    ui.menu_button(text, |ui| {
        ui.set_min_width(240.0);
        if let Some(i) = crate::properties::pick_list(
            ui,
            "Click edges or faces in the viewport",
            Icon::Axis,
            edges.iter().map(|e| e.label.as_str()),
        ) && let Some(edges) = editor.edges_mut()
        {
            edges.remove(i);
        }
    });
}

fn source_plane(source: &soliddraft_model::SketchPlane) -> soliddraft_model::PlaneRef {
    use soliddraft_model::{PlaneRef, SketchPlane};
    match source {
        SketchPlane::Datum { datum, .. } => PlaneRef::Datum { datum: *datum, offset: 0.0 },
        SketchPlane::Feature { feature, .. } => PlaneRef::Feature { feature: *feature },
        SketchPlane::Face { face } | SketchPlane::FaceOffset { face, .. } => PlaneRef::Face { face: face.clone() },
    }
}

fn set_source(source: &mut soliddraft_model::SketchPlane, plane: soliddraft_model::PlaneRef, offset: f64) {
    use soliddraft_model::{PlaneRef, SketchPlane};
    *source = match plane {
        PlaneRef::Datum { datum, .. } => SketchPlane::Datum { datum, offset },
        PlaneRef::Feature { feature } => SketchPlane::Feature { feature, offset },
        PlaneRef::Face { face } => SketchPlane::FaceOffset { face, offset },
    };
}

fn render(
    ui: &mut egui::Ui,
    key: Key,
    editor: &mut crate::state::FeatureEditor,
    context: &Context,
    actions: &mut Vec<crate::state::AppAction>,
) -> bool {
    use crate::{
        document::{EndKind, SketchPlane},
        properties,
        state::AppAction,
    };
    use soliddraft_kernel_api::{HoleDepth, LatticeCell};
    if key == Key::Edges {
        picked_edges(ui, editor);
        return false;
    }
    let sketch_name =
        |id| context.sketches.iter().find(|(i, _)| *i == id).map(|(_, n)| n.as_str()).unwrap_or("Missing sketch");
    match (&mut editor.params, key) {
        (EditorParams::Loft(l), Key::LoftProfiles) => {
            let count = l.profiles.len();
            readout(ui, format!("{count} {}", if count == 1 { "profile" } else { "profiles" }));
        }
        (EditorParams::Loft(l), Key::LoftShape) => {
            use soliddraft_kernel_api::LoftShape;
            choice(ui, "loft_shape", &mut l.shape, &[(LoftShape::Ruled, "Ruled"), (LoftShape::Smooth, "Smooth")]);
        }
        (EditorParams::Loft(l), Key::LoftProfileList) => crate::loft_panel::profiles(ui, l, &context.loft_choices),
        (EditorParams::Extrude(p), Key::Profile) => match p.sketch {
            Some(id) => readout(ui, sketch_name(id)),
            None => {
                let mut rectangle = matches!(p.profile, ProfileKind::Rectangle { .. });
                let was = rectangle;
                choice(ui, "profile", &mut rectangle, &[(true, "Rectangle"), (false, "Circle")]);
                if rectangle != was {
                    p.profile = if rectangle {
                        ProfileKind::Rectangle { width: 60.0, height: 40.0 }
                    } else {
                        ProfileKind::Circle { diameter: 24.0 }
                    };
                }
            }
        },
        (EditorParams::Extrude(p), Key::Width) => {
            if let ProfileKind::Rectangle { width, .. } = &mut p.profile {
                mm(ui, "Width", width);
            }
        }
        (EditorParams::Extrude(p), Key::Height) => {
            if let ProfileKind::Rectangle { height, .. } = &mut p.profile {
                mm(ui, "Height", height);
            }
        }
        (EditorParams::Extrude(p), Key::Diameter) => {
            if let ProfileKind::Circle { diameter } = &mut p.profile {
                mm(ui, "Diameter", diameter);
            }
        }
        (EditorParams::Extrude(p), Key::Plane) => {
            let text = context.datums.iter().find(|(v, _)| *v == p.plane).map(|(_, n)| n.as_str()).unwrap_or("Plane");
            egui::ComboBox::from_id_salt("plane").width(ui.available_width()).selected_text(text).show_ui(ui, |ui| {
                for (plane, name) in &context.datums {
                    ui.selectable_value(&mut p.plane, *plane, name);
                }
            });
        }
        (EditorParams::Extrude(p), Key::End) => end(ui, &mut p.end, false, p.sketch.is_some()),
        (EditorParams::Extrude(p), Key::Depth) => mm(ui, "Depth", &mut p.depth),
        (EditorParams::Extrude(p), Key::UpToFace) => {
            if let EndKind::UpToFace(face) = &p.end {
                readout(ui, face.as_ref().map(|f| f.label.as_str()).unwrap_or("Click a planar face"));
            }
        }
        (EditorParams::Extrude(p), Key::Flip) => {
            ui.checkbox(&mut p.flip, "Flip direction");
        }
        (EditorParams::Cut(p), Key::Profile) => readout(ui, sketch_name(p.sketch)),
        (EditorParams::Cut(p), Key::End) => {
            let mut value = p.end();
            end(ui, &mut value, true, true);
            if value != p.end() {
                p.set_end(value);
            }
        }
        (EditorParams::Cut(p), Key::Depth) => mm(ui, "Depth", &mut p.depth),
        (EditorParams::Cut(p), Key::UpToFace) => {
            if let EndKind::UpToFace(face) = p.end() {
                readout(ui, face.as_ref().map(|f| f.label.as_str()).unwrap_or("Click a planar face"));
            }
        }
        (EditorParams::Cut(p), Key::Flip) => {
            ui.checkbox(&mut p.flip, "Flip direction");
        }
        (EditorParams::Revolve(p), Key::Profile) => readout(ui, sketch_name(p.sketch.0 as u64)),
        (EditorParams::Revolve(p), Key::RevolveAxis) => {
            ui.menu_button("Point and direction", |ui| {
                ui.set_width(260.0);
                for (name, values) in [("origin", &mut p.axis_origin), ("direction", &mut p.axis_direction)] {
                    ui.label(name);
                    ui.horizontal(|ui| {
                        for (i, label) in ["X", "Y", "Z"].iter().enumerate() {
                            properties::coord(ui, &format!("revolve {name} {label}"), label, &mut values[i]);
                        }
                    });
                }
            });
        }
        (EditorParams::Revolve(p), Key::Angle) => {
            number(ui, "Angle", "°", egui::DragValue::new(&mut p.angle_deg).speed(1.0).range(0.0..=360.0))
        }
        (EditorParams::Sweep(p), Key::Profile) => readout(ui, sketch_name(p.sketch.0 as u64)),
        (EditorParams::Sweep(p), Key::PathStart) => {
            if ui
                .button(format!("{:.1}, {:.1}, {:.1}", p.path.start[0], p.path.start[1], p.path.start[2]))
                .on_hover_text("Edit path")
                .clicked()
            {
                return true;
            }
        }
        (EditorParams::Sweep(p), Key::Orientation) => {
            egui::ComboBox::from_id_salt("orientation")
                .width(ui.available_width())
                .selected_text(match p.path.orientation {
                    soliddraft_kernel_api::SweepOrientation::Fixed => "Keep direction",
                    soliddraft_kernel_api::SweepOrientation::FollowPath => "Turn with the path",
                })
                .show_ui(ui, |ui| crate::sweep_panel::orientation(ui, &mut p.path));
            crate::sweep_panel::keep_valid(&mut p.path);
        }
        (EditorParams::Sweep(p), Key::SweepPath) => {
            crate::sweep_panel::page(ui, &mut p.path, context.sweep_radius, properties::coord)
        }
        (EditorParams::Fillet(p), Key::Radius) => {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
            properties::size_input(
                ui,
                "Radius",
                &mut p.radius,
                &mut editor.feedback.size,
                rect,
                egui::Id::new(("feature_size", editor.target, key)),
            );
        }
        (EditorParams::Chamfer(p), Key::Distance) => {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::hover());
            properties::size_input(
                ui,
                "Distance",
                &mut p.distance,
                &mut editor.feedback.size,
                rect,
                egui::Id::new(("feature_size", editor.target, key)),
            );
        }
        (EditorParams::Hole(p), Key::HoleFace) => {
            readout(ui, p.face.as_ref().map(|f| f.label.as_str()).unwrap_or("Click a planar face"))
        }
        (EditorParams::Hole(p), Key::Positions) => {
            if ui.button(format!("{} position{}", p.points.len(), if p.points.len() == 1 { "" } else { "s" })).clicked()
            {
                return true;
            }
        }
        (EditorParams::Hole(p), Key::PositionList) => {
            let mut remove = None;
            for (i, point) in p.points.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}", i + 1));
                    properties::coord(ui, &format!("hole_u{i}"), "u", &mut point.u);
                    properties::coord(ui, &format!("hole_v{i}"), "v", &mut point.v);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove.filter(|_| p.points.len() > 1) {
                p.points.remove(i);
            }
            if ui.button("Add point").clicked() {
                let next = p.points.last().copied().unwrap_or_default();
                p.points.push(crate::sketch::P2::new(next.u + 10.0, next.v));
            }
        }
        (EditorParams::Hole(p), Key::Diameter) => mm(ui, "Diameter", &mut p.diameter),
        (EditorParams::Hole(p), Key::HoleDepth) => {
            let mut through = matches!(p.depth, HoleDepth::Through);
            let was = through;
            choice(ui, "hole_depth", &mut through, &[(true, "Through"), (false, "Blind")]);
            if through != was {
                p.depth = if through { HoleDepth::Through } else { HoleDepth::Blind { depth: 10.0 } };
            }
        }
        (EditorParams::Hole(p), Key::Depth) => {
            if let HoleDepth::Blind { depth } = &mut p.depth {
                mm(ui, "Depth", depth);
            }
        }
        (EditorParams::Hole(p), Key::TipKind) => {
            let mut cone = matches!(p.tip, HoleTip::Cone { .. });
            let was = cone;
            choice(ui, "tip", &mut cone, &[(false, "Flat"), (true, "Cone")]);
            if cone != was {
                p.tip = if cone { HoleTip::Cone { angle_deg: 118.0 } } else { HoleTip::Flat };
            }
        }
        (EditorParams::Hole(p), Key::TipAngle) => {
            if let HoleTip::Cone { angle_deg } = &mut p.tip {
                number(ui, "Tip angle", "°", egui::DragValue::new(angle_deg).speed(0.5).range(1.0..=179.0));
            }
        }
        (EditorParams::Hole(p), Key::HeadKind) => {
            let mut kind = match p.head {
                None => 0,
                Some(HoleHead::Counterbore { .. }) => 1,
                Some(HoleHead::Countersink { .. }) => 2,
            };
            let was = kind;
            choice(ui, "head", &mut kind, &[(0, "None"), (1, "Counterbore"), (2, "Countersink")]);
            if kind != was {
                p.head = match kind {
                    1 => Some(HoleHead::Counterbore { diameter: p.diameter * 1.8, depth: p.diameter }),
                    2 => Some(HoleHead::Countersink { diameter: p.diameter * 2.0, angle_deg: 90.0 }),
                    _ => None,
                };
            }
        }
        (EditorParams::Hole(p), Key::HeadDiameter) => {
            if let Some(HoleHead::Counterbore { diameter, .. } | HoleHead::Countersink { diameter, .. }) = &mut p.head {
                mm(ui, "Head diameter", diameter);
            }
        }
        (EditorParams::Hole(p), Key::HeadDepth) => {
            if let Some(HoleHead::Counterbore { depth, .. }) = &mut p.head {
                mm(ui, "Head depth", depth);
            }
        }
        (EditorParams::Hole(p), Key::HeadAngle) => {
            if let Some(HoleHead::Countersink { angle_deg, .. }) = &mut p.head {
                number(ui, "Head angle", "°", egui::DragValue::new(angle_deg).speed(0.5).range(1.0..=179.0));
            }
        }
        (EditorParams::Pattern(_), Key::Body)
        | (EditorParams::Lattice(_), Key::Body)
        | (EditorParams::Gyroid(_), Key::Body) => readout(ui, "Part body"),
        (EditorParams::Pattern(p), Key::Layout) => readout(
            ui,
            match p.layout {
                PatternSpec::Linear { .. } => "Linear",
                PatternSpec::Circular { .. } => "Circular",
            },
        ),
        (EditorParams::Pattern(p), Key::Count) => {
            let (PatternSpec::Linear { count, .. } | PatternSpec::Circular { count, .. }) = &mut p.layout;
            number(ui, "Copies", "", egui::DragValue::new(count).speed(1.0).range(2..=1000));
        }
        (EditorParams::Pattern(p), Key::Spacing) => {
            if let PatternSpec::Linear { spacing, .. } = &mut p.layout {
                mm(ui, "Spacing", spacing);
            }
        }
        (EditorParams::Pattern(p), Key::Angle) => {
            if let PatternSpec::Circular { angle, .. } = &mut p.layout {
                number(
                    ui,
                    "Angle",
                    "°",
                    egui::DragValue::new(angle).speed(0.5).range(0.01..=100_000.0).fixed_decimals(2),
                );
            }
        }
        (EditorParams::Pattern(p), Key::Direction) => {
            let direction = match &mut p.layout {
                PatternSpec::Linear { direction, .. } => direction,
                PatternSpec::Circular { axis_direction, .. } => axis_direction,
            };
            axis(ui, "pattern_dir", direction);
        }
        (EditorParams::Pattern(p), Key::AxisPoint) => {
            if let PatternSpec::Circular { axis_origin, .. } = &mut p.layout {
                ui.menu_button("Coordinates", |ui| {
                    ui.set_width(260.0);
                    ui.horizontal(|ui| {
                        properties::coord(ui, "pattern origin X", "X", &mut axis_origin.x);
                        properties::coord(ui, "pattern origin Y", "Y", &mut axis_origin.y);
                        properties::coord(ui, "pattern origin Z", "Z", &mut axis_origin.z);
                    });
                });
            }
        }
        (EditorParams::ReferencePlane(source), Key::PlaneSource) => {
            let mut plane = source_plane(source);
            let offset = source.offset();
            properties::plane_combo(ui, "reference_plane_source", &mut plane, &context.planes);
            set_source(source, plane, offset);
        }
        (EditorParams::ReferencePlane(source), Key::Offset) => {
            let mut offset = source.offset();
            let plane = source_plane(source);
            number(ui, "Offset", "mm", egui::DragValue::new(&mut offset).speed(0.5).range(-100_000.0..=100_000.0));
            set_source(source, plane, offset);
        }
        (EditorParams::Mirror(p), Key::PlaneSource) => {
            properties::plane_combo(ui, "mirror_plane", &mut p.plane, &context.planes)
        }
        (EditorParams::Mirror(p), Key::KeepOriginal) => {
            ui.checkbox(&mut p.keep_original, "Keep original");
        }
        (EditorParams::Draft(p), Key::DraftFaces) => {
            ui.menu_button(format!("{} faces", p.faces.len()), |ui| {
                ui.set_min_width(240.0);
                if let Some(i) = properties::pick_list(
                    ui,
                    "Click faces in the viewport",
                    Icon::Plane,
                    p.faces.iter().map(|f| f.label.as_str()),
                ) {
                    p.faces.remove(i);
                }
            });
        }
        (EditorParams::Draft(p), Key::NeutralPlane) => {
            ui.menu_button(crate::document::plane_ref_label(&p.neutral), |ui| {
                ui.set_min_width(260.0);
                properties::plane_combo(ui, "draft_neutral", &mut p.neutral, &context.planes);
                ui.checkbox(&mut p.picking_neutral, "Next face click sets the neutral plane");
            });
        }
        (EditorParams::Draft(p), Key::Angle) => number(
            ui,
            "Angle",
            "°",
            egui::DragValue::new(&mut p.angle).speed(0.5).range(0.01..=100_000.0).fixed_decimals(2),
        ),
        (EditorParams::Draft(p), Key::Reverse) => {
            ui.checkbox(&mut p.reverse, "Reverse pull");
        }
        (EditorParams::Shell(p), Key::OpenFaces) => {
            let label = if p.open_faces.is_all() {
                "Every face".into()
            } else if let soliddraft_model::ShellOpenings::Faces(faces) = &p.open_faces {
                format!("{} faces", faces.len())
            } else {
                "Faces".into()
            };
            ui.menu_button(label, |ui| {
                ui.set_min_width(280.0);
                let mut all = p.open_faces.is_all();
                if ui.checkbox(&mut all, "Open every face").changed() {
                    p.open_faces = if all {
                        soliddraft_model::ShellOpenings::All
                    } else {
                        soliddraft_model::ShellOpenings::Faces(Vec::new())
                    };
                }
                if let soliddraft_model::ShellOpenings::Faces(faces) = &mut p.open_faces
                    && let Some(i) = properties::pick_list(
                        ui,
                        "Click faces to open; leave empty for a closed shell",
                        Icon::Plane,
                        faces.iter().map(|f| f.label.as_str()),
                    )
                {
                    faces.remove(i);
                }
            });
        }
        (EditorParams::Shell(p), Key::Thickness) => {
            number(ui, "Wall thickness", "mm", egui::DragValue::new(&mut p.thickness).speed(0.1))
        }
        (EditorParams::Lattice(p), Key::Cell) => {
            p.cell = Some(LatticeCell::Cubic);
            readout(ui, "Cubic");
        }
        (EditorParams::Lattice(p), Key::CellSize) => {
            number(ui, "Cell size", "mm", egui::DragValue::new(p.cell_size_mm.get_or_insert(0.0)).speed(0.1))
        }
        (EditorParams::Lattice(p), Key::BeamRadius) => match p.radius_mm.get_or_insert(Grade::Constant(0.0)) {
            Grade::Constant(v) => number(ui, "Beam radius", "mm", egui::DragValue::new(v).speed(0.1)),
            Grade::Linear(_) => {
                if ui.button("Linear along an axis").clicked() {
                    return true;
                }
            }
        },
        (EditorParams::Lattice(p), Key::BeamGrade) => {
            properties::grade_options(ui, "Beam radius", p.radius_mm.get_or_insert(Grade::Constant(0.0)), "mm", 0.1)
        }
        (EditorParams::Lattice(p), Key::Accuracy) => {
            number(ui, "Accuracy", "mm", egui::DragValue::new(&mut p.accuracy_mm).speed(0.01))
        }
        (EditorParams::Lattice(p), Key::MinimumDetail) => {
            number(ui, "Minimum detail", "mm", egui::DragValue::new(&mut p.min_detail_mm).speed(0.01))
        }
        (EditorParams::Gyroid(p), Key::Period) => {
            number(ui, "Period", "mm", egui::DragValue::new(&mut p.period_mm).speed(0.1))
        }
        (EditorParams::Gyroid(p), Key::Level) => match &mut p.level {
            Grade::Constant(v) => number(ui, "Level", "", egui::DragValue::new(v).speed(0.01)),
            Grade::Linear(_) => {
                if ui.button("Linear along an axis").clicked() {
                    return true;
                }
            }
        },
        (EditorParams::Gyroid(p), Key::LevelGrade) => {
            properties::grade_options(ui, "Network level", &mut p.level, "", 0.01)
        }
        (EditorParams::Gyroid(p), Key::Accuracy) => {
            number(ui, "Accuracy", "mm", egui::DragValue::new(&mut p.accuracy_mm).speed(0.01))
        }
        (EditorParams::Gyroid(p), Key::MinimumDetail) => {
            number(ui, "Minimum detail", "mm", egui::DragValue::new(&mut p.min_detail_mm).speed(0.01))
        }
        (EditorParams::PickPlane, Key::PlaneDatum(i)) => {
            if let Some((plane, name)) = context.datums.get(i as usize)
                && ui.button(name).clicked()
            {
                actions.push(AppAction::StartSketchOn(SketchPlane::Reference { plane: *plane, offset: 0.0 }));
            }
        }
        (EditorParams::PickPlane, Key::PlaneFeature(id)) => {
            if let Some((_, name)) = context.planes.iter().find(|(v, _)| v.0 as u64 == id)
                && ui.button(name).clicked()
            {
                actions.push(AppAction::StartSketchOn(SketchPlane::Feature { feature: id, offset: 0.0 }));
            }
        }
        (EditorParams::Repair { items, active, .. }, Key::RepairWas) => {
            readout(ui, items.get(*active).map(|v| v.label.as_str()).unwrap_or("No missing references"));
        }
        (EditorParams::Repair { chosen, active, .. }, Key::RepairReplacement) => {
            readout(
                ui,
                chosen.get(*active).and_then(|v| v.as_ref()).map(|v| v.label()).unwrap_or("Pick a replacement"),
            );
        }
        (EditorParams::Repair { items, active, .. }, Key::RepairNavigation) => {
            if ui.small_button("Previous").clicked() {
                *active = active.saturating_sub(1);
            }
            if ui.small_button("Next").clicked() {
                *active = (*active + 1).min(items.len().saturating_sub(1));
            }
        }
        (EditorParams::Repair { items, chosen, active }, Key::RepairList) => {
            for (i, item) in items.iter().enumerate() {
                if ui.selectable_label(*active == i, &item.label).clicked() {
                    *active = i;
                }
                ui.label(chosen.get(i).and_then(|v| v.as_ref()).map(|v| v.label()).unwrap_or("Pick a replacement"));
            }
        }
        _ => {}
    }
    false
}
