//! Application-level Hole Wizard acceptance tests:
//! - Self-contained Hole Wizard panel (`Type` and `Positions` tabs, collapsible sections)
//! - ISO M6 counterbore 4-hole pattern via inline position sketch -> edit positions later ->
//!   switch standard to ANSI Inch -> save -> reopen -> rebuild -> later feature on top
//! - Favorites: save, apply, and delete hole presets
//! - Custom sizing override & Restore Default Values
//! - Through-all tapped hole with cosmetic thread (viewport thread marks + PMI + drawing callout)
//! - Non-planar face refusal by name

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::document::{Document, FeatureKind, HoleParams, HoleWizardTab};
use soliddraft_app::settings::{self, Store, store::Memory};
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_kernel_api::{HoleHead, SurfaceKind, Vec3};
use soliddraft_model::file::HOLE_WIZARD_FORMAT;
use soliddraft_model::hole_wizard::{HoleEndCondition, HoleFit, HoleStandard, HoleType};
use ux_tools::{STANDARD, launch, settle};

fn click(app: &mut App, label: &str) {
    let rect = app.get_by_label(label).rect();
    ribbon::click_at(app, rect.center());
    settle(app);
}

fn hole_params(app: &App) -> HoleParams {
    match &app.state().state.editor.as_ref().expect("hole editor open").params {
        EditorParams::Hole(h) => h.clone(),
        other => panic!("{other:?}"),
    }
}

fn volume(doc: &Document) -> f64 {
    let model = doc.model();
    model.kernel().mass_properties(model.body().unwrap()).unwrap().volume
}

#[test]
fn hole_wizard_iso_m6_four_holes_edit_positions_switch_ansi_inch_favorites_save_reopen() {
    // Isolate settings storage in memory so favorites tests never touch host files.
    let shelf = Memory::new();
    settings::store::use_store(Some(Store::Shelf(shelf.clone())));

    let mut app = launch(ribbon::plate(), STANDARD);
    settle(&mut app);
    let plate_vol = volume(&app.state().state.doc);

    // 1. Open Hole Wizard and pick the plate's top face.
    ribbon::run_command(&mut app, "hole");
    settle(&mut app);
    ribbon::click_world(&mut app, glam::Vec3::new(-20.0, -10.0, 20.0));
    settle(&mut app);
    assert!(hole_params(&app).face.is_some());

    // 2. Choose Counterbore in the self-contained Hole Wizard panel (ISO M6 SHCS default).
    click(&mut app, "Hole type: Counterbore");
    let p = hole_params(&app);
    assert_eq!(p.wizard.as_ref().unwrap().standard, HoleStandard::Iso);
    assert_eq!(p.wizard.as_ref().unwrap().hole_type, HoleType::Counterbore);
    assert_eq!(p.wizard.as_ref().unwrap().size, "M6");
    assert_eq!(p.diameter, 6.6);
    assert_eq!(p.head, Some(HoleHead::Counterbore { diameter: 11.0, depth: 6.4 }));

    // 3. Custom sizing override + Restore Default Values.
    {
        let EditorParams::Hole(h) = &mut app.state_mut().state.editor.as_mut().unwrap().params else { panic!() };
        h.show_custom_sizing = true;
        h.diameter = 7.25;
        h.ensure_wizard().custom_sizing = true;
    }
    settle(&mut app);
    assert_eq!(hole_params(&app).diameter, 7.25);
    click(&mut app, "Restore Default Values");
    assert_eq!(hole_params(&app).diameter, 6.6);
    assert!(!hole_params(&app).wizard.unwrap().custom_sizing);

    // 4. Save a Favorite preset, mutate settings, apply it back, and test delete.
    {
        let EditorParams::Hole(h) = &mut app.state_mut().state.editor.as_mut().unwrap().params else { panic!() };
        h.favorite_name = "M6 SHCS Plate".into();
    }
    settle(&mut app);
    click(&mut app, "Save favorite");
    assert_eq!(settings::hole_favorites().len(), 1);
    assert_eq!(settings::hole_favorites()[0].name, "M6 SHCS Plate");

    // Mutate to DIN M8 Close fit, then apply favorite back.
    {
        let EditorParams::Hole(h) = &mut app.state_mut().state.editor.as_mut().unwrap().params else { panic!() };
        h.ensure_wizard().set_standard(HoleStandard::Din);
        h.ensure_wizard().size = "M8".into();
        h.ensure_wizard().fit = HoleFit::Close;
        h.apply_wizard_defaults();
        let fav = settings::hole_favorites()[0].clone();
        h.apply_favorite(&fav);
    }
    settle(&mut app);
    assert_eq!(hole_params(&app).wizard.as_ref().unwrap().standard, HoleStandard::Iso);
    assert_eq!(hole_params(&app).diameter, 6.6);

    // Delete favorite.
    click(&mut app, "Delete favorite");
    assert!(settings::hole_favorites().is_empty());

    // 5. Switch to Positions tab and build a 2x2 (4-hole) linear point pattern inside the position sketch.
    click(&mut app, "Hole Wizard Positions tab");
    assert_eq!(hole_params(&app).wizard_tab, HoleWizardTab::Positions);
    {
        let EditorParams::Hole(h) = &mut app.state_mut().state.editor.as_mut().unwrap().params else { panic!() };
        h.set_position_point(0, soliddraft_sketch::P2::new(-20.0, -10.0));
        h.lin_pat.count_x = 2;
        h.lin_pat.spacing_x = 40.0;
        h.lin_pat.count_y = 2;
        h.lin_pat.spacing_y = 20.0;
        h.apply_linear_point_pattern().unwrap();
    }
    settle(&mut app);
    assert_eq!(hole_params(&app).points.len(), 4);

    // Toggle inline full sketch mode on and back off while staying inside Hole Wizard.
    click(&mut app, "Toggle inline position sketch");
    assert!(app.state().state.in_sketch());
    assert!(app.state().state.editor.is_some());
    click(&mut app, "Toggle inline position sketch");
    assert!(!app.state().state.in_sketch());
    assert!(app.state().state.editor.is_some());

    // 6. Commit Hole Wizard.
    click(&mut app, "Apply Hole Wizard");
    ux_tools::rebuilt(&mut app);
    assert!(app.state().state.editor.is_none());

    let doc = &app.state().state.doc;
    let hole_feat = doc.features.iter().find(|f| matches!(f.kind, FeatureKind::Hole(_))).expect("hole feature");
    let hole_id = hole_feat.id;
    let sketch_id = hole_feat.kind.sketch().expect("absorbed position sketch under hole");
    let vol_iso_4 = volume(doc);
    assert!(vol_iso_4 < plate_vol);

    // 7. Edit positions later by editing the absorbed position sketch (add a 5th point at origin).
    {
        let mut sk = app.state().state.doc.feature(sketch_id).unwrap().sketch().unwrap().sketch.clone();
        sk.add_entity(soliddraft_sketch::EntityKind::Point { p: soliddraft_sketch::P2::new(0.0, 0.0) });
        soliddraft_sketch::solve::solve(&mut sk, None, true);
        app.state_mut().state.push(AppAction::Command(soliddraft_app::document::Command::EditSketch {
            id: sketch_id,
            sketch: sk,
        }));
    }
    ux_tools::rebuilt(&mut app);
    let vol_iso_5 = volume(&app.state().state.doc);
    assert!(vol_iso_5 < vol_iso_4, "editing position sketch adds 5th hole and reduces volume");

    // 8. Re-open Hole Wizard feature editor and switch to ANSI Inch 1/4" SHCS Counterbore.
    app.state_mut().state.push(AppAction::EditFeature(hole_id));
    settle(&mut app);
    {
        let EditorParams::Hole(h) = &mut app.state_mut().state.editor.as_mut().unwrap().params else { panic!() };
        h.ensure_wizard().set_standard(HoleStandard::AnsiInch);
        h.apply_wizard_defaults();
    }
    settle(&mut app);
    assert_eq!(hole_params(&app).wizard.as_ref().unwrap().standard, HoleStandard::AnsiInch);
    assert_eq!(hole_params(&app).wizard.as_ref().unwrap().size, "1/4");
    click(&mut app, "Apply Hole Wizard");
    ux_tools::rebuilt(&mut app);
    let vol_inch_5 = volume(&app.state().state.doc);
    assert!((vol_inch_5 - vol_iso_5).abs() > 1.0, "switching to ANSI Inch 1/4 changes geometry");

    // 9. Save (.dwprt format v4) and reopen in a fresh kernel.
    let model = app.state().state.doc.model();
    assert_eq!(model.truth()["format"], HOLE_WIZARD_FORMAT);
    let bytes = model.to_bytes().unwrap();
    let reopened = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
    let reopened_doc = Document::from_model(reopened);
    assert!((volume(&reopened_doc) - vol_inch_5).abs() < 1e-4);

    settings::store::use_store(None);
}

