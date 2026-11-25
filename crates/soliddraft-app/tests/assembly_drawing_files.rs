//! Drawings of saved assemblies through the tools, as an agent or the CLI drives them:
//! opening one beside unsaved work, views at the revision the drawing shows, the bill of
//! materials of placements in one configuration, and the ids the configuration tools take.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::command_tools::board_bracket;
use soliddraft_app::drawing::LinkState;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use std::path::PathBuf;

struct Session {
    state: AppState,
    dir: PathBuf,
}

impl Session {
    fn new(name: &str) -> Session {
        let dir = std::env::temp_dir().join(format!("soliddraft-assembly-drawing-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Session { state: AppState::new(board_bracket()), dir }
    }

    fn try_call(&mut self, name: &str, input: Value) -> Result<Value, String> {
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state: &mut self.state }, name, input, &caller);
        if out.is_error { Err(out.content) } else { Ok(serde_json::from_str(&out.content).unwrap()) }
    }

    fn call(&mut self, name: &str, input: Value) -> Value {
        self.try_call(name, input.clone()).unwrap_or_else(|e| panic!("{name} {input}: {e}"))
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }

    /// A new part: a block `w` x 20 x 10, saved as `name` and left open.
    fn block(&mut self, name: &str, w: f64) -> String {
        self.call("assembly_show_part", json!({}));
        self.call("new_part", json!({}));
        let rectangle = json!({ "shape": "rectangle", "center": [0, 0, 0], "width": w, "height": 20 });
        self.call("sketch", json!({ "plane": "top", "shapes": [rectangle] }));
        self.call("extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
        let path = self.path(name);
        self.call("save", json!({ "path": path }));
        path
    }

    fn assembly(&self) -> &soliddraft_assembly::AssemblyDocument {
        &self.state.assembly.session.as_ref().expect("an assembly is open").doc
    }

    fn link(&mut self) -> LinkState {
        soliddraft_app::drawing::reference::refresh(&mut self.state);
        self.state.drawing.session.as_ref().expect("a drawing is open").link_state()
    }
}

/// A saved assembly of one block, `name`.dwasm, with its saved drawing `name`.dwdrw open.
fn drawn(s: &mut Session, name: &str) -> Value {
    s.block(&format!("{name}.dwprt"), 40.0);
    s.call("make_assembly", json!({}));
    s.call("assembly_save", json!({ "path": s.path(&format!("{name}.dwasm")) }));
    let drawn = s.call("assembly_make_drawing", json!({}));
    s.call("drawing_save", json!({ "path": s.path(&format!("{name}.dwdrw")) }));
    drawn
}

#[test]
fn opening_a_drawing_never_discards_unsaved_edits_to_another_assembly() {
    let mut s = Session::new("unsaved");
    drawn(&mut s, "a");
    let a = s.assembly().id();
    s.call("drawing_close", json!({}));

    // A second assembly, with an edit not saved yet.
    s.block("b.dwprt", 20.0);
    s.call("make_assembly", json!({}));
    s.call("assembly_save", json!({ "path": s.path("b.dwasm") }));
    s.call("assembly_set_value", json!({ "name": "gap", "expression": "5 mm" }));
    let b = s.assembly().id();
    assert_ne!(a, b);
    assert!(s.assembly().is_dirty());

    // Opening A's drawing does not open A over B's edit, and says why.
    let refused = s.try_call("open", json!({ "path": s.path("a.dwdrw") })).unwrap_err();
    assert!(refused.contains("unsaved"), "{refused}");
    assert_eq!(s.assembly().id(), b, "B stays open");
    assert!(s.assembly().is_dirty(), "with its edit");
    assert!(s.assembly().assembly().values.id_of("gap").is_some());
    // Rebuild, which opens the drawing's assembly first, holds back the same way.
    let refused = s.try_call("drawing_rebuild", json!({})).unwrap_err();
    assert!(refused.contains("unsaved"), "{refused}");
    assert_eq!(s.assembly().id(), b);
    assert!(s.assembly().is_dirty());

    // Once B is saved, Rebuild opens A and brings the drawing to it.
    s.call("assembly_save", json!({ "path": s.path("b.dwasm") }));
    s.call("drawing_rebuild", json!({}));
    assert_eq!(s.assembly().id(), a);
}

#[test]
fn a_drawing_of_an_assembly_that_shows_a_mesh_is_refused() {
    let mut s = Session::new("mesh");
    let block = s.block("block.dwprt", 40.0);
    let stl = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ci/fixtures/scene-import/sand3d-bracket/bracket.stl");
    s.call("import_scene", json!({ "path": stl, "out": s.path("scene.dwasm") }));
    s.call("insert_component", json!({ "paths": [block] }));
    s.call("assembly_save", json!({ "path": s.path("scene.dwasm") }));
    let refused = s.try_call("assembly_make_drawing", json!({})).unwrap_err();
    assert!(refused.contains("mesh"), "{refused}");
    assert!(s.state.drawing.session.is_none(), "no drawing leaves the mesh out");
}

#[test]
fn derived_views_wait_for_a_rebuild_when_the_assembly_moved_on() {
    let mut s = Session::new("revision");
    let made = drawn(&mut s, "rig");
    let front = made["drawing"]["sheets"][0]["views"][1]["id"].clone();
    assert_eq!(s.link(), LinkState::Current);

    // Another configuration, made active: the drawing still shows the first revision.
    s.call("show_assembly", json!({}));
    s.call("assembly_add_configuration", json!({ "name": "Wide" }));
    s.call("assembly_activate_configuration", json!({ "name": "Wide" }));
    let refused = s.try_call("drawing_projected", json!({ "view": front, "direction": "right" })).unwrap_err();
    assert!(refused.contains("Rebuild"), "{refused}");
    let at = s.call("model_summary", json!({}))["assembly"]["components"][0]["translation"].clone();
    let refused = s.try_call("drawing_detail", json!({ "view": front, "center": at, "radius": 10, "factor": 2 }));
    assert!(refused.unwrap_err().contains("Rebuild"));

    // Saved, the drawing sees a newer revision and still waits for its Rebuild.
    s.call("assembly_save", json!({ "path": s.path("rig.dwasm") }));
    assert!(matches!(s.link(), LinkState::Newer { .. }), "the saved revision is newer than the drawing's");
    assert!(s.try_call("drawing_projected", json!({ "view": front, "direction": "right" })).is_err());
    s.call("drawing_rebuild", json!({}));
    assert_eq!(s.link(), LinkState::Current);
    let placed = s.call("drawing_projected", json!({ "view": front, "direction": "right" }));
    assert!(placed["placed"].is_string(), "{placed}");
}

#[test]
fn placements_in_the_same_configuration_share_a_bom_row() {
    let mut s = Session::new("bom");
    let block = s.block("a.dwprt", 40.0);
    s.call("make_assembly", json!({}));
    let placement = json!({ "translation": [60, 0, 0], "rotation": [0, 0, 0, 1] });
    let placed = s.call("insert_component", json!({ "paths": [block], "placement": placement }));
    let second: u64 = placed["inserted"].as_str().unwrap().parse().unwrap();
    // The second placement names Default, the configuration the first shows as saved.
    s.call("assembly_component_configuration", json!({ "component": second, "configuration": "Default" }));
    s.call("assembly_save", json!({ "path": s.path("rig.dwasm") }));
    let drawn = s.call("assembly_make_drawing", json!({}));
    let bom = &drawn["drawing"]["sheets"][0]["tables"][0]["bom"];
    assert_eq!(bom.as_array().map(Vec::len), Some(1), "{bom}");
    assert_eq!(bom[0]["quantity"], 2, "{bom}");
}

#[test]
fn configuration_tools_refuse_ids_past_the_largest() {
    let mut s = Session::new("ids");
    s.block("a.dwprt", 40.0);
    s.call("make_assembly", json!({}));
    let added = s.call("assembly_explode_step", json!({ "paths": ["2"], "direction": [0, 0, 1], "distance": 30 }));
    let step = added["added"].as_u64().unwrap();
    let wrapped = step + (1u64 << 32);
    assert!(s.try_call("assembly_edit_explode_step", json!({ "step": wrapped, "distance": 40 })).is_err());
    assert!(s.try_call("assembly_delete_explode_step", json!({ "step": wrapped })).is_err());
    let explode = &s.assembly().assembly().explode;
    assert_eq!((explode.len(), explode[0].distance), (1, 30.0), "step {step} is untouched");
    let component = 2u64 + (1u64 << 32);
    let refused =
        s.try_call("assembly_component_configuration", json!({ "component": component, "configuration": "Default" }));
    assert!(refused.is_err());
    assert!(s.assembly().assembly().sources.iter().all(|x| x.document.configuration.is_none()));
}

#[test]
fn kernel_body_and_placed_bodies_adapters_match_on_catallax_view_section_and_rebuild() {
    use soliddraft_drawing::assembly::{
        AssemblyModel, BomRow, NewAssemblyDrawing, make_assembly_drawing, new_assembly_section, update_assembly_drawing,
    };
    use soliddraft_drawing::display::layout_sheet;
    use soliddraft_drawing::placed::PlacedPart;
    use soliddraft_drawing::{
        AnnotationKind, AnnotationState, Command, ContentHash, DocKind, DocRef, DocumentId, DrawingMeta, HiddenLines,
        KernelBody, NewDrawing, PART_VISIBILITY, Part, PartIndex, Rigid, Scale, Template, apply, derived, export,
        make_drawing, update_from_model,
    };
    use soliddraft_kernel_api::Vec3;
    use soliddraft_kernel_api::drawing::{StandardView, ViewSpec};
    use soliddraft_kernel_catallax::CatallaxKernel;
    use soliddraft_model::sketch::{P2, Sketch, solve::solve};
    use soliddraft_model::{
        Command as ModelCommand, DatumPlane, Document as PartDoc, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind,
        ProfileSource, SketchFeature, SketchPlane,
    };
    use std::sync::Arc;

    let build_box = |kernel: &Arc<CatallaxKernel>, w: f64, d: f64, h: f64| {
        let mut doc = PartDoc::new("Box", kernel.clone());
        let mut sketch = Sketch::new();
        sketch.rectangle(P2::new(0.0, 0.0), w, d).unwrap();
        solve(&mut sketch, None, true);
        let s = doc
            .execute(ModelCommand::AddFeature {
                kind: FeatureKind::Sketch(SketchFeature {
                    plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
                    sketch,
                }),
                name: None,
                position: None,
                via_assistant: false,
            })
            .unwrap()
            .created
            .unwrap();
        doc.execute(ModelCommand::AddFeature {
            kind: FeatureKind::Extrude(ExtrudeFeature {
                profile: ProfileSource::Sketch { feature: s },
                operation: ExtrudeOp::Boss,
                end: EndCondition::Blind { depth: h },
                reverse: false,
                options: Default::default(),
            }),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap();
        doc
    };

    let kernel = Arc::new(CatallaxKernel::new());
    let base_v1 = build_box(&kernel, 80.0, 40.0, 10.0);
    let base_v2 = build_box(&kernel, 100.0, 40.0, 10.0);
    let post = build_box(&kernel, 16.0, 16.0, 30.0);

    // 1. KernelBody adapter: proved view, section, preview refusal on export, and rebuild.
    let part_v1 = Part {
        geometry: Box::new(KernelBody::new(kernel.as_ref(), base_v1.body().unwrap())),
        digest: base_v1.semantic_hash(),
        dimensions: Vec::new(),
        namer: None,
        preview: false,
        source: Default::default(),
        requirements: Default::default(),
    };
    let mut part_drawing = make_drawing(
        &part_v1,
        &NewDrawing {
            model: DocRef::new(base_v1.id(), DocKind::Part, "base.dwprt", part_v1.digest.clone()),
            template: Template::by_id("iso-a3").unwrap(),
            meta: DrawingMeta { title: "Base".into(), ..DrawingMeta::default() },
            views: vec![StandardView::Front],
            scale: Some(Scale::FULL),
            hidden: HiddenLines::Visible,
            model_items: false,
            centre_marks: false,
            requirements: false,
        },
    )
    .unwrap()
    .drawing;
    let part_front = part_drawing.sheets[0].views[0].id;
    let index = PartIndex::build(&part_v1).unwrap();
    let line =
        derived::cutting_line(&part_drawing, &part_v1, &index, part_front, Vec3::new(0.0, 0.0, 5.0), false).unwrap();
    let (part_sec, _) =
        derived::new_section(&part_drawing, &part_v1, &index, 0, part_front, line, false, None).unwrap();
    assert_eq!(part_sec.geometry.hatch.len(), 1);
    assert!(!part_sec.geometry.unproved && part_sec.geometry.catallax);
    apply(&mut part_drawing, &Command::PlaceView { sheet: 0, view: part_sec }).unwrap();

    let preview_v2 = Part {
        geometry: Box::new(KernelBody::new(kernel.as_ref(), base_v2.body().unwrap())),
        digest: base_v2.semantic_hash(),
        dimensions: Vec::new(),
        namer: None,
        preview: true,
        source: Default::default(),
        requirements: Default::default(),
    };
    update_from_model(&mut part_drawing, &preview_v2).unwrap();
    assert!(part_drawing.sheets[0].views.iter().all(|v| v.geometry.unproved && v.geometry.coarse));
    let preview_list = layout_sheet(&part_drawing, 0);
    assert!(preview_list.unproved);
    assert!(export::bytes(&preview_list, export::Format::Svg, "Base").is_err(), "unproved preview must not export");
    let part_v2 = Part {
        geometry: Box::new(KernelBody::new(kernel.as_ref(), base_v2.body().unwrap())),
        digest: base_v2.semantic_hash(),
        dimensions: Vec::new(),
        namer: None,
        preview: false,
        source: Default::default(),
        requirements: Default::default(),
    };
    update_from_model(&mut part_drawing, &part_v2).unwrap();
    assert!(part_drawing.sheets[0].views.iter().all(|v| !v.geometry.unproved && v.geometry.catallax));
    export::bytes(&layout_sheet(&part_drawing, 0), export::Format::Svg, "Base").unwrap();

    // 2. PlacedBodies adapter backed by KernelBody via PlacedPart::from_kernel: same view,
    //    section, inter-part visibility evidence, rebuild and balloon attachment.
    let asm_id = DocumentId::new();
    let make_model = |rev: &[u8], base_doc: &PartDoc, post_x: f64| AssemblyModel {
        reference: Some(DocRef::new(asm_id, DocKind::Assembly, "rig.dwasm", ContentHash::of_bytes(rev))),
        parts: vec![
            PlacedPart::from_kernel(kernel.as_ref(), base_doc.body().unwrap(), Rigid::IDENTITY, "2", "Base<1>"),
            PlacedPart::from_kernel(
                kernel.as_ref(),
                post.body().unwrap(),
                Rigid { translation: Vec3::new(post_x, -25.0, -5.0), ..Rigid::IDENTITY },
                "3",
                "Post<1>",
            ),
        ],
        items: vec![1, 2],
        exploded: Vec::new(),
        bom: vec![
            BomRow { item: 1, name: "Base".into(), file: "base.dwprt".into(), quantity: 1 },
            BomRow { item: 2, name: "Post".into(), file: "post.dwprt".into(), quantity: 1 },
        ],
    };
    let asm_v1 = make_model(b"v1", &base_v1, 0.0);
    let bodies = asm_v1.bodies(false).unwrap();
    let front_curves =
        soliddraft_drawing::DrawingGeometry::view_curves(&bodies, &ViewSpec::standard(StandardView::Front)).unwrap();
    assert!(!front_curves.unproved && front_curves.catallax);
    assert!(front_curves.approximate.contains(&PART_VISIBILITY.to_owned()), "{:?}", front_curves.approximate);

    let mut asm_drawing = make_assembly_drawing(
        &asm_v1,
        &NewAssemblyDrawing {
            template: Template::by_id("iso-a3").unwrap(),
            meta: DrawingMeta { title: "Rig".into(), ..DrawingMeta::default() },
            views: vec![StandardView::Front],
            scale: Some(Scale::FULL),
            balloons: true,
            exploded: false,
        },
    )
    .unwrap();
    let asm_front = asm_drawing.sheets[0].views[0].id;
    let asm_sec =
        new_assembly_section(&asm_drawing, &asm_v1, 0, asm_front, Vec3::new(0.0, 0.0, 5.0), false, false).unwrap();
    assert_eq!(asm_sec.geometry.hatch.len(), 2);
    apply(&mut asm_drawing, &Command::PlaceView { sheet: 0, view: asm_sec }).unwrap();
    export::bytes(&layout_sheet(&asm_drawing, 0), export::Format::Svg, "Rig").unwrap();

    let asm_v2 = make_model(b"v2", &base_v2, 20.0);
    let updated = update_assembly_drawing(&asm_drawing, &asm_v2).unwrap();
    apply(&mut asm_drawing, &Command::UpdateFromAssembly { drawing: Box::new(updated) }).unwrap();
    assert!(asm_drawing.views_match_pin());
    let balloons: Vec<_> =
        asm_drawing.sheets[0].annotations.iter().filter(|a| matches!(a.body, AnnotationKind::Balloon { .. })).collect();
    assert_eq!(balloons.len(), 2);
    assert!(balloons.iter().all(|b| b.state == AnnotationState::Ok));
}
