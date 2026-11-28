//! The section tool's offset and aligned cutting lines, and the break and half section
//! tools, driven by clicks on a drawing of the sample bracket in the application: the
//! cut faces have their true areas, the panel buttons flip and remove a half section,
//! a break undoes and redoes, and all of it is there when the drawing reopens.

use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::document::{Command, CutParams, Document, ExtrudeParams, PlaneKind, SketchPlane};
use soliddraft_app::drawing::{DrawingAction, DrawingSel, DrawingSession, DrawingTool};
use soliddraft_app::sketch::{EntityKind, P2, Sketch};
use soliddraft_app::state::AppAction;
use soliddraft_drawing::{
    AlignAxis, AnnotationKind, Command as DrawCommand, CurveKind, DimValue, SetKind, View, ViewId, ViewKind, ViewScale,
};
use soliddraft_kernel_api::{Point2, Vec3};

type App = Harness<'static, SolidDraftApp>;

fn rectangle(a: P2, b: P2) -> Sketch {
    let mut s = Sketch::new();
    let corners = [a, P2::new(b.u, a.v), b, P2::new(a.u, b.v)];
    for i in 0..4 {
        s.add(EntityKind::Line { a: corners[i], b: corners[(i + 1) % 4] }, false);
    }
    s
}

fn circles(list: &[(P2, f64)]) -> Sketch {
    let mut s = Sketch::new();
    for (center, radius) in list {
        s.add(EntityKind::Circle { center: *center, radius: *radius }, false);
    }
    s
}

/// The sample bracket: a 120 x 90 x 10 plate, a 10 x 90 x 60 flange on its -X end, a
/// 24 mm boss 30 high with a 12 mm bore through, and four 9 mm holes at x 25 and 115,
/// y 23 and 67.
fn bracket() -> Document {
    let mut doc = Document::new("Part1", std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()));
    doc.name = "Bracket".into();
    let sketch = |doc: &mut Document, sketch: Sketch| -> u64 {
        let plane = SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 };
        let id = doc.apply(Command::AddSketch { plane }).unwrap().unwrap();
        doc.apply(Command::EditSketch { id, sketch }).unwrap();
        id
    };
    let extrude = |doc: &mut Document, sketch: u64, depth: f64| {
        let params = ExtrudeParams { sketch: Some(sketch), depth, ..Default::default() };
        doc.apply(Command::AddExtrude { params, via_assistant: false }).unwrap().unwrap();
    };
    let cut = |doc: &mut Document, sketch: u64| {
        let params = CutParams {
            sketch,
            depth: 40.0,
            through_all: true,
            flip: true,
            up_to: None,
            draft: None,
            choices: Default::default(),
        };
        doc.apply(Command::AddCut { params }).unwrap().unwrap();
    };
    let s = sketch(&mut doc, rectangle(P2::ZERO, P2::new(120.0, 90.0)));
    extrude(&mut doc, s, 10.0);
    let s = sketch(&mut doc, rectangle(P2::ZERO, P2::new(10.0, 90.0)));
    extrude(&mut doc, s, 60.0);
    let s = sketch(&mut doc, circles(&[(P2::new(70.0, 45.0), 12.0)]));
    extrude(&mut doc, s, 30.0);
    let s = sketch(&mut doc, circles(&[(P2::new(70.0, 45.0), 6.0)]));
    cut(&mut doc, s);
    let holes = [(25.0, 23.0), (25.0, 67.0), (115.0, 23.0), (115.0, 67.0)].map(|(u, v)| (P2::new(u, v), 4.5));
    let s = sketch(&mut doc, circles(&holes));
    cut(&mut doc, s);
    for f in &doc.features {
        assert_eq!(f.status, soliddraft_app::document::FeatureStatus::Ok, "{}: {:?}", f.name, f.status);
    }
    doc.selection = None;
    doc.dirty = false;
    doc
}

fn action(app: &mut App, action: AppAction) {
    app.state_mut().state.push(action);
    app.run_steps(3);
}

fn drawing(app: &mut App, command: DrawingAction) {
    action(app, AppAction::Drawing(command));
}

fn session(app: &App) -> &DrawingSession {
    app.state().state.drawing.session.as_ref().expect("a drawing is open")
}

fn settle(app: &mut App) {
    app.run_steps(12);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    while app.state().state.doc.proof_pending() {
        assert!(std::time::Instant::now() < deadline, "the part proof did not settle");
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.run_steps(1);
    }
    app.run_steps(2);
}

fn click_at(app: &mut App, pos: Pos2) {
    app.hover_at(pos);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        app.step();
    }
    app.run_steps(3);
}

fn click_model(app: &mut App, view: ViewId, model: Vec3) {
    let at = session(app).screen_of_model(view, model).expect("the view has a transform");
    click_at(app, at);
}

fn click_label(app: &mut App, label: &str) {
    app.get_by_label(label).click();
    app.run_steps(3);
}

fn view_named(app: &App, name: &str) -> ViewId {
    let s = session(app);
    let sheet = s.sheet();
    sheet
        .views
        .iter()
        .find(|v| soliddraft_drawing::project::view_label_in(&s.doc.drawing, sheet, v) == name)
        .unwrap_or_else(|| panic!("no view {name}"))
        .id
}

fn view(app: &App, id: ViewId) -> View {
    session(app).doc.drawing.view(id).expect("the view is on the drawing").clone()
}

fn sections(app: &App) -> Vec<View> {
    let s = session(app);
    s.doc
        .drawing
        .sheets
        .iter()
        .flat_map(|s| &s.views)
        .filter(|v| matches!(v.kind, ViewKind::Section { .. }))
        .cloned()
        .collect()
}

