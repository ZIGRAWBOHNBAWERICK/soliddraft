//! Sheet history through the application loop, including redraw and file export.

use egui_kittest::Harness;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::drawing::{self, DrawingAction, DrawingDoc, DrawingSession, DrawingTool, PendingView, PlaceKind};
use soliddraft_app::state::AppAction;
use soliddraft_drawing::export::Format;
use soliddraft_drawing::project::{Projected, ViewTransform};
use soliddraft_kernel_api::Point2;
use soliddraft_kernel_api::drawing::{StandardView, ViewSpec};
use std::sync::Once;

type App = Harness<'static, SolidDraftApp>;

fn app() -> App {
    static CONFIG: Once = Once::new();
    CONFIG.call_once(|| {
        let dir = std::env::temp_dir().join(format!("soliddraft-sheet-history-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        soliddraft_app::file::set_config_dir(Some(dir));
    });
    let mut app = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|cc| SolidDraftApp::new(cc, soliddraft_app::new_document(), Automation::default()));
    app.state_mut().state.drawing.session =
        Some(DrawingSession::new(DrawingDoc::new(soliddraft_drawing::fixture::bracket_a3())));
    action(&mut app, DrawingAction::ShowDrawing);
    app
}

fn action(app: &mut App, action: DrawingAction) {
    app.state_mut().state.push(AppAction::Drawing(action));
    app.run_steps(2);
}

fn session(app: &App) -> &DrawingSession {
    app.state().state.drawing.session.as_ref().unwrap()
}

fn redraw_and_export(app: &mut App, name: &str) {
    let dir = std::env::temp_dir().join(format!("soliddraft-sheet-history-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // PDF is a control: it enumerates the restored sheet list rather than the selection.
    for format in [Format::Pdf, Format::Svg, Format::Dxf] {
        let path = dir.join(format!("sheet.{}", format.extension()));
        action(app, DrawingAction::Export { format, path: Some(path.clone()) });
        assert!(std::fs::metadata(&path).unwrap().len() > 100);
        assert_eq!(session(app).last_export.as_ref(), Some(&path));
    }
    let s = session(app);
    assert!(s.sheet < s.doc.drawing.sheets.len());
    assert!(!soliddraft_drawing::display::layout_sheet(&s.doc.drawing, s.sheet).items.is_empty());
    assert_eq!(
        drawing::status(&app.state().state).unwrap().details[0],
        format!("Sheet {} of {}", s.sheet + 1, s.doc.drawing.sheets.len())
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn add_sheet_undo_redraws_and_exports_a_surviving_sheet() {
    let mut app = app();
    action(&mut app, DrawingAction::AddSheet);
    assert_eq!(session(&app).sheet, 1);
    app.state_mut().state.push(AppAction::Undo);
    app.run_steps(2);
    redraw_and_export(&mut app, "add-undo");
    assert_eq!(session(&app).sheet, 0);
    action(&mut app, DrawingAction::Redo);
    action(&mut app, DrawingAction::SetSheet(1));
    redraw_and_export(&mut app, "manual-tab");
}

#[test]
fn delete_undo_select_redo_redraws_and_exports_a_surviving_sheet() {
    let mut app = app();
    action(&mut app, DrawingAction::AddSheet);
    action(&mut app, DrawingAction::DeleteSheet(1));
    action(&mut app, DrawingAction::Undo);
    action(&mut app, DrawingAction::SetSheet(1));
    action(&mut app, DrawingAction::Redo);
    redraw_and_export(&mut app, "delete-redo");
    assert_eq!(session(&app).sheet, 0);
}

#[test]
fn history_discards_a_cached_preview_from_the_removed_sheet() {
    let mut app = app();
    action(&mut app, DrawingAction::AddSheet);
    let s = app.state_mut().state.drawing.session.as_mut().unwrap();
    s.set_tool(DrawingTool::PlaceView(PlaceKind::Standard(StandardView::Front)));
    let view = s.doc.drawing.sheets[0].views[0].clone();
    s.pending = Some(PendingView {
        projected: Projected {
            geometry: view.geometry.clone(),
            transform: ViewTransform {
                frame: ViewSpec::standard(StandardView::Front).frame().unwrap(),
                scale: 1.0,
                centre: Point2::new(0.0, 0.0),
            },
            circles: Vec::new(),
        },
        view,
        direction: None,
        aligned: false,
    });
    s.draft.points.push(Point2::new(1.0, 1.0));
    action(&mut app, DrawingAction::Undo);
    app.hover_at(session(&app).canvas_rect.center());
    app.run_steps(2);
    let s = session(&app);
    assert!(s.pending.is_none(), "the preview belongs to the removed sheet");
    assert!(s.draft.points.is_empty());
    assert_eq!(s.tool, DrawingTool::Select);
    redraw_and_export(&mut app, "preview-undo");
}

#[test]
fn history_keeps_the_selected_sheet_when_an_earlier_sheet_returns() {
    let mut app = app();
    action(&mut app, DrawingAction::AddSheet);
    action(&mut app, DrawingAction::AddSheet);
    let name = session(&app).sheet().name.clone();
    action(&mut app, DrawingAction::DeleteSheet(0));
    action(&mut app, DrawingAction::SetSheet(1));
    action(&mut app, DrawingAction::Undo);
    assert_eq!(session(&app).sheet().name, name);
    action(&mut app, DrawingAction::Redo);
    assert_eq!(session(&app).sheet().name, name);
}

#[test]
fn export_refuses_an_invalid_sheet_without_panicking() {
    let mut app = app();
    app.state_mut().state.drawing.session.as_mut().unwrap().sheet = 1;
    let dir = std::env::temp_dir().join(format!("soliddraft-sheet-export-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for format in [Format::Svg, Format::Dxf] {
        let path = dir.join(format!("sheet.{}", format.extension()));
        action(&mut app, DrawingAction::Export { format, path: Some(path.clone()) });
        assert!(!path.exists());
        assert!(app.state().state.note.as_ref().unwrap().text.contains("select a sheet"));
    }
    let pdf = dir.join("all.pdf");
    action(&mut app, DrawingAction::Export { format: Format::Pdf, path: Some(pdf.clone()) });
    assert!(pdf.is_file());
    std::fs::remove_dir_all(dir).unwrap();
}
