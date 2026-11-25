//! Replacing an imported STEP file through Catallax's verified file transaction. Each face
//! of the new file with a verified partner keeps that face's name, so references to it
//! hold in the part and in its drawings, as one undo step, and only while the part is as
//! it was when the replacement was matched. A reference to a face without a partner is
//! named in the review and left for repair; nothing else is lost. A two-solid file whose
//! solids changed cannot be matched and changes nothing.

use serde_json::{Value, json};
use soliddraft_app::{
    command_tools::{CommandTools, ProposalBook, Surface},
    document::{Command, Document, FeatureKind, FeatureStatus},
    import::{Preview, Review, commit},
};
use soliddraft_assistant::{Caller, ToolHost};
use soliddraft_model::{FaceRef, Generator, sha256_hex};

const STEP: &[u8] = include_bytes!("../../../ci/fixtures/import/two-blocks-5um.step");

/// The same file with every entity number moved, as another exporter might write it.
fn renumbered(source: &[u8]) -> Vec<u8> {
    let text = String::from_utf8(source.to_vec()).unwrap();
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '#' {
            let mut digits = String::new();
            while chars.peek().is_some_and(char::is_ascii_digit) {
                digits.push(chars.next().unwrap());
            }
            out.push_str(&(digits.parse::<u64>().unwrap() + 1_000_000).to_string());
        }
    }
    out.into_bytes()
}

/// The same file with every point moved 1 mm along x.
fn moved(source: &[u8]) -> Vec<u8> {
    moved_by(source, 1.0)
}

/// The same file with every point moved `dx` mm along x.
fn moved_by(source: &[u8], dx: f64) -> Vec<u8> {
    let text = String::from_utf8(source.to_vec()).unwrap();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("CARTESIAN_POINT('',(") {
        let start = at + "CARTESIAN_POINT('',(".len();
        let end = start + rest[start..].find(',').unwrap();
        let x: f64 = rest[start..end].parse().unwrap();
        out.push_str(&rest[..start]);
        out.push_str(&format!("{:?}", x + dx));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.into_bytes()
}

fn call(doc: &mut Document, name: &str, input: Value) -> Result<Value, String> {
    let output = CommandTools { doc, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }.call(
        name,
        input,
        &Caller::Mcp { client: "file replacement".into() },
    );
    if output.is_error { Err(output.content) } else { Ok(serde_json::from_str(&output.content).unwrap()) }
}

/// Each import's file hash and selected solid, in tree order.
fn sources(doc: &Document) -> Vec<(String, Option<String>)> {
    doc.features
        .iter()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::ImportStep(import) => Some((import.sha256.clone(), import.source_solid.clone())),
            _ => None,
        })
        .collect()
}

/// The faces the fillet's edges are named by.
fn fillet_faces(doc: &Document) -> Vec<FaceRef> {
    let fillet = doc.model().features().last().unwrap();
    let soliddraft_model::FeatureKind::Fillet(blend) = &fillet.kind else { panic!("the last feature is the fillet") };
    blend.edges.iter().flat_map(|edge| edge.faces.clone()).collect()
}

fn volumes(doc: &Document) -> Vec<f64> {
    doc.model().live_bodies().iter().map(|(_, body)| doc.kernel().mass_properties(*body).unwrap().volume).collect()
}

fn all_built(doc: &Document) {
    for feature in &doc.features {
        assert_eq!(feature.status, FeatureStatus::Ok, "{}", feature.name);
    }
}

/// The two-solid import with a fillet on one edge of its second solid.
fn imported() -> Document {
    let mut doc = Document::new("Two blocks", soliddraft_app::startup::kernel());
    let preview = Preview::from_bytes(&doc, "blocks.step", STEP.to_vec()).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    assert_eq!(sources(&doc).len(), 2);
    let second = doc.features[1].name.clone();
    let body = doc.model().live_bodies()[1].1;
    let edge = doc
        .kernel()
        .topology(body)
        .unwrap()
        .edges
        .iter()
        .find_map(|edge| {
            let named = doc.named_edge(edge.id)?;
            let feature = doc.feature(named.feature)?;
            (feature.name == second).then(|| format!("{} · {}", feature.name, named.label))
        })
        .expect("the second solid has a named edge");
    call(&mut doc, "fillet", json!({"edges":[edge],"radius":1})).unwrap();
    all_built(&doc);
    doc
}