fn ring_area(ring: &[Point2]) -> f64 {
    let twice: f64 = (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a.u * b.v - b.u * a.v
        })
        .sum();
    twice.abs() / 2.0
}

/// The hatched area in model square millimetres.
fn cut_area(app: &App, v: &View) -> f64 {
    let s = session(app);
    let scale = soliddraft_drawing::project::effective_scale(s.sheet(), v);
    let area: f64 =
        v.geometry.hatch.iter().map(|h| ring_area(&h.outer) - h.holes.iter().map(|r| ring_area(r)).sum::<f64>()).sum();
    area / (scale * scale)
}

fn near(actual: f64, expected: f64, tolerance: f64, what: &str) {
    assert!((actual - expected).abs() <= tolerance, "{what}: {actual} is not {expected}");
}

/// Places the pending section to the right of `beside`.
fn place_right_of(app: &mut App, beside: ViewId, by: f64) {
    assert!(session(app).draft.pending.is_some(), "the section is computed once its line is drawn");
    let p = view(app, beside).position;
    let at = session(app).screen_of(Point2::new(p.u + by, p.v));
    click_at(app, at);
    settle(app);
}

fn bracket_drawing(root: &std::path::Path) -> App {
    let _ = std::fs::remove_dir_all(root);
    std::fs::create_dir_all(root.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(root.join("config")));
    let doc = bracket();
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc, Automation::default()));
    app.run_steps(6);
    drawing(&mut app, DrawingAction::MakeDrawing);
    drawing(&mut app, DrawingAction::CreateDrawing);
    settle(&mut app);
    app.state_mut().state.options_details = true;
    app
}

