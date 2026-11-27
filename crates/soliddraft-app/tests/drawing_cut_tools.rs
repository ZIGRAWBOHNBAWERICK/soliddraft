//! The section tool's offset and aligned cutting lines, and the break and half section
//! tools, driven by clicks on a drawing of the sample bracket in the application: the
//! cut faces have their true areas, the panel buttons flip and remove a half section,
//! a break undoes and redoes, and all of it is there when the drawing reopens.

use egui::{Key, Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::document::{Command, CutParams, Document, ExtrudeParams, PlaneKind, SketchPlane};
use soliddraft_app::drawing::{DrawingAction, DrawingSel, DrawingSession, DrawingTool};
use soliddraft_app::sketch::{EntityKind, P2, Sketch};
use soliddraft_app::state::AppAction;
use soliddraft_drawing::{AlignAxis, View, ViewId, ViewKind};
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

#[test]
fn offset_aligned_half_and_broken_views_from_the_drawing_tools() {
    let root = std::env::temp_dir().join(format!("soliddraft-cut-tools-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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
    let (top, front) = (view_named(&app, "Top view"), view_named(&app, "Front view"));

    // An offset section through two staggered holes: their centres clicked, the last
    // one twice. On y = 23 up to x = 70 the flange (10 x 60) and plate (60 x 10) less the
    // hole (9 x 10); on y = 67 beyond it the plate (50 x 10) less the other hole.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::Section { replace: None }));
    click_label(&mut app, "Offset (stepped)");
    click_model(&mut app, top, Vec3::new(25.0, 23.0, 10.0));
    click_model(&mut app, top, Vec3::new(115.0, 67.0, 10.0));
    click_model(&mut app, top, Vec3::new(115.0, 67.0, 10.0));
    place_right_of(&mut app, top, 160.0);
    let offset = sections(&app).pop().expect("an offset section");
    let ViewKind::Section { line, model_line, .. } = &offset.kind else { unreachable!() };
    assert_eq!((line.len(), model_line.len()), (4, 4), "two legs and a jog: {line:?}");
    near(cut_area(&app, &offset), 1110.0 + 410.0, 1e-3, "offset cut area");

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

    // A half section of the front view about the bore's axis; the panel flips it,
    // then removes it.
    drawing(&mut app, DrawingAction::SetTool(DrawingTool::HalfSection));
    click_model(&mut app, front, Vec3::new(70.0, 0.0, 0.0));
    click_model(&mut app, front, Vec3::new(70.0, 0.0, 30.0));
    let half = view(&app, front);
    let made = half.half.clone().expect("the front view is a half section");
    assert!((made.axis[0].x - 70.0).abs() < 1e-6 && (made.axis[1].x - 70.0).abs() < 1e-6, "{made:?}");
    assert!(half.geometry.symmetry.is_some() && !half.geometry.hatch.is_empty(), "the cut half is hatched");
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

    // Saved and reopened as it is.
    let path = root.join("bracket.dwdrw");
    drawing(&mut app, DrawingAction::Save(Some(path.clone())));
    drawing(&mut app, DrawingAction::CloseDrawing);
    drawing(&mut app, DrawingAction::OpenFile(path));
    settle(&mut app);
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

/// The sheet format panel: the paper picked from its list, a title block field's cell
/// moved by typing its x, then undo, redo and reopen.
#[test]
fn paper_and_title_cell_from_the_sheet_format_panel() {
    let root = std::env::temp_dir().join(format!("soliddraft-format-panel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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

    // The legal owner's cell moves to x 120 of the block.
    app.get_by_label("Legal owner x").click();
    app.run_steps(3);
    // The click opens the value for typing; the keys go to that editor.
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("120".into()));
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(4);
    let moved = session(&app).sheet().title_layout.get("owner").copied().expect("the owner field moved");
    assert_eq!(moved.cell, [120.0, 9.0, 60.0, 9.0]);
    drawing(&mut app, DrawingAction::Undo);
    assert!(session(&app).sheet().title_layout.is_empty(), "undo puts the field back");
    assert_eq!(session(&app).sheet().template.as_str(), "iso-a0", "and keeps the paper");
    drawing(&mut app, DrawingAction::Undo);
    assert_eq!(session(&app).sheet().template.as_str(), "iso-a3", "a second undo the paper");
    drawing(&mut app, DrawingAction::Redo);
    drawing(&mut app, DrawingAction::Redo);
    assert_eq!(session(&app).sheet().title_layout.get("owner"), Some(&moved));

    let path = root.join("bracket.dwdrw");
    drawing(&mut app, DrawingAction::Save(Some(path.clone())));
    drawing(&mut app, DrawingAction::CloseDrawing);
    drawing(&mut app, DrawingAction::OpenFile(path));
    settle(&mut app);
    let sheet = session(&app).sheet();
    assert_eq!(sheet.template.as_str(), "iso-a0");
    assert_eq!(sheet.title_layout.get("owner"), Some(&moved));
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}