#[test]
fn a_renumbered_file_replaces_both_solids_and_moves_their_references_as_one_step() {
    let mut doc = imported();
    let first = doc.features[0].id;
    let fillet = doc.features.last().unwrap().id;
    let old_sources = sources(&doc);
    let old_faces = fillet_faces(&doc);
    let old_volumes = volumes(&doc);
    let old_sha = sha256_hex(STEP);
    assert!(old_faces.iter().any(|face| face.generator == Generator::Imported { sha256: old_sha.clone().into() }));
    let new = renumbered(STEP);
    let new_sha = sha256_hex(&new);

    // A replacement matched before an edit cannot commit over that edit.
    let stale = Preview::from_bytes(&doc, "renumbered.step", new.clone()).unwrap().replacing(&doc, first).unwrap();
    doc.apply(Command::Rename { id: fillet, name: "Kept edit".into() }).unwrap();
    let depth = doc.undo_depth();
    let error = commit(&mut doc, &stale, &Review::default()).unwrap_err();
    assert!(error.contains("changed during the preview"), "{error}");
    assert_eq!(sources(&doc), old_sources);
    assert_eq!(fillet_faces(&doc), old_faces);
    assert_eq!(doc.feature(fillet).unwrap().name, "Kept edit");
    assert_eq!(doc.undo_depth(), depth);

    // Matched again, both solids and the fillet's faces move to the new file together.
    let preview = Preview::from_bytes(&doc, "renumbered.step", new.clone()).unwrap().replacing(&doc, first).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    all_built(&doc);
    assert_eq!(doc.undo_depth(), depth + 1, "the replacement is one undo step");
    let new_sources = sources(&doc);
    assert!(new_sources.iter().all(|(sha, _)| *sha == new_sha), "{new_sources:?}");
    assert!(new_sources.iter().all(|(_, solid)| solid.is_some()));
    assert_ne!(new_sources, old_sources);
    let new_faces = fillet_faces(&doc);
    assert_eq!(new_faces, old_faces, "the new file's faces keep the names the fillet uses");
    assert_eq!(doc.feature(fillet).unwrap().name, "Kept edit");
    for (old, new) in old_volumes.iter().zip(volumes(&doc)) {
        assert!((old - new).abs() < 1e-6, "{old} became {new}");
    }

    // One Undo restores the old file, its solids and the fillet's old faces; Redo moves them again.
    doc.undo().unwrap();
    all_built(&doc);
    assert_eq!(sources(&doc), old_sources);
    assert_eq!(fillet_faces(&doc), old_faces);
    doc.redo().unwrap();
    all_built(&doc);
    assert_eq!(sources(&doc), new_sources);
    assert_eq!(fillet_faces(&doc), new_faces);

    // The new version saves and reopens with its new file and references.
    let saved = doc.model().to_bytes().unwrap();
    let reopened =
        soliddraft_app::file::open_bytes("replaced.dwprt", &saved, soliddraft_app::startup::kernel(), None).unwrap();
    all_built(&reopened);
    assert_eq!(sources(&reopened), new_sources);
    assert_eq!(fillet_faces(&reopened), new_faces);
}

/// A two-solid file whose every point moved: Catallax verifies a selected solid only when
/// all its faces have partners, so neither solid can be chosen and nothing changes.
#[test]
fn a_two_solid_file_with_changed_geometry_changes_nothing() {
    let mut doc = imported();
    let first = doc.features[0].id;
    let saved = doc.model().to_bytes().unwrap();
    let (revision, depth) = (doc.revision(), doc.undo_depth());
    // Every point moved 1 mm along x: the same shapes, but no face keeps its definition.
    let changed = moved(STEP);
    assert_ne!(changed, STEP);
    let outcome = Preview::from_bytes(&doc, "changed.step", changed)
        .and_then(|preview| preview.replacing(&doc, first))
        .and_then(|preview| commit(&mut doc, &preview, &Review::default()));
    let error = outcome.unwrap_err();
    assert!(error.contains("Nothing was changed"), "{error}");
    assert_eq!(doc.model().to_bytes().unwrap(), saved);
    assert_eq!((doc.revision(), doc.undo_depth()), (revision, depth));
    all_built(&doc);
}