#[test]
fn hole_wizard_through_all_tapped_hole_with_cosmetic_thread_and_non_planar_refusal() {
    let mut app = launch(ribbon::plate(), STANDARD);
    settle(&mut app);

    ribbon::run_command(&mut app, "hole");
    settle(&mut app);
    ribbon::click_world(&mut app, glam::Vec3::new(0.0, 0.0, 20.0));
    settle(&mut app);

    // Choose Straight Tap (ISO M6 coarse, Through All, cosmetic thread on).
    click(&mut app, "Hole type: Straight Tap");
    {
        let p = hole_params(&app);
        assert_eq!(p.diameter, 5.0);
        assert_eq!(p.wizard.as_ref().unwrap().end_condition, HoleEndCondition::ThroughAll);
        assert_eq!(p.thread.as_ref().map(|t| t.designation()).as_deref(), Some("M6x1-6H"));
    }
    click(&mut app, "Apply Hole Wizard");
    ux_tools::rebuilt(&mut app);

    let doc = &app.state().state.doc;
    let walls = doc.model().threaded_walls();
    assert_eq!(walls.len(), 1);
    assert!((walls[0].radius - 2.5).abs() < 1e-6);
    assert!(soliddraft_app::viewport::threads::marks(doc).is_some());
    let pmi: Vec<_> = doc.model().pmi_rows().into_iter().filter(|r| r.feature.is_some()).collect();
    assert_eq!(pmi.len(), 1);
    assert_eq!(pmi[0].callout(), "M6x1-6H THRU");

    // Now test refusing a non-planar face (the cylindrical wall of the hole we just drilled!).
    let cyl_face = doc
        .faces
        .iter()
        .find(|f| f.surface == SurfaceKind::Cylinder)
        .expect("cylindrical hole wall")
        .id;
    ribbon::run_command(&mut app, "hole");
    settle(&mut app);
    app.state_mut().state.push(AppAction::Select(Some(soliddraft_app::document::Selection::Face(cyl_face))));
    settle(&mut app);
    let err = app.state().state.editor.as_ref().unwrap().error.as_deref().unwrap_or("");
    assert!(err.contains("planar"), "expected planar refusal on cylindrical face, got: {err}");
}
