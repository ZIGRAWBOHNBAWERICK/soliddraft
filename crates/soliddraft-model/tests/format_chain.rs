//! The part format as one chain after the branches that each claimed `soliddraft-part/4`
//! were joined: each kind of new data writes the newest tag it needs, every older tag
//! still opens, and a file that holds data its tag cannot carry is refused.
//!
//! Tags: `/2` plain, `/3` several bodies, `/4` development files of the extrusion-option,
//! part-option and Hole Wizard work (read only), `/5` parameter, configuration and
//! appearance data, `/6` extrusion options, `/7` Hole Wizard features.

use std::sync::Arc;

use soliddraft_kernel_api::{FaceId, Kernel, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::file::{FORMAT, HOLE_WIZARD_FORMAT, OPTIONS_FORMAT, PART_OPTIONS_FORMAT};
use soliddraft_model::hole_wizard::{HoleStandard, HoleType, HoleWizardSpec};
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, ExtrudeOptions, FeatureId, FeatureKind,
    FileError, HoleFeature, ProfileSource, SketchFeature, SketchPlane,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

fn all_ok(doc: &Document) {
    for (id, status) in doc.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
}

fn extrude(sketch: FeatureId, options: ExtrudeOptions) -> FeatureKind {
    FeatureKind::Extrude(ExtrudeFeature {
        profile: ProfileSource::Sketch { feature: sketch },
        operation: ExtrudeOp::Boss,
        end: EndCondition::Blind { depth: 10.0 },
        reverse: false,
        options,
    })
}

/// A 60 x 40 x 10 plate on the Top plane: its sketch and its extrusion.
fn plate(doc: &mut Document) -> (FeatureId, FeatureId) {
    let mut s = Sketch::new();
    s.rectangle(P2::ZERO, 60.0, 40.0).unwrap();
    solve(&mut s, None, true);
    let sketch = add(
        doc,
        FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 }, sketch: s }),
    );
    (sketch, add(doc, extrude(sketch, ExtrudeOptions::default())))
}

/// A parameter written as a spreadsheet formula, which only format 5 and later carry.
fn formula(doc: &mut Document) {
    doc.execute(Command::SetValue { name: "edge".into(), expression: "=ROUND(10.4,0)".into() }).unwrap();
}

/// The extrusion goes a second direction, which only format 6 and later carry.
fn second_direction(doc: &mut Document, sketch: FeatureId, extrusion: FeatureId) {
    let options = ExtrudeOptions { direction2: Some(EndCondition::Blind { depth: 2.0 }), ..Default::default() };
    doc.execute(Command::EditFeature { id: extrusion, kind: extrude(sketch, options) }).unwrap();
}

/// A Hole Wizard hole in the plate's top, which only format 7 carries.
fn wizard_hole(doc: &mut Document) {
    let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
    let top: FaceId = topology
        .faces
        .iter()
        .find(|f| f.normal.is_some_and(|n| n.dot(Vec3::Z) > 1.0 - 1e-6) && (f.centroid.z - 10.0).abs() < 1e-4)
        .expect("the plate's top")
        .id;
    let face = doc.face_ref(top).unwrap();
    let spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Simple);
    let resolved = spec.resolve_defaults();
    add(
        doc,
        FeatureKind::Hole(HoleFeature {
            face,
            points: vec![P2::new(10.0, 5.0)],
            diameter: resolved.diameter,
            depth: spec.legacy_depth(),
            tip: resolved.tip,
            head: resolved.head,
            thread: resolved.thread,
            sketch: None,
            wizard: Some(spec),
        }),
    );
}

/// Saves and opens `doc`: the same truth and shape come back.
fn round_trip(doc: &Document) -> Document {
    let back = Document::from_bytes(&doc.to_bytes().unwrap(), kernel()).unwrap();
    all_ok(&back);
    assert_eq!(back.part_json(), doc.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), doc.geometry_hash());
    back
}

/// A plate with every kind of new data: a formula, a second direction and a wizard hole.
fn everything() -> Document {
    let mut d = Document::new("Chain", kernel());
    let (sketch, extrusion) = plate(&mut d);
    formula(&mut d);
    second_direction(&mut d, sketch, extrusion);
    wizard_hole(&mut d);
    all_ok(&d);
    d
}