/// A file on disk for the MCP import tools, which read sources by path.
fn written(name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("soliddraft-file-replacement-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn an_mcp_replacement_token_is_bound_to_the_part_it_was_reviewed_against() {
    let mut doc = imported();
    let first = doc.features[0].name.clone();
    let fillet = doc.features.last().unwrap().id;
    let path = written("renumbered.step", &renumbered(STEP));
    let preview = |doc: &mut Document| {
        call(doc, "preview_import", json!({"path":path, "feature":first})).unwrap()["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let replace = |doc: &mut Document, token: &str| {
        call(doc, "replace_import", json!({"path":path, "feature":first, "preview_token":token}))
    };

    // A rename after the review: the token no longer matches the part.
    let token = preview(&mut doc);
    doc.apply(Command::Rename { id: fillet, name: "Renamed after review".into() }).unwrap();
    let before = doc.model().to_bytes().unwrap();
    let error = replace(&mut doc, &token).unwrap_err();
    assert!(error.contains("changed since review"), "{error}");
    assert_eq!(doc.model().to_bytes().unwrap(), before);

    // Another import of the same file after the review: the matched imports changed.
    let token = preview(&mut doc);
    let again = Preview::from_bytes(&doc, "blocks.step", STEP.to_vec()).unwrap();
    commit(&mut doc, &again, &Review::default()).unwrap();
    assert_eq!(sources(&doc).len(), 4);
    let before = doc.model().to_bytes().unwrap();
    let error = replace(&mut doc, &token).unwrap_err();
    assert!(error.contains("changed since review"), "{error}");
    assert_eq!(doc.model().to_bytes().unwrap(), before);

    // Reviewed again, all four imports move to the new file.
    let token = preview(&mut doc);
    replace(&mut doc, &token).unwrap();
    all_built(&doc);
    let new_sha = sha256_hex(&renumbered(STEP));
    assert!(sources(&doc).iter().all(|(sha, _)| *sha == new_sha), "{:?}", sources(&doc));
}

#[test]
fn a_suppressed_feature_keeps_its_state_through_a_replacement() {
    let mut doc = imported();
    let first = doc.features[0].id;
    let fillet = doc.features.last().unwrap().id;
    doc.apply(Command::Suppress { id: fillet, suppressed: true }).unwrap();
    assert_eq!(doc.feature(fillet).unwrap().status, FeatureStatus::Inactive);
    let faces = fillet_faces(&doc);
    let new = renumbered(STEP);
    let preview = Preview::from_bytes(&doc, "renumbered.step", new.clone()).unwrap().replacing(&doc, first).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    assert_eq!(doc.feature(fillet).unwrap().status, FeatureStatus::Inactive, "suppression is kept");
    let new_sha = sha256_hex(&new);
    assert!(sources(&doc).iter().all(|(sha, _)| *sha == new_sha));
    assert_eq!(fillet_faces(&doc), faces, "the suppressed fillet's references are kept");
    doc.apply(Command::Suppress { id: fillet, suppressed: false }).unwrap();
    all_built(&doc);
}

/// A whole-file replacement carries no repair answers yet, so imports whose file was
/// answered are refused with nothing changed. The two-block file asks no question, so
/// the test gives one import an answer in the tree it is matched from.
#[test]
fn imports_with_repair_answers_refuse_a_file_replacement_and_change_nothing() {
    let mut doc = imported();
    let first = doc.features[0].id;
    let FeatureKind::ImportStep(import) = &mut doc.features[0].kind else { panic!("the first feature is an import") };
    import.decisions.insert("repair:example:g0".into(), "decline".into());
    let saved = doc.model().to_bytes().unwrap();
    let (old_sources, old_faces, depth) = (sources(&doc), fillet_faces(&doc), doc.undo_depth());
    let error = Preview::from_bytes(&doc, "renumbered.step", renumbered(STEP))
        .and_then(|preview| preview.replacing(&doc, first))
        .unwrap_err();
    assert!(error.contains("repair answers") && error.contains("Nothing was changed"), "{error}");
    assert_eq!(doc.model().to_bytes().unwrap(), saved);
    assert_eq!(sources(&doc), old_sources);
    assert_eq!(fillet_faces(&doc), old_faces);
    assert_eq!(doc.undo_depth(), depth);
}

const SEAM: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step");

/// Two copies of the seamed block in one file, the second 100 mm along x.
fn two_seamed_blocks() -> Vec<u8> {
    let text = String::from_utf8(SEAM.to_vec()).unwrap();
    let start = text.find("DATA;\n").unwrap() + "DATA;\n".len();
    let end = text.find("#179=").unwrap();
    let solid = &text[start..end];
    let mut copy = String::new();
    let mut chars = solid.chars().peekable();
    while let Some(c) = chars.next() {
        copy.push(c);
        if c == '#' {
            let mut digits = String::new();
            while chars.peek().is_some_and(char::is_ascii_digit) {
                digits.push(chars.next().unwrap());
            }
            copy.push_str(&(digits.parse::<u64>().unwrap() + 1000).to_string());
        }
    }
    let copy = String::from_utf8(moved_by(copy.as_bytes(), 100.0)).unwrap();
    let mut out = format!("{}{}{}", &text[..end], copy, &text[end..]);
    out = out.replacen(
        "ADVANCED_BREP_SHAPE_REPRESENTATION('',(#178,",
        "ADVANCED_BREP_SHAPE_REPRESENTATION('',(#178,#1178,",
        1,
    );
    out.into_bytes()
}

/// A replacement cannot carry repair answers. The original small seams open tolerantly;
/// the replacement's wider seams require review, and declining one must change nothing.
#[test]
fn a_reviewed_new_file_with_repair_answers_refuses_and_changes_nothing() {
    let file = two_seamed_blocks();
    let mut doc = Document::new("Seamed blocks", soliddraft_app::startup::kernel());
    let preview = Preview::from_bytes(&doc, "seams.step", file.clone()).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    assert_eq!(sources(&doc).len(), 2);
    all_built(&doc);
    let first = doc.features[0].id;
    let saved = doc.model().to_bytes().unwrap();
    let (old_sources, depth, revision) = (sources(&doc), doc.undo_depth(), doc.revision());
    let reviewed = String::from_utf8(renumbered(&file)).unwrap().replace("10.004", "10.02");
    let error = Preview::from_bytes(&doc, "renumbered.step", reviewed.into_bytes())
        .unwrap()
        .replacing(&doc, first)
        .unwrap_err();
    assert!(error.contains("repair answers"), "{error}");
    assert!(!error.contains("import.invalid-decision") && !error.contains("import_0"), "{error}");
    assert_eq!(doc.model().to_bytes().unwrap(), saved);
    assert_eq!(sources(&doc), old_sources);
    assert_eq!((doc.undo_depth(), doc.revision()), (depth, revision));
    all_built(&doc);
}

/// A 40 x 30 mm block from z = 0 to `height`, centred on the origin, with an 8 mm hole
/// straight through it at `hole` when given, written as STEP by the kernel.
fn block_step(height: f64, hole: Option<[f64; 2]>) -> Vec<u8> {
    holes_step(height, &hole.map(|at| vec![(at, 8.0)]).unwrap_or_default())
}

/// The 40 x 30 mm block `height` mm tall with a hole of each diameter straight through it
/// at each point.
fn holes_step(height: f64, holes: &[([f64; 2], f64)]) -> Vec<u8> {
    let mut doc = Document::new("Source block", soliddraft_app::startup::kernel());
    let rectangle = json!({"shape":"rectangle","center":[0,0,0],"width":40,"height":30});
    call(&mut doc, "sketch", json!({"plane":"top","shapes":[rectangle]})).unwrap();
    call(&mut doc, "extrude", json!({"sketch":"Sketch1","depth":height,"flip":false})).unwrap();
    if !holes.is_empty() {
        let circles: Vec<Value> = holes
            .iter()
            .map(|([x, y], diameter)| json!({"shape":"circle","center":[x,y,height],"diameter":diameter}))
            .collect();
        call(&mut doc, "sketch", json!({"plane":"Boss-Extrude1 · End cap","shapes":circles})).unwrap();
        call(&mut doc, "cut", json!({"sketch":"Sketch2","through_all":true,"depth":1,"flip":false})).unwrap();
    }
    all_built(&doc);
    let body = doc.model().live_bodies()[0].1;
    let options = soliddraft_kernel_api::StepOptions { product_name: "block".into(), ..Default::default() };
    doc.kernel().export_step(body, &options).unwrap().text.into_bytes()
}

/// The imported face whose centroid passes `at`, as the tools name it.
fn face_named(doc: &Document, at: impl Fn(Point) -> bool) -> String {
    let face = doc.faces.iter().find(|face| at(Point::of(face.centroid))).expect("a face there");
    format!("{} · {}", doc.feature(face.feature).unwrap().name, face.label)
}

/// The imported edge whose midpoint passes `at`, as the tools name it.
fn edge_named(doc: &Document, at: impl Fn(Point) -> bool) -> String {
    let edge = doc.edges.iter().find(|edge| at(Point::of(edge.midpoint))).expect("an edge there");
    format!("{} · {}", doc.feature(edge.feature).unwrap().name, edge.label)
}

/// A display point, in millimetres.
struct Point {
    x: f64,
    y: f64,
    z: f64,
}

impl Point {
    fn of<P: Into<[f32; 3]>>(p: P) -> Self {
        let [x, y, z] = p.into();
        Point { x: x.into(), y: y.into(), z: z.into() }
    }
}

/// Display points are single precision.
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// The 20 mm block with its hole at the origin, imported, and built on:
/// - Fillet1 rounds the vertical edge between the +x and +y sides;
/// - Boss-Extrude1 stands on a circle sketched on the -x side;
/// - Boss-Extrude2 stands on a circle sketched on the bottom;
/// - Chamfer1 bevels the hole's top edge.
fn holed_block() -> Document {
    let mut doc = Document::new("Holed block", soliddraft_app::startup::kernel());
    let preview = Preview::from_bytes(&doc, "block.step", block_step(20.0, Some([0.0, 0.0]))).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    assert_eq!(sources(&doc).len(), 1);
    let corner = edge_named(&doc, |m| near(m.x, 20.0) && near(m.y, 15.0));
    call(&mut doc, "fillet", json!({"edges":[corner],"radius":2})).unwrap();
    let side = face_named(&doc, |c| near(c.x, -20.0));
    let boss = json!({"shape":"circle","center":[-20,0,10],"diameter":6});
    call(&mut doc, "sketch", json!({"plane":side,"shapes":[boss]})).unwrap();
    call(&mut doc, "extrude", json!({"sketch":"Sketch1","depth":5,"flip":false})).unwrap();
    let bottom = face_named(&doc, |c| near(c.z, 0.0));
    let foot = json!({"shape":"circle","center":[12,8,0],"diameter":4});
    call(&mut doc, "sketch", json!({"plane":bottom,"shapes":[foot]})).unwrap();
    call(&mut doc, "extrude", json!({"sketch":"Sketch2","depth":3,"flip":false})).unwrap();
    let rim = edge_named(&doc, |m| near(m.z, 20.0) && near((m.x * m.x + m.y * m.y).sqrt(), 4.0));
    call(&mut doc, "chamfer", json!({"edges":[rim],"distance":1})).unwrap();
    all_built(&doc);
    doc
}

/// Each feature's name and whether it built.
fn built(doc: &Document) -> Vec<(String, bool)> {
    doc.features.iter().map(|f| (f.name.clone(), f.status == FeatureStatus::Ok)).collect()
}

/// Replaces the part's one import with `bytes` through the review, returning what the
/// review said would need repair.
fn replace(doc: &mut Document, bytes: Vec<u8>) -> Vec<String> {
    let import = doc.features[0].id;
    let preview = Preview::from_bytes(doc, "new.step", bytes).unwrap().replacing(doc, import).unwrap();
    let lost = preview.lost().iter().map(ToString::to_string).collect();
    commit(doc, &preview, &Review::default()).unwrap();
    lost
}

/// The same file, byte for byte, and the same file renumbered both keep every reference of
/// a single-solid import: each feature builds, to the same volume, from the same names.
#[test]
fn an_identical_file_keeps_every_reference() {
    let source = block_step(20.0, Some([0.0, 0.0]));
    for (case, bytes) in [("identical", source.clone()), ("renumbered", renumbered(&source))] {
        let mut doc = holed_block();
        let refs = doc.model().stored_face_refs();
        let volume = volumes(&doc);
        let depth = doc.undo_depth();
        let lost = replace(&mut doc, bytes.clone());
        assert!(lost.is_empty(), "{case}: {lost:?}");
        all_built(&doc);
        assert_eq!(doc.model().stored_face_refs(), refs, "{case}");
        for (old, new) in volume.iter().zip(volumes(&doc)) {
            assert!((old - new).abs() < 1e-6, "{case}: {old} became {new}");
        }
        assert_eq!(sources(&doc)[0].0, sha256_hex(&bytes), "{case}");
        assert_eq!(doc.undo_depth(), depth + 1, "{case}");
    }
}

/// The new file drops the hole. The sides keep their definitions; the top and bottom lost
/// their hole loop and follow by shape. The cylinder has no partner, so the chamfer on its
/// rim is named in the review, and after Accept it needs repair while the rest builds,
/// the bottom sketch included. One Undo puts everything back; save and reopen keep the
/// new file with its names.
#[test]
fn a_deleted_face_names_the_references_it_breaks_and_keeps_the_rest() {
    let mut doc = holed_block();
    let refs = doc.model().stored_face_refs();
    let before = built(&doc);
    let depth = doc.undo_depth();

    // The MCP preview lists the same report as the dialog.
    let path = written("solid-block.step", &block_step(20.0, None));
    let reviewed = call(&mut doc, "preview_import", json!({"path":path, "feature":"Import1"})).unwrap();
    let listed: Vec<&str> =
        reviewed["needs_repair"].as_array().unwrap().iter().map(|item| item["owner"].as_str().unwrap()).collect();

    let lost = replace(&mut doc, block_step(20.0, None));
    let owners: Vec<&str> = lost.iter().map(|line| line.split(':').next().unwrap()).collect();
    assert_eq!(owners, ["Chamfer1"], "only the rim lost a face: {lost:?}");
    assert!(lost[0].contains(": Import1 · Face ") && lost[0].ends_with("has no matching face in the new file"));
    assert_eq!(listed, owners, "MCP preview_import reports the same items");

    let state = |doc: &Document, name: &str| doc.features.iter().find(|f| f.name == name).unwrap().status.clone();
    for name in ["Import1", "Fillet1", "Sketch1", "Boss-Extrude1", "Sketch2", "Boss-Extrude2"] {
        assert_eq!(state(&doc, name), FeatureStatus::Ok, "{name}");
    }
    assert!(matches!(state(&doc, "Chamfer1"), FeatureStatus::NeedsRepair(_)), "{:?}", state(&doc, "Chamfer1"));
    // The import's faces read as distinct labels, inherited and new alike.
    let import = soliddraft_model::FeatureId(doc.features[0].id as u32);
    let mut labels: Vec<String> = doc
        .model()
        .generated_faces(import)
        .iter()
        .map(|face| doc.model().name_label(import, face.generator.clone(), face.ordinal).unwrap().text)
        .collect();
    let count = labels.len();
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), count, "{labels:?}");
    assert_eq!(count, 6, "{labels:?}");
    // Nothing is rewritten or dropped: every stored reference is as it was.
    assert_eq!(doc.model().stored_face_refs(), refs);
    assert_eq!(doc.undo_depth(), depth + 1);

    doc.undo().unwrap();
    assert_eq!(built(&doc), before);
    assert_eq!(sources(&doc)[0].0, sha256_hex(&block_step(20.0, Some([0.0, 0.0]))));
    doc.redo().unwrap();
    let after = built(&doc);
    assert_ne!(after, before);

    let saved = doc.model().to_bytes().unwrap();
    let reopened =
        soliddraft_app::file::open_bytes("replaced.dwprt", &saved, soliddraft_app::startup::kernel(), None).unwrap();
    assert_eq!(built(&reopened), after);
    assert_eq!(reopened.model().stored_face_refs(), refs);
    assert_eq!(sources(&reopened)[0].0, sha256_hex(&block_step(20.0, None)));
}

