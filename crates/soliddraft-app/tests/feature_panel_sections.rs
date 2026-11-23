//! The feature property panel as a user drives it: sections fold and unfold, a ticked
//! section stays ticked when the feature is edited again and after the part is saved
//! and reopened, Enter applies and Escape cancels. Each extrude option is set in the
//! panel, applied, saved, reopened and built on with a later feature.

use egui::Key;
use egui_kittest::kittest::{NodeT, Queryable};
use soliddraft_app::document::{
    Command, CutParams, Document, FeatureKind, FeatureStatus, PlaneKind, Selection, SketchPlane,
};
use soliddraft_app::state::AppAction;
use soliddraft_kernel_api::Vec3;

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{Case, STANDARD, launch, pointer_to_chrome, settle};

const PI: f64 = std::f64::consts::PI;

/// The plate (60 by 40 by 20 on Top) with a 10 mm circle sketched on `plane`.
fn plate_with_circle(plane: impl FnOnce(&Document) -> SketchPlane) -> (Document, u64) {
    let mut doc = ribbon::plate();
    let plane = plane(&doc);
    let sketch = doc.apply(Command::AddSketch { plane }).unwrap().unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.circle_at(soliddraft_sketch::P2::new(0.0, 0.0), 10.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: drawing }).unwrap();
    doc.selection = Some(Selection::Feature(sketch));
    (doc, sketch)
}

fn on_top(offset: f64) -> impl FnOnce(&Document) -> SketchPlane {
    move |_| SketchPlane::Reference { plane: PlaneKind::Top, offset }
}

/// The plate's top face, by its persistent name.
fn top_face(doc: &Document) -> SketchPlane {
    let model = doc.model();
    let body = model.body().unwrap();
    let topology = model.kernel().topology(body).unwrap();
    let face = topology
        .faces
        .iter()
        .find(|f| f.normal.is_some_and(|n| n.dot(Vec3::Z) > 1.0 - 1e-9) && (f.centroid.z - 20.0).abs() < 1e-6)
        .unwrap();
    SketchPlane::Face { face: model.face_ref(face.id).unwrap() }
}

fn open(app: &mut App, action: AppAction) {
    app.state_mut().state.push(action);
    settle(app);
    pointer_to_chrome(app);
}

fn shows(app: &App, label: &str) -> bool {
    app.query_by_label(label).is_some()
}

fn click(app: &mut App, label: &str) {
    let node = app.get_by_label(label);
    node.scroll_to_me();
    app.run_steps(24);
    app.get_by_label(label).click();
    app.run_steps(8);
}

fn ticked(app: &App, label: &str) -> bool {
    app.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// Opens combo box `combo` and picks `entry` from its list.
fn choose(app: &mut App, combo: &str, entry: &str) {
    let before: Vec<egui::Rect> = app.query_all_by_label(entry).map(|n| n.rect()).collect();
    click(app, combo);
    let rect = app
        .query_all_by_label(entry)
        .map(|n| n.rect())
        .find(|r| !before.contains(r))
        .unwrap_or_else(|| panic!("{combo} lists {entry}"));
    ribbon::click_at(app, rect.center());
    settle(app);
}

fn volumes(doc: &Document) -> Vec<f64> {
    let model = doc.model();
    let mut v: Vec<f64> =
        model.live_bodies().into_iter().map(|(_, body)| model.kernel().mass_properties(body).unwrap().volume).collect();
    v.sort_by(f64::total_cmp);
    v
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-6 * b.abs().max(1.0), "{what}: {a} is not {b}");
}

fn all_ok(doc: &Document) {
    for f in &doc.features {
        assert_eq!(f.status, FeatureStatus::Ok, "{} builds", f.name);
    }
}

/// The part saved to bytes and opened in a fresh app, as a reopened file is.
fn reopen(case: &mut Case, app: &App) -> App {
    let bytes = app.state().state.doc.model().to_bytes().unwrap();
    std::fs::write(case.dir.join("part.dwprt"), &bytes).unwrap();
    let model = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
    launch(Document::from_model(model), STANDARD)
}

