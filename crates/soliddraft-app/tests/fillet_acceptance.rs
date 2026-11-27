//! Plain fillets on the owner's part, a box and the sample part, through the tools the
//! Fillet panel's commands use.
//!
//! Each case adds one fillet with the `fillet` tool, as the panel's OK does, and reports
//! whether it built or which construction refused it and why. The rows are written to
//! `<target>/tmp/fillet-acceptance/<case>.json` before any assertion, so a failing run
//! still leaves the whole table.

use std::{collections::BTreeSet, sync::Arc, time::Instant};

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::{Aabb, BodyAccuracy, EdgeId, FaceId, KernelError, KernelResult};
use soliddraft_kernel_catallax::{APP_REQUEST_WORK, CatallaxKernel};

#[path = "support/import_open.rs"]
mod import_open;
#[path = "support/ui.rs"]
mod ribbon;

/// The owner's Boss-Extrude1: an 80 x 60 profile with R25 top corners, so its flat top
/// is 30 mm wide and runs tangentially into the rounded sides, extruded 76 mm.
const OWNER_BOSS: &str = r#"{"plane":"front","operations":[
 {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":-40,"v":0},"b":{"u":40,"v":0}}},
 {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":40,"v":0},"b":{"u":40,"v":35}}},
 {"tool":"add","construction":false,"kind":{"type":"arc","center":{"u":15,"v":35},"start":{"u":40,"v":35},"end":{"u":15,"v":60}}},
 {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":15,"v":60},"b":{"u":-15,"v":60}}},
 {"tool":"add","construction":false,"kind":{"type":"arc","center":{"u":-15,"v":35},"start":{"u":-15,"v":60},"end":{"u":-40,"v":35}}},
 {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":-40,"v":35},"b":{"u":-40,"v":0}}}]}"#;

/// The owner's Cut-Extrude1: a 34 x 22.67 pocket sketched on the flat top, centred on
/// the line where the flat top meets the rounded side, cut 35 mm deep.
const OWNER_POCKET: &str =
    r#"{"plane":"top","offset":60,"shapes":[{"shape":"rectangle","center":[15,-38,60],"width":34,"height":22.67}]}"#;

/// The correct pocket part has 13 faces: the flat top opened by the pocket mouth, two
/// rounded sides, two flat sides, the bottom, two ends, four pocket walls and the floor.
const OWNER_FACES: usize = 13;

/// The radius at which the rim fillet first meets a face beside it. On the rounded side
/// the pocket wall stands at x = 32 and the part's outer side at x = 40, so 8 mm of wall
/// remain under the R25 round. A rim fillet along that wall of radius r has its centre
/// where (17 + r)^2 + (z - 35)^2 = (25 - r)^2, which has a solution only up to r = 4,
/// where the fillet reaches the outer side face. OpenCASCADE builds the whole rim at
/// 3.998 and refuses it at 4.
const OWNER_CONTACT: f64 = 4.0;

/// The largest radius the test asks of the whole rim, just under [`OWNER_CONTACT`].
const OWNER_LARGEST: f64 = 3.99;

/// What became of one fillet request.
#[derive(Debug)]
struct Row {
    case: String,
    radius: f64,
    edges: Vec<String>,
    /// `pass`, `name` (the app could not name an edge) or `refused` (the kernel refused).
    result: &'static str,
    /// The construction that refused, read from the kernel's refusal code.
    stage: String,
    /// The kernel's own refusal, for the kernel lanes; never shown to the user.
    kernel: String,
    /// What the user reads.
    message: String,
    /// What the `fillet` tool answered.
    tool: String,
    /// The Catallax recipe of the last build the fillet asked for, so the kernel lanes
    /// can run the same case. Kept only for a refused fillet.
    recipe: Option<Value>,
    accuracy: String,
    faces: usize,
    work: Option<u64>,
    seconds: f64,
    /// What became of the built part's STEP export, where the case asks for one:
    /// `exported N bytes` and the export's report, or the refusal.
    step: String,
}

impl Row {
    fn json(&self) -> Value {
        json!({
            "case": self.case, "radius": self.radius, "edges": self.edges, "result": self.result,
            "stage": self.stage, "kernel": self.kernel, "message": self.message, "tool": self.tool,
            "accuracy": self.accuracy, "faces": self.faces, "work": self.work, "seconds": self.seconds,
            "step": self.step,
        })
    }
}

/// The construction a kernel refusal came from. A refusal that tried both constructions
/// names both, exact first.
fn stage(kernel: &str) -> String {
    let code = |text: &str| -> Option<String> {
        let at = text.find(|c: char| c.is_ascii_lowercase())?;
        let word: String =
            text[at..].chars().take_while(|c| c.is_ascii_lowercase() || *c == '.' || *c == '-').collect();
        word.contains('.').then_some(word)
    };
    let exact = kernel.split("exact path refused ").nth(1).and_then(code);
    let numerical = kernel.split("numerical continuation refused ").nth(1).and_then(code);
    match (exact, numerical) {
        (Some(exact), Some(numerical)) => format!("exact {exact}; numerical {numerical}"),
        _ => kernel
            .split_whitespace()
            .find_map(|word| code(word.trim_end_matches(':')).filter(|c| c.contains('-') || c.contains('.')))
            .unwrap_or_else(|| "unknown".into()),
    }
}

struct Part {
    state: AppState,
    kernel: Arc<CatallaxKernel>,
}

impl Part {
    fn new(name: &str) -> Self {
        let kernel = Arc::new(CatallaxKernel::with_diagnostics(APP_REQUEST_WORK));
        let state = AppState::new(Document::new(name, kernel.clone()));
        Part { state, kernel }
    }

    fn tool(&mut self, tool: &str, arguments: Value) -> soliddraft_assistant::tools::ToolOutput {
        call_checked(
            &mut AppTools { state: &mut self.state },
            tool,
            arguments,
            &Caller::Mcp { client: "fillet-acceptance".into() },
        )
    }

    fn build(&mut self, tool: &str, arguments: &str) {
        let out = self.tool(tool, serde_json::from_str(arguments).unwrap());
        assert!(!out.is_error, "{tool}: {}", out.content);
    }

    fn doc(&self) -> &Document {
        &self.state.doc
    }

    fn topology(&self) -> soliddraft_kernel_api::Topology {
        let model = self.doc().model();
        model.kernel().topology(model.body().unwrap()).unwrap()
    }

    fn feature_of(&self, face: FaceId) -> String {
        let named = self.doc().named_face(face).expect("every face has a name");
        self.doc().feature(named.feature).unwrap().name.clone()
    }

    fn edge_name(&self, edge: EdgeId) -> String {
        let named = self.doc().named_edge(edge).expect("every edge has a name");
        format!("{} · {}", self.doc().feature(named.feature).unwrap().name, named.label)
    }

    /// Adds a fillet as the panel's OK does, reports it, and undoes it so the next case
    /// starts from the same part.
    fn fillet(&mut self, case: &str, edges: Vec<String>, radius: f64) -> Row {
        self.fillet_and(case, edges, radius, false)
    }

    /// Exports the open part as STEP, as File › Export does, and keeps the file beside the
    /// reports. OpenCASCADE opens these files outside this test, as CI's
    /// `ci/check-step.py` opens the bracket's.
    fn export(&mut self, name: &str) -> String {
        let path = step_path(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let exported = self.tool("export_step", json!({ "path": path.to_str().unwrap() }));
        if exported.is_error {
            return format!("export refused: {}", exported.content);
        }
        let report = serde_json::from_str::<Value>(&exported.content).unwrap();
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        if !written.starts_with("ISO-10303-21;") || !written.trim_end().ends_with("END-ISO-10303-21;") {
            return "export refused: the file written is not a complete STEP file".into();
        }
        format!("exported {} bytes: {}", written.len(), report["report"].as_str().unwrap_or_default())
    }

    /// As [`Part::fillet`]; `export` also writes the built part to STEP.
    fn fillet_and(&mut self, case: &str, edges: Vec<String>, radius: f64, export: bool) -> Row {
        let before = self.doc().model().part_json();
        let (features, faces) = (self.doc().features.len(), self.topology().faces.len());
        self.kernel.take_diagnostics().unwrap();
        let start = Instant::now();
        let out = self.tool("fillet", json!({"edges": edges, "radius": radius}));
        let seconds = start.elapsed().as_secs_f64();
        let calls = self.kernel.take_diagnostics().unwrap();
        let fillet_calls: Vec<&Value> =
            calls["calls"].as_array().unwrap().iter().filter(|c| c["operation"] == "fillet").collect();
        let work = fillet_calls.iter().filter_map(|c| c["work"]["used"].as_u64()).max();
        let kernel = fillet_calls.iter().find_map(|c| c["error"].as_str()).unwrap_or_default().to_owned();
        let mut row = Row {
            case: case.into(),
            radius,
            edges,
            result: "pass",
            stage: String::new(),
            kernel: kernel.clone(),
            message: String::new(),
            tool: out.content.clone(),
            recipe: None,
            accuracy: String::new(),
            faces: 0,
            work,
            seconds,
            step: String::new(),
        };
        if out.is_error {
            // The panel shows the kernel's refusal through the same words, with the faces
            // the document names for it. The kernel's own record never carries those
            // names, so the tool's answer is read when the document added them.
            let named = [soliddraft_model::RUNS_INTO, soliddraft_model::FITS_BETWEEN]
                .iter()
                .any(|marker| out.content.contains(marker));
            row.message = soliddraft_app::geometry_message::for_user(if kernel.is_empty() || named {
                &out.content
            } else {
                &kernel
            });
            if fillet_calls.is_empty() {
                // Refused before the kernel was asked: an edge the app could not name.
                row.result = "name";
                row.stage = "naming".into();
                row.kernel = out.content.clone();
            } else if kernel.is_empty() {
                // The kernel built the fillet, and the app refused a body it could not show.
                row.result = "refused";
                row.stage = "display".into();
                row.kernel = out.content.clone();
            } else {
                row.result = "refused";
                row.stage = stage(&kernel);
                row.recipe = calls["questions"].as_array().and_then(|q| q.last()).map(|q| q["problem"].clone());
            }
            // A fillet refused after it was built, because its body could not be displayed,
            // used up its feature id, as an undone feature does: ids are never reused.
            // Everything else is as before.
            let ids = |part: &str| {
                let mut part: Value = serde_json::from_str(part).unwrap();
                part.as_object_mut().unwrap().remove("next_feature");
                part
            };
            assert_eq!(ids(&self.doc().model().part_json()), ids(&before), "{case}: a refused fillet changed the part");
            return row;
        }
        let model = self.doc().model();
        let body = model.body().unwrap();
        row.accuracy = match model.kernel().body_accuracy(body).unwrap() {
            BodyAccuracy::Exact => "exact".into(),
            BodyAccuracy::Approximate { tolerance_mm } => format!("approximate within {tolerance_mm} mm"),
            BodyAccuracy::Tolerant => "tolerant".into(),
        };
        let topology = model.kernel().topology(body);
        row.faces = topology.as_ref().map_or(0, |topology| topology.faces.len());
        let fillet = self.doc().features.last().unwrap();
        if let Err(error) = topology {
            // The kernel made the body but cannot sample it, so the user cannot see or pick it.
            row.result = "refused";
            row.stage = "display".into();
            row.kernel = error.to_string();
            row.message = format!(
                "{:?}; {} triangles shown; the user reads {:?}",
                fillet.status,
                self.doc().triangle_count(),
                self.doc().proof_message()
            );
        } else if !matches!(fillet.status, soliddraft_app::document::FeatureStatus::Ok) {
            row.result = "refused";
            row.stage = "status".into();
            row.message = format!("{:?}", fillet.status);
        } else if self.doc().triangle_count() == 0 {
            row.result = "refused";
            row.stage = "display".into();
            row.message = "the filleted part shows nothing".into();
        }
        if export && row.result == "pass" {
            row.step = self.export(&format!("{case}-r{radius}"));
            if !row.step.starts_with("exported") {
                row.result = "refused";
                row.stage = "step".into();
                row.message = row.step.clone();
            }
        }
        self.state.doc.undo().unwrap();
        assert_eq!(self.doc().features.len(), features, "{case}: undo did not remove the fillet");
        assert_eq!(self.topology().faces.len(), faces, "{case}: undo did not restore the part");
        row
    }
}

fn step_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("fillet-acceptance")
        .join("step")
        .join(format!("{name}.step"))
}

/// Imports a case's exported STEP file through the dialog and returns the imported body's
/// volume, or why it has none, and its box.
fn import_exported_step(name: &str, faces: usize) -> (KernelResult<f64>, Aabb) {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::{
        document::FeatureKind,
        import::report::{self, Tier},
    };
    let path = step_path(name);
    let case = ribbon::case(&format!("fillet-roundtrip-{name}"));
    let mut app = ribbon::app(Document::new("Reimported fillets", soliddraft_app::startup::kernel()));
    app.state_mut().state.push(soliddraft_app::state::AppAction::ImportStep(Some(path.clone())));
    import_open::settle(&mut app);
    assert!(app.state().state.import_dialog.is_some(), "{name}: Import STEP did not open");
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
    import_open::settle(&mut app);
    let doc = &app.state().state.doc;
    assert_eq!(doc.model().live_bodies().len(), 1, "{name}");
    assert_eq!(doc.features.len(), 1, "{name}");
    assert_eq!(
        doc.kernel().topology(doc.display_body().unwrap()).unwrap().faces.len(),
        faces,
        "{name}: an exported face was lost"
    );
    assert!(doc.triangle_count() > 0, "{name}: the imported body is not displayed");
    let body = doc.display_body().unwrap();
    let tier = report::rows(doc, &[])[0].tier;
    match doc.kernel().body_accuracy(body).unwrap() {
        BodyAccuracy::Exact => assert!(matches!(tier, Tier::Exact | Tier::Repaired | Tier::FileRepaired)),
        BodyAccuracy::Tolerant => {
            assert_eq!(tier, Tier::Tolerant);
            assert!(doc.body_tolerance(0).is_none(), "local fits are not a whole-body bound");
            assert_eq!(
                doc.model().proof_report().features[0].answer_kind,
                soliddraft_exchange::proof::AnswerKind::Approximate
            );
        }
        accuracy => panic!("{name}: imported accuracy is not labelled correctly: {accuracy:?}"),
    }
    let FeatureKind::ImportStep(import) = &doc.features[0].kind else { panic!("import feature") };
    assert_eq!(import.source, std::fs::read(&path).unwrap(), "{name}: source bytes changed");
    let measured = (doc.kernel().mass_properties(body).map(|m| m.volume), doc.kernel().topology(body).unwrap().bbox);
    case.frame(&mut app, "imported");
    measured
}

/// How far a STEP round trip may move each corner of the box, as the STEP import rollout's
/// re-export check allows the volume to move.
const ROUND_TRIP_TOLERANCE: f64 = 1e-6;

/// Writes the rows where the triage reads them, then fails on any row that did not pass.
fn report(case: &str, rows: &[Row], must_pass: impl Fn(&Row) -> bool) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("fillet-acceptance");
    std::fs::create_dir_all(dir.join("recipes")).unwrap();
    let table: Vec<Value> = rows.iter().map(Row::json).collect();
    std::fs::write(dir.join(format!("{case}.json")), serde_json::to_vec_pretty(&table).unwrap()).unwrap();
    // One recipe per refused fillet, outside the table: the triage reads only the rows.
    // A case of several fillets names each recipe after its first edge.
    for row in rows {
        let Some(recipe) = &row.recipe else { continue };
        let name = if rows.len() == 1 {
            case.to_owned()
        } else {
            let edge: String = row.edges[0].chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            format!("{case}-{edge}")
        };
        std::fs::write(dir.join("recipes").join(format!("{name}.json")), serde_json::to_vec_pretty(recipe).unwrap())
            .unwrap();
    }
    for row in rows {
        eprintln!(
            "{} R{} {} [{}] {} {:.1}s {} {}",
            row.case,
            row.radius,
            row.result,
            row.stage,
            row.accuracy,
            row.seconds,
            row.step,
            if row.result == "pass" { String::new() } else { format!("{} | {}", row.message, row.kernel) }
        );
    }
    let failed: Vec<String> = rows
        .iter()
        .filter(|row| row.result != "pass" && must_pass(row))
        .map(|row| format!("{} R{} {:?}: {} ({})", row.case, row.radius, row.edges, row.stage, row.kernel))
        .collect();
    assert!(failed.is_empty(), "{} of {} fillets did not build:\n{}", failed.len(), rows.len(), failed.join("\n"));
}

/// The owner's part, and the names of the pocket rim's edges: every edge between a
/// pocket wall and a face of the boss.
fn owner_part() -> (Part, Vec<String>) {
    let mut part = Part::new("Owner pocket");
    part.build("sketch_geometry", OWNER_BOSS);
    part.build("extrude", r#"{"sketch":"Sketch1","depth":76,"flip":false}"#);
    part.build("sketch", OWNER_POCKET);
    part.build("cut", r#"{"sketch":"Sketch2","through_all":false,"depth":35,"flip":false}"#);
    let topology = part.topology();
    assert_eq!(topology.faces.len(), OWNER_FACES, "the pocket did not open the flat top");
    let walls: BTreeSet<FaceId> = topology
        .faces
        .iter()
        .filter(|f| part.feature_of(f.id) == "Cut-Extrude1" && f.bbox.max.z - f.bbox.min.z > 1.0)
        .map(|f| f.id)
        .collect();
    assert_eq!(walls.len(), 4, "the pocket has four walls");
    let rim: Vec<String> = topology
        .edges
        .iter()
        .filter(|e| {
            e.faces.iter().filter(|f| walls.contains(f)).count() == 1
                && e.faces.iter().any(|f| part.feature_of(*f) == "Boss-Extrude1")
        })
        .map(|e| part.edge_name(e.id))
        .collect();
    assert_eq!(rim.len(), 6, "the pocket rim: three edges on the flat top and three on the rounded side");
    (part, rim)
}

/// The whole rim through a session, as the owner would keep working on it: built at R2,
/// saved, reopened by a new kernel, edited to the largest radius that fits, and an edit
/// past that refused with the last good solid kept and still exported.
#[test]
fn owner_pocket_rim_saves_reopens_edits_and_keeps_its_solid_when_an_edit_is_refused() {
    use soliddraft_app::document::{APPROXIMATE_BODY_NAME, FeatureStatus};
    /// The faces the fillet made, by the names the tree and the picks show.
    fn rounds(doc: &Document, fillet: &str) -> Vec<String> {
        let id = doc.features.iter().find(|f| f.name == fillet).expect("the fillet is in the tree").id;
        doc.faces.iter().filter(|face| face.feature == id).map(|face| face.label.clone()).collect()
    }
    /// The part as it stands: its fillet is built, the body is approximate and shown.
    fn standing(part: &Part, fillet: &str, stage: &str) -> Option<f64> {
        let doc = part.doc();
        let feature = doc.features.iter().find(|f| f.name == fillet).expect("the fillet is in the tree");
        assert_eq!(feature.status, FeatureStatus::Ok, "{stage}");
        assert_eq!(doc.body_name(0), APPROXIMATE_BODY_NAME, "{stage}");
        assert!(doc.triangle_count() > 0, "{stage}: the part shows nothing");
        assert_eq!(part.topology().faces.len(), OWNER_FACES + 6, "{stage}");
        doc.mass_properties(feature.id).map(|mass| mass.volume)
    }

    let (mut part, rim) = owner_part();
    let built = part.tool("fillet", json!({"edges": rim, "radius": 2.0}));
    assert!(!built.is_error, "R2: {}", built.content);
    let fillet = part.doc().features.last().unwrap().name.clone();
    let names = rounds(part.doc(), &fillet);
    assert_eq!(names.len(), 6, "one round for each rim edge: {names:?}");
    let at_r2 = standing(&part, &fillet, "built at R2");

    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("fillet-acceptance").join("saved");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("owner-pocket-rim-r2.dwprt");
    let path = path.to_str().unwrap();
    let saved = part.tool("save", json!({ "path": path }));
    assert!(!saved.is_error, "save: {}", saved.content);

    // A new kernel builds the saved part again from its recipe.
    let mut part = Part::new("Reopened");
    let opened = part.tool("open", json!({ "path": path }));
    assert!(!opened.is_error, "open: {}", opened.content);
    assert_eq!(rounds(part.doc(), &fillet), names, "the rounds keep their names through save and reopen");
    let reopened = standing(&part, &fillet, "reopened at R2");
    if let (Some(before), Some(after)) = (at_r2, reopened) {
        assert!((before - after).abs() < before * 1e-9, "the reopened part is the saved one: {before} != {after}");
    }

    let edited = part.tool("edit_fillet", json!({ "feature": fillet, "radius": OWNER_LARGEST }));
    assert!(!edited.is_error, "edit to R{OWNER_LARGEST}: {}", edited.content);
    assert_eq!(rounds(part.doc(), &fillet), names, "the rounds keep their names through the edit");
    let at_largest = standing(&part, &fillet, "edited to the largest radius");
    if let (Some(smaller), Some(larger)) = (reopened, at_largest) {
        assert!(larger < smaller, "a larger rim round removes more: {larger} !< {smaller}");
    }

    // R5 is past the wall: the edit is refused in plain words and the R3.99 part stays.
    let kept = part.doc().model().part_json();
    let refused = part.tool("edit_fillet", json!({ "feature": fillet, "radius": 5.0 }));
    assert!(refused.is_error, "R5 must not build: {}", refused.content);
    let why = soliddraft_app::geometry_message::for_user(&refused.content);
    assert!(why.contains("Try a radius under"), "{why}");
    assert!(!["fillet.", "features.", "numbers."].iter().any(|code| why.contains(code)), "{why}");
    assert_eq!(part.doc().model().part_json(), kept, "a refused edit changed the part");
    assert_eq!(rounds(part.doc(), &fillet), names);
    let after_refusal = standing(&part, &fillet, "after the refused R5");
    if let (Some(before), Some(after)) = (at_largest, after_refusal) {
        assert!((before - after).abs() < before * 1e-9, "the refused edit kept the solid: {before} != {after}");
    }
    let step = part.export("owner-pocket-rim-session");
    assert!(step.starts_with("exported") && step.contains("19 faces"), "{step}");
    eprintln!(
        "owner rim session: volume at R2 {at_r2:?}, reopened {reopened:?}, at R{OWNER_LARGEST} {at_largest:?}, after the refused R5 {after_refusal:?}; {step}"
    );
}

fn owner_rim(radius: f64) {
    let (mut part, rim) = owner_part();
    let rows = vec![part.fillet_and("owner-pocket-rim", rim, radius, true)];
    report(&format!("owner-pocket-rim-r{radius}"), &rows, |_| true);
}

#[test]
fn owner_pocket_rim_takes_r0_5() {
    owner_rim(0.5);
}

#[test]
fn owner_pocket_rim_takes_r1() {
    owner_rim(1.0);
}

/// Since Catallax's check-once change (its interface note of 21 Nov 2026), reopening an
/// exported file trusts the tolerance the file records, so the R2 round trip that used to
/// exceed the work limit now opens within it, with every face and the same box.
#[test]
fn owner_pocket_rim_takes_r2() {
    let (mut part, rim) = owner_part();
    let rows = vec![part.fillet_and("owner-pocket-rim", rim, 2.0, true)];
    report("owner-pocket-rim-r2", &rows, |_| true);
    let exported_box = part.topology().bbox;
    let (volume, bbox) = import_exported_step("owner-pocket-rim-r2", OWNER_FACES + 6);
    // The reimported rim is a tolerant body whose approximate display is not closed, so its
    // mass is refused by name rather than estimated. Firstmate tracks that open display as a
    // follow-on; until it closes, the box stands in for the volume.
    assert!(
        matches!(&volume, Err(KernelError::Unsupported(why)) if why == "The approximate display of this body is not closed, so its mass is not estimated."),
        "the reimported rim's mass: {volume:?}"
    );
    for (got, want) in [(bbox.min, exported_box.min), (bbox.max, exported_box.max)] {
        assert!(got.distance(want) < ROUND_TRIP_TOLERANCE, "the reimported rim keeps its box: {want:?} -> {got:?}");
    }
}

/// Catallax's one model (3.0B) drills a through hole in a body with fitted faces. The
/// owner's pocket with its rim rounded at R2 by the rolling ball takes a 6 mm hole along
/// its length, through the two planar end caps and clear of the pocket and the rounds.
/// The part stays approximate, the rounds keep their names, the hole names its walls, and
/// the part saves, reopens and exports to STEP. A blind hole there has no construction
/// and is refused, leaving the part as it was.
#[test]
fn owner_pocket_rim_at_r2_takes_a_through_hole_along_its_length() {
    use soliddraft_app::document::{APPROXIMATE_BODY_NAME, FeatureStatus};
    fn made_by(doc: &Document, feature: &str) -> Vec<String> {
        let id = doc.features.iter().find(|f| f.name == feature).expect("the feature is in the tree").id;
        doc.faces.iter().filter(|face| face.feature == id).map(|face| face.label.clone()).collect()
    }
    /// Every feature built, the body is approximate and shown, with `faces` faces.
    fn standing(part: &Part, faces: usize, stage: &str) {
        let doc = part.doc();
        assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok), "{stage}: {:?}", doc.features);
        assert_eq!(doc.body_name(0), APPROXIMATE_BODY_NAME, "{stage}");
        assert!(doc.triangle_count() > 0, "{stage}: the part shows nothing");
        assert_eq!(part.topology().faces.len(), faces, "{stage}");
    }

    fn volume(part: &Part, feature: &str) -> f64 {
        let doc = part.doc();
        let id = doc.features.iter().find(|f| f.name == feature).expect("the feature is in the tree").id;
        doc.mass_properties(id).unwrap_or_else(|| panic!("{feature}: volume measurement unavailable")).volume
    }

    fn check_bore(before: f64, after: f64, diameter: f64) {
        // The faceted volume must agree with a straight bore through the 76 mm boss.
        let expected = std::f64::consts::PI * (diameter / 2.0).powi(2) * 76.0;
        assert!(
            ((before - after) - expected).abs() < expected * 0.02,
            "the hole removed {} of {expected}",
            before - after
        );
    }

    let (mut part, rim) = owner_part();
    let built = part.tool("fillet", json!({"edges": rim, "radius": 2.0}));
    assert!(!built.is_error, "R2: {}", built.content);
    let fillet = part.doc().features.last().unwrap().name.clone();
    let rounds = made_by(part.doc(), &fillet);
    assert_eq!(rounds.len(), 6, "one round for each rim edge: {rounds:?}");
    standing(&part, OWNER_FACES + 6, "rounded at R2");
    let rounded = volume(&part, &fillet);

    // An end cap of the boss: one of the two planar faces square to its 76 mm extrusion.
    let topology = part.topology();
    let edge_names: Vec<_> = topology.edges.iter().map(|edge| part.edge_name(edge.id)).collect();
    let caps: Vec<_> = topology
        .faces
        .iter()
        .filter(|f| part.feature_of(f.id) == "Boss-Extrude1" && f.normal.is_some_and(|n| n.y.abs() > 1.0 - 1e-9))
        .collect();
    assert_eq!(caps.len(), 2, "the boss has two planar end caps: {:?}", topology.faces);
    let at = caps[0].centroid.y;
    let named = part.doc().named_face(caps[0].id).expect("every face has a name");
    let cap = format!("{} · {}", part.doc().feature(named.feature).unwrap().name, named.label);
    // 25 mm left of the middle and 15 mm up: clear of the pocket and of both rounded sides.
    let drill = |depth: Option<f64>| {
        let mut hole = json!({"face": cap, "points": [[-25.0, at, 15.0]], "diameter": 6.0});
        if let Some(depth) = depth {
            hole["depth"] = json!(depth);
        }
        hole
    };

    // A blind hole has a floor, which the construction for tolerant bodies does not make.
    let kept = part.doc().model().part_json();
    let blind = part.tool("hole", drill(Some(10.0)));
    assert!(blind.is_error, "a blind hole must not build in the rounded part: {}", blind.content);
    assert_eq!(part.doc().model().part_json(), kept, "a refused hole changed the part");
    standing(&part, OWNER_FACES + 6, "after the refused blind hole");

    let drilled = part.tool("hole", drill(None));
    assert!(!drilled.is_error, "through hole: {}", drilled.content);
    let hole = part.doc().features.last().unwrap().name.clone();
    assert_eq!(made_by(part.doc(), &hole).len(), 4, "the hole's wall is four quarter cylinders");
    assert_eq!(made_by(part.doc(), &fillet), rounds, "the rounds keep their names through the hole");
    standing(&part, OWNER_FACES + 6 + 4, "drilled");
    let drilled_topology = part.topology();
    let doc = part.doc().model();
    let hole_history = doc.kernel().history(doc.body().unwrap()).unwrap().pop().unwrap();
    assert_eq!(hole_history.op, soliddraft_kernel_api::OpKind::Hole);
    assert!(
        topology
            .edges
            .iter()
            .all(|edge| drilled_topology.edge(edge.id).is_some() && !hole_history.edges.mentions(edge.id)),
        "the drill is clear of every existing rim and edge: {:?}",
        hole_history.edges
    );
    assert_eq!(
        topology.edges.iter().map(|edge| part.edge_name(edge.id)).collect::<Vec<_>>(),
        edge_names,
        "the viewport still names the untouched edges after their original feature"
    );
    assert!(
        topology.vertices.iter().all(|old| drilled_topology.vertices.iter().any(|new| new.id == old.id)
            && !hole_history.vertices.mentions(old.id)),
        "the drill is clear of every existing vertex: {:?}",
        hole_history.vertices
    );
    assert_eq!(hole_history.faces.created.len(), 4, "only the hole wall is new");
    let modified: BTreeSet<_> = hole_history.faces.modified.into_iter().collect();
    assert_eq!(modified, caps.iter().map(|cap| cap.id).collect(), "only the end caps get new trims");
    assert!(hole_history.faces.deleted.is_empty() && hole_history.faces.split.is_empty());
    let at_drilled = volume(&part, &hole);
    check_bore(rounded, at_drilled, 6.0);

    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("fillet-acceptance").join("saved");
    std::fs::create_dir_all(&dir).unwrap();
    let save_and_reopen = |part: &mut Part, name: &str| {
        let path = dir.join(name);
        let path = path.to_str().unwrap();
        let saved = part.tool("save", json!({ "path": path }));
        assert!(!saved.is_error, "save {name}: {}", saved.content);
        let mut reopened = Part::new("Reopened");
        let opened = reopened.tool("open", json!({ "path": path }));
        assert!(!opened.is_error, "open {name}: {}", opened.content);
        reopened
    };
    let mut part = save_and_reopen(&mut part, "owner-pocket-rim-r2-drilled.dwprt");
    standing(&part, OWNER_FACES + 6 + 4, "reopened");
    assert_eq!(made_by(part.doc(), &fillet), rounds, "the rounds keep their names through save and reopen");
    assert_eq!(made_by(part.doc(), &hole).len(), 4);
    let fillet_prefix = format!("{fillet} · ");
    assert_eq!(
        part.topology()
            .edges
            .iter()
            .map(|edge| part.edge_name(edge.id))
            .filter(|name| name.starts_with(&fillet_prefix))
            .collect::<Vec<_>>(),
        edge_names.iter().filter(|name| name.starts_with(&fillet_prefix)).cloned().collect::<Vec<_>>(),
        "the reopened viewport still assigns the rim edges to the fillet"
    );
    let reopened = volume(&part, &hole);
    assert!(
        (at_drilled - reopened).abs() < at_drilled * 1e-9,
        "the reopened part is the saved one: {at_drilled} != {reopened}"
    );

    let step = part.export("owner-pocket-rim-r2-drilled");
    assert!(step.starts_with("exported") && step.contains("23 faces"), "{step}");
    eprintln!("owner rim drilled: volume {rounded} to {at_drilled}, reopened {reopened}; {step}");

    let edited = part.tool("edit_hole", json!({"feature": hole, "diameter": 8.0}));
    assert!(!edited.is_error, "edit the reopened hole: {}", edited.content);
    standing(&part, OWNER_FACES + 6 + 4, "edited");
    let at_edited = volume(&part, &hole);
    check_bore(rounded, at_edited, 8.0);

    let mut part = save_and_reopen(&mut part, "owner-pocket-rim-r2-edited.dwprt");
    standing(&part, OWNER_FACES + 6 + 4, "reopened after edit");
    let reopened = volume(&part, &hole);
    assert!((at_edited - reopened).abs() < at_edited * 1e-9, "the edited bore changed on reopen");

    let later = part.tool("hole", json!({"face":cap, "points":[[25.0, at, 15.0]], "diameter":6.0}));
    assert!(!later.is_error, "second through hole: {}", later.content);
    standing(&part, OWNER_FACES + 6 + 8, "drilled again");
    let second = &part.doc().features.last().unwrap().name;
    assert_eq!(made_by(part.doc(), second).len(), 4, "the second hole has its own wall");
    check_bore(reopened, volume(&part, second), 6.0);
}

#[test]
fn owner_pocket_rim_takes_the_largest_radius_that_fits() {
    owner_rim(OWNER_LARGEST);
}

/// Past [`OWNER_CONTACT`] the whole rim cannot be rounded, in any CAD system. The user
/// reads, in plain words, the faces the fillet runs into or cannot fit between, and the
/// largest radius that fits.
#[test]
fn owner_pocket_rim_at_r5_names_the_face_it_reaches_and_the_radius_that_fits() {
    let (mut part, rim) = owner_part();
    let mut row = part.fillet("owner-pocket-rim", rim, 5.0);
    let message = row.message.clone();
    let limit =
        message.split("under ").nth(1).and_then(|rest| rest.split(" mm").next()).and_then(|n| n.parse::<f64>().ok());
    let plain = !["fillet.", "features.", "intersect.", "numbers.", "shapes.", "Catallax"]
        .iter()
        .any(|code| message.contains(code));
    let names_face = message.contains("Boss-Extrude1 ·");
    if row.result == "refused" && !(plain && names_face && limit.is_some_and(|l| (3.9..=OWNER_CONTACT).contains(&l))) {
        row.result = "unclear";
        row.stage = format!("{}; message", row.stage);
    } else if row.result == "refused" {
        row.result = "pass";
    } else {
        row.result = "built";
        row.stage = "a rim fillet past the contact radius built".into();
    }
    report("owner-pocket-rim-r5", &[row], |_| true);
}

/// Without the edge along the thin wall, the rest of the rim takes R5: its arcs end on
/// that wall. OpenCASCADE builds these five edges up to R8.
#[test]
fn owner_pocket_rim_without_the_thin_wall_edge_takes_r5() {
    let (mut part, rim) = owner_part();
    let topology = part.topology();
    let thin_wall_edge = topology
        .edges
        .iter()
        .find(|e| (e.bbox.min.x - 32.0).abs() < 0.01 && (e.bbox.max.x - 32.0).abs() < 0.01 && e.bbox.min.z > 50.0)
        .map(|e| part.edge_name(e.id))
        .expect("the rim edge along the thin wall");
    let edges: Vec<String> = rim.into_iter().filter(|e| *e != thin_wall_edge).collect();
    assert_eq!(edges.len(), 5);
    let rows = vec![part.fillet("owner-pocket-rim-five", edges, 5.0)];
    report("owner-pocket-rim-five-r5", &rows, |_| true);
}

/// At R8 the two arcs of that five-edge rim no longer end on the thin wall: the ball's
/// contact with the pocket wall would leave it. The refusal comes at once, in plain words,
/// with the radius that fits.
#[test]
fn owner_pocket_rim_without_the_thin_wall_edge_refuses_r8_at_once() {
    let (mut part, rim) = owner_part();
    let topology = part.topology();
    let thin_wall_edge = topology
        .edges
        .iter()
        .find(|e| (e.bbox.min.x - 32.0).abs() < 0.01 && (e.bbox.max.x - 32.0).abs() < 0.01 && e.bbox.min.z > 50.0)
        .map(|e| part.edge_name(e.id))
        .expect("the rim edge along the thin wall");
    let edges: Vec<String> = rim.into_iter().filter(|e| *e != thin_wall_edge).collect();
    let mut row = part.fillet("owner-pocket-rim-five", edges, 8.0);
    let message = row.message.clone();
    let limit =
        message.split("under ").nth(1).and_then(|rest| rest.split(" mm").next()).and_then(|n| n.parse::<f64>().ok());
    let plain = !["fillet.", "features.", "intersect.", "numbers.", "shapes.", "Catallax"]
        .iter()
        .any(|code| message.contains(code));
    if row.result == "refused" && plain && limit.is_some_and(|l| (7.9..=8.0).contains(&l)) && row.seconds < 5.0 {
        row.result = "pass";
    } else if row.result == "refused" {
        row.result = "unclear";
        row.stage = format!("{}; message or time", row.stage);
    } else {
        row.result = "built";
        row.stage = "a rim fillet past its end contact built".into();
    }
    report("owner-pocket-rim-five-r8", &[row], |_| true);
}

/// A 60 x 40 x 20 box with all twelve edges rounded. Its 20 mm height takes at most R10.
fn box_all_edges(radius: f64) {
    let mut part = Part::new("Box");
    part.build("sketch", r#"{"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":60,"height":40}]}"#);
    part.build("extrude", r#"{"sketch":"Sketch1","depth":20,"flip":false}"#);
    let edges: Vec<String> = part.topology().edges.iter().map(|e| part.edge_name(e.id)).collect();
    assert_eq!(edges.len(), 12);
    let rows = vec![part.fillet_and("box-all-edges", edges, radius, true)];
    report(&format!("box-all-edges-r{radius}"), &rows, |_| true);
    let _ = import_exported_step(&format!("box-all-edges-r{radius}"), rows[0].faces);
}

#[test]
fn box_all_edges_take_r1() {
    box_all_edges(1.0);
}

#[test]
fn box_all_edges_take_r5() {
    box_all_edges(5.0);
}

#[test]
fn box_all_edges_take_r9_9() {
    box_all_edges(9.9);
}

/// Each of the sample part's 48 edges filleted alone at R1, in four runs of twelve so no
/// run passes CI's ten-minute limit.
fn demo_edges(quarter: usize) {
    let mut part = Part::new("Part1");
    soliddraft_app::startup::populate_demo(&mut part.state.doc);
    let edges: Vec<EdgeId> = part.topology().edges.iter().map(|e| e.id).collect();
    assert_eq!(edges.len(), 48, "the sample part has 48 edges");
    let rows: Vec<Row> = edges[quarter * 12..(quarter + 1) * 12]
        .iter()
        .map(|&edge| {
            let name = part.edge_name(edge);
            part.fillet("demo-edge", vec![name], 1.0)
        })
        .collect();
    report(&format!("demo-edges-{}", quarter + 1), &rows, |_| true);
}

#[test]
fn demo_part_edges_1_to_12_fillet_alone_at_r1() {
    demo_edges(0);
}

#[test]
fn demo_part_edges_13_to_24_fillet_alone_at_r1() {
    demo_edges(1);
}

#[test]
fn demo_part_edges_25_to_36_fillet_alone_at_r1() {
    demo_edges(2);
}

#[test]
fn demo_part_edges_37_to_48_fillet_alone_at_r1() {
    demo_edges(3);
}

#[test]
fn refusal_stage_names_both_constructions() {
    let kernel = "fillet.unsupported-overflow: step op_4: exact path refused fillet.unsupported-overflow: a non-end \
                  neighbour intersects the blend carrier; numerical continuation refused \
                  features.numerical-fillet-network: the retained spline assembly needs one edge";
    assert_eq!(stage(kernel), "exact fillet.unsupported-overflow; numerical features.numerical-fillet-network");
    assert_eq!(stage("fillet.radius-too-large: no ball rolls"), "fillet.radius-too-large");
}
