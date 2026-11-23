//! A tapped hole made as a user makes one: the Hole form on a plate's top face, Thread
//! set to Tapped, a size, a fine pitch, a blind depth and a thread depth, then OK. The
//! viewport draws the thread on the wall. The hole is edited to another size, a later cut
//! mills the plate's top, and the part is saved and reopened. The thread holds through
//! all of it, in the feature, in the part's manufacturing information and on screen.
//!
//! Frames go to `target/ux-tools/hole-thread/`.

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use egui::Rect;
use egui_kittest::kittest::Queryable;
use ribbon::App;
use serde_json::json;
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, FeatureKind};
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::HoleDepth;
use soliddraft_model::thread::HoleThread;
use ux_tools::{Case, STANDARD, launch, settle};

/// The feature property panel, where the form's OK is.
fn bar(app: &App) -> Rect {
    ribbon::feature_panel(app)
}

fn click_bar(app: &mut App, label: &str) {
    let rect = ribbon::control(app, label, bar(app)).rect;
    ribbon::click_at(app, rect.center());
    settle(app);
}

/// Unfolds the feature panel's folded sections.
fn more(app: &mut App) {
    ribbon::open_options(app);
    settle(app);
}

fn click(app: &mut App, label: &str) {
    ribbon::open_form_sheet_for(app, label);
    let rect = app.get_by_label(label).rect();
    ribbon::click_at(app, rect.center());
    settle(app);
}

/// Opens the form's choice that shows `shown`, on the bar or in its sheet, and picks
/// `entry` in its list.
fn choose(app: &mut App, shown: &str, entry: &str) {
    if !app.query_all(egui_kittest::kittest::by()).any(|node| node.value().as_deref() == Some(shown)) {
        more(app);
    }
    let rect = app
        .query_all(egui_kittest::kittest::by())
        .filter(|node| node.value().as_deref() == Some(shown))
        .map(|node| node.rect())
        .next()
        .unwrap_or_else(|| panic!("a choice showing {shown}"));
    ribbon::click_at(app, rect.center());
    settle(app);
    click(app, entry);
}

/// Opens a combo box of the form and picks the entry `entry`, scrolled into its list.
fn pick(app: &mut App, combo: &str, entry: &str) {
    app.get_by_label(combo).scroll_to_me();
    app.run_steps(24);
    click(app, combo);
    app.get_by_label(entry).scroll_to_me();
    app.run_steps(24);
    click(app, entry);
}

fn editing(app: &App) -> soliddraft_app::document::HoleParams {
    match &app.state().state.editor.as_ref().expect("the hole form is open").params {
        EditorParams::Hole(h) => (**h).clone(),
        other => panic!("{other:?}"),
    }
}

fn hole(doc: &Document) -> (u64, soliddraft_model::HoleFeature) {
    doc.features
        .iter()
        .find_map(|f| match &f.kind {
            FeatureKind::Hole(h) => Some((f.id, h.clone())),
            _ => None,
        })
        .expect("a hole")
}

/// The callouts of the items the part's threaded holes state.
fn thread_callouts(doc: &Document) -> Vec<String> {
    doc.model().pmi_rows().iter().filter(|row| row.feature.is_some()).map(|row| row.callout()).collect()
}

