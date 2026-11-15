//! Replacing a two-solid STEP import with a copy whose entity numbers alone changed,
//! through Catallax's verified file transaction: both imported bodies and a fillet on one
//! of them move to the new file together, as one undo step, and only while the part is as
//! it was when the replacement was matched. A file with changed geometry changes nothing.

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
    let text = String::from_utf8(source.to_vec()).unwrap();
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("CARTESIAN_POINT('',(") {
        let start = at + "CARTESIAN_POINT('',(".len();
        let end = start + rest[start..].find(',').unwrap();
        let x: f64 = rest[start..end].parse().unwrap();
        out.push_str(&rest[..start]);
        out.push_str(&format!("{:?}", x + 1.0));
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
    let mut doc = Document::new("Two blocks", soliddraft_app::kernel());
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
    assert!(new_faces.iter().any(|face| face.generator == Generator::Imported { sha256: new_sha.clone().into() }));
    assert!(!new_faces.iter().any(|face| face.generator == Generator::Imported { sha256: old_sha.clone().into() }));
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
    let reopened = soliddraft_app::file::open_bytes("replaced.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    all_built(&reopened);
    assert_eq!(sources(&reopened), new_sources);
    assert_eq!(fillet_faces(&reopened), new_faces);
}

#[test]
fn a_file_with_changed_geometry_changes_nothing() {
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
    let new = renumbered(STEP);
    let preview = Preview::from_bytes(&doc, "renumbered.step", new.clone()).unwrap().replacing(&doc, first).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    assert_eq!(doc.feature(fillet).unwrap().status, FeatureStatus::Inactive, "suppression is kept");
    let new_sha = sha256_hex(&new);
    assert!(
        fillet_faces(&doc).iter().any(|face| face.generator == Generator::Imported { sha256: new_sha.clone().into() })
    );
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
