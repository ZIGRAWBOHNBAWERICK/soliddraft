//! Field command availability and the terminal field workflow on imported solids.

use serde_json::{Value, json};
use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
use soliddraft_app::document::{Document, FeatureStatus};
use soliddraft_app::import::{Preview, Review, commit};
use soliddraft_app::state::AppState;
use soliddraft_assistant::{Caller, ToolHost};
use soliddraft_exchange::{Format, Report};
use soliddraft_kernel_api::ImportFormat;

const STEP: &[u8] = include_bytes!("../../../ci/fixtures/import/block-closed.step");
const MESH: &[u8] = include_bytes!("../../../ci/fixtures/import/tetrahedron.obj");

fn source(format: ImportFormat) -> Document {
    let (name, bytes) = match format {
        ImportFormat::Step => ("block.step", STEP),
        ImportFormat::Mesh => ("tetrahedron.obj", MESH),
        _ => unreachable!(),
    };
    import(format, name, bytes)
}

fn import(format: ImportFormat, name: &str, bytes: &[u8]) -> Document {
    let mut doc = Document::new("Imported field", soliddraft_app::startup::kernel());
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
fn imported_solids_offer_field_commands() {
    for format in [ImportFormat::Step, ImportFormat::Mesh] {
        let state = AppState::new(source(format));
        for (tool, _, _, _, _) in fields() {
            let command = soliddraft_app::commands::get(tool).unwrap();
            let mut palette = soliddraft_app::palette::Palette::default();
            palette.query = tool.into();
            assert!((command.shown)(&state), "{format:?} should show {tool}");
            assert!(command.is_enabled(&state), "{format:?} should enable {tool}");
            assert!(palette.results(&state).iter().any(|result| result.command.id == tool), "{format:?} {tool}");
        }
    }
}

fn displayed_field(doc: &Document, name: &str) -> soliddraft_kernel_api::BodyId {
    let feature = doc.features.last().unwrap();
    assert_eq!(feature.name, name);
    assert_eq!(feature.status, FeatureStatus::Ok);
    assert!(feature.summary().starts_with('±') && !feature.summary().contains("unproved"), "{}", feature.summary());
    assert!(doc.triangle_count() > 0);
    doc.display_body().unwrap()
}

#[test]
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
                soliddraft_app::startup::kernel(),
                None,
            )
            .unwrap();
            std::fs::remove_file(path).unwrap();
            let field = displayed_field(&doc, name);
            assert_eq!(doc.model().part_json(), edited);

            let (offset, center, expected_volume) = if format == ImportFormat::Mesh {
                (0.0, [0.0, 0.0, 0.0], 518.0 / 3.0)
            } else {
                (10.004, [2.0, 2.0, 10.004], 10012.0)
            };
            let sketch = json!({"plane":"top","offset":offset,"shapes":[
                {"shape":"rectangle","center":center,"width":2,"height":2}
            ]});
            assert!(call(&mut doc, "sketch", sketch.clone()).is_err());
            assert_eq!(doc.display_body(), Some(field));

            call(&mut doc, "rollback", json!({"before":name})).unwrap();
            call(&mut doc, "sketch", sketch).unwrap();
            call(&mut doc, "extrude", json!({"sketch":"Sketch1","depth":2,"flip":false})).unwrap();
            let boss = doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap();
            assert!((boss.volume - expected_volume).abs() < 1e-6, "{format:?}: {}", boss.volume);
            call(&mut doc, "rollback", json!({})).unwrap_or_else(|e| panic!("{format:?} {tool}: {e}"));

            assert_ne!(displayed_field(&doc, name), field);
            assert_eq!(doc.features.len(), 4);
            assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
        }
    }
}

#[test]
fn the_named_face_extrusion_keeps_catallax_field_refusal() {
    let mut doc = source(ImportFormat::Step);
    call(
        &mut doc,
        "sketch",
        json!({"plane":"Import1 · Face 6","shapes":[
            {"shape":"rectangle","center":[2.0,2.0,10.004],"width":2,"height":2}
        ]}),
    )
    .unwrap();
    call(&mut doc, "extrude", json!({"sketch":"Sketch1","depth":2,"flip":false})).unwrap();
    let body = doc.display_body().unwrap();
    let boss = doc.kernel().mass_properties(body).unwrap();
    assert!((boss.volume - 10012.0).abs() < 1e-6);
    let original = doc.model().part_json();
    let depth = doc.undo_depth();
    for (tool, _, arguments, _, _) in fields() {
        let reason = call(&mut doc, tool, arguments).unwrap_err();
        assert!(reason.contains("distance.planar-domain"), "{tool}: {reason}");
        assert_eq!(doc.model().part_json(), original);
        assert_eq!(doc.undo_depth(), depth);
        assert_eq!(doc.display_body(), Some(body));
    }
}