/// A later feature: a 2 mm hole cut through all, both ways, from the Top plane.
fn later_cut(app: &mut App) {
    let doc = &mut app.state_mut().state.doc;
    let sketch = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.circle_at(soliddraft_sketch::P2::new(20.0, 10.0), 2.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: drawing }).unwrap();
    let mut cut = CutParams::new(sketch);
    cut.choices.direction2 = Some(Default::default());
    cut.choices.direction2.as_mut().unwrap().end = soliddraft_app::document::EndKind::ThroughAll;
    doc.apply(Command::AddCut { params: cut }).unwrap();
    settle(app);
}

fn extrusion(app: &App) -> soliddraft_app::document::ExtrudeParams {
    app.state()
        .state
        .doc
        .features
        .iter()
        .rev()
        .find_map(|f| match &f.kind {
            FeatureKind::Extrude(p) if p.sketch.is_some() => Some(p.clone()),
            _ => None,
        })
        .expect("an extrusion of the sketch")
}

#[test]
fn sections_fold_and_ticked_sections_persist() {
    let mut case = Case::new("feature-panel/sections");
    let (doc, _) = plate_with_circle(on_top(20.0));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        assert!(shows(app, "Feature panel: Boss-Extrude"), "the side area is the extrusion's panel");
        assert!(shows(app, "OK") && shows(app, "Cancel") && shows(app, "Preview"), "OK, Cancel and Preview on top");
        // The common choices are open, the advanced ones folded.
        assert!(shows(app, "Fold Direction 1"));
        for folded in ["Unfold From", "Unfold Direction 2", "Unfold Thin feature", "Unfold Selected contours"] {
            assert!(shows(app, folded), "{folded} starts folded");
        }
        assert!(!ticked(app, "Direction 2") && !ticked(app, "Thin feature"));
        case.frame(app, "opened");
        // Unfolding shows the section's fields; folding hides them.
        click(app, "Unfold From");
        assert!(shows(app, "Extrude from"), "From's choice shows");
        click(app, "Fold From");
        assert!(!shows(app, "Extrude from"), "From folds again");
        // Ticking Direction 2 opens it with its depth.
        click(app, "Direction 2");
        assert!(ticked(app, "Direction 2") && shows(app, "Fold Direction 2"), "ticking opens the section");
        ribbon::type_field(app, "Depth 2 value", "4");
        // The Preview switch hides and shows the preview.
        click(app, "Preview");
        assert!(!app.state().state.editor.as_ref().unwrap().show_preview);
        click(app, "Preview");
        assert!(app.state().state.editor.as_ref().unwrap().show_preview);
        case.frame(app, "direction-2");
        // Enter applies.
        app.key_press(Key::Enter);
        settle(app);
        assert!(app.state().state.editor.is_none(), "Enter applies the feature");
        let id = app.state().state.doc.features.last().unwrap().id;
        let p = extrusion(app);
        assert_eq!(p.choices.direction2.as_ref().map(|d| d.depth), Some(4.0));
        // Editing it again: Direction 2 is still ticked and open, with its depth.
        open(app, AppAction::EditFeature(id));
        assert!(ticked(app, "Direction 2") && shows(app, "Fold Direction 2"), "a ticked section stays ticked");
        assert!(shows(app, "Depth 2 value"));
        // A section the user opened stays open for this kind of feature.
        click(app, "Unfold Selected contours");
        // Escape cancels without a change.
        let before = app.state().state.doc.model().part_json();
        app.key_press(Key::Escape);
        settle(app);
        assert!(app.state().state.editor.is_none(), "Escape cancels");
        assert_eq!(app.state().state.doc.model().part_json(), before, "a cancelled edit changes nothing");
        open(app, AppAction::EditFeature(id));
        assert!(shows(app, "Fold Selected contours"), "the opened section is still open");
        app.key_press(Key::Escape);
        settle(app);
        // Saved and reopened, the section is still ticked.
        let mut again = reopen(case, app);
        open(&mut again, AppAction::EditFeature(id));
        assert!(ticked(&again, "Direction 2"), "a ticked section survives the file");
        case.frame(&mut again, "reopened");
    });
}