/// The hole moves 5 mm along x. The top, bottom and cylinder change, and Catallax pairs
/// none of them exactly; they follow by shape, so the chamfer, the bottom sketch and
/// everything else build on the new file with their references unchanged.
#[test]
fn a_moved_face_takes_its_references_with_it() {
    let mut doc = holed_block();
    let refs = doc.model().stored_face_refs();
    let lost = replace(&mut doc, block_step(20.0, Some([5.0, 0.0])));
    assert!(lost.is_empty(), "{lost:?}");
    all_built(&doc);
    assert_eq!(doc.model().stored_face_refs(), refs);
    // The chamfer now bevels the moved rim: its outer edge rings the new hole centre.
    let body = doc.model().live_bodies()[0].1;
    let topology = doc.kernel().topology(body).unwrap();
    let rim = |e: &soliddraft_kernel_api::EdgeInfo| {
        let m = e.midpoint;
        (m.z - 20.0).abs() < 1e-6 && ((((m.x - 5.0).powi(2) + m.y * m.y).sqrt()) - 5.0).abs() < 1e-6
    };
    assert!(topology.edges.iter().any(rim), "a chamfered rim around the moved hole");
}

/// The block grows from 20 to 25 mm. The top moves, the sides grow and the hole lengthens;
/// only the bottom keeps its exact definition. The rest follow by shape, so the fillet,
/// the side boss and the chamfer build on the taller block with their references unchanged.
#[test]
fn a_resized_face_takes_its_references_with_it() {
    let mut doc = holed_block();
    let refs = doc.model().stored_face_refs();
    let lost = replace(&mut doc, block_step(25.0, Some([0.0, 0.0])));
    assert!(lost.is_empty(), "{lost:?}");
    all_built(&doc);
    assert_eq!(doc.model().stored_face_refs(), refs);
    // The fillet now rounds the taller corner, 25 mm long.
    let body = doc.model().live_bodies()[0].1;
    let topology = doc.kernel().topology(body).unwrap();
    let corner = |f: &&soliddraft_kernel_api::FaceInfo| {
        f.surface == soliddraft_kernel_api::SurfaceKind::Cylinder && f.centroid.x > 18.0 && f.centroid.y > 13.0
    };
    let fillet = topology.faces.iter().find(corner).expect("the fillet face");
    assert!((fillet.centroid.z - 12.5).abs() < 1e-6, "{:?}", fillet.centroid);
}

