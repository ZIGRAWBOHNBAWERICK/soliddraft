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
use soliddraft_kernel_api::{BodyAccuracy, EdgeId, FaceId};
use soliddraft_kernel_catallax::{APP_REQUEST_WORK, CatallaxKernel};

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
    accuracy: String,
    faces: usize,
    work: Option<u64>,
    seconds: f64,
}

impl Row {
    fn json(&self) -> Value {
        json!({
            "case": self.case, "radius": self.radius, "edges": self.edges, "result": self.result,
            "stage": self.stage, "kernel": self.kernel, "message": self.message,
            "accuracy": self.accuracy, "faces": self.faces, "work": self.work, "seconds": self.seconds,
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
            accuracy: String::new(),
            faces: 0,
            work,
            seconds,
        };
        if out.is_error {
            // The panel shows the kernel's refusal through the same words.
            row.message =
                soliddraft_app::geometry_message::for_user(if kernel.is_empty() { &out.content } else { &kernel });
            if fillet_calls.is_empty() {
                // Refused before the kernel was asked: an edge the app could not name.
                row.result = "name";
                row.stage = "naming".into();
                row.kernel = out.content.clone();
            } else {
                row.result = "refused";
                row.stage = stage(&kernel);
            }
            assert_eq!(self.doc().model().part_json(), before, "{case}: a refused fillet changed the part");
            return row;
        }
        let model = self.doc().model();
        let body = model.body().unwrap();
        row.accuracy = match model.kernel().body_accuracy(body).unwrap() {
            BodyAccuracy::Exact => "exact".into(),
            BodyAccuracy::Approximate { tolerance_mm } => format!("approximate within {tolerance_mm} mm"),
        };
        row.faces = self.topology().faces.len();
        let fillet = self.doc().features.last().unwrap();
        if !matches!(fillet.status, soliddraft_app::document::FeatureStatus::Ok) {
            row.result = "refused";
            row.stage = "status".into();
            row.message = format!("{:?}", fillet.status);
        } else if self.doc().triangle_count() == 0 {
            row.result = "refused";
            row.stage = "display".into();
            row.message = "the filleted part shows nothing".into();
        }
        self.state.doc.undo().unwrap();
        assert_eq!(self.doc().features.len(), features, "{case}: undo did not remove the fillet");
        assert_eq!(self.topology().faces.len(), faces, "{case}: undo did not restore the part");
        row
    }
}

/// Writes the rows where the triage reads them, then fails on any row that did not pass.
fn report(case: &str, rows: &[Row], must_pass: impl Fn(&Row) -> bool) {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("fillet-acceptance");
    std::fs::create_dir_all(&dir).unwrap();
    let table: Vec<Value> = rows.iter().map(Row::json).collect();
    std::fs::write(dir.join(format!("{case}.json")), serde_json::to_vec_pretty(&table).unwrap()).unwrap();
    for row in rows {
        eprintln!(
            "{} R{} {} [{}] {} {:.1}s {}",
            row.case,
            row.radius,
            row.result,
            row.stage,
            row.accuracy,
            row.seconds,
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

fn owner_rim(radius: f64) {
    let (mut part, rim) = owner_part();
    let rows = vec![part.fillet("owner-pocket-rim", rim, radius)];
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

#[test]
fn owner_pocket_rim_takes_r2() {
    owner_rim(2.0);
}

#[test]
fn owner_pocket_rim_takes_the_largest_radius_that_fits() {
    owner_rim(OWNER_LARGEST);
}

/// Past [`OWNER_CONTACT`] the whole rim cannot be rounded, in any CAD system. The user
/// reads which face the fillet runs into and the largest radius that fits, in plain words.
#[test]
fn owner_pocket_rim_at_r5_names_the_face_it_reaches_and_the_radius_that_fits() {
    let (mut part, rim) = owner_part();
    let mut row = part.fillet("owner-pocket-rim", rim, 5.0);
    let message = row.message.clone();
    let limit =
        message.split("under ").nth(1).and_then(|rest| rest.split(" mm").next()).and_then(|n| n.parse::<f64>().ok());
    let plain =
        !["fillet.", "features.", "intersect.", "numbers.", "Catallax"].iter().any(|code| message.contains(code));
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

/// A 60 x 40 x 20 box with all twelve edges rounded. Its 20 mm height takes at most R10.
fn box_all_edges(radius: f64) {
    let mut part = Part::new("Box");
    part.build("sketch", r#"{"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":60,"height":40}]}"#);
    part.build("extrude", r#"{"sketch":"Sketch1","depth":20,"flip":false}"#);
    let edges: Vec<String> = part.topology().edges.iter().map(|e| part.edge_name(e.id)).collect();
    assert_eq!(edges.len(), 12);
    let rows = vec![part.fillet("box-all-edges", edges, radius)];
    report(&format!("box-all-edges-r{radius}"), &rows, |_| true);
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
    soliddraft_app::populate_demo(&mut part.state.doc);
    let edges: Vec<EdgeId> = part.topology().edges.iter().map(|e| e.id).collect();
    assert_eq!(edges.len(), 48, "the sample part has 48 edges");
    let rows: Vec<Row> = edges[quarter * 12..(quarter + 1) * 12]
        .iter()
        .map(|&edge| {
            let name = part.edge_name(edge);
            part.fillet("demo-edge", vec![name], 1.0)
        })
        .collect();
    report(&format!("demo-edges-{}", quarter + 1), &rows, |_| false);
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
