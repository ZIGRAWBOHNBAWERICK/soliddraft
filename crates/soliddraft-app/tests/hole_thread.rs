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

fn bar(app: &App) -> Rect {
    soliddraft_app::options_bar::shown_rect(&app.ctx).expect("the options bar is drawn")
}

fn click_bar(app: &mut App, label: &str) {
    let rect = ribbon::control(app, label, bar(app)).rect;
    ribbon::click_at(app, rect.center());
    settle(app);
}

fn more(app: &mut App) {
    let (name, _) = ribbon::buttons_in(app, bar(app)).into_iter().find(|(s, _)| s.starts_with("More (")).unwrap();
    click_bar(app, &name);
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
    click(app, combo);
    app.get_by_label(entry).scroll_to_me();
    app.run_steps(24);
    click(app, entry);
}

fn editing(app: &App) -> soliddraft_app::document::HoleParams {
    match &app.state().state.editor.as_ref().expect("the hole form is open").params {
        EditorParams::Hole(h) => h.clone(),
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
        assert!((walls[0].span.1 - 9.0).abs() < 1e-6 && (walls[0].radius - 3.5).abs() < 1e-6, "{walls:?}");
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

        // A later cut mills 4 mm off the top: the thread stays on what is left of the wall.
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
        assert!((walls[0].span.0 - 4.0).abs() < 1e-6 && (walls[0].span.1 - 9.0).abs() < 1e-6, "{walls:?}");
        assert_eq!(thread_callouts(doc), ["M10x1.5-6H DEEP 9"]);
        ux_tools::pointer_to_chrome(app);
        assert!({ let image = case.frame(app, "milled"); thread_blue(app, &image) } > before + 50);

        // Saved and reopened, the thread is the same.
        let bytes = app.state().state.doc.model().to_bytes().unwrap();
        let reopened = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
        let reopened = Document::from_model(reopened);
        assert_eq!(hole(&reopened).1, hole(&app.state().state.doc).1);
        assert_eq!(thread_callouts(&reopened), ["M10x1.5-6H DEEP 9"]);
        assert_eq!(reopened.model().threaded_walls(), app.state().state.doc.model().threaded_walls());
        let mut again = launch(reopened, STANDARD);
        ux_tools::rebuilt(&mut again);
        ux_tools::pointer_to_chrome(&mut again);
        assert!({ let image = case.frame(&mut again, "reopened"); thread_blue(&again, &image) } > before + 50);
    });
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
    // Left out, the diameter is the tap drill.
    call("hole", json!({"face": face, "points": [[15, 20, 10], [45, 20, 10]], "thread": {"size": 6}})).unwrap();
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