/// Two holes of different sizes move past each other: the 2 mm one from x = -10 to x = 1,
/// the 4 mm one from x = -2 to x = -5. By position alone the small hole is nearest the
/// big hole's new place, and the big hole is as near one place as the other. Radius keeps
/// each hole's identity, so the chamfer on the small hole's rim follows it to x = 1.
#[test]
fn two_holes_that_move_past_each_other_keep_their_identities() {
    let old = [([-10.0, 0.0], 2.0), ([-2.0, 0.0], 4.0)];
    let new = [([1.0, 0.0], 2.0), ([-5.0, 0.0], 4.0)];
    let mut doc = Document::new("Two holes", soliddraft_app::startup::kernel());
    let preview = Preview::from_bytes(&doc, "holes.step", holes_step(20.0, &old)).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    let rim = edge_named(&doc, |m| near(m.z, 20.0) && near(((m.x + 10.0).powi(2) + m.y * m.y).sqrt(), 1.0));
    call(&mut doc, "chamfer", json!({"edges":[rim],"distance":0.5})).unwrap();
    all_built(&doc);
    let refs = doc.model().stored_face_refs();

    let lost = replace(&mut doc, holes_step(20.0, &new));
    assert!(lost.is_empty(), "{lost:?}");
    all_built(&doc);
    assert_eq!(doc.model().stored_face_refs(), refs);
    let body = doc.model().live_bodies()[0].1;
    let topology = doc.kernel().topology(body).unwrap();
    let ring = |cx: f64, r: f64| {
        topology.edges.iter().any(|e| {
            let m = e.midpoint;
            (m.z - 20.0).abs() < 1e-6 && ((((m.x - cx).powi(2) + m.y * m.y).sqrt()) - r).abs() < 1e-6
        })
    };
    assert!(ring(1.0, 1.5), "the chamfer rings the small hole at x = 1");
    assert!(!ring(-5.0, 2.5), "the big hole at x = -5 has no chamfer");
}