#[test]
fn each_kind_of_data_writes_the_newest_tag_it_needs_and_round_trips() {
    let mut d = Document::new("Chain", kernel());
    let (sketch, extrusion) = plate(&mut d);
    assert_eq!(d.truth()["format"], FORMAT, "a plain part stays at format 2");
    formula(&mut d);
    assert_eq!(d.truth()["format"], PART_OPTIONS_FORMAT);
    round_trip(&d);
    second_direction(&mut d, sketch, extrusion);
    assert_eq!(d.truth()["format"], OPTIONS_FORMAT, "extrusion options outrank part options");
    round_trip(&d);
    wizard_hole(&mut d);
    all_ok(&d);
    assert_eq!(d.truth()["format"], HOLE_WIZARD_FORMAT, "a Hole Wizard hole outranks both");
    assert_eq!(d.manifest().format, HOLE_WIZARD_FORMAT);
    let back = round_trip(&d);
    assert_eq!(back.to_bytes().unwrap(), d.to_bytes().unwrap(), "saving the opened part gives the same bytes");
}

#[test]
fn every_older_tag_opens_and_a_newer_one_is_refused() {
    let d = everything();
    let json = d.part_json();
    // A development file of any of the joined branches says 4; 5 and 6 are the tags in
    // between. Each opens with all it holds and is written back in the tag it needs.
    for tag in ["soliddraft-part/4", "soliddraft-part/5", "soliddraft-part/6"] {
        let older = json.replace(HOLE_WIZARD_FORMAT, tag);
        let opened = Document::from_part_json(&older, kernel()).unwrap_or_else(|e| panic!("{tag}: {e}"));
        all_ok(&opened);
        assert_eq!(opened.part_json(), json, "{tag} is written back as format 7");
        assert_eq!(opened.geometry_hash(), d.geometry_hash(), "{tag} builds the same part");
    }
    // A tag from before the options cannot carry them: the file is damaged, not read
    // as a different part.
    for tag in ["soliddraft-part/2", "soliddraft-part/3"] {
        let old = json.replace(HOLE_WIZARD_FORMAT, tag);
        assert!(matches!(Document::from_part_json(&old, kernel()), Err(FileError::Invalid(_))), "{tag}");
    }
    // A later version is refused, naming every tag this one reads.
    let later = json.replace(HOLE_WIZARD_FORMAT, "soliddraft-part/8");
    let Err(error) = Document::from_part_json(&later, kernel()) else { panic!("format 8 opened") };
    assert!(matches!(error, FileError::Format { .. }), "{error}");
    assert!(
        error.to_string().ends_with(
            "this version reads soliddraft-part/7, soliddraft-part/6, soliddraft-part/5, soliddraft-part/4, \
             soliddraft-part/3, soliddraft-part/2 and soliddraft-part/1"
        ),
        "{error}"
    );
}

#[test]
fn each_new_kind_alone_needs_format_4_or_later() {
    let mut extruded = Document::new("Options", kernel());
    let (sketch, extrusion) = plate(&mut extruded);
    second_direction(&mut extruded, sketch, extrusion);
    let mut drilled = Document::new("Wizard", kernel());
    plate(&mut drilled);
    wizard_hole(&mut drilled);
    let mut valued = Document::new("Values", kernel());
    plate(&mut valued);
    formula(&mut valued);
    for (doc, tag) in [(&extruded, OPTIONS_FORMAT), (&drilled, HOLE_WIZARD_FORMAT), (&valued, PART_OPTIONS_FORMAT)] {
        assert_eq!(doc.truth()["format"], tag);
        let json = doc.part_json();
        let dev = json.replace(tag, "soliddraft-part/4");
        assert_eq!(Document::from_part_json(&dev, kernel()).unwrap().part_json(), json, "{tag} from format 4");
        let old = json.replace(tag, "soliddraft-part/3");
        assert!(matches!(Document::from_part_json(&old, kernel()), Err(FileError::Invalid(_))), "{tag} as format 3");
    }
}