/// Applies the open extrusion with OK and checks the part.
fn apply(app: &mut App) {
    click(app, "OK");
    settle(app);
    assert!(
        app.state().state.editor.is_none(),
        "OK applies: {:?}",
        app.state().state.editor.as_ref().map(|e| e.refusal())
    );
    all_ok(&app.state().state.doc);
}

/// Saves, reopens, adds the later cut and checks every feature builds.
fn later(case: &mut Case, app: &App, expect: usize) {
    let mut back = reopen(case, app);
    assert_eq!(volumes(&back.state().state.doc).len(), expect, "the bodies survive the file");
    later_cut(&mut back);
    all_ok(&back.state().state.doc);
    case.frame(&mut back, "later");
}

#[test]
fn from_an_offset_face_starts_inside_the_plate() {
    let mut case = Case::new("feature-panel/from-offset-face");
    let (doc, _) = plate_with_circle(top_face);
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        click(app, "Unfold From");
        choose(app, "Extrude from", "Offset");
        ribbon::type_field(app, "Start offset value", "4");
        click(app, "Reverse offset");
        case.frame(app, "set");
        apply(app);
        let v = volumes(&app.state().state.doc);
        close(v[0], 48000.0 + PI * 25.0 * 6.0, "4 mm in, 10 mm out: 6 mm above the plate");
        later(case, app, 1);
    });
}

#[test]
fn flip_turns_the_extrusion_down_through_the_plate() {
    let mut case = Case::new("feature-panel/flip");
    let (doc, _) = plate_with_circle(on_top(20.0));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        ribbon::type_field(app, "Depth value", "30");
        click(app, "Flip direction");
        apply(app);
        let v = volumes(&app.state().state.doc);
        close(v[0], 48000.0 + PI * 25.0 * 10.0, "20 mm in the plate and 10 mm below it");
        later(case, app, 1);
    });
}

#[test]
fn merge_off_makes_a_second_body() {
    let mut case = Case::new("feature-panel/merge-off");
    let (doc, _) = plate_with_circle(on_top(20.0));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        assert!(ticked(app, "Merge result"), "an extrusion on a part merges by default");
        click(app, "Merge result");
        apply(app);
        let v = volumes(&app.state().state.doc);
        assert_eq!(v.len(), 2, "the extrusion is a second body");
        close(v[0], PI * 25.0 * 10.0, "the new body");
        close(v[1], 48000.0, "the plate alone");
        // Edited back on, it joins the plate; off again, it is separate again.
        let id = app.state().state.doc.features.last().unwrap().id;
        open(app, AppAction::EditFeature(id));
        assert!(!ticked(app, "Merge result"), "the panel reads Merge result back");
        click(app, "Merge result");
        apply(app);
        assert_eq!(volumes(&app.state().state.doc).len(), 1, "merged into the plate");
        open(app, AppAction::EditFeature(id));
        click(app, "Merge result");
        apply(app);
        assert_eq!(volumes(&app.state().state.doc).len(), 2);
        later(case, app, 2);
    });
}

#[test]
fn direction_2_goes_both_ways() {
    let mut case = Case::new("feature-panel/direction-2");
    let (doc, _) = plate_with_circle(on_top(30.0));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        click(app, "Direction 2");
        ribbon::type_field(app, "Depth 2 value", "10");
        apply(app);
        // 10 mm up from 30 and 10 mm down to the plate's top at 20.
        let v = volumes(&app.state().state.doc);
        close(v[0], 48000.0 + PI * 25.0 * 20.0, "both directions, joined to the plate");
        later(case, app, 1);
    });
}

#[test]
fn a_thin_feature_makes_a_tube() {
    let mut case = Case::new("feature-panel/thin");
    let (doc, _) = plate_with_circle(on_top(20.0));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        click(app, "Thin feature");
        assert!(shows(app, "Fold Thin feature"), "ticking opens the section");
        ribbon::type_field(app, "Wall thickness value", "1");
        case.frame(app, "set");
        apply(app);
        let v = volumes(&app.state().state.doc);
        close(v[0], 48000.0 + PI * (36.0 - 25.0) * 10.0, "a 1 mm tube 10 mm tall on the plate");
        let p = extrusion(app);
        assert!(p.choices.thin.is_some());
        later(case, app, 1);
    });
}