/// A drawing's dimension between two vertical side edges of an imported block still finds
/// both edges after the file is replaced by one without the hole, and reads the same 40.
#[test]
fn a_drawing_dimension_survives_a_file_replacement() {
    use soliddraft_app::drawing::{DrawingDoc, part::PartContext};
    use soliddraft_drawing::{
        Annotation, AnnotationKind, AnnotationState, Command as DrawingCommand, DimValue, Dimension, DimensionKind,
        LinearOrientation, PartIndex, Scale, ViewId, annotate,
    };
    use soliddraft_kernel_api::{Point2, Vec3};

    let mut doc = Document::new("Drawn block", soliddraft_app::startup::kernel());
    let preview = Preview::from_bytes(&doc, "block.step", block_step(20.0, Some([0.0, 0.0]))).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    let ctx = PartContext::new(&doc).unwrap();
    let part = ctx.part();
    let template = soliddraft_drawing::Template::by_id("iso-a3").unwrap();
    let new = soliddraft_drawing::NewDrawing {
        views: soliddraft_drawing::default_views(&template),
        template,
        model: soliddraft_drawing::DocRef::new(
            soliddraft_drawing::DocumentId::new(),
            soliddraft_drawing::DocKind::Part,
            "part.dwprt",
            ctx.digest.clone(),
        ),
        meta: Default::default(),
        hidden: soliddraft_drawing::HiddenLines::Visible,
        model_items: false,
        centre_marks: false,
        requirements: false,
        scale: Some(Scale::FULL),
    };
    let made = soliddraft_drawing::make_drawing(&part, &new).unwrap();
    let mut drawing = DrawingDoc::new(made.drawing);
    for (id, projected) in &made.projections {
        drawing.remember(*id, projected);
    }
    let front = ViewId(1);
    let index = PartIndex::build(&part).unwrap();
    let anchors: Vec<_> = [Vec3::new(-20.0, -15.0, 10.0), Vec3::new(20.0, -15.0, 10.0)]
        .map(|at| annotate::anchor_near(&index, &drawing.transforms[&front], at).unwrap())
        .to_vec();
    assert!(anchors.iter().all(|anchor| anchor.entity.model.is_some()), "anchors carry the part's names");
    let annotation = Annotation {
        id: drawing.drawing.next_annotation_id(),
        view: front,
        body: AnnotationKind::Dimension(Dimension::measured(
            DimensionKind::Linear { orientation: LinearOrientation::Horizontal },
            40.0,
            0,
        )),
        anchors,
        placement: Point2::new(0.0, 30.0),
        style: Default::default(),
        state: AnnotationState::Ok,
        source: None,
    };
    let id = annotation.id;
    drawing.apply(DrawingCommand::AddDimension { annotation }, None).unwrap();

    assert!(replace(&mut doc, block_step(20.0, None)).is_empty(), "the drawing is not part of the part");
    let ctx = PartContext::new(&doc).unwrap();
    let report = soliddraft_drawing::update_from_model(&mut drawing.drawing, &ctx.part()).unwrap();
    assert!(report.dangling.is_empty(), "{:?}", report.dangling);
    let annotation = drawing.drawing.annotation(id).unwrap();
    assert_eq!(annotation.state, AnnotationState::Ok);
    let AnnotationKind::Dimension(dimension) = &annotation.body else { panic!("a dimension") };
    assert_eq!(dimension.value, DimValue::Measured { value: 40.0 });
}