#[test]
fn offset_aligned_half_and_broken_views_from_the_drawing_tools() {
    let root = std::env::temp_dir().join(format!("soliddraft-cut-tools-{}", std::process::id()));
    let mut app = bracket_drawing(&root);
    let (top, front) = (view_named(&app, "Top view"), view_named(&app, "Front view"));

    // An offset section through two staggered holes. On y = 23 up to x = 70 the flange
    // (10 x 60) and plate (60 x 10) less the
    // hole (9 x 10); on y = 67 beyond it the plate (50 x 10) less the other hole.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Section { replace: None }));
    click_label(&mut app, "Offset (stepped)");
    let first = Vec3::new(25.0, 23.0, 10.0);
    click_model(&mut app, top, first);
    click_model(&mut app, top, first);
    assert_eq!(session(&app).draft.points.len(), 2);
    assert!(session(&app).draft.pending.is_none());
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Section { replace: None }));
    click_label(&mut app, "Offset (stepped)");
    click_model(&mut app, top, first);
    click_model(&mut app, top, Vec3::new(115.0, 67.0, 10.0));
    assert!(session(&app).draft.pending.is_none());
    click_label(&mut app, "Finish cutting line");
    place_right_of(&mut app, top, 160.0);
    let offset = sections(&app).pop().expect("an offset section");
    let ViewKind::Section { line, model_line, .. } = &offset.kind else { unreachable!() };
    assert_eq!((line.len(), model_line.len()), (4, 4), "two legs and a jog: {line:?}");
    near(cut_area(&app, &offset), 1110.0 + 410.0, 1e-3, "offset cut area");
    drawing(&mut app, DrawingAction::Select(Some(DrawingSel::View(offset.id))));
    assert!(shows(&app, "offset, 2 parallel cuts"), "the panel names the offset cut");

    // An aligned section about the bore: a leg along +X, the pivot on the bore's axis
    // (it snaps), a leg along +Y turned into the plane of the first. Each leg cuts the
    // boss wall beside the bore (6 x 30) and the plate beyond it.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Section { replace: None }));
    click_label(&mut app, "Aligned (turned)");
    click_model(&mut app, top, Vec3::new(100.0, 45.0, 10.0));
    click_model(&mut app, top, Vec3::new(70.5, 45.5, 10.0));
    click_model(&mut app, top, Vec3::new(70.0, 75.0, 10.0));
    place_right_of(&mut app, top, 280.0);
    let aligned = sections(&app).into_iter().find(|v| v.id != offset.id).expect("an aligned section");
    let ViewKind::Section { line, .. } = &aligned.kind else { unreachable!() };
    assert_eq!(line.len(), 3, "two legs about a pivot: {line:?}");
    near(cut_area(&app, &aligned), (180.0 + 380.0) + (180.0 + 330.0), 1e-3, "aligned cut area");
    drawing(&mut app, DrawingAction::Select(Some(DrawingSel::View(aligned.id))));
    assert!(shows(&app, "aligned, 2 legs about a pivot"), "the panel names the aligned cut");

    // A half section of the front view about the bore's axis; the panel flips it,
    // then removes it.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::HalfSection));
    click_model(&mut app, front, Vec3::new(70.0, 0.0, 0.0));
    click_model(&mut app, front, Vec3::new(70.0, 0.0, 30.0));
    let half = view(&app, front);
    let made = half.half.clone().expect("the front view is a half section");
    assert!((made.axis[0].x - 70.0).abs() < 1e-6 && (made.axis[1].x - 70.0).abs() < 1e-6, "{made:?}");
    assert!(half.geometry.symmetry_segments.len() == 1 && !half.geometry.hatch.is_empty(), "the cut half is hatched");
    for point in made.axis {
        near(point.y, 45.0, 1e-6, "the cutting plane is midway through the 90 mm plate");
    }
    drawing(
        &mut app,
        DrawingAction::Command(DrawCommand::SetViewScale { view: front, scale: ViewScale::Custom { factor: 0.5 } }),
    );
    let after = view(&app, front);
    let [[a, b]] = after.geometry.symmetry_segments[..] else { panic!("one symmetry line") };
    near(a.u, b.u, 1e-6, "the scaled centreline is still vertical");
    let axis_x = session(&app).doc.transforms[&front].point(made.axis[0]).u;
    near(a.u, axis_x, 1e-6, "the scaled centreline follows the model axis");
    drawing(&mut app, DrawingAction::Undo);
    drawing(&mut app, DrawingAction::Select(Some(DrawingSel::View(front))));
    click_label(&mut app, "Flip half");
    assert_eq!(view(&app, front).half.map(|h| h.flip), Some(!made.flip));
    click_label(&mut app, "Remove half section");
    assert!(view(&app, front).half.is_none() && view(&app, front).geometry.hatch.is_empty());

    // A break leaving out x 30 to 60 of the front view: 30 mm closed up to a 6 mm gap.
    let width = |v: &View| v.geometry.bbox[2] - v.geometry.bbox[0];
    let (whole, scale) = {
        let v = view(&app, front);
        (width(&v), soliddraft_drawing::project::effective_scale(session(&app).sheet(), &v))
    };
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Break));
    click_model(&mut app, front, Vec3::new(30.0, 0.0, 5.0));
    click_model(&mut app, front, Vec3::new(60.0, 0.0, 6.0));
    let broken = view(&app, front);
    assert_eq!(broken.breaks.len(), 1);
    assert_eq!(broken.breaks[0].axis, AlignAxis::Horizontal);
    near(broken.breaks[0].from.x.min(broken.breaks[0].to.x), 30.0, 0.5, "the break starts at x 30");
    near(broken.breaks[0].from.x.max(broken.breaks[0].to.x), 60.0, 0.5, "the break ends at x 60");
    let shortened = whole - (broken.breaks[0].to.x - broken.breaks[0].from.x).abs() * scale + 6.0;
    // The clicks land in single-precision screen coordinates.
    near(width(&broken), shortened, 1e-3, "the broken view's width");
    assert_eq!(broken.geometry.breaks.len(), 1, "the gap is drawn");
    drawing(&mut app, DrawingAction::Undo);
    assert!(view(&app, front).breaks.is_empty());
    near(width(&view(&app, front)), whole, 1e-6, "undo restores the whole view");
    drawing(&mut app, DrawingAction::Redo);
    assert_eq!(view(&app, front).breaks, broken.breaks);

    drawing(
        &mut app,
        DrawingAction::Command(DrawCommand::SetViewScale { view: front, scale: ViewScale::Custom { factor: 0.5 } }),
    );
    let scaled = view(&app, front);
    let gap = scaled.geometry.breaks[0];
    let xf = session(&app).doc.transforms[&front];
    let left = xf.point(broken.breaks[0].from).u.min(xf.point(broken.breaks[0].to).u);
    near(gap.at, xf.gaps()[0].at, 1e-6, "the scaled break mark follows the picking transform");
    near(gap.gap, 6.0, 1e-9, "the paper gap stays 6 mm");
    assert!(
        scaled
            .geometry
            .curves
            .iter()
            .flat_map(|c| c.path.elements())
            .filter_map(|e| e.end_point())
            .all(|p| p.x <= left + 1e-5 || p.x >= left + gap.gap - 1e-5)
    );
    drawing(&mut app, DrawingAction::Undo);

    // Saved and reopened as it is.
    let path = root.join("bracket.dwdrw");
    drawing(&mut app, DrawingAction::Save(Some(path.clone())));
    drawing(&mut app, DrawingAction::CloseDrawing);
    drawing(&mut app, DrawingAction::OpenFile(path));
    settle(&mut app);
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::SmartDimension));
    let reopened = view(&app, front);
    let end = reopened
        .geometry
        .curves
        .iter()
        .find(|curve| {
            curve.visible
                && curve.ends.is_some_and(|ends| {
                    let gap = reopened.geometry.breaks[0];
                    (ends[0].v - ends[1].v).abs() < 1e-6
                        && ends[0].u.min(ends[1].u) < gap.at
                        && ends[0].u.max(ends[1].u) > gap.at + gap.gap
                })
        })
        .expect("a line crossing the break");
    let [p, q] = end.ends.unwrap();
    let gap = reopened.geometry.breaks[0];
    let left = if p.u < q.u { p } else { q };
    let at = Point2::new((left.u + gap.at) / 2.0, left.v);
    click_view_point(&mut app, &reopened, at);
    let proposal = session(&app).proposal.as_ref().expect("a measured proposal after reopen");
    let scale = soliddraft_drawing::project::view_scale(&session(&app).doc.drawing, front).unwrap();
    near(
        proposal.distance(soliddraft_drawing::LinearOrientation::Horizontal, scale),
        120.0,
        1e-5,
        "full bracket width",
    );
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Select));
    assert_eq!(view(&app, front).breaks, broken.breaks);
    near(width(&view(&app, front)), width(&broken), 1e-9, "the reopened broken view's width");
    for (before, what) in [(&offset, "offset"), (&aligned, "aligned")] {
        let after = view(&app, before.id);
        assert_eq!(after.kind, before.kind, "the {what} section's line reopens");
        near(cut_area(&app, &after), cut_area(&app, before), 1e-6, what);
    }
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

