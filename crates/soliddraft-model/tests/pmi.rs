//! A part's manufacturing information on the Catallax backend: items follow their faces
//! through edits, travel in STEP and back, survive save and reopen, and undo.

use std::sync::Arc;

use soliddraft_kernel_api::pmi::{Anchor, At, Characteristic, Decimal, Limits, PartNotes, PmiItem, PmiKind, SizeKind};
use soliddraft_kernel_api::{FaceId, StepOptions, SurfaceKind, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, SolveStatus, solve::solve};
use soliddraft_model::{
    Command, CommandError, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FaceRef, FeatureId,
    FeatureKind, ImportStepFeature, ProfileSource, SketchFeature, SketchPlane,
};

fn doc() -> Document {
    Document::new("Part1", Arc::new(CatallaxKernel::new()))
}

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

fn sketch(build: impl FnOnce(&mut Sketch)) -> FeatureKind {
    let mut s = Sketch::new();
    build(&mut s);
    solve(&mut s, None, true);
    assert_eq!(s.status, SolveStatus::FullyDefined);
    FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 }, sketch: s })
}

fn extrude(sketch: FeatureId, operation: ExtrudeOp, end: EndCondition) -> FeatureKind {
    // Every sketch is on the plate's underside, so bosses and cuts both run upwards.
    FeatureKind::Extrude(ExtrudeFeature {
        profile: ProfileSource::Sketch { feature: sketch },
        operation,
        end,
        reverse: false,
    })
}

fn bore_sketch(diameter: f64) -> FeatureKind {
    sketch(|s| {
        s.circle_at(P2::new(0.0, 0.0), diameter).unwrap();
    })
}

/// A 40 x 30 x 10 block centred on the z axis with a bore of `diameter` through its
/// middle. Returns the bore's sketch.
fn bored_block(doc: &mut Document, diameter: f64) -> FeatureId {
    let plate = add(
        doc,
        sketch(|s| {
            s.rectangle(P2::new(0.0, 0.0), 40.0, 30.0).unwrap();
        }),
    );
    add(doc, extrude(plate, ExtrudeOp::Boss, EndCondition::Blind { depth: 10.0 }));
    let bore = add(doc, bore_sketch(diameter));
    add(doc, extrude(bore, ExtrudeOp::Cut, EndCondition::ThroughAll));
    bore
}

fn faces(doc: &Document, wanted: impl Fn(&soliddraft_kernel_api::FaceInfo) -> bool) -> Vec<FaceId> {
    let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
    topology.faces.iter().filter(|face| wanted(face)).map(|face| face.id).collect()
}

fn facing(doc: &Document, normal: Vec3) -> Vec<FaceRef> {
    let found = faces(doc, |face| face.normal.is_some_and(|n| n.dot(normal) > 1.0 - 1e-9));
    assert_eq!(found.len(), 1, "one face facing {normal:?}");
    doc.pmi_feature(&found).unwrap()
}

fn cylinders(doc: &Document) -> Vec<FaceRef> {
    let found = faces(doc, |face| face.surface == SurfaceKind::Cylinder);
    assert!(!found.is_empty(), "the bore's faces");
    doc.pmi_feature(&found).unwrap()
}

fn decimal(text: &str) -> Decimal {
    Decimal::parse(text).unwrap()
}

fn put(doc: &mut Document, id: &str, kind: PmiKind<FaceRef>) {
    doc.execute(Command::PutPmiItem { item: Box::new(PmiItem { id: id.to_owned(), kind }), replaces: None }).unwrap();
}

/// Datums on the underside and one side, the bore's diameter with a fit, its position
/// and its finish, the plate's thickness, and the notes of the part.
fn annotate(doc: &mut Document) {
    let bottom = facing(doc, Vec3::new(0.0, 0.0, -1.0));
    let top = facing(doc, Vec3::Z);
    let side = facing(doc, Vec3::new(-1.0, 0.0, 0.0));
    let bore = cylinders(doc);
    put(doc, "Datum A", PmiKind::Datum { letter: "A".into(), feature: bottom.clone() });
    put(doc, "Datum B", PmiKind::Datum { letter: "B".into(), feature: side });
    put(
        doc,
        "Bore diameter",
        PmiKind::Size {
            size: SizeKind::Diameter,
            features: vec![bore.clone()],
            limits: Limits::fit("H7"),
            nominal: None,
        },
    );
    put(
        doc,
        "Bore position",
        PmiKind::Tolerance {
            characteristic: Characteristic::Position,
            value: decimal("0.05"),
            diameter_zone: true,
            material: None,
            datums: vec!["A".into(), "B".into()],
            features: vec![bore.clone()],
        },
    );
    put(
        doc,
        "Bore finish",
        PmiKind::Texture { parameter: "Ra".into(), value: decimal("1.6"), process: Default::default(), feature: bore },
    );
    put(
        doc,
        "Thickness",
        PmiKind::Location {
            from: Anchor { feature: bottom, at: At::Feature },
            to: Anchor { feature: top, at: At::Feature },
            limits: Some(Limits::Deviations { upper: decimal("0.1"), lower: decimal("-0.1") }),
            nominal: None,
        },
    );
    doc.execute(Command::SetPartNotes {
        notes: PartNotes {
            general_tolerance: Some("ISO 2768-m".into()),
            material: Some("6061-T6".into()),
            notes: vec!["Break sharp edges".into()],
        },
    })
    .unwrap();
}