fn assert_stl_clipped_to_source(bytes: &[u8], lo: [f64; 3], hi: [f64; 3], accuracy: f64) {
    assert_eq!(&bytes[..12], b"CXPRINT1\x02\x01\x01\x01");
    let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    assert!(count > 0);
    assert_eq!(bytes.len(), 84 + 50 * count);

    // The fixtures' half-spaces are convex, so checking the vertices covers every triangle.
    let mut clips_at_source_plane = false;
    let (facets, _) = bytes[84..].as_chunks::<50>();
    for facet in facets {
        let (vertices, _) = facet[12..48].as_chunks::<12>();
        for vertex in vertices {
            let point: [f64; 3] = std::array::from_fn(|axis| {
                let start = 4 * axis;
                f64::from(f32::from_le_bytes(vertex[start..start + 4].try_into().unwrap()))
            });
            for (axis, coordinate) in point.into_iter().enumerate() {
                assert!(
                    (lo[axis] - accuracy..=hi[axis] + accuracy).contains(&coordinate),
                    "{point:?} escapes the imported region {lo:?} to {hi:?}"
                );
            }
            clips_at_source_plane |= point[2] <= lo[2] + accuracy;
        }
    }
    // Both fields contain material on the fixture's bottom plane.
    assert!(clips_at_source_plane, "STL misses the clip at z = {}", lo[2]);
}

#[test]
fn certified_imported_fields_clip_stl_and_preserve_print_refusals() {
    const CUBE: &[u8] = b"v -1 -1 -0.5\nv 1 -1 -0.5\nv 1 1 -0.5\nv -1 1 -0.5\n\
v -1 -1 1.5\nv 1 -1 1.5\nv 1 1 1.5\nv -1 1 1.5\n\
f 1 4 3\nf 1 3 2\nf 5 6 7\nf 5 7 8\nf 1 2 6\nf 1 6 5\n\
f 2 3 7\nf 2 7 6\nf 3 4 8\nf 3 8 7\nf 4 1 5\nf 4 5 8\n";
    let mut block = Document::new("Print fixture", soliddraft_app::startup::kernel());
    call(
        &mut block,
        "sketch",
        json!({"plane":"top","offset":-0.75,"shapes":[
            {"shape":"rectangle","center":[-0.25,0.0,-0.75],"width":2,"height":1.5}
        ]}),
    )
    .unwrap();
    call(&mut block, "extrude", json!({"sketch":"Sketch1","depth":2,"flip":false})).unwrap();
    let step = AppState::new(block).export_part(Format::Step).unwrap().bytes;
    for (format, name, source, lo, hi, tool, arguments) in [
        (
            ImportFormat::Step,
            "clip.step",
            step.as_slice(),
            [-1.25, -0.75, -0.75],
            [0.75, 0.75, 1.25],
            "lattice",
            json!({"cell":"cubic","cell_size":4,"radius":0.35,"accuracy":0.5,"min_detail":0.5}),
        ),
        (
            ImportFormat::Mesh,
            "clip.obj",
            CUBE,
            [-1.0, -1.0, -0.5],
            [1.0, 1.0, 1.5],
            "gyroid",
            json!({"kind":"network","period":8,"level":0,"accuracy":0.5,"min_detail":0.5}),
        ),
    ] {
        let mut doc = import(format, name, source);
        call(&mut doc, tool, arguments).unwrap_or_else(|e| panic!("{format:?} {tool}: {e}"));
        let state = AppState::new(doc);
        for (override_mm, ratio) in [(None, (1, 2)), (Some(0.375), (3, 8))] {
            let accuracy = override_mm.unwrap_or(0.5);
            let output = state.export_part_at(Format::Stl, override_mm);
            if format == ImportFormat::Mesh && override_mm.is_some() {
                let refusal = output.unwrap_err();
                assert!(refusal.contains("field.print-unresolved: local boundary refinement needs a grid finer than the minimum detail allows"), "{refusal}");
                continue;
            }
            let out = output.unwrap_or_else(|e| panic!("{format:?} {tool} at {accuracy} mm: {e}"));
            let Report::FieldMesh(report) = out.report else { panic!("expected a certified field report") };
            assert!(report.closed && report.non_self_intersecting && report.connected);
            assert_eq!(report.components, 1);
            assert_eq!(report.minimum_detail_mm, "1/2");
            assert!(0.0 < report.deviation_bound_mm && report.deviation_bound_mm <= accuracy);
            assert_eq!(i64::from_le_bytes(out.bytes[12..20].try_into().unwrap()), ratio.0);
            assert_eq!(u64::from_le_bytes(out.bytes[20..28].try_into().unwrap()), ratio.1);
            assert_eq!(f64::from_le_bytes(out.bytes[68..76].try_into().unwrap()), report.deviation_bound_mm);
            assert_stl_clipped_to_source(&out.bytes, lo, hi, report.deviation_bound_mm);
        }
    }

    let mut doc = import(ImportFormat::Step, "clip.step", &step);
    call(&mut doc, "gyroid", json!({"kind":"network","period":8,"level":0,"accuracy":0.5,"min_detail":0.5})).unwrap();
    displayed_field(&doc, "Gyroid1");
    let refusal = AppState::new(doc).export_part(Format::Stl).unwrap_err();
    assert!(
        refusal.contains("field.print-unresolved: boundary coverage needs a grid finer than the minimum detail allows"),
        "{refusal}"
    );
}
