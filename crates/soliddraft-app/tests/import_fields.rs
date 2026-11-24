//! Field command availability and the terminal field workflow on imported solids.

use serde_json::{Value, json};
use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
use soliddraft_app::document::{Document, FeatureStatus};
use soliddraft_app::import::{Preview, Review, commit};
use soliddraft_app::state::AppState;
use soliddraft_assistant::{Caller, ToolHost};
use soliddraft_kernel_api::ImportFormat;

const STEP: &[u8] = include_bytes!("../../../ci/fixtures/import/block-closed.step");
const MESH: &[u8] = include_bytes!("../../../ci/fixtures/import/tetrahedron.obj");

fn source(format: ImportFormat) -> Document {
    let mut doc = Document::new("Imported field", soliddraft_app::kernel());
    let (name, bytes) = match format {
        ImportFormat::Step => ("block.step", STEP),
        ImportFormat::Mesh => ("tetrahedron.obj", MESH),
        _ => unreachable!(),
    };
    let preview = Preview::from_source(&doc, name, bytes.to_vec(), format).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    doc
}

fn call(doc: &mut Document, tool: &str, input: Value) -> Result<(), String> {
    let out = CommandTools { doc, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }.call(
        tool,
        input,
        &Caller::Mcp { client: "imported fields".into() },
    );
    if out.is_error { Err(out.content) } else { Ok(()) }
}

type FieldCommands = (&'static str, &'static str, Value, &'static str, Value);

fn fields() -> [FieldCommands; 2] {
    [
        (
            "lattice",
            "Lattice1",
            json!({"cell":"cubic","cell_size":5,"radius":1,"accuracy":0.1,"min_detail":0.5}),
            "edit_lattice",
            json!({"feature":"Lattice1","radius":0.9}),
        ),
        (
            "gyroid",
            "Gyroid1",
            json!({"kind":"network","period":10,"level":0,"accuracy":0.1,"min_detail":0.5}),
            "edit_gyroid",
            json!({"feature":"Gyroid1","level":0.1}),
        ),
    ]
}

#[test]
fn pinned_kernel_refuses_fields_on_imported_solids() {
    for format in [ImportFormat::Step, ImportFormat::Mesh] {
        for (tool, _, arguments, _, _) in fields() {
            let mut state = AppState::new(source(format));
            let command = soliddraft_app::commands::get(tool).unwrap();
            let shown = (command.shown)(&state);
            let enabled = command.is_enabled(&state);
            let mut palette = soliddraft_app::palette::Palette::default();
            palette.query = tool.into();
            let searchable = palette.results(&state).iter().any(|result| result.command.id == tool);
            let original = state.doc.model().part_json();
            let depth = state.doc.undo_depth();
            let message = call(&mut state.doc, tool, arguments)
                .expect_err("the pinned terminal stage requires a recipe without imports");
            assert!(message.ends_with(&format!("A {tool} on an imported body is coming soon.")), "{message}");
            assert!(!shown && !enabled && !searchable, "{format:?} should hide {tool}");
            assert_eq!(state.doc.model().part_json(), original);
            assert_eq!(state.doc.undo_depth(), depth);
        }
    }
}

fn displayed_field(doc: &Document, name: &str) -> soliddraft_kernel_api::BodyId {
    let feature = doc.features.last().unwrap();
    assert_eq!(feature.name, name);
    assert_eq!(feature.status, FeatureStatus::Ok);
    assert!(feature.summary().contains("unproved preview"));
    assert!(doc.triangle_count() > 0);
    doc.display_body().unwrap()
}

#[test]
#[ignore = "requires a Catallax pin with a file-aware terminal stage"]
fn imported_fields_build_edit_reopen_and_rebuild_after_a_cad_feature() {
    for format in [ImportFormat::Step, ImportFormat::Mesh] {
        for (tool, name, arguments, edit, changes) in fields() {
            let mut doc = source(format);
            call(&mut doc, tool, arguments).unwrap_or_else(|e| panic!("{format:?} {tool}: {e}"));
            displayed_field(&doc, name);
            let initial = doc.model().part_json();
            call(&mut doc, edit, changes).unwrap();
            displayed_field(&doc, name);
            let edited = doc.model().part_json();
            assert_ne!(edited, initial);

            let path = std::env::temp_dir()
                .join(format!("soliddraft-imported-{format:?}-{tool}-{}.dwprt", std::process::id()));
            call(&mut doc, "save", json!({"path":path})).unwrap();
            let mut doc = soliddraft_app::file::open_bytes(
                "field.dwprt",
                &std::fs::read(&path).unwrap(),
                soliddraft_app::kernel(),
                None,
            )
            .unwrap();
            std::fs::remove_file(path).unwrap();
            let field = displayed_field(&doc, name);
            assert_eq!(doc.model().part_json(), edited);

            let (plane, center, expected_volume) = if format == ImportFormat::Mesh {
                ("top", [0.0, 0.0, 0.0], 518.0 / 3.0)
            } else {
                ("Import1 · Face 6", [2.0, 2.0, 10.004], 10012.0)
            };
            let sketch = json!({"plane":plane,"shapes":[
                {"shape":"rectangle","center":center,"width":2,"height":2}
            ]});
            assert!(call(&mut doc, "sketch", sketch.clone()).is_err());
            assert_eq!(doc.display_body(), Some(field));

            call(&mut doc, "rollback", json!({"before":name})).unwrap();
            call(&mut doc, "sketch", sketch).unwrap();
            call(&mut doc, "extrude", json!({"sketch":"Sketch1","depth":2,"flip":false})).unwrap();
            let boss = doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap();
            assert!((boss.volume - expected_volume).abs() < 1e-6, "{format:?}: {}", boss.volume);
            call(&mut doc, "rollback", json!({})).unwrap();

            assert_ne!(displayed_field(&doc, name), field);
            assert_eq!(doc.features.len(), 4);
            assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
        }
    }
}
