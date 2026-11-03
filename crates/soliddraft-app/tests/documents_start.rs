//! The start page, autosave and crash recovery in the real app on a real adapter, driven
//! as a user drives them: the start page lists the documents saved before with their
//! pictures and opens one on a click, its buttons make documents, Preferences sets the
//! autosave interval, and an app killed in the middle of editing offers its unsaved work
//! at the next start. The kill is real: the editing app is a child process of this test,
//! and it is ended with SIGKILL (TerminateProcess on Windows).
//!
//! Frames land under `target/ribbon-acceptance/<case>/`.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::vec2;
use egui_kittest::kittest::Queryable;
use ribbon::{App, app_at, blank, case, plate};
use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, ProfileKind};
use soliddraft_app::state::AppAction;
use soliddraft_document::{AutosavePolicy, DocKind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const WINDOW: egui::Vec2 = vec2(1440.0, 900.0);

/// The variable that turns this test binary into the app that is killed.
const CHILD: &str = "SOLIDDRAFT_KILLED_APP_DIR";

fn block(depth: f64) -> Command {
    Command::AddExtrude {
        params: ExtrudeParams {
            sketch: None,
            profile: ProfileKind::Rectangle { width: 40.0, height: 30.0 },
            plane: PlaneKind::Top,
            depth,
            flip: false,
            end: Default::default(),
        },
        via_assistant: false,
    }
}

/// Shows the start page and waits for its recent list.
fn start_page(app: &mut App) {
    let ctx = app.ctx.clone();
    app.state_mut().show_start_page(&ctx);
    let until = Instant::now() + Duration::from_secs(30);
    while app.state().state.start.as_ref().is_some_and(|p| p.docs.is_none()) {
        assert!(Instant::now() < until, "the recent list is read");
        app.run_steps(2);
        std::thread::sleep(Duration::from_millis(20));
    }
    app.run_steps(4);
}

fn click(app: &mut App, label: &str) {
    app.get_by_label(label).click();
    app.run_steps(4);
}

fn texts(app: &App) -> Vec<String> {
    use egui_kittest::kittest::NodeT;
    app.query_all(egui_kittest::kittest::by())
        .flat_map(|n| [n.accesskit_node().label(), n.accesskit_node().value()])
        .flatten()
        .collect()
}

#[test]
fn the_start_page_lists_saved_documents_with_their_pictures_and_opens_one() {
    let case = case("start-page-recent");
    let folder = case.path("documents");
    std::fs::create_dir_all(&folder).unwrap();
    // Two parts saved the way the user saves them, so they carry the picture a save makes.
    for (name, doc) in [("Bracket", plate()), ("Spacer", plate())] {
        let mut app = app_at(doc, WINDOW, 1.0);
        app.state_mut().state.push(AppAction::SaveAs(Some(folder.join(format!("{name}.dwprt")))));
        app.run_steps(4);
    }
    for name in ["Bracket", "Spacer"] {
        let file = folder.join(format!("{name}.dwprt"));
        assert!(soliddraft_document::thumbnail(&file).unwrap().is_some(), "{name} was saved with its picture");
    }

    // The next start, with no document asked for.
    let mut app = app_at(blank(), WINDOW, 1.0);
    case.run(&mut app, |app| {
        start_page(app);
        for label in ["New part", "New assembly", "New drawing", "Open…", "Bracket.dwprt", "Spacer.dwprt"] {
            assert!(app.query_by_label(label).is_some(), "the start page offers {label}");
        }
        let docs = app.state().state.start.as_ref().unwrap().docs.clone().unwrap();
        let names: Vec<&str> = docs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Spacer.dwprt", "Bracket.dwprt"], "newest first");
        assert!(docs.iter().all(|d| d.thumbnail.is_some()), "each card shows the part's picture");
        assert!(docs.iter().all(|d| d.kind == Some(DocKind::Part)));
        let shown = texts(app);
        let location = std::fs::canonicalize(&folder).unwrap().display().to_string();
        assert!(shown.contains(&location), "the card says where the file is: {location}");
        assert!(shown.iter().any(|t| t.starts_with("Part · Today ")), "and when it was saved: {shown:?}");
        assert!(app.query_by_label("Solid").is_none(), "the ribbon waits for a document");
        assert!(
            !shown.iter().any(|t| t.ends_with(".dwprt") && !t.starts_with("Bracket") && !t.starts_with("Spacer")),
            "the title bar names no document: {shown:?}"
        );
        case.frame(app, "start-page");

        click(app, "Bracket.dwprt");
        app.run_steps(6);
        let state = &app.state().state;
        assert!(state.start.is_none(), "opening a document leaves the start page");
        assert_eq!(state.doc.name, "Bracket");
        assert_eq!(state.doc.features.len(), 1);
        case.frame(app, "opened-from-start-page");
    });
}

#[test]
fn the_start_page_buttons_make_documents_and_an_edit_leaves_it() {
    let _case = case("start-page-buttons");
    let mut app = app_at(blank(), WINDOW, 1.0);
    start_page(&mut app);
    assert!(texts(&app).iter().any(|t| t == "Documents you open or save appear here."), "an empty list says so");
    click(&mut app, "New part");
    assert!(app.state().state.start.is_none(), "New part shows the new part");

    start_page(&mut app);
    click(&mut app, "New assembly");
    app.run_steps(4);
    assert!(app.state().state.assembly.session.is_some(), "New assembly opens an assembly");
    assert!(app.state().state.start.is_none());

    // New drawing on a part with a body starts the drawing's setup at once.
    let mut app = app_at(plate(), WINDOW, 1.0);
    start_page(&mut app);
    click(&mut app, "New drawing");
    app.run_steps(4);
    assert!(app.state().state.drawing.new.is_some(), "New drawing starts the drawing setup");
    assert!(app.state().state.start.is_none(), "and the setup shows instead of the start page");

    // A command typed or run elsewhere edits the empty part: the part shows.
    let mut app = app_at(blank(), WINDOW, 1.0);
    start_page(&mut app);
    app.state_mut().state.doc.apply(block(10.0)).unwrap();
    app.run_steps(2);
    assert!(app.state().state.start.is_none(), "an edit leaves the start page");
    app.state_mut().state.push(AppAction::ShowStartPage);
    app.run_steps(4);
    assert!(app.state().state.start.is_some(), "File > Start page shows it again");
    assert!(app.query_by_label("Solid").is_none());
}

/// The browser checks (`ci/browser-documents.mjs`, `ci/browser-preferences.mjs`) leave the
/// start page with a click at a fixed point of a 1400 x 813 page.
#[test]
fn the_browser_checks_find_new_part_where_they_click() {
    let _case = case("start-page-new-part-point");
    let mut app = app_at(blank(), vec2(1400.0, 813.0), 1.0);
    start_page(&mut app);
    let tile = app.get_by_label("New part").rect();
    assert!(tile.shrink(8.0).contains(egui::pos2(124.0, 143.0)), "New part is at {tile:?}");
}

/// The app this test kills: edits a part called Bracket without saving, and keeps editing
/// after autosave's first copy is on disk. Does nothing unless the parent test started it.
#[test]
fn killed_app_edits_until_it_is_killed() {
    let Some(dir) = std::env::var_os(CHILD).map(PathBuf::from) else { return };
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut app = app_at(Document::new("Bracket", soliddraft_app::kernel()), WINDOW, 1.0);
    let policy = AutosavePolicy { enabled: true, interval_seconds: 1, edits: 1 };
    app.state_mut().start_recovery(&dir.join("recovery"), policy);
    let mut depth = 10.0;
    loop {
        app.state_mut().state.doc.apply(block(depth)).unwrap();
        depth += 1.0;
        app.run_steps(3);
        let copies = sessions(&dir.join("recovery")).iter().flat_map(|s| copies(s)).count();
        if copies > 0 && !dir.join("ready").exists() {
            std::fs::write(dir.join("ready"), b"").unwrap();
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn sessions(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("session-"))
                .collect()
        })
        .unwrap_or_default()
}

fn copies(folder: &Path) -> Vec<PathBuf> {
    soliddraft_document::recovery::scan(folder).copies.into_iter().map(|c| c.file).collect()
}

/// Starts the editing app in a child process, waits for its first recovery copy, lets it
/// edit a little longer and kills it.
fn kill_while_editing(dir: &Path) {
    let _ = std::fs::remove_file(dir.join("ready"));
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["killed_app_edits_until_it_is_killed", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, dir)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("the editing app starts");
    let until = Instant::now() + Duration::from_secs(180);
    while !dir.join("ready").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the editing app stopped by itself: {status}");
        }
        assert!(Instant::now() < until, "the editing app wrote no recovery copy");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    child.kill().expect("the editing app is killed");
    let status = child.wait().unwrap();
    assert!(!status.success(), "it was killed, not ended: {status}");
}