/// The bracket with a ledge on the flange's inner face: 15 x 90, from z 30 to 40.
fn ledged_bracket() -> Document {
    let mut doc = bracket();
    let plane = SketchPlane::Reference { plane: PlaneKind::Top, offset: 30.0 };
    let id = doc.apply(Command::AddSketch { plane }).unwrap().unwrap();
    doc.apply(Command::EditSketch { id, sketch: rectangle(P2::new(5.0, 0.0), P2::new(20.0, 90.0)) }).unwrap();
    let params = ExtrudeParams { sketch: Some(id), depth: 10.0, ..Default::default() };
    doc.apply(Command::AddExtrude { params, via_assistant: false }).unwrap().unwrap();
    for f in &doc.features {
        assert_eq!(f.status, soliddraft_app::document::FeatureStatus::Ok, "{}: {:?}", f.name, f.status);
    }
    doc.selection = None;
    doc.dirty = false;
    doc
}

/// Whether the window shows `text` as a label or a value.
fn shows(app: &App, text: &str) -> bool {
    app.query_all(by()).any(|n| {
        let node = n.accesskit_node();
        node.label().as_deref() == Some(text) || node.value().as_deref() == Some(text)
    })
}

/// The straight visible edges a section draws along the sheet's X from aligned leg
/// `leg`: index, height and the two ends' X, in view millimetres.
fn level_edges(v: &View, leg: u8) -> Vec<(usize, f64, [f64; 2])> {
    v.geometry
        .curves
        .iter()
        .enumerate()
        .filter(|(_, c)| c.visible && c.kind == CurveKind::Edge && c.source.section_leg == leg)
        .filter_map(|(i, c)| {
            let [kurbo::PathEl::MoveTo(a), kurbo::PathEl::LineTo(b)] = c.path.elements() else { return None };
            ((a.y - b.y).abs() < 1e-6 && (a.x - b.x).abs() > 5.0).then_some((i, a.y, [a.x, b.x]))
        })
        .collect()
}

fn click_view_point(app: &mut App, view: &View, p: Point2) {
    let at = session(app).screen_of(Point2::new(view.position.u + p.u, view.position.v + p.v));
    click_at(app, at);
}

/// On a square aligned cut the second leg's Y-directed edges show along the sheet's X.
/// The chamfer and centreline tools measure them as drawn, turned with the leg.
#[test]
fn aligned_section_line_tools_measure_the_turned_edges() {
    let root = std::env::temp_dir().join(format!("soliddraft-aligned-lines-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(root.join("config")));
    let doc = ledged_bracket();
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc, Automation::default()));
    app.run_steps(6);
    drawing(&mut app, DrawingAction::MakeDrawing);
    drawing(&mut app, DrawingAction::CreateDrawing);
    settle(&mut app);
    app.state_mut().state.options_details = true;
    let top = view_named(&app, "Top view");

    // A leg along -X to the bore's axis, then one along -Y that looks back at the flange.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Section { replace: None }));
    click_label(&mut app, "Aligned (turned)");
    click_model(&mut app, top, Vec3::new(100.0, 45.0, 10.0));
    click_model(&mut app, top, Vec3::new(70.5, 45.5, 10.0));
    click_model(&mut app, top, Vec3::new(70.0, 15.0, 10.0));
    place_right_of(&mut app, top, 220.0);
    let made = sections(&app).pop().expect("an aligned section");
    // The clicks land in single-precision screen coordinates: square the legs exactly.
    let ViewKind::Section { parent, flip, label, model_line, hatch, .. } = made.kind.clone() else { unreachable!() };
    let z = model_line[0].z;
    let exact = vec![Vec3::new(100.0, 45.0, z), Vec3::new(70.0, 45.0, z), Vec3::new(70.0, 15.0, z)];
    let xf = session(&app).doc.transforms[&top];
    let line = exact.iter().map(|p| xf.point(*p)).collect();
    let kind = ViewKind::Section { parent, line, flip, label, model_line: exact, hatch };
    drawing(&mut app, DrawingAction::Command(DrawCommand::EditView { view: made.id, kind }));
    settle(&mut app);
    let section = view(&app, made.id);

    // The flange top (z 60) and the ledge top (z 40): the two levels furthest from the
    // cut plate.
    let edges = level_edges(&section, 1);
    let plate = section.geometry.hatch.iter().flat_map(|h| &h.outer).map(|p| p.v).sum::<f64>()
        / section.geometry.hatch.iter().map(|h| h.outer.len()).sum::<usize>() as f64;
    let mut levels: Vec<f64> = edges.iter().map(|e| e.1).collect();
    levels.sort_by(|a, b| (b - plate).abs().total_cmp(&(a - plate).abs()));
    levels.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    assert!(levels.len() >= 2, "the second leg draws the flange and ledge edges: {edges:?}");
    let scale = soliddraft_drawing::project::view_scale(&session(&app).doc.drawing, made.id).unwrap();
    near((levels[0] - levels[1]).abs(), 20.0 * scale, 1e-6, "flange top to ledge top");
    let pick = |level: f64| *edges.iter().find(|e| (e.1 - level).abs() < 1e-6).expect("an edge at the level");
    let (upper, lower) = (pick(levels[0]), pick(levels[1]));
    let middle = |e: (usize, f64, [f64; 2])| Point2::new((e.2[0] + e.2[1]) / 2.0, e.1);

    // A chamfer starts from a bevel seen at full length in the view.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Chamfer));
    click_view_point(&mut app, &section, middle(upper));
    assert_eq!(session(&app).draft.anchors.len(), 2, "{:?}", session(&app).message);
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Select));

    // A centreline halfway between the two levels.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Centreline));
    click_view_point(&mut app, &section, middle(upper));
    click_view_point(&mut app, &section, middle(lower));
    let s = session(&app);
    let centreline = s
        .doc
        .drawing
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.annotations)
        .find(|a| a.view == made.id && matches!(a.body, AnnotationKind::Centreline))
        .unwrap_or_else(|| panic!("a centreline: {:?}", s.message));
    assert_eq!(centreline.anchors.len(), 4);
    for (ends, edge) in centreline.anchors.chunks(2).zip([upper, lower]) {
        assert!(ends.iter().all(|a| a.entity.section_leg == 1));
        for end in ends {
            near(end.cached.v, edge.1, 1e-6, "an end on its edge's level");
        }
        let drawn = (edge.2[0] - edge.2[1]).abs();
        assert!((ends[0].cached.u - ends[1].cached.u).abs() >= drawn - 1e-6, "the whole edge, along the sheet's X");
    }
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