fn callouts(doc: &Document) -> Vec<(String, String, Option<String>)> {
    doc.pmi_rows().into_iter().map(|row| (row.item.id.clone(), row.callout(), row.problem)).collect()
}

fn expected(bore: &str) -> Vec<(String, String, Option<String>)> {
    [
        ("Datum A", "Datum A".to_owned()),
        ("Datum B", "Datum B".to_owned()),
        ("Bore diameter", format!("\u{d8}{bore} H7")),
        ("Bore position", "Position \u{d8}0.05 A|B".to_owned()),
        ("Bore finish", "Ra 1.6".to_owned()),
        ("Thickness", "10 \u{b1}0.1".to_owned()),
    ]
    .into_iter()
    .map(|(id, callout)| (id.to_owned(), callout, None))
    .collect()
}

#[test]
fn dimensions_read_the_size_the_part_has() {
    let mut d = doc();
    bored_block(&mut d, 8.0);
    annotate(&mut d);
    assert_eq!(callouts(&d), expected("8"));
}

#[test]
fn an_edited_bore_keeps_its_fit_and_its_position_tolerance() {
    let mut d = doc();
    let bore = bored_block(&mut d, 8.0);
    annotate(&mut d);
    d.execute(Command::EditFeature { id: bore, kind: bore_sketch(10.0) }).unwrap();
    assert_eq!(callouts(&d), expected("10"));
    d.undo().unwrap();
    assert_eq!(callouts(&d), expected("8"));
}