#[test]
fn an_app_killed_mid_edit_offers_its_work_at_the_next_start() {
    let case = case("killed-mid-edit");
    let dir = case.path("run");
    std::fs::create_dir_all(dir.join("config")).unwrap();
    let root = dir.join("recovery");

    // First kill: the next start offers the part, and Discard throws it away.
    kill_while_editing(&dir);
    let left: Vec<PathBuf> = sessions(&root).iter().flat_map(|s| copies(s)).collect();
    assert_eq!(left.len(), 1, "the killed app's copy survived it");
    let mut app = app_at(blank(), WINDOW, 1.0);
    case.run(&mut app, |app| {
        app.state_mut().start_recovery(&root, AutosavePolicy::default());
        start_page(app);
        let recovery = app.state().state.recovery.as_ref().unwrap();
        assert!(recovery.show, "the Recover documents window opens by itself");
        assert_eq!(recovery.offers.len(), 1);
        assert_eq!(recovery.offers[0].record.name, "Bracket");
        assert!(app.query_by_label("Recover documents").is_some());
        let shown = texts(app);
        assert!(shown.iter().any(|t| t == "Bracket · Part"), "what: {shown:?}");
        assert!(shown.iter().any(|t| t == "Never saved"), "where");
        assert!(shown.iter().any(|t| t.starts_with("Today ") && t.ends_with("(just now)")), "when: {shown:?}");
        case.frame(app, "offered-after-kill");
        click(app, "Discard");
        assert!(app.state().state.recovery.as_ref().unwrap().offers.is_empty());
        assert!(!app.state().state.recovery.as_ref().unwrap().show, "with nothing left, the window closes");
        assert!(!left[0].exists(), "Discard deletes the copy");
    });
    drop(app);

    // Second kill: Restore brings the part back, unsaved.
    kill_while_editing(&dir);
    let mut app = app_at(blank(), WINDOW, 1.0);
    case.run(&mut app, |app| {
        app.state_mut().start_recovery(&root, AutosavePolicy::default());
        start_page(app);
        assert_eq!(app.state().state.recovery.as_ref().unwrap().offers.len(), 1, "a discarded copy is not offered");
        assert!(app.query_by_label("Recover documents…").is_some(), "the start page points at the unsaved work");
        click(app, "Restore");
        app.run_steps(6);
        let state = &app.state().state;
        assert!(state.start.is_none(), "the restored part shows");
        assert_eq!(state.doc.name, "Bracket");
        assert!(state.doc.dirty, "restored work is still unsaved");
        assert!(!state.doc.features.is_empty(), "with the edits made before the kill");
        assert_eq!(state.doc.path, None);
        assert!(!state.recovery.as_ref().unwrap().show, "with nothing left, the window closes");
        case.frame(app, "restored-after-kill");
    });
}