/// An offset section moved to a 1:2 sheet keeps its 1:1 parent's scale. Turning its
/// horizontal dimension to aligned on a horizontal edge keeps the model length.
#[test]
fn a_dimension_on_a_section_from_another_sheet_keeps_its_length_when_turned_aligned() {
    use soliddraft_drawing::measure::{Pick, propose};
    use soliddraft_drawing::{Annotation, AnnotationState, Dimension, DimensionKind, LinearOrientation, Scale};
    let root = std::env::temp_dir().join(format!("soliddraft-cross-sheet-dim-{}", std::process::id()));
    let mut app = bracket_drawing(&root);
    let top = view_named(&app, "Top view");
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Section { replace: None }));
    click_label(&mut app, "Offset (stepped)");
    click_model(&mut app, top, Vec3::new(25.0, 23.0, 10.0));
    click_model(&mut app, top, Vec3::new(115.0, 67.0, 10.0));
    click_label(&mut app, "Finish cutting line");
    place_right_of(&mut app, top, 160.0);
    let id = sections(&app).pop().expect("an offset section").id;
    let parent = soliddraft_drawing::project::view_scale(&session(&app).doc.drawing, top).unwrap();
    let other = if (parent - 0.5).abs() < 1e-9 { Scale::new(1, 1) } else { Scale::new(1, 2) };
    drawing(&mut app, DrawingAction::AddSheet);
    drawing(&mut app, DrawingAction::Command(DrawCommand::SetSheetScale { sheet: 1, scale: other }));
    let to = Point2::new(200.0, 150.0);
    drawing(&mut app, DrawingAction::Command(DrawCommand::MoveViewToSheet { view: id, sheet: 1, to }));
    drawing(&mut app, DrawingAction::SetSheet(1));
    settle(&mut app);
    let s = session(&app);
    assert_eq!(soliddraft_drawing::project::sheet_of(&s.doc.drawing, id), Some(1));
    let scale = soliddraft_drawing::project::view_scale(&s.doc.drawing, id).unwrap();
    near(scale, parent, 1e-12, "the section follows its parent on sheet 1");
    assert!((s.doc.drawing.sheets[1].scale.factor() - scale).abs() > 1e-9);

    // A horizontal dimension on the longest horizontal edge.
    let moved = view(&app, id);
    let (curve, _, ends) = level_edges(&moved, 0)
        .into_iter()
        .max_by(|a, b| (a.2[0] - a.2[1]).abs().total_cmp(&(b.2[0] - b.2[1]).abs()))
        .expect("a horizontal edge");
    let proposal = propose(&moved, &[Pick { view: id, curve, end: None }], &[]).expect("a proposal");
    // The whole model edge, though the cut draws only its stretch.
    let length = proposal.distance(LinearOrientation::Horizontal, scale);
    assert!(length >= (ends[0] - ends[1]).abs() / scale - 1e-6, "{length}");
    let dimension =
        Dimension::measured(DimensionKind::Linear { orientation: LinearOrientation::Horizontal }, length, 1);
    let annotation = Annotation {
        id: session(&app).doc.drawing.next_annotation_id(),
        view: id,
        body: AnnotationKind::Dimension(dimension),
        anchors: proposal.anchors,
        placement: Point2::new(0.0, 20.0),
        style: Default::default(),
        state: AnnotationState::Ok,
        source: None,
    };
    let dim = annotation.id;
    drawing(&mut app, DrawingAction::Command(DrawCommand::AddDimension { annotation }));
    drawing(&mut app, DrawingAction::Select(Some(DrawingSel::Annotation(dim))));
    app.run_steps(4);
    click_label(&mut app, "Aligned");
    let after = session(&app).doc.drawing.annotation(dim).expect("the dimension").clone();
    let AnnotationKind::Dimension(d) = &after.body else { panic!("a dimension") };
    assert_eq!(d.kind, DimensionKind::Linear { orientation: LinearOrientation::Aligned });
    let DimValue::Measured { value } = d.value else { panic!("a measured value: {:?}", d.value) };
    near(value, length, 1e-6, "the aligned value is the edge's model length");
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn broken_view_dimensions_measure_the_model_from_native_picks() {
    let root = std::env::temp_dir().join(format!("soliddraft-break-dimensions-{}", std::process::id()));
    let mut app = bracket_drawing(&root);
    let front = view_named(&app, "Front view");
    drawing(
        &mut app,
        DrawingAction::Command(DrawCommand::SetViewBreaks {
            view: front,
            breaks: vec![soliddraft_drawing::Break {
                from: Vec3::new(30.0, 0.0, 0.0),
                to: Vec3::new(60.0, 0.0, 0.0),
                axis: AlignAxis::Horizontal,
            }],
        }),
    );
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::SmartDimension));
    click_model(&mut app, front, Vec3::new(30.0, 0.0, 0.0));
    assert_eq!(session(&app).picks.first().expect("a clipped edge pick").end, None);
    // The right piece's end at the gap is no model end either.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Select));
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::SmartDimension));
    click_model(&mut app, front, Vec3::new(60.0, 0.0, 0.0));
    assert_eq!(session(&app).picks.first().expect("a clipped edge pick").end, None);
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Select));
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::SmartDimension));
    let ends = [Vec3::ZERO, Vec3::new(120.0, 0.0, 0.0)];
    for point in ends {
        click_model(&mut app, front, point);
    }
    assert!(session(&app).picks.iter().all(|pick| pick.end.is_some()));
    let current = session(&app);
    let proposal = current.proposal.as_ref().expect("the dimension preview");
    let scale = soliddraft_drawing::project::view_scale(&current.doc.drawing, front).unwrap();
    near(proposal.distance(soliddraft_drawing::LinearOrientation::Horizontal, scale), 120.0, 1e-6, "preview value");
    let base = session(&app).sheet_point(front, Vec3::new(50.0, 0.0, 0.0)).unwrap();
    let place = session(&app).screen_of(Point2::new(base.u, base.v - 15.0));
    click_at(&mut app, place);
    let dimension = session(&app).sheet().annotations.iter().rev().find(|a| a.view == front &&
        matches!(&a.body, AnnotationKind::Dimension(d) if matches!(d.value, DimValue::Measured { value: 120.0 }))).unwrap().clone();
    for kind in [SetKind::Ordinate, SetKind::Baseline, SetKind::Chain] {
        drawing(&mut app, DrawingAction::SetTool(DrawingTool::DimensionSet(kind)));
        for point in ends {
            click_model(&mut app, front, point);
        }
        assert_eq!(session(&app).draft.anchors.len(), 2, "{kind:?}: {:?}", session(&app).message);
        click_at(&mut app, place);
        let s = session(&app);
        let set = s
            .sheet()
            .annotations
            .iter()
            .find_map(|a| match &a.body {
                AnnotationKind::DimensionSet(set) if a.view == front && set.kind == kind => Some(set),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{kind:?}: {:?}", s.message));
        assert_eq!(set.values, [120.0], "{kind:?}");
    }
    let AnnotationKind::Dimension(mut edited) = dimension.body else { unreachable!() };
    edited.kind =
        soliddraft_drawing::DimensionKind::Linear { orientation: soliddraft_drawing::LinearOrientation::Aligned };
    drawing(
        &mut app,
        DrawingAction::Command(DrawCommand::EditDimension { annotation: dimension.id, dimension: edited }),
    );
    let annotation = session(&app).doc.drawing.annotation(dimension.id).unwrap();
    assert!(matches!(&annotation.body, AnnotationKind::Dimension(d) if
        d.kind == soliddraft_drawing::DimensionKind::Linear { orientation: soliddraft_drawing::LinearOrientation::Aligned }
        && matches!(d.value, DimValue::Measured { value: 120.0 })));

    let dxf =
        soliddraft_drawing::export::dxf(&soliddraft_drawing::display::layout_sheet(&session(&app).doc.drawing, 0))
            .unwrap();
    let entities = soliddraft_drawing::export::dxf::count_entities(&dxf).unwrap();
    assert!(entities.get("DIMENSION/DIMENSION").is_some_and(|n| *n >= 4));
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

/// Model items on a drawing whose front view has a break: each dimension it reports is
/// on the sheet, those of the broken view measured from the model.
#[test]
fn model_items_on_a_broken_view_add_every_dimension_they_report() {
    let root = std::env::temp_dir().join(format!("soliddraft-break-model-items-{}", std::process::id()));
    let mut app = bracket_drawing(&root);
    let front = view_named(&app, "Front view");
    let breaks = vec![soliddraft_drawing::Break {
        from: Vec3::new(30.0, 0.0, 0.0),
        to: Vec3::new(60.0, 0.0, 0.0),
        axis: AlignAxis::Horizontal,
    }];
    drawing(&mut app, DrawingAction::Command(DrawCommand::SetViewBreaks { view: front, breaks }));
    assert_eq!(view(&app, front).breaks.len(), 1);
    let before = session(&app).sheet().annotations.len();
    drawing(&mut app, DrawingAction::ModelItems);
    let s = session(&app);
    let added = s.sheet().annotations.len() - before;
    let said = s.message.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
    assert_eq!(said, format!("Model items: {added} dimension{} imported", if added == 1 { "" } else { "s" }));
    let values: Vec<f64> = s.sheet().annotations[before..]
        .iter()
        .filter(|a| a.view == front)
        .filter_map(|a| match &a.body {
            AnnotationKind::Dimension(d) => match d.value {
                DimValue::Measured { value } => Some(value),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(!values.is_empty(), "the broken front view gets its dimensions: {said}");
    // The bracket's sizes: plate, flange, boss and bore.
    let sizes = [120.0, 90.0, 10.0, 60.0, 30.0, 24.0, 12.0, 9.0];
    assert!(values.iter().all(|v| sizes.iter().any(|s| (v - s).abs() < 1e-6)), "{values:?}");
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A part edit leaves the front view's break above the flange: the view keeps its last
/// line work. The break can still be removed, and the break tool still sets a new one
/// from clicks on the view as drawn.
#[test]
fn a_break_the_part_outgrew_is_removed_or_replaced_from_the_drawing_tools() {
    use soliddraft_app::document::FeatureKind;
    let root = std::env::temp_dir().join(format!("soliddraft-outgrown-break-{}", std::process::id()));
    let mut app = bracket_drawing(&root);
    let front = view_named(&app, "Front view");
    let vertical = |from: f64, to: f64| soliddraft_drawing::Break {
        from: Vec3::new(0.0, 0.0, from),
        to: Vec3::new(0.0, 0.0, to),
        axis: AlignAxis::Vertical,
    };
    let breaks = vec![vertical(42.0, 58.0)];
    drawing(&mut app, DrawingAction::Command(DrawCommand::SetViewBreaks { view: front, breaks }));
    let broken = view(&app, front);
    assert_eq!(broken.breaks, [vertical(42.0, 58.0)], "{:?}", session(&app).message);

    // The flange drops from 60 to 50 high: the break's top end is above the part.
    let (flange, mut params) = app
        .state()
        .state
        .doc
        .features
        .iter()
        .find_map(|f| match &f.kind {
            FeatureKind::Extrude(p) if (p.depth - 60.0).abs() < 1e-9 => Some((f.id, p.clone())),
            _ => None,
        })
        .expect("the flange");
    params.depth = 50.0;
    app.state_mut().state.doc.apply(Command::EditExtrude { id: flange, params }).unwrap();
    settle(&mut app);
    drawing(&mut app, DrawingAction::Rebuild);
    settle(&mut app);
    let stale = view(&app, front);
    assert_eq!(stale.breaks, broken.breaks);
    assert_eq!(stale.geometry, broken.geometry, "the view keeps its last line work");

    drawing(&mut app, DrawingAction::Command(DrawCommand::SetViewBreaks { view: front, breaks: Vec::new() }));
    let whole = view(&app, front);
    assert!(whole.breaks.is_empty() && whole.geometry.breaks.is_empty(), "{:?}", session(&app).message);
    let scale = soliddraft_drawing::project::view_scale(&session(&app).doc.drawing, front).unwrap();
    let [_, y0, _, y1] = whole.geometry.bbox;
    near(y1 - y0, 50.0 * scale, 1e-3, "the whole view of the lower flange");

    drawing(&mut app, DrawingAction::Undo);
    assert_eq!(view(&app, front).breaks, broken.breaks, "undo brings the outgrown break back");
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Break));
    click_model(&mut app, front, Vec3::new(5.0, 0.0, 15.0));
    click_model(&mut app, front, Vec3::new(5.0, 0.0, 35.0));
    let replaced = view(&app, front);
    let [b] = &replaced.breaks[..] else { panic!("one break: {:?}", session(&app).message) };
    assert_eq!(b.axis, AlignAxis::Vertical);
    near(b.from.z.min(b.to.z), 15.0, 0.5, "the new break starts at z 15");
    near(b.from.z.max(b.to.z), 35.0, 0.5, "the new break ends at z 35");
    let [gap] = replaced.geometry.breaks[..] else { panic!("one gap") };
    near(gap.gap, 6.0, 1e-9, "gap");
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

/// The sheet format panel: the paper picked from its list, a title block field's cell
/// moved by typing its x, then undo, redo and reopen.
#[test]
fn paper_and_title_cell_from_the_sheet_format_panel() {
    let root = std::env::temp_dir().join(format!("soliddraft-format-panel-{}", std::process::id()));
    let mut app = bracket_drawing(&root);
    drawing(&mut app, DrawingAction::Select(Some(DrawingSel::Format)));
    assert_eq!(session(&app).sheet().template.as_str(), "iso-a3");

    // A0 from the paper list: a popup's first frame only sizes it.
    click_label(&mut app, "Paper size");
    app.run_steps(5);
    click_label(&mut app, "ISO A0 landscape · first angle");
    settle(&mut app);
    let sheet = session(&app).sheet().clone();
    assert_eq!((sheet.template.as_str(), sheet.size.width_mm, sheet.size.height_mm), ("iso-a0", 1189.0, 841.0));
    let paper = soliddraft_drawing::display::layout_sheet(&session(&app).doc.drawing, 0).paper;
    assert_eq!(paper, (1189.0, 841.0));

    for (control, value) in [("Legal owner width", "54"), ("Legal owner x", "4")] {
        app.get_by_label(control).click();
        app.run_steps(3);
        app.key_press_modifiers(Modifiers::COMMAND, Key::A);
        app.event(egui::Event::Text(value.into()));
        app.run_steps(2);
        app.key_press(Key::Enter);
        app.run_steps(4);
    }
    let moved = session(&app).sheet().title_layout.get("owner").copied().expect("the owner field moved");
    assert_eq!(moved.cell, [4.0, 9.0, 54.0, 9.0]);
    drawing(&mut app, DrawingAction::Select(None));
    let original_owner = session(&app).sheet().format_overrides.get("owner").cloned();
    app.get_by_label("Owner field").focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("Rig Shop".into()));
    app.run_steps(2);
    assert_eq!(session(&app).sheet().format_overrides.get("owner"), original_owner.as_ref());
    app.key_press(Key::Enter);
    app.run_steps(4);
    assert_eq!(session(&app).sheet().format_overrides["owner"], "Rig Shop");
    drawing(&mut app, DrawingAction::Undo);
    assert_eq!(session(&app).sheet().format_overrides.get("owner"), original_owner.as_ref());
    drawing(&mut app, DrawingAction::Undo);
    drawing(&mut app, DrawingAction::Undo);
    assert!(session(&app).sheet().title_layout.is_empty(), "undo puts the field back");
    assert_eq!(session(&app).sheet().template.as_str(), "iso-a0", "and keeps the paper");
    drawing(&mut app, DrawingAction::Undo);
    assert_eq!(session(&app).sheet().template.as_str(), "iso-a3", "a final undo restores the paper");
    for _ in 0..4 {
        drawing(&mut app, DrawingAction::Redo);
    }
    assert_eq!(session(&app).sheet().title_layout.get("owner"), Some(&moved));
    assert_eq!(session(&app).sheet().format_overrides["owner"], "Rig Shop");

    let path = root.join("bracket.dwdrw");
    drawing(&mut app, DrawingAction::Save(Some(path.clone())));
    drawing(&mut app, DrawingAction::CloseDrawing);
    drawing(&mut app, DrawingAction::OpenFile(path));
    settle(&mut app);
    let sheet = session(&app).sheet();
    assert_eq!(sheet.template.as_str(), "iso-a0");
    assert_eq!(sheet.title_layout.get("owner"), Some(&moved));
    assert_eq!(sheet.format_overrides["owner"], "Rig Shop");
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

/// A title value set on the sheet, as `--title-field` sets it, is what the sheet panel
/// shows and edits, and the panel can clear it back to the document value.
#[test]
fn the_sheet_panel_edits_and_clears_a_title_value_set_on_the_sheet() {
    let root = std::env::temp_dir().join(format!("soliddraft-title-panel-{}", std::process::id()));
    let mut app = bracket_drawing(&root);
    let set = DrawCommand::SetTitleField { sheet: 0, key: "title".into(), value: "Release".into() };
    drawing(&mut app, DrawingAction::Command(set));
    drawing(&mut app, DrawingAction::Select(None));
    let template = session(&app).sheet().template.clone();
    let block = soliddraft_drawing::library::template(template.as_str());
    let title = block.title_block.fields.iter().find(|f| f.key == "title").expect("the block has a title field");
    let shown = |app: &App| soliddraft_drawing::display::field_value(&session(app).doc.drawing, 0, title);
    let document_title = session(&app).doc.drawing.meta.title.clone();
    assert_ne!(document_title, "Release");
    assert_eq!(app.get_by_label("Title field").value().as_deref(), Some("Release"));

    app.get_by_label("Title field").focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("Updated".into()));
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(4);
    assert_eq!(session(&app).sheet().format_overrides["title"], "Updated");
    assert_eq!(session(&app).doc.drawing.meta.title, document_title, "the document title stays");
    assert_eq!(shown(&app), "Updated");
    let list = soliddraft_drawing::display::layout_sheet(&session(&app).doc.drawing, 0);
    let drawn =
        list.items.iter().any(|i| matches!(i, soliddraft_drawing::display::Item::Text(t) if t.text == "Updated"));
    assert!(drawn, "the sheet draws the edited title");

    click_label(&mut app, "Title: use document value");
    app.run_steps(2);
    assert_eq!(session(&app).sheet().format_overrides.get("title"), None);
    assert_eq!(shown(&app), document_title);
    assert_eq!(app.get_by_label("Title field").value().as_deref(), Some(document_title.as_str()));
    drawing(&mut app, DrawingAction::Undo);
    assert_eq!(shown(&app), "Updated");

    app.state_mut()
        .state
        .drawing
        .session
        .as_mut()
        .unwrap()
        .doc
        .drawing
        .source_properties
        .insert("Description".into(), "Bracket arm".into());
    drawing(
        &mut app,
        DrawingAction::Command(DrawCommand::BindTitleProperty {
            sheet: 0,
            key: "title".into(),
            property: "Description".into(),
        }),
    );
    assert_eq!(app.get_by_label("Title field").value().as_deref(), Some("Bracket arm"));
    app.get_by_label("Title from part property Description");
    app.get_by_label("Title field").focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("Release copy".into()));
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(4);
    assert_eq!(shown(&app), "Release copy");
    click_label(&mut app, "Title: use part property Description");
    assert_eq!(shown(&app), "Bracket arm");
    drawing(
        &mut app,
        DrawingAction::Command(DrawCommand::AddRevision {
            sheet: 0,
            row: soliddraft_drawing::RevisionRow { revision: "B".into(), ..Default::default() },
        }),
    );
    assert_eq!(app.get_by_label("Revision field").value().as_deref(), Some("B"));
    app.get_by_label("Revision from revision table");
    app.get_by_label("Revision field").focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("C".into()));
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(4);
    let revision = block.title_block.fields.iter().find(|f| f.key == "revision").unwrap();
    let shown = |app: &App| soliddraft_drawing::display::field_value(&session(app).doc.drawing, 0, revision);
    assert_eq!(shown(&app), "C");
    click_label(&mut app, "Revision: use revision table");
    assert_eq!(shown(&app), "B");
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn mixed_sheet_reference_dimensions_use_the_owning_sheets_style() {
    use soliddraft_app::drawing::document::DrawingDoc;
    let mut drawing = soliddraft_drawing::fixture::bracket_a3();
    let mut second = drawing.sheets[0].clone();
    second.template = "ansi-e".into();
    drawing.sheets.push(second);
    let dim = soliddraft_drawing::Dimension::measured(
        soliddraft_drawing::DimensionKind::Linear { orientation: soliddraft_drawing::LinearOrientation::Horizontal },
        120.0,
        0,
    );
    assert_eq!(DrawingDoc::dimension_text(&drawing.sheets[0], &dim), "120");
    assert_eq!(DrawingDoc::dimension_text(&drawing.sheets[1], &dim), "(120)");
}