/// Pixels of the viewport in the thread marks' blue (scene.wgsl), give or take shading.
fn thread_blue(app: &App, image: &image::RgbaImage) -> usize {
    let rect = ux_tools::viewport(app);
    let ppp = app.ctx.pixels_per_point();
    let (x0, y0) = ((rect.left() * ppp) as u32, (rect.top() * ppp) as u32);
    let (x1, y1) = ((rect.right() * ppp) as u32, (rect.bottom() * ppp) as u32);
    let mut count = 0;
    for y in y0..y1.min(image.height()) {
        for x in x0..x1.min(image.width()) {
            let [r, g, b, _] = image.get_pixel(x, y).0;
            let d = (i32::from(r) - 46).abs() + (i32::from(g) - 102).abs() + (i32::from(b) - 184).abs();
            if d < 45 {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn a_tapped_hole_is_made_in_the_form_edited_milled_saved_and_reopened() {
    let mut case = Case::new("hole-thread");
    let mut app = launch(ribbon::plate(), STANDARD);
    settle(&mut app);
    case.run(&mut app, |case, app| {
        ux_tools::pointer_to_chrome(app);
        let before = { let image = case.frame(app, "plate"); thread_blue(app, &image) };

        // Hole on the plate's top face, at the face's origin.
        ribbon::run_command(app, "hole");
        settle(app);
        ribbon::click_world(app, glam::Vec3::new(10.0, 5.0, 20.0));
        settle(app);
        assert!(editing(app).face.is_some(), "the top face is the hole's face");
        assert_eq!(
            editing(app).points,
            [soliddraft_app::sketch::P2::new(10.0, 5.0)],
            "the first hole is where the face was clicked"
        );

        // Tapped: the coarse thread whose tap drill is nearest the drawn Ø6.6 is M8.
        choose(app, "Plain", "Tapped");
        let tapped = editing(app);
        assert_eq!(tapped.thread.as_ref().map(HoleThread::designation).as_deref(), Some("M8x1.25-6H"));
        assert_eq!(tapped.diameter, 6.8, "the drill is M8's tap drill");
        // A fine pitch drills Ø7.
        pick(app, "Thread pitch", "1 mm fine");
        assert_eq!(editing(app).thread.as_ref().map(HoleThread::designation).as_deref(), Some("M8x1-6H"));
        assert_eq!(editing(app).diameter, 7.0);
        // 12 mm deep, threaded for 9.
        more(app);
        choose(app, "Through", "Blind");
        // Blind brings its depth in a section of its own, folded like the rest.
        more(app);
        ribbon::type_field(app, "Depth value", "12");
        click(app, "Full depth");
        ribbon::type_field(app, "Thread depth value", "9");
        let set = editing(app);
        assert_eq!(set.depth, HoleDepth::Blind { depth: 12.0 });
        assert_eq!(set.thread.as_ref().and_then(|t| t.length), Some(9.0));
        case.frame(app, "form");
        click_bar(app, "OK");
        ux_tools::rebuilt(app);
        assert!(app.state().state.editor.is_none());

        let doc = &app.state().state.doc;
        let (id, made) = hole(doc);
        assert_eq!(made.diameter, 7.0);
        assert_eq!(made.thread.as_ref().map(HoleThread::designation).as_deref(), Some("M8x1-6H"));
        assert!(doc.feature(id).is_some_and(|f| f.summary() == "1 × M8x1-6H"));
        assert_eq!(thread_callouts(doc), ["M8x1-6H DEEP 9"]);
        let walls = doc.model().threaded_walls();
        assert_eq!(walls.len(), 1);
        assert!((walls[0].end() - 9.0).abs() < 1e-6 && (walls[0].radius - 3.5).abs() < 1e-6, "{walls:?}");
        ux_tools::pointer_to_chrome(app);
        let tapped_blue = { let image = case.frame(app, "tapped"); thread_blue(app, &image) };
        case.note(format!("thread-blue pixels: plate {before}, tapped {tapped_blue}"));
        assert!(tapped_blue > before + 50, "the viewport draws the thread: {before} -> {tapped_blue}");

        // Edit: M10 coarse, and the drill follows.
        app.state_mut().state.push(AppAction::EditFeature(id));
        settle(app);
        assert_eq!(editing(app).thread.as_ref().map(HoleThread::designation).as_deref(), Some("M8x1-6H"));
        pick(app, "Thread size", "M10");
        assert_eq!(editing(app).thread.as_ref().map(HoleThread::designation).as_deref(), Some("M10x1.5-6H"));
        assert_eq!(editing(app).diameter, 8.5);
        click_bar(app, "OK");
        ux_tools::rebuilt(app);
        assert_eq!(thread_callouts(&app.state().state.doc), ["M10x1.5-6H DEEP 9"]);

        // A later cut mills 4 mm off the top: the thread stays on what is left of the wall,
        // and the callout measures from the new top.
        let state = &mut app.state_mut().state;
        for (tool, arguments) in [
            (
                "sketch",
                json!({"plane": "top", "offset": 16, "shapes": [{"shape": "rectangle", "center": [0, 0, 16], "width": 80, "height": 60}]}),
            ),
            ("cut", json!({"sketch": "Sketch1", "depth": 10, "through_all": false, "flip": true})),
        ] {
            let out = call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "hole-thread".into() });
            assert!(!out.is_error, "{tool}: {}", out.content);
        }
        ux_tools::rebuilt(app);
        let doc = &app.state().state.doc;
        assert!(doc.model().statuses().iter().all(|(_, s)| s.is_ok()), "{:?}", doc.model().statuses());
        let walls = doc.model().threaded_walls();
        assert_eq!(walls.len(), 1);
        assert!((walls[0].start() - 4.0).abs() < 1e-6 && (walls[0].end() - 9.0).abs() < 1e-6, "{walls:?}");
        assert_eq!(thread_callouts(doc), ["M10x1.5-6H DEEP 5"]);
        ux_tools::pointer_to_chrome(app);
        assert!({ let image = case.frame(app, "milled"); thread_blue(app, &image) } > before + 50);

        // Saved and reopened, the thread is the same.
        let bytes = app.state().state.doc.model().to_bytes().unwrap();
        let reopened = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
        let reopened = Document::from_model(reopened);
        assert_eq!(hole(&reopened).1, hole(&app.state().state.doc).1);
        assert_eq!(thread_callouts(&reopened), ["M10x1.5-6H DEEP 5"]);
        // The same threads on the same walls; the kernel numbers the reopened faces afresh.
        let threads = |doc: &Document| -> Vec<_> {
            doc.model()
                .threaded_walls()
                .into_iter()
                .map(|w| (w.feature, w.placement, w.thread, [w.entry.x, w.entry.y, w.entry.z, w.top], w.spans, w.through))
                .collect()
        };
        assert_eq!(threads(&reopened), threads(&app.state().state.doc));
        assert_eq!(threads(&reopened).len(), 1);
        let mut again = launch(reopened, STANDARD);
        ux_tools::rebuilt(&mut again);
        ux_tools::pointer_to_chrome(&mut again);
        assert!({ let image = case.frame(&mut again, "reopened"); thread_blue(&again, &image) } > before + 50);
    });
}

/// A point of the face to the tenth of a millimetre, as Hole places its first hole: the
/// plate's top face lies on the world's x and y.
fn tenth(at: glam::Vec3) -> soliddraft_app::sketch::P2 {
    let tenth = |v: f32| (f64::from(v) * 10.0).round() / 10.0 + 0.0;
    soliddraft_app::sketch::P2::new(tenth(at.x), tenth(at.y))
}

#[test]
fn the_first_hole_goes_where_the_click_that_picked_the_face_ended() {
    let mut app = launch(ribbon::plate(), STANDARD);
    settle(&mut app);
    ribbon::run_command(&mut app, "hole");
    settle(&mut app);
    let a = {
        let state = &app.state().state;
        state.viewport.camera.project(glam::Vec3::new(10.0, 5.0, 20.0), state.viewport.rect).unwrap()
    };
    let b = a + egui::vec2(4.0, 3.0);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    // The button goes down at A, and in the next frame the pointer moves a few points on to
    // B and the button comes up there: one click on the face, which ends at B.
    let (at_a, at_b) = ribbon::render_steps::rendered(&mut app, |app| {
        ribbon::render_steps::await_pick(app, a);
        app.step();
        let at_a = app.state().state.viewport.hover_point.expect("the pointer is on the face at A");
        app.event(button(a, true));
        app.step();
        // Both in one frame: the harness gives each queued event a frame of its own.
        app.input_mut().events.extend([egui::Event::PointerMoved(b), button(b, false)]);
        app.step();
        (at_a, app.state().state.viewport.hover_point.expect("the pointer is on the face at B"))
    });
    app.run_steps(3);
    assert_ne!(tenth(at_a), tenth(at_b), "A and B are different places on the face");
    let editing = editing(&app);
    assert!(editing.face.is_some(), "the click picked the top face");
    assert_eq!(editing.points, [tenth(at_b)], "the hole is where the click ended, not where it was a frame before");
}

#[test]
fn scripted_lines_tap_a_hole_and_the_drawing_calls_it_out() {
    let kernel = std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    let mut state = soliddraft_app::state::AppState::new(Document::new("Tapped plate", kernel));
    let mut call = |tool: &str, arguments: serde_json::Value| -> Result<serde_json::Value, String> {
        let out = call_checked(
            &mut AppTools { state: &mut state },
            tool,
            arguments,
            &Caller::Mcp { client: "hole-thread".into() },
        );
        if out.is_error { Err(out.content) } else { Ok(serde_json::from_str(&out.content).unwrap_or_default()) }
    };
    call(
        "sketch",
        json!({"plane": "top", "shapes": [{"shape": "rectangle", "center": [30, 20, 0], "width": 60, "height": 40}]}),
    )
    .unwrap();
    call("extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false})).unwrap();
    let face = "Boss-Extrude1 \u{b7} End cap";
    // A clearance-sized drill does not take an M6 thread, and says why.
    let refused = call("hole", json!({"face": face, "points": [[15, 20, 10]], "diameter": 6.6, "thread": {"size": 6}}))
        .unwrap_err();
    assert!(refused.contains("this hole is Ø6.6"), "{refused}");
    // A tolerance class that no internal thread has is refused by name.
    for class in ["99", "6NOPE"] {
        let refused =
            call("hole", json!({"face": face, "points": [[15, 20, 10]], "thread": {"size": 6, "class": class}}))
                .unwrap_err();
        assert!(refused.contains(&format!("{class} is not a tolerance class of an internal thread")), "{refused}");
    }
    // Left out, the diameter is the tap drill.
    call("hole", json!({"face": face, "points": [[15, 20, 10], [45, 20, 10]], "thread": {"size": 6}})).unwrap();
    let refused = call("edit_hole", json!({"feature": "Hole1", "thread": {"size": 6, "class": "99"}})).unwrap_err();
    assert!(refused.contains("99 is not a tolerance class of an internal thread"), "{refused}");
    let listed = call("pmi_list", json!({})).unwrap();
    let item = &listed["items"][0];
    assert_eq!(item["callout"], "2\u{d7} M6x1-6H THRU", "{listed}");
    assert_eq!(item["stated_by"], "Hole1");
    // A stated item is changed through its hole, not removed on its own.
    let refused = call("pmi_remove", json!({"name": "Hole1 thread"})).unwrap_err();
    assert!(refused.contains("change the thread in Hole1"), "{refused}");

    // Fine pitch, 8 mm of thread.
    call("edit_hole", json!({"feature": "Hole1", "thread": {"size": 8, "pitch": 1, "length": 8}})).unwrap();
    let listed = call("pmi_list", json!({})).unwrap();
    assert_eq!(listed["items"][0]["callout"], "2\u{d7} M8x1-6H DEEP 8", "{listed}");

    // One action writes the STEP file and a drawing: the thread is called out once and
    // the file carries its designation.
    let dir = std::env::temp_dir().join(format!("soliddraft-hole-thread-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let step = dir.join("tapped-plate.step");
    let report = call("manufacturing_package", json!({"path": step})).unwrap();
    assert_eq!(report["manufacturing_information"], json!({"items": 1}), "{report}");
    let drawing = &report["drawing"];
    assert_eq!(drawing["not_shown"], json!([]), "{drawing:#}");
    assert_eq!(drawing["shown"]["Hole1 thread"], json!(1), "{drawing:#}");
    assert_eq!(drawing["not_in_file"], json!([]));
    assert_eq!(drawing["overlaps"], json!([]));
    let text = std::fs::read_to_string(&step).unwrap();
    assert!(text.contains("'M8x1-6H'"), "the STEP file carries the designation");
    let dxf = std::fs::read_to_string(step.with_extension("dxf")).unwrap();
    assert!(dxf.contains("2\u{d7} M8x1-6H DEEP 8") || dxf.contains("M8x1-6H DEEP 8"), "the DXF calls the thread out");

    // Taken out, the hole is plain and states nothing.
    call("edit_hole", json!({"feature": "Hole1", "diameter": 7, "plain": true})).unwrap();
    let listed = call("pmi_list", json!({})).unwrap();
    assert_eq!(listed["items"], json!([]), "{listed}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_hole_callout_names_the_head_of_the_hole_it_is_put_on() {
    let kernel = std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    let mut state = soliddraft_app::state::AppState::new(Document::new("Headed holes", kernel));
    let call = |state: &mut soliddraft_app::state::AppState, tool: &str, arguments: serde_json::Value| {
        let out =
            call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "hole-callout".into() });
        assert!(!out.is_error, "{tool}: {}", out.content);
    };
    call(
        &mut state,
        "sketch",
        json!({"plane": "top", "shapes": [{"shape": "rectangle", "center": [30, 20, 0], "width": 60, "height": 40}]}),
    );
    call(&mut state, "extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false}));
    let face = "Boss-Extrude1 \u{b7} End cap";
    // Two Ø5 holes through, counterbored Ø10 for 3 mm; one tapped M6 through, countersunk
    // Ø12 at 90°.
    call(
        &mut state,
        "hole",
        json!({"face": face, "points": [[12, 20, 10], [48, 20, 10]], "diameter": 5, "counterbore": {"diameter": 10, "depth": 3}}),
    );
    call(
        &mut state,
        "hole",
        json!({"face": face, "points": [[30, 20, 10]], "thread": {"size": 6}, "countersink": {"diameter": 12, "angle": 90}}),
    );
    let callouts = |state: &soliddraft_app::state::AppState| -> Vec<(f64, String)> {
        let mut top = soliddraft_app::drawing::NewDrawing::new();
        for (view, on) in &mut top.views {
            *on = *view == soliddraft_kernel_api::drawing::StandardView::Top;
        }
        let session = soliddraft_app::drawing::create_drawing(state, &top).unwrap();
        let sheet = &session.doc.drawing.sheets[0];
        let projected = session.doc.projected_of(sheet.views[0].id).expect("the top view is projected");
        let ctx = soliddraft_app::drawing::part::PartContext::new(&state.doc).expect("the part is proved");
        // Every circle the top view sees is a hole's: the plate and the cut are square.
        let mut out: Vec<(f64, String)> = projected
            .circles
            .iter()
            .map(|circle| {
                let text =
                    soliddraft_app::drawing::part::hole_callout(&ctx, circle, &projected, sheet.template.as_str());
                ((circle.model_radius * 1000.0).round() / 1000.0, text)
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out.dedup();
        out
    };
    let symbols = soliddraft_drawing::library::template("iso-a3").style_file().symbols.clone();
    let (d, deep, cbore, csink, times, degree, thru) = (
        symbols.diameter.as_str(),
        symbols.depth.as_str(),
        symbols.counterbore.as_str(),
        symbols.countersink.as_str(),
        symbols.times.as_str(),
        symbols.degree.as_str(),
        symbols.through.as_str(),
    );
    let counterbored = format!("2{times} {d}5 {thru}\n{cbore} {d}10 {deep} 3");
    let countersunk = format!("M6x1-6H {thru}\n{csink} {d}12 {times} 90{degree}");
    // Every circle of a hole, the head's as well as the drill's, calls out the whole hole
    // with its head: the Ø10 circle is not called out as the Ø5 drill alone.
    assert_eq!(
        callouts(&state),
        [(2.5, counterbored.clone()), (2.5, countersunk.clone()), (5.0, counterbored), (6.0, countersunk),]
    );
    // Milled 1 mm, the counterbore is 2 mm deep and the countersink Ø10 at the top.
    call(
        &mut state,
        "sketch",
        json!({"plane": "top", "offset": 9, "shapes": [{"shape": "rectangle", "center": [30, 20, 9], "width": 80, "height": 60}]}),
    );
    call(&mut state, "cut", json!({"sketch": "Sketch2", "depth": 5, "through_all": false, "flip": true}));
    let counterbored = format!("2{times} {d}5 {thru}\n{cbore} {d}10 {deep} 2");
    let countersunk = format!("M6x1-6H {thru}\n{csink} {d}10 {times} 90{degree}");
    assert_eq!(
        callouts(&state),
        [(2.5, counterbored.clone()), (2.5, countersunk.clone()), (5.0, counterbored), (5.0, countersunk)]
    );
}

/// A plate 60 x 40 x 10 made through the tools, with the tool caller.
fn scripted_plate(name: &str) -> soliddraft_app::state::AppState {
    let kernel = std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    let mut state = soliddraft_app::state::AppState::new(Document::new(name, kernel));
    tool(
        &mut state,
        "sketch",
        json!({"plane": "top", "shapes": [{"shape": "rectangle", "center": [30, 20, 0], "width": 60, "height": 40}]}),
    );
    tool(&mut state, "extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false}));
    state
}

fn tool(state: &mut soliddraft_app::state::AppState, tool: &str, arguments: serde_json::Value) -> serde_json::Value {
    let out = call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "hole-thread".into() });
    assert!(!out.is_error, "{tool}: {}", out.content);
    serde_json::from_str(&out.content).unwrap_or_default()
}

/// The name of the one cylinder whose centroid is at (x, y, z).
fn cylinder(state: &soliddraft_app::state::AppState, x: f64, y: f64, z: f64) -> String {
    let found: Vec<_> = state
        .doc
        .faces
        .iter()
        .filter(|f| f.surface == soliddraft_kernel_api::SurfaceKind::Cylinder)
        .filter(|f| {
            [f.centroid.x, f.centroid.y, f.centroid.z]
                .map(f64::from)
                .iter()
                .zip([x, y, z])
                .all(|(a, b)| (a - b).abs() < 1e-3)
        })
        .collect();
    assert_eq!(found.len(), 1, "one cylinder at ({x}, {y}, {z})");
    format!("{} \u{b7} {}", state.doc.feature(found[0].feature).unwrap().name, found[0].label)
}

/// The drawing's annotations made from the part's items: the item's name, whether the
/// annotation is a note or a dimension, and a note's text, in the drawing's order.
fn drawn_items(state: &soliddraft_app::state::AppState) -> Vec<(String, &'static str, String)> {
    let session = state.drawing.session.as_ref().expect("the drawing is open");
    session
        .doc
        .drawing
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.annotations)
        .filter_map(|a| {
            let item = a.source.as_ref()?.item()?.to_owned();
            Some(match &a.body {
                soliddraft_drawing::AnnotationKind::Note { text, .. } => (item, "note", text.clone()),
                soliddraft_drawing::AnnotationKind::Dimension(_) => (item, "dimension", String::new()),
                _ => (item, "other", String::new()),
            })
        })
        .collect()
}

fn skim_first_hole(state: &mut soliddraft_app::state::AppState) {
    tool(
        state,
        "sketch",
        json!({"plane": "top", "offset": 9, "shapes": [{"shape": "rectangle", "center": [15, 20, 9], "width": 16, "height": 16}]}),
    );
    tool(state, "cut", json!({"sketch": "Sketch2", "depth": 5, "through_all": false, "flip": true}));
}

fn browser_fixture(name: &str, doc: &Document) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_BROWSER_FIXTURES") {
        std::fs::write(std::path::PathBuf::from(dir).join(name), doc.model().to_bytes().unwrap()).unwrap();
    }
}

#[test]
fn a_drawing_calls_out_each_counterbore_after_one_opening_is_skimmed() {
    let mut state = scripted_plate("Counterbored plate");
    tool(
        &mut state,
        "hole",
        json!({"face": "Boss-Extrude1 · End cap", "points": [[15, 20, 10], [45, 20, 10]], "diameter": 5, "depth": 8,
            "counterbore": {"diameter": 10, "depth": 3}}),
    );
    skim_first_hole(&mut state);
    browser_fixture("drawing-skimmed-counterbores.dwprt", &state.doc);
    let mut top = soliddraft_app::drawing::NewDrawing::new();
    for (view, on) in &mut top.views {
        *on = *view == soliddraft_kernel_api::drawing::StandardView::Top;
    }
    let session = soliddraft_app::drawing::create_drawing(&state, &top).unwrap();
    let sheet = &session.doc.drawing.sheets[0];
    let projected = session.doc.projected_of(sheet.views[0].id).unwrap();
    let ctx = soliddraft_app::drawing::part::PartContext::new(&state.doc).unwrap();
    let symbols = &soliddraft_drawing::library::template(sheet.template.as_str()).style_file().symbols;
    for radius in [2.5, 5.0] {
        let mut callouts = projected
            .circles
            .iter()
            .filter(|c| (c.model_radius - radius).abs() < 1e-6)
            .map(|c| soliddraft_app::drawing::part::hole_callout(&ctx, c, &projected, sheet.template.as_str()))
            .collect::<Vec<_>>();
        callouts.sort();
        callouts.dedup();
        let (d, deep, cb) = (&symbols.diameter, &symbols.depth, &symbols.counterbore);
        assert_eq!(
            callouts,
            [format!("{d}5 {deep} 7\n{cb} {d}10 {deep} 2"), format!("{d}5 {deep} 8\n{cb} {d}10 {deep} 3")]
        );
    }
}

#[test]
fn updating_the_drawing_keeps_one_thread_note_when_two_groups_join() {
    let mut state = scripted_plate("Tapped plate");
    tool(
        &mut state,
        "hole",
        json!({"face": "Boss-Extrude1 · End cap", "points": [[15, 20, 10], [45, 20, 10]], "depth": 8,
            "thread": {"size": 6, "length": 6}}),
    );
    skim_first_hole(&mut state);
    browser_fixture("drawing-skimmed-threads.dwprt", &state.doc);
    let dir = std::env::temp_dir().join(format!("soliddraft-thread-groups-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    tool(&mut state, "manufacturing_package", json!({"path": dir.join("tapped.step")}));
    let before = drawn_items(&state);
    assert_eq!(before.len(), 2, "{before:?}");
    assert!(before.iter().any(|(_, _, text)| text.ends_with(" 5")));
    assert!(before.iter().any(|(_, _, text)| text.ends_with(" 6")));

    tool(&mut state, "suppress", json!({"feature": "Cut-Extrude1", "suppressed": true}));
    tool(&mut state, "drawing_rebuild", json!({}));
    let merged = drawn_items(&state);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(merged.len(), 1, "{merged:?}");
    assert!(merged[0].2.starts_with("2× M6x1-6H") && merged[0].2.ends_with(" 6"), "{merged:?}");
}

#[test]
fn hiding_a_body_in_a_display_state_hides_its_thread_marks() {
    use soliddraft_document::display_state::{DisplayMode, DisplayScope, DisplayState};

    let mut state = scripted_plate("Tapped plate");
    tool(
        &mut state,
        "hole",
        json!({"face": "Boss-Extrude1 · End cap", "points": [[30, 20, 10]], "thread": {"size": 6}}),
    );
    let mut app = launch(state.doc, STANDARD);
    ux_tools::rebuilt(&mut app);
    ux_tools::pointer_to_chrome(&mut app);
    let mut case = Case::new("thread-display-state");
    let image = case.frame(&mut app, "shown");
    let before = thread_blue(&app, &image);
    assert!(before > 50, "the shown thread draws {before} blue pixels");

    let doc = &mut app.state_mut().state.doc;
    let mut hidden =
        DisplayState::<soliddraft_model::PartDisplay>::new("Hidden".into(), DisplayScope::All, DisplayMode::Shaded);
    hidden.view.hidden.insert(0);
    let id = hidden.id.clone();
    let mut states = doc.display_states().clone();
    states.put(hidden).unwrap();
    states.select(None, Some(id)).unwrap();
    doc.change_display_states(states).unwrap();
    browser_fixture("thread-hidden.dwprt", doc);
    settle(&mut app);
    let image = case.frame(&mut app, "hidden");
    let after = thread_blue(&app, &image);
    assert_eq!(after, 0, "the hidden thread still draws {after} blue pixels");
}

#[test]
fn hiding_the_active_body_keeps_thread_marks_on_a_visible_body() {
    use soliddraft_app::document::ViewItem;

    let mut state = scripted_plate("Two bodies");
    tool(
        &mut state,
        "hole",
        json!({"face": "Boss-Extrude1 · End cap", "points": [[30, 20, 10]], "thread": {"size": 6}}),
    );
    state.doc.new_body();
    tool(
        &mut state,
        "sketch",
        json!({"plane": "top", "shapes": [{"shape": "rectangle", "center": [90, 20, 0], "width": 20, "height": 20}]}),
    );
    tool(&mut state, "extrude", json!({"sketch": "Sketch2", "depth": 10, "flip": false}));
    assert_eq!(state.doc.model().live_bodies().len(), 2);
    let active = state.doc.active_body;
    assert_ne!(active, 0);
    let mut app = launch(state.doc, STANDARD);
    ux_tools::rebuilt(&mut app);
    ux_tools::pointer_to_chrome(&mut app);
    let mut case = Case::new("thread-visible-other-body");
    let before = case.frame(&mut app, "both-bodies");
    assert!(thread_blue(&app, &before) > 50);
    app.state_mut().state.doc.set_view_choice(ViewItem::Body(active), Some(false));
    settle(&mut app);
    let after = case.frame(&mut app, "active-body-hidden");
    assert!(thread_blue(&app, &after) > 50, "the first body still shows its thread marks");
}

#[test]
fn a_drawing_redraws_a_size_that_became_a_thread_and_back() {
    let mut state = scripted_plate("Bore plate");
    let face = "Boss-Extrude1 \u{b7} End cap";
    tool(&mut state, "hole", json!({"face": face, "points": [[30, 20, 10]], "diameter": 5}));
    let wall = cylinder(&state, 30.0, 20.0, 5.0);
    tool(&mut state, "pmi_add", json!({"kind": "diameter", "name": "Bore", "faces": [wall], "through": true}));
    let dir = std::env::temp_dir().join(format!("soliddraft-thread-conversion-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    tool(&mut state, "manufacturing_package", json!({"path": dir.join("bore.step")}));
    assert_eq!(drawn_items(&state), [("Bore".to_owned(), "dimension", String::new())]);

    // The stored size is made a thread: the drawing calls it out as one, and keeps no
    // dimension that says Ø5.
    tool(
        &mut state,
        "pmi_add",
        json!({"kind": "diameter", "name": "Bore", "faces": [wall], "through": true, "thread": "M6x1-6H", "replaces": "Bore"}),
    );
    tool(&mut state, "drawing_rebuild", json!({}));
    let thru = soliddraft_drawing::library::template("iso-a3").style_file().symbols.through.clone();
    assert_eq!(drawn_items(&state), [("Bore".to_owned(), "note", format!("M6x1-6H {thru}"))]);

    // And back: a plain size is a dimension again, and the thread's note goes.
    tool(
        &mut state,
        "pmi_add",
        json!({"kind": "diameter", "name": "Bore", "faces": [wall], "through": true, "replaces": "Bore"}),
    );
    tool(&mut state, "drawing_rebuild", json!({}));
    assert_eq!(drawn_items(&state), [("Bore".to_owned(), "dimension", String::new())]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_drawing_keeps_a_thread_with_its_hole_through_a_rename_and_a_name_taken() {
    let mut state = scripted_plate("Tapped plate");
    let face = "Boss-Extrude1 \u{b7} End cap";
    tool(&mut state, "hole", json!({"face": face, "points": [[15, 20, 10], [45, 20, 10]], "thread": {"size": 6}}));
    let dir = std::env::temp_dir().join(format!("soliddraft-thread-identity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    tool(&mut state, "manufacturing_package", json!({"path": dir.join("tapped.step")}));
    let thru = soliddraft_drawing::library::template("iso-a3").style_file().symbols.through.clone();
    let note = format!("2\u{d7} M6x1-6H {thru}");
    assert_eq!(drawn_items(&state), [("Hole1 thread".to_owned(), "note", note.clone())]);
    let placed = |state: &soliddraft_app::state::AppState| {
        let session = state.drawing.session.as_ref().unwrap();
        session.doc.drawing.sheets[0]
            .annotations
            .iter()
            .find(|a| matches!(a.body, soliddraft_drawing::AnnotationKind::Note { .. }) && a.source.is_some())
            .map(|a| (a.id, a.placement))
            .unwrap()
    };
    // The note is moved off the place the drawing gave it, as a drafter moves one: a note
    // drawn again would go back there.
    let before = {
        let session = state.drawing.session.as_mut().unwrap();
        let note = session.doc.drawing.sheets[0]
            .annotations
            .iter_mut()
            .find(|a| matches!(a.body, soliddraft_drawing::AnnotationKind::Note { .. }) && a.source.is_some())
            .unwrap();
        note.placement = soliddraft_kernel_api::Point2::new(note.placement.u + 6.0, note.placement.v + 4.0);
        (note.id, note.placement)
    };

    // Renamed, the hole's thread is the same note in the same place, under the new name.
    tool(&mut state, "rename", json!({"feature": "Hole1", "name": "Tapped holes"}));
    tool(&mut state, "drawing_rebuild", json!({}));
    assert_eq!(drawn_items(&state), [("Tapped holes thread".to_owned(), "note", note.clone())]);
    assert_eq!(placed(&state), before, "the same note, not one drawn again");

    // A stored item takes the thread's name: the note stays with the hole, and the stored
    // item gets a dimension of its own.
    let wall = cylinder(&state, 15.0, 20.0, 5.0);
    tool(&mut state, "pmi_add", json!({"kind": "diameter", "name": "Tapped holes thread", "faces": [wall]}));
    tool(&mut state, "drawing_rebuild", json!({}));
    let mut drawn = drawn_items(&state);
    drawn.sort();
    assert_eq!(
        drawn,
        [
            ("Tapped holes thread".to_owned(), "dimension", String::new()),
            ("Tapped holes thread 2".to_owned(), "note", note),
        ]
    );
    assert_eq!(placed(&state), before, "the thread's note did not move to the stored item");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_thread_marks_leave_out_what_later_cuts_took_from_the_wall() {
    let mut state = scripted_plate("Pocketed plate");
    let face = "Boss-Extrude1 \u{b7} End cap";
    tool(&mut state, "hole", json!({"face": face, "points": [[15, 20, 10]], "thread": {"size": 6}}));
    // A pocket in from the plate's side takes the wall away all round from 4 to 6 mm up;
    // another, 7 to 9 mm up, takes the half of the wall beyond y 20.
    for (sketch, offset, centre, height) in [("Sketch2", 4, 32.0, 30.0), ("Sketch3", 7, 33.5, 27.0)] {
        tool(
            &mut state,
            "sketch",
            json!({"plane": "top", "offset": offset, "shapes": [{"shape": "rectangle", "center": [15, centre, offset], "width": 10, "height": height}]}),
        );
        tool(&mut state, "cut", json!({"sketch": sketch, "depth": 2, "through_all": false, "flip": true}));
    }
    let doc = &state.doc;
    assert!(doc.model().statuses().iter().all(|(_, s)| s.is_ok()), "{:?}", doc.model().statuses());
    let marks = soliddraft_app::viewport::threads::marks(doc).expect("the thread is marked");
    let (mut above, mut below, mut beside) = (0, 0, 0);
    for pair in marks.edges.iter().flat_map(|edge| edge.segments.as_chunks::<2>().0) {
        let mid = (pair[0] + pair[1]) / 2.0;
        let (z, y) = (mid.z, mid.y);
        assert!(!(4.05..5.95).contains(&z), "a mark crosses the pocket at {mid}");
        assert!(!((7.1..8.9).contains(&z) && y > 20.1), "a mark crosses the notch at {mid}");
        above += usize::from(z > 6.0);
        below += usize::from(z < 4.0);
        beside += usize::from((7.1..8.9).contains(&z) && y < 19.9);
    }
    assert!(above > 0 && below > 0 && beside > 0, "{above} {below} {beside}");
}