#[test]
fn preferences_set_the_autosave_interval() {
    let case = case("autosave-preferences");
    let mut app = app_at(plate(), WINDOW, 1.0);
    let root = case.path("recovery");
    app.state_mut().start_recovery(&root, AutosavePolicy::default());
    case.run(&mut app, |app| {
        app.state_mut().state.push(AppAction::ShowPreferences);
        app.run_steps(4);
        click(app, "Autosave and recovery");
        assert!(app.query_by_label("General and documents > Autosave and recovery").is_some());
        let minutes = app.get_by_label("Minutes between recovery copies");
        minutes.click();
        app.run_steps(2);
        app.get_by_label("Minutes between recovery copies").type_text("5");
        app.run_steps(2);
        app.key_press(egui::Key::Enter);
        app.run_steps(4);
        case.frame(app, "autosave-preferences");
        let policy = app.state().state.recovery.as_ref().unwrap().policy;
        assert_eq!(policy.interval_seconds, 300, "the running autosave follows the new interval");
        assert_eq!(soliddraft_app::settings::documents().autosave.interval_seconds, 300, "and it is saved");
        click(app, "Keep recovery copies of unsaved work");
        assert!(!app.state().state.recovery.as_ref().unwrap().policy.enabled);
        assert!(!soliddraft_app::settings::documents().autosave.enabled);
    });
}