#[test]
fn step_carries_the_items_to_another_part_and_back_onto_the_same_faces() {
    let mut d = doc();
    bored_block(&mut d, 8.0);
    annotate(&mut d);
    let on_body = d.pmi_on_body();
    assert_eq!(on_body.left_out, Vec::new());
    let step = d.kernel().export_step_with_pmi(d.body().unwrap(), &StepOptions::default(), &on_body.pmi).unwrap();

    // Another part opens the file: the body, then what the file says about it.
    let mut other = doc();
    let import =
        add(&mut other, FeatureKind::ImportStep(ImportStepFeature::new("block.step", step.text.clone().into_bytes())));
    let read = other.kernel().read_step_pmi(step.text.as_bytes()).unwrap();
    assert_eq!(read.skipped, Vec::new());
    let (items, report) = other.pmi_from_step(&[import], &read);
    assert_eq!(report.left_out, Vec::new());
    assert_eq!(report.added, ["Datum A", "Datum B", "Bore diameter", "Bore position", "Bore finish", "Thickness"]);
    for item in items {
        other.execute(Command::PutPmiItem { item: Box::new(item), replaces: None }).unwrap();
    }
    other.execute(Command::SetPartNotes { notes: read.pmi.part.clone() }).unwrap();
    assert_eq!(callouts(&other), expected("8"));
    assert_eq!(other.pmi().part, d.pmi().part);

    // Each item sits on the face of the same shape and place as in the first part.
    let place = |doc: &Document, id: &str| -> Vec<(SurfaceKind, [i64; 3])> {
        let row = doc.pmi_rows().into_iter().find(|row| row.item.id == id).unwrap();
        let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
        let mut places: Vec<_> = row
            .on_body
            .unwrap()
            .features()
            .into_iter()
            .flatten()
            .map(|face| {
                let info = topology.faces.iter().find(|f| f.id == *face).unwrap();
                let c = info.centroid;
                (info.surface, [c.x, c.y, c.z].map(|v| (v * 1e6).round() as i64))
            })
            .collect();
        places.sort_by_key(|place| place.1);
        places
    };
    for id in ["Datum A", "Datum B", "Bore diameter", "Bore position", "Bore finish", "Thickness"] {
        assert_eq!(place(&other, id), place(&d, id), "{id}");
    }

    // Saved and reopened, the imported part still has every item on its face.
    let reopened = Document::from_bytes(&other.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(reopened.pmi(), other.pmi());
    assert_eq!(callouts(&reopened), expected("8"));

    // And it writes the same information again.
    let again = reopened.pmi_on_body();
    assert_eq!(again.left_out, Vec::new());
    let second =
        reopened.kernel().export_step_with_pmi(reopened.body().unwrap(), &StepOptions::default(), &again.pmi).unwrap();
    let reread = reopened.kernel().read_step_pmi(second.text.as_bytes()).unwrap();
    assert_eq!(reread.pmi.items.len(), read.pmi.items.len());
    assert_eq!(reread.pmi.part, read.pmi.part);
}

#[test]
fn a_saved_part_reopens_with_its_items() {
    let mut d = doc();
    bored_block(&mut d, 8.0);
    annotate(&mut d);
    let before = d.semantic_hash();
    let shape = d.geometry_hash();
    let back = Document::from_bytes(&d.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(back.pmi(), d.pmi());
    assert_eq!(back.semantic_hash(), before);
    assert_eq!(callouts(&back), expected("8"));

    // Manufacturing information is part of the revision and not of the shape.
    let mut plain = doc();
    bored_block(&mut plain, 8.0);
    assert_ne!(plain.semantic_hash(), before);
    assert_eq!(plain.geometry_hash(), shape);
}

#[test]
fn an_item_whose_face_is_gone_stays_with_its_problem() {
    let mut d = doc();
    let bore = bored_block(&mut d, 8.0);
    annotate(&mut d);
    let cut = d.features().iter().find(|f| f.id.0 == bore.0 + 1).unwrap().id;
    d.execute(Command::SetSuppressed { id: cut, suppressed: true }).unwrap();
    let rows = d.pmi_rows();
    for id in ["Bore diameter", "Bore position", "Bore finish"] {
        let row = rows.iter().find(|row| row.item.id == id).unwrap();
        assert!(row.problem.is_some(), "{id} has lost its face");
    }
    assert!(rows.iter().find(|row| row.item.id == "Thickness").unwrap().applies());
    let on_body = d.pmi_on_body();
    let left: Vec<&str> = on_body.left_out.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(left, ["Bore diameter", "Bore position", "Bore finish"]);
    assert_eq!(on_body.pmi.items.len(), 3);

    // The face comes back and so do the items.
    d.execute(Command::SetSuppressed { id: cut, suppressed: false }).unwrap();
    assert_eq!(callouts(&d), expected("8"));
}

#[test]
fn commands_refuse_what_does_not_fit_and_undo_what_they_did() {
    let mut d = doc();
    bored_block(&mut d, 8.0);
    annotate(&mut d);
    let bore = cylinders(&d);
    let invalid = |result: Result<_, CommandError>| match result {
        Err(CommandError::Invalid(message)) => message,
        other => panic!("refused as invalid, got {:?}", other.map(|_: soliddraft_model::Executed| ())),
    };

    // A datum letter taken, a datum nobody defined, a name taken.
    let again = PmiItem { id: "Datum A2".into(), kind: PmiKind::Datum { letter: "A".into(), feature: bore.clone() } };
    assert_eq!(
        invalid(d.execute(Command::PutPmiItem { item: Box::new(again), replaces: None })),
        "Datum A is already defined."
    );
    let orphan = PmiItem {
        id: "Runout".into(),
        kind: PmiKind::Tolerance {
            characteristic: Characteristic::TotalRunout,
            value: decimal("0.1"),
            diameter_zone: false,
            material: None,
            datums: vec!["D".into()],
            features: vec![bore.clone()],
        },
    };
    assert_eq!(
        invalid(d.execute(Command::PutPmiItem { item: Box::new(orphan), replaces: None })),
        "Datum D is not defined."
    );
    // A datum in use stays.
    assert!(
        invalid(d.execute(Command::RemovePmiItem { id: "Datum A".into() }))
            .starts_with("Datum A is used by Bore position")
    );

    // A renamed datum takes its tolerance along, and undo puts both back.
    let renamed = PmiItem {
        id: "Datum A".into(),
        kind: PmiKind::Datum { letter: "D".into(), feature: facing(&d, Vec3::new(0.0, 0.0, -1.0)) },
    };
    d.execute(Command::PutPmiItem { item: Box::new(renamed), replaces: Some("Datum A".into()) }).unwrap();
    let position = d.pmi().item("Bore position").unwrap().callout(None);
    assert_eq!(position, "Position \u{d8}0.05 D|B");
    d.undo().unwrap();
    assert_eq!(callouts(&d), expected("8"));

    d.execute(Command::RemovePmiItem { id: "Bore finish".into() }).unwrap();
    assert!(d.pmi().item("Bore finish").is_none());
    d.undo().unwrap();
    assert_eq!(callouts(&d), expected("8"));
}
