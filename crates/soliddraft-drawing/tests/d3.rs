//! Phase D3 without the C++ kernel, on a kernel of boxes: detail and auxiliary views,
//! crops, dimension sets, symbols, revisions, several sheets with location labels, and
//! DXF dimensions and hatches. Sections need booleans; the `soliddraft draw` exit test
//! covers them on Catallax.

mod support;

use soliddraft_drawing::display::{Item, Role, layout_sheet, location_labels};
use soliddraft_drawing::export;
use soliddraft_drawing::{
    AnnotationKind, AnnotationState, Boundary, Command, ContentHash, DatumRef, DocKind, DocRef, DocumentId, Drawing,
    DrawingMeta, GdtCharacteristic, HiddenLines, LinearOrientation, MaterialCondition, NewDrawing, Part, PartIndex,
    RevisionRow, SetKind, SurfaceProcess, Template, UndoStack, ViewId, ViewKind, annotate, apply, derived, edge_near,
    next_letter, project, update_from_model,
};
use soliddraft_kernel_api::{Point2, Vec3};
use support::{BoxKernel, box_part};

const FRONT: ViewId = ViewId(1);

fn new_drawing() -> NewDrawing {
    let template = Template::by_id("iso-a3").unwrap();
    NewDrawing {
        views: soliddraft_drawing::default_views(&template),
        template,
        model: DocRef::new(DocumentId::new(), DocKind::Part, "box.json", ContentHash::of_bytes(b"")),
        meta: DrawingMeta { title: "Box".into(), ..Default::default() },
        hidden: HiddenLines::Visible,
        model_items: false,
        centre_marks: false,
        requirements: false,
        scale: None,
    }
}

fn drawing(part: &Part) -> Drawing {
    soliddraft_drawing::make_drawing(part, &new_drawing()).unwrap().drawing
}

fn texts(d: &Drawing, sheet: usize) -> Vec<String> {
    layout_sheet(d, sheet)
        .items
        .into_iter()
        .filter_map(|i| match i {
            Item::Text(t) => Some(t.text),
            _ => None,
        })
        .collect()
}

fn run(stack: &mut UndoStack, d: &mut Drawing, part: &Part, c: Command) {
    stack.run(d, &c, Some(part)).unwrap_or_else(|e| panic!("{}: {e}", c.name()));
}

#[test]
fn rebuild_commits_successful_views_alongside_base_and_projected_refusals() {
    for projected in [false, true] {
        let mut k = BoxKernel::new();
        k.detail_strokes = true;
        let mut d = drawing(&box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0)));
        export::check_sheet(&d, &d.sheets[0]).unwrap();
        let view = d.sheets[0]
            .views
            .iter()
            .find(|v| if projected { matches!(v.kind, ViewKind::Projected { .. }) } else { v.id == FRONT })
            .unwrap();
        let id = view.id;
        let target = usize::from(projected);
        if projected {
            apply(&mut d, &Command::AddSheet { name: "Sheet2".into(), template: "iso-a3".into() }).unwrap();
            apply(&mut d, &Command::MoveViewToSheet { view: id, sheet: target, to: Point2::new(140.0, 100.0) })
                .unwrap();
        }
        let view = d.view(id).unwrap();
        let before = view.geometry.clone();
        k.refuse_direction = Some(project::spec_in(&d, target, view).unwrap().direction);
        let healthy = d.sheets[0].views.iter().find(|v| v.id != id).unwrap().id;
        let mut saved = Vec::new();
        for view in [id, healthy] {
            let note = soliddraft_drawing::Annotation {
                id: d.next_annotation_id(),
                view,
                body: AnnotationKind::Note { text: "Old manufacturing requirement".into(), leader: vec![] },
                anchors: vec![],
                placement: Point2::new(0.0, 0.0),
                style: Default::default(),
                state: AnnotationState::Ok,
                source: Some(soliddraft_drawing::PmiSource::Item { item: "removed requirement".into() }),
            };
            apply(&mut d, &Command::AddAnnotation { annotation: note.clone() }).unwrap();
            saved.push(note);
        }
        let changed = box_part(&k, "block", Vec3::new(60.0, 30.0, 10.0));
        let report = UndoStack::new().run(&mut d, &Command::UpdateFromModel, Some(&changed)).unwrap().unwrap();
        assert!(report.views_updated > 0);
        let failure = report.dangling_views.iter().find(|f| f.view == id).unwrap();
        assert!(failure.cause.contains("test direction refused"));
        assert_eq!(failure.name, if projected { "Top view" } else { "Front view" });
        assert_eq!(d.model.pinned, changed.digest);
        assert_eq!(d.view(id).unwrap().geometry, before);
        assert_eq!(d.annotation(saved[0].id), Some(&saved[0]));
        assert!(d.annotation(saved[1].id).is_none());
        assert!(!d.views_match_pin());
        for rebuilt in d.sheets[0].views.iter().filter(|v| v.id != id) {
            assert_eq!(rebuilt.geometry.source_digest, changed.digest.as_str());
        }
        // The refusal is saved with the drawing, so export names its cause before and after reopening.
        for copy in [&d, &drawing_roundtrip(&d)] {
            let error = export::check_sheet(copy, &copy.sheets[target]).unwrap_err().to_string();
            assert!(error.contains(&failure.name) && error.contains(&failure.cause), "{error}");
        }

        let mut repaired = BoxKernel::new();
        repaired.detail_strokes = true;
        let part = box_part(&repaired, "block", Vec3::new(60.0, 30.0, 10.0));
        run(&mut UndoStack::new(), &mut d, &part, Command::SetViewCrop { view: id, crop: None });
        assert!(d.annotation(saved[0].id).is_none());
        export::check_sheet(&d, &d.sheets[target]).unwrap();
    }
}

#[test]
fn a_refused_section_keeps_its_cutting_line_on_the_rebuilt_parent() {
    let mut k = BoxKernel::new();
    k.detail_strokes = true;
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let parent = derived::context(&d, &part, &index, FRONT).unwrap();
    let model_line = vec![Vec3::new(20.0, 0.0, 1.0), Vec3::new(20.0, 0.0, 9.0)];
    let line: Vec<_> = model_line.iter().map(|&p| parent.xf.point(p)).collect();
    let mut section = d.view(FRONT).unwrap().clone();
    section.id = d.next_view_id();
    section.kind = ViewKind::Section {
        parent: FRONT,
        line: line.clone(),
        model_line,
        flip: false,
        label: "A".into(),
        hatch: None,
    };
    let id = section.id;
    let saved_geometry = section.geometry.clone();
    apply(&mut d, &Command::AddSheet { name: "Section sheet".into(), template: "iso-a3".into() }).unwrap();
    d.sheets[1].views.push(section);

    let changed = box_part(&k, "block", Vec3::new(40.0, 30.0, 30.0));
    let report = update_from_model(&mut d, &changed).unwrap();
    assert_eq!(report.dangling_views[0].view, id);
    let section = d.view(id).unwrap();
    assert_eq!(section.geometry, saved_geometry);
    let ViewKind::Section { line: moved, .. } = &section.kind else { panic!("saved section") };
    for (before, after) in line.iter().zip(moved) {
        assert_eq!(after.u, before.u);
        assert!((after.v - before.v + 10.0 * parent.xf.scale).abs() < 1e-6);
    }
    export::check_sheet(&d, &d.sheets[0]).unwrap();
    assert!(layout_sheet(&d, 0).items.iter().any(|item| item.role() == Role::CuttingPlane));
}

#[test]
fn letters_skip_i_o_and_q() {
    assert_eq!(next_letter("A"), "B");
    assert_eq!(next_letter("H"), "J");
    assert_eq!(next_letter("N"), "P");
    assert_eq!(next_letter("P"), "R");
    assert_eq!(next_letter("Z"), "AA");
}

#[test]
fn a_detail_magnifies_inside_its_circle_and_stays_on_the_part() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let ctx = derived::context(&d, &part, &index, FRONT).unwrap();
    // The front view's top right corner.
    let corner = Vec3::new(40.0, 0.0, 10.0);
    let (view, _) =
        derived::new_detail(&d, &part, &index, 0, FRONT, ctx.xf.point(corner), 5.0, Vec::new(), 2.0, None).unwrap();
    let id = view.id;
    let mut stack = UndoStack::new();
    run(&mut stack, &mut d, &part, Command::PlaceView { sheet: 0, view });
    assert_eq!(d.next_view_label, "B", "placing detail A moves the letter on");
    let scale = project::view_scale(&d, id).unwrap();
    assert!((scale - 2.0 * ctx.xf.scale).abs() < 1e-12, "a detail follows its parent's scale times its factor");
    let v = d.view(id).unwrap();
    let Some(Boundary::Circle { radius, .. }) = &v.geometry.outline else { panic!("{:?}", v.geometry.outline) };
    assert!((radius - 10.0).abs() < 1e-9);
    for c in &v.geometry.curves {
        for poly in soliddraft_drawing::display::flatten(&c.path, 0.01) {
            for p in poly {
                assert!(p.x.hypot(p.y) <= 10.0 + 1e-6, "{p:?} is outside the detail circle");
            }
        }
    }
    assert!(!v.geometry.curves.is_empty(), "the corner's edges are in the detail");
    let shown = texts(&d, 0);
    assert!(shown.contains(&"DETAIL A".to_owned()), "{shown:?}");
    assert!(shown.iter().any(|t| t.starts_with("SCALE ")), "{shown:?}");
    assert!(layout_sheet(&d, 0).items.iter().any(|i| i.role() == Role::Boundary), "the circle on the front view");

    // The part grows: the circle stays on the same corner of the part.
    let wider = box_part(&k, "block", Vec3::new(60.0, 30.0, 10.0));
    let report = update_from_model(&mut d, &wider).unwrap();
    assert!(report.dangling_views.is_empty(), "{report:?}");
    let index = PartIndex::build(&wider).unwrap();
    let ctx = derived::context(&d, &wider, &index, FRONT).unwrap();
    let ViewKind::Detail { center, .. } = &d.view(id).unwrap().kind else { panic!() };
    assert_eq!(*center, ctx.xf.point(corner));
}

#[test]
fn an_auxiliary_view_looks_square_to_its_edge_and_dangles_when_it_goes() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    // The front view's right edge: the view looks at it from the right.
    let edge = edge_near(&part, Vec3::new(40.0, 0.0, 5.0)).unwrap();
    let (view, ctx) = derived::new_auxiliary(&d, &part, &index, 0, FRONT, edge, false, None).unwrap();
    assert!(ctx.xf.frame.direction.distance(Vec3::X) < 1e-9, "{:?}", ctx.xf.frame);
    let id = view.id;
    let mut stack = UndoStack::new();
    run(&mut stack, &mut d, &part, Command::PlaceView { sheet: 0, view });
    assert!(texts(&d, 0).contains(&"VIEW A".to_owned()));
    assert!(layout_sheet(&d, 0).items.iter().any(|i| i.role() == Role::CuttingPlane), "the arrow on the front view");

    // A part made by another feature: the edge is gone; the view keeps its line work.
    let other = box_part(&k, "other", Vec3::new(40.0, 30.0, 10.0));
    let before = d.view(id).unwrap().geometry.clone();
    let report = update_from_model(&mut d, &other).unwrap();
    assert_eq!(report.dangling_views.len(), 1, "{report:?}");
    assert_eq!(report.dangling_views[0].view, id);
    assert!(report.dangling_views[0].cause.contains("block"), "{}", report.dangling_views[0].cause);
    assert_eq!(d.view(id).unwrap().geometry, before);
    // Reattach it to the new part's edge.
    let edge = edge_near(&other, Vec3::new(40.0, 0.0, 5.0)).unwrap();
    run(&mut stack, &mut d, &other, Command::ReattachView { view: id, edge });
    assert!(update_from_model(&mut d, &other).unwrap().dangling_views.is_empty());
}

#[test]
fn crops_keep_only_what_is_inside() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let full = d.view(FRONT).unwrap().geometry.curves.len();
    let crop = Boundary::rectangle(Point2::new(-200.0, -200.0), Point2::new(0.0, 200.0));
    let mut stack = UndoStack::new();
    run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: Some(crop) });
    let v = d.view(FRONT).unwrap();
    assert!(!v.geometry.curves.is_empty());
    for c in &v.geometry.curves {
        assert!(kurbo::Shape::bounding_box(&c.path).x1 <= 1e-6, "a curve right of the crop");
    }
    assert!(v.geometry.bbox[2] <= 1e-6);
    run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: None });
    assert_eq!(d.view(FRONT).unwrap().geometry.curves.len(), full);
    assert!(matches!(
        apply(&mut d, &Command::SetViewCrop { view: FRONT, crop: None }),
        Err(soliddraft_drawing::DrawingError::NeedsModel(_))
    ));
}

fn corner_detail(d: &mut Drawing, part: &Part, stack: &mut UndoStack, parent: ViewId, sheet: usize) -> ViewId {
    let index = PartIndex::build(part).unwrap();
    let ctx = derived::context(d, part, &index, parent).unwrap();
    let (view, _) = derived::new_detail(
        d,
        part,
        &index,
        sheet,
        parent,
        ctx.xf.point(Vec3::new(40.0, 0.0, 10.0)),
        5.0,
        Vec::new(),
        2.0,
        None,
    )
    .unwrap();
    let id = view.id;
    run(stack, d, part, Command::PlaceView { sheet, view });
    id
}

fn left_half() -> Boundary {
    Boundary::rectangle(Point2::new(-200.0, -200.0), Point2::new(0.0, 200.0))
}

fn assert_fresh_detail(d: &Drawing, part: &Part, id: ViewId, empty: bool) {
    let index = PartIndex::build(part).unwrap();
    let fresh = derived::context(d, part, &index, id).unwrap();
    assert_eq!(fresh.projected.geometry.curves.is_empty(), empty);
    assert_eq!(d.view(id).unwrap().geometry, fresh.projected.geometry, "{id} must show its current parent");
}

fn drawing_roundtrip(d: &Drawing) -> Drawing {
    let meta = soliddraft_drawing::file::FileMeta::new("box test", d.meta.units);
    let bytes = soliddraft_drawing::file::to_bytes_with(d, &meta, false).unwrap();
    soliddraft_drawing::file::from_bytes(&bytes).unwrap().0
}

#[test]
fn parent_crop_refreshes_nested_details_across_sheets_history_and_save() {
    let mut k = BoxKernel::new();
    k.detail_strokes = true;
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    for sheet in [0, 1] {
        let mut d = drawing(&part);
        let mut stack = UndoStack::new();
        if sheet == 1 {
            run(&mut stack, &mut d, &part, Command::AddSheet { name: "Sheet2".into(), template: "iso-a3".into() });
        }
        let detail = corner_detail(&mut d, &part, &mut stack, FRONT, sheet);
        let nested = corner_detail(&mut d, &part, &mut stack, detail, 0);
        for id in [detail, nested] {
            assert_fresh_detail(&d, &part, id, false);
        }
        let before = d.clone();
        run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
        for id in [detail, nested] {
            assert_fresh_detail(&d, &part, id, true);
        }
        let cropped = d.clone();
        stack.undo(&mut d).unwrap();
        assert_eq!(d, before);
        stack.redo(&mut d).unwrap();
        assert_eq!(d, cropped);
        let reopened = drawing_roundtrip(&d);
        for id in [detail, nested] {
            assert_fresh_detail(&reopened, &part, id, true);
        }
        for i in 0..reopened.sheets.len() {
            assert!(export::bytes(&layout_sheet(&reopened, i), export::Format::Svg, "Box").is_ok());
        }
    }
}

#[test]
fn removing_a_parent_crop_restores_detail_linework() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let mut stack = UndoStack::new();
    run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    let detail = corner_detail(&mut d, &part, &mut stack, FRONT, 0);
    assert_fresh_detail(&d, &part, detail, true);
    run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: None });
    assert_fresh_detail(&d, &part, detail, false);
    assert_fresh_detail(&drawing_roundtrip(&d), &part, detail, false);
}

#[test]
fn same_revision_rebuild_repairs_an_obsolete_fine_detail_cache() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let mut stack = UndoStack::new();
    let detail = corner_detail(&mut d, &part, &mut stack, FRONT, 0);
    let obsolete = d.view(detail).unwrap().geometry.clone();
    assert!(!obsolete.unproved && !obsolete.coarse && !obsolete.curves.is_empty());
    run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    // An older saved drawing can already contain this obsolete cache.
    d.view_mut(detail).unwrap().geometry = obsolete;
    let mut d = drawing_roundtrip(&d);
    assert_eq!(d.view(detail).unwrap().geometry.source_digest, part.digest.as_str());
    update_from_model(&mut d, &part).unwrap();
    assert_fresh_detail(&d, &part, detail, true);
}

#[test]
fn detail_updates_preserve_coarse_and_fine_quality() {
    let mut k = BoxKernel::new();
    k.detail_strokes = true;
    let mut part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let mut stack = UndoStack::new();
    let detail = corner_detail(&mut d, &part, &mut stack, FRONT, 0);
    part.preview = true;
    run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    assert_fresh_detail(&d, &part, detail, true);
    assert!(d.view(detail).unwrap().geometry.coarse);
    part.preview = false;
    update_from_model(&mut d, &part).unwrap();
    assert_fresh_detail(&d, &part, detail, true);
    assert!(!d.view(detail).unwrap().geometry.coarse);
    assert!(export::bytes(&layout_sheet(&d, 0), export::Format::Svg, "Box").is_ok());
}

#[test]
fn excluded_details_keep_their_intentionally_frozen_geometry() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let mut stack = UndoStack::new();
    let detail = corner_detail(&mut d, &part, &mut stack, FRONT, 0);
    d.view_mut(detail).unwrap().excluded_from_update = true;
    let frozen = d.view(detail).unwrap().geometry.clone();
    run(&mut stack, &mut d, &part, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    assert_eq!(d.view(detail).unwrap().geometry, frozen);
    update_from_model(&mut d, &part).unwrap();
    assert_eq!(d.view(detail).unwrap().geometry, frozen);
}

#[test]
fn dimension_sets_measure_and_follow_the_part() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let ctx = derived::context(&d, &part, &index, FRONT).unwrap();
    // The middles of the front face's edges: each names one edge of the box after it
    // grows, where a corner would be shared by three.
    let (left, right) = (Vec3::new(0.0, 0.0, 5.0), Vec3::new(40.0, 0.0, 5.0));
    let (bottom, top) = (Vec3::new(20.0, 0.0, 0.0), Vec3::new(20.0, 0.0, 10.0));
    let mut stack = UndoStack::new();
    let mut ids = Vec::new();
    for (kind, orientation, points) in [
        (SetKind::Ordinate, LinearOrientation::Horizontal, vec![left, right]),
        (SetKind::Baseline, LinearOrientation::Vertical, vec![bottom, top]),
        (SetKind::Chain, LinearOrientation::Horizontal, vec![left, right]),
    ] {
        let a = annotate::dimension_set(&d, &index, &ctx, kind, orientation, &points, 12.0).unwrap();
        ids.push(a.id);
        run(&mut stack, &mut d, &part, Command::AddAnnotation { annotation: a });
    }
    let value = |d: &Drawing, i: usize| match &d.annotation(ids[i]).unwrap().body {
        AnnotationKind::DimensionSet(s) => s.values.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(value(&d, 0), vec![40.0]);
    assert_eq!(value(&d, 1), vec![10.0]);
    assert_eq!(value(&d, 2), vec![40.0]);
    let list = layout_sheet(&d, 0);
    assert_eq!(list.dimensions.len(), 3, "{:?}", list.dimensions);
    assert!(
        list.dimensions.iter().any(|r| matches!(
            r.geometry,
            soliddraft_drawing::display::DimensionGeometry::Ordinate { along_x: true, .. }
        ))
    );
    assert!(texts(&d, 0).contains(&"0".to_owned()), "the ordinate origin is marked 0");

    let wider = box_part(&k, "block", Vec3::new(42.0, 30.0, 10.0));
    let report = update_from_model(&mut d, &wider).unwrap();
    assert_eq!(value(&d, 0), vec![42.0]);
    let changed: Vec<&str> = report.changed.iter().map(|c| c.now.as_str()).collect();
    assert_eq!(changed, ["42", "42"], "{report:?}");
    assert!(matches!(&d.annotation(ids[0]).unwrap().state, AnnotationState::Changed { previous } if previous == "40"));
    assert_eq!(d.annotation(ids[1]).unwrap().state, AnnotationState::Ok);
}

#[test]
fn symbols_revisions_and_their_table() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let ctx = derived::context(&d, &part, &index, FRONT).unwrap();
    let mut stack = UndoStack::new();
    let top_edge = Vec3::new(20.0, 0.0, 10.0);
    let bodies = [
        AnnotationKind::Gdt {
            characteristic: GdtCharacteristic::Position,
            tolerance: 0.1,
            diameter: true,
            material: MaterialCondition::Maximum,
            datums: vec![DatumRef::new("A"), DatumRef { letter: "B".into(), material: MaterialCondition::Least }],
        },
        AnnotationKind::Datum { letter: d.next_datum_label.clone() },
        AnnotationKind::SurfaceFinish {
            text: "Ra 1.6".into(),
            process: SurfaceProcess::Removal,
            method: "Ground".into(),
            lay: Some(soliddraft_drawing::Lay::Perpendicular),
            allowance: String::new(),
            all_around: false,
        },
        AnnotationKind::RevisionSymbol { revision: "B".into() },
    ];
    let mut gdt = None;
    for body in bodies {
        let a = annotate::attached(&d, &index, &ctx, top_edge, body, Point2::new(10.0, 10.0)).unwrap();
        gdt.get_or_insert(a.id);
        run(&mut stack, &mut d, &part, Command::AddAnnotation { annotation: a });
    }
    assert_eq!(d.next_datum_label, "B", "datum A moves the letter on");
    for (rev, what) in [("A", "First issue"), ("B", "Wider top")] {
        let row = RevisionRow {
            revision: rev.into(),
            description: what.into(),
            date: "2026-10-14".into(),
            ..Default::default()
        };
        run(&mut stack, &mut d, &part, Command::AddRevision { sheet: 0, row });
    }
    assert_eq!(d.meta.revision, "B");
    let shown = texts(&d, 0);
    for t in ["Ø0.1", "M", "L", "Ra 1.6", "Ground", "⊥", "REVISIONS", "First issue", "Wider top"] {
        assert!(shown.contains(&t.to_owned()), "{t} not on the sheet: {shown:?}");
    }
    // The revision table lists the zone of revision B's symbol.
    let zone = soliddraft_drawing::display::zone_at(
        soliddraft_drawing::library::template("iso-a3"),
        Point2::new(
            d.view(FRONT).unwrap().position.u + ctx.xf.point(top_edge).u + 10.0,
            d.view(FRONT).unwrap().position.v + ctx.xf.point(top_edge).v + 10.0,
        ),
    );
    assert!(shown.contains(&zone), "zone {zone} not in {shown:?}");
    // Symbols follow the part like any annotation, and edit in place.
    let report = update_from_model(&mut d, &box_part(&k, "block", Vec3::new(40.0, 30.0, 12.0))).unwrap();
    assert!(report.dangling.is_empty(), "{report:?}");
    let gdt = gdt.unwrap();
    let edited = AnnotationKind::Gdt {
        characteristic: GdtCharacteristic::Flatness,
        tolerance: 0.05,
        diameter: false,
        material: MaterialCondition::Regardless,
        datums: Vec::new(),
    };
    apply(&mut d, &Command::EditAnnotation { annotation: gdt, body: edited }).unwrap();
    assert!(apply(&mut d, &Command::EditAnnotation { annotation: gdt, body: AnnotationKind::CentreMark }).is_err());
    apply(&mut d, &Command::FreezeAsNote { annotation: gdt }).unwrap();
    assert!(
        matches!(&d.annotation(gdt).unwrap().body, AnnotationKind::Note { text, .. } if text == "Flatness 0.05 (ref)")
    );
}

#[test]
fn a_detail_on_sheet_two_and_its_parent_point_at_each_other() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let mut stack = UndoStack::new();
    run(&mut stack, &mut d, &part, Command::AddSheet { name: "Sheet2".into(), template: "iso-a3".into() });
    let index = PartIndex::build(&part).unwrap();
    let ctx = derived::context(&d, &part, &index, FRONT).unwrap();
    let (view, _) = derived::new_detail(
        &d,
        &part,
        &index,
        1,
        FRONT,
        ctx.xf.point(Vec3::new(40.0, 0.0, 10.0)),
        5.0,
        Vec::new(),
        2.0,
        None,
    )
    .unwrap();
    let id = view.id;
    run(&mut stack, &mut d, &part, Command::PlaceView { sheet: 1, view });
    let labels = location_labels(&d);
    assert_eq!(labels.len(), 1, "{labels:?}");
    let l = &labels[0];
    assert_eq!((l.view, l.parent, l.view_sheet, l.parent_sheet), (id, FRONT, 1, 0));
    let (on_parent, on_view) = (l.on_parent().unwrap(), l.on_view().unwrap());
    assert_eq!(on_parent, format!("SEE SHEET 2, ZONE {}", l.view_zone));
    assert_eq!(on_view, format!("FROM SHEET 1, ZONE {}", l.parent_zone));
    assert!(texts(&d, 0).contains(&on_parent), "the parent's sheet says where the detail is");
    assert!(texts(&d, 1).contains(&on_view), "the detail's sheet says where it comes from");
    assert!(texts(&d, 1).contains(&"DETAIL A".to_owned()));
    // The title block counts both sheets.
    assert!(texts(&d, 1).iter().any(|t| t == "2 / 2"), "{:?}", texts(&d, 1));
    // The parent's sheet cannot go while the detail is made from it.
    assert!(apply(&mut d, &Command::DeleteSheet { sheet: 0 }).is_err());
    // The update follows across sheets, and the file keeps both sheets.
    let report = update_from_model(&mut d, &box_part(&k, "block", Vec3::new(40.0, 30.0, 12.0))).unwrap();
    assert!(report.dangling_views.is_empty(), "{report:?}");
    let bytes = soliddraft_drawing::file::to_bytes(&d, &soliddraft_drawing::file::FileMeta::new("boxes", d.meta.units))
        .unwrap();
    let (back, _) = soliddraft_drawing::file::from_bytes(&bytes).unwrap();
    assert_eq!(back, d);
    // Moving the detail back to sheet 1 drops the labels.
    apply(&mut d, &Command::MoveViewToSheet { view: id, sheet: 0, to: Point2::new(300.0, 200.0) }).unwrap();
    assert!(location_labels(&d)[0].on_parent().is_none());
    apply(&mut d, &Command::DeleteSheet { sheet: 1 }).unwrap();
    assert_eq!(d.sheets.len(), 1);
}

#[test]
fn unmarked_circular_hatch_boundaries_are_approximate_after_sampling() {
    use soliddraft_kernel_api::drawing::{Curve3, CurveSource, CutRegion, ViewCurve, ViewSpec};
    let frame = ViewSpec::new(Vec3::Z, Vec3::Y).frame().unwrap();
    let xf = project::ViewTransform { frame, scale: 1.0, centre: Point2::new(0.0, 0.0) };
    let circle = |radius| {
        ViewCurve::new(Curve3::Circle { center: Vec3::ZERO, normal: Vec3::Z, radius }, CurveSource::Unknown, true)
    };
    let region = CutRegion { outer: vec![circle(10.0)], holes: vec![vec![circle(4.0)]] };
    let polygon = project::region_polygon(&region, &xf, None);
    assert!(polygon.outer.len() > 4 && polygon.holes[0].len() > 4);
    assert!(polygon.approximate);
}

#[test]
fn dxf_marks_the_polygon_used_to_clip_a_circular_hatch() {
    let k = BoxKernel::new();
    let mut drawing = drawing(&box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0)));
    let view = drawing.view_mut(FRONT).unwrap();
    view.geometry.hatch.push(soliddraft_drawing::Polygon {
        outer: vec![
            Point2::new(-10.0, -10.0),
            Point2::new(10.0, -10.0),
            Point2::new(10.0, 10.0),
            Point2::new(-10.0, 10.0),
        ],
        clip: Some(Boundary::Circle { center: Point2::new(0.0, 0.0), radius: 4.0 }),
        ..Default::default()
    });
    let list = layout_sheet(&drawing, 0);
    assert_eq!(list.hatches[0].loops[0].len(), 96);
    assert_eq!(list.dxf_accuracy().approximate, 1);
}

fn fitted_hatch(d: &mut Drawing, xf: &project::ViewTransform) -> soliddraft_drawing::display::DisplayList {
    use soliddraft_kernel_api::drawing::{Curve3, CurveSource, CutRegion, ViewCurve};
    let square = [(-8.0, -8.0), (8.0, -8.0), (8.0, 8.0), (-8.0, 8.0)];
    let point = |(u, v)| xf.unproject(Point2::new(u, v));
    let outer = (0..4)
        .map(|i| ViewCurve {
            approximate: i == 0,
            ..ViewCurve::new(
                Curve3::Line { a: point(square[i]), b: point(square[(i + 1) % 4]) },
                CurveSource::Unknown,
                true,
            )
        })
        .collect();
    let cut = CutRegion { outer, holes: Vec::new() };
    let clip = Boundary::rectangle(Point2::new(-5.0, -5.0), Point2::new(5.0, 5.0));
    let polygon = project::region_polygon(&cut, xf, Some(clip));
    d.view_mut(FRONT).unwrap().geometry.hatch.push(polygon);
    let meta = soliddraft_drawing::file::FileMeta::new("test", d.meta.units);
    let bytes = soliddraft_drawing::file::to_bytes(d, &meta).unwrap();
    let (d, _) = soliddraft_drawing::file::from_bytes(&bytes).unwrap();
    layout_sheet(&d, 0)
}

fn fitted_hatch_fixture() -> soliddraft_drawing::display::DisplayList {
    let mut k = BoxKernel::new();
    k.detail_strokes = true;
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let ctx = derived::context(&d, &part, &index, FRONT).unwrap();
    fitted_hatch(&mut d, &ctx.xf)
}

#[test]
fn fitted_hatch_reports_approximation_in_svg() {
    let list = fitted_hatch_fixture();
    let svg = export::svg(&list);
    assert_eq!(list.accuracy().approximate, 1);
    assert!(svg.contains(r#"data-accuracy="approximate""#));
}

#[test]
fn fitted_hatch_reports_approximation_in_pdf() {
    let list = fitted_hatch_fixture();
    let pdf = export::pdf(std::slice::from_ref(&list), &export::PdfInfo::default()).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert_eq!(list.accuracy().approximate, 1);
}

#[test]
fn fitted_hatch_reports_approximation_in_dxf() {
    let list = fitted_hatch_fixture();
    let bytes = export::dxf(&list).unwrap();
    let counts = export::dxf::count_entities(&bytes).unwrap();
    assert_eq!(counts.get("HATCH/HATCH"), Some(&1), "{counts:?}");
    assert_eq!(list.dxf_accuracy().approximate, 1);
}

#[test]
fn the_dxf_keeps_dimensions_and_hatches_as_such() {
    let mut k = BoxKernel::new();
    k.detail_strokes = true;
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut d =
        soliddraft_drawing::make_drawing(&part, &NewDrawing { model_items: true, ..new_drawing() }).unwrap().drawing;
    let index = PartIndex::build(&part).unwrap();
    let ctx = derived::context(&d, &part, &index, FRONT).unwrap();
    let set = annotate::dimension_set(
        &d,
        &index,
        &ctx,
        SetKind::Ordinate,
        LinearOrientation::Horizontal,
        &[Vec3::ZERO, Vec3::new(40.0, 0.0, 0.0)],
        12.0,
    )
    .unwrap();
    apply(&mut d, &Command::AddAnnotation { annotation: set }).unwrap();
    let list = fitted_hatch(&mut d, &ctx.xf);
    assert!(list.items.iter().any(|i| i.role() == Role::Hatch));
    let bytes = export::dxf(&list).unwrap();
    let counts = export::dxf::count_entities(&bytes).unwrap();
    assert_eq!(counts.get("HATCH/HATCH"), Some(&1), "{counts:?}");
    assert!(!counts.contains_key("HATCH/LINE"), "hatch lines go out as a HATCH: {counts:?}");
    let dims = counts.get("DIMENSION/DIMENSION").copied().unwrap_or(0);
    assert_eq!(dims, list.dimensions.len(), "{counts:?}");
    assert!(dims >= 2, "{counts:?}");
    let back = dxf::Drawing::load(&mut bytes.as_slice()).unwrap();
    let ordinate = back.entities().find_map(|e| match &e.specific {
        dxf::entities::EntityType::OrdinateDimension(o) => Some(o),
        _ => None,
    });
    let ordinate = ordinate.expect("the ordinate dimension");
    assert!(ordinate.dimension_base.is_ordinate_x_type);
    let style = back.dim_styles().find(|s| s.name == ordinate.dimension_base.dimension_style_name).unwrap();
    assert!((ordinate.dimension_base.actual_measurement - 40.0 * ctx.xf.scale).abs() < 1e-9);
    assert!(
        (ordinate.dimension_base.actual_measurement * style.dimension_linear_measurement_scale_factor - 40.0).abs()
            < 1e-9
    );
    assert!(back.blocks().any(|b| b.name == ordinate.dimension_base.block_name));
}

#[test]
fn export_accuracy_counts_printed_paths_only() {
    use soliddraft_drawing::Accuracy;

    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut drawing = drawing(&part);
    let view = drawing.view_mut(FRONT).unwrap();
    let hidden = view.geometry.curves.iter_mut().find(|curve| !curve.visible).unwrap();
    hidden.accuracy = Accuracy::Exact;
    hidden.path = kurbo::BezPath::from_svg("M 0 0 C 0 5 5 10 10 10").unwrap();
    assert_eq!(view.geometry.accuracy().approximate, 1);
    assert_eq!(layout_sheet(&drawing, 0).accuracy().approximate, 1);
    drawing.view_mut(FRONT).unwrap().display.hidden = HiddenLines::Removed;
    let layout = layout_sheet(&drawing, 0);
    assert_eq!(layout.accuracy().approximate, 0);
    assert!(layout.accuracy().exact + layout.accuracy().polygon > 0);
}

#[test]
fn unsupported_views_use_mesh_only_for_preview() {
    use soliddraft_drawing::DrawingError;
    use soliddraft_kernel_api::{
        KernelError,
        drawing::{StandardView, ViewSpec},
    };

    let mut k = BoxKernel::new();
    k.unsupported_view = true;
    let mut part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let spec = ViewSpec::standard(StandardView::Front);
    assert!(matches!(project::view_curves(&part, &spec), Err(DrawingError::Kernel(KernelError::Unsupported(_)))));
    part.preview = true;
    let view = project::view_curves(&part, &spec).unwrap();
    assert!(view.unproved && !view.catallax && !view.curves.is_empty());
}

#[test]
fn export_refuses_unmarked_saved_crops_even_when_proved() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut drawing = drawing(&part);
    run(&mut UndoStack::new(), &mut drawing, &part, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    for view in &mut drawing.sheets[0].views {
        assert!(!view.geometry.unproved);
        view.geometry.catallax = false;
    }
    let saved = drawing_roundtrip(&drawing);
    let list = layout_sheet(&saved, 0);
    let path = std::env::temp_dir().join(format!("soliddraft-legacy-crop-{}.svg", std::process::id()));
    let errors = [
        export::write(&list, export::Format::Svg, "Box", &path).unwrap_err(),
        export::pdf(std::slice::from_ref(&list), &export::PdfInfo::default()).unwrap_err(),
        export::dxf(&list).unwrap_err(),
    ];
    for error in errors {
        assert!(error.to_string().contains("Front view") && error.to_string().contains("Redraw it once"));
    }
    assert!(!path.exists());
    assert_eq!(saved.view(FRONT).unwrap().crop, Some(left_half()));
}

#[test]
fn export_refuses_geometry_regenerated_against_an_unaccepted_revision() {
    let k = BoxKernel::new();
    let old = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let current = box_part(&k, "block", Vec3::new(60.0, 30.0, 10.0));
    let mut drawing = drawing(&old);
    run(&mut UndoStack::new(), &mut drawing, &current, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    assert_eq!(drawing.model.pinned, old.digest);
    assert_eq!(drawing.view(FRONT).unwrap().geometry.source_digest, current.digest.as_str());
    for view in &mut drawing.sheets[0].views {
        view.geometry.catallax = true;
    }
    assert!(export::check_sheet(&drawing, &drawing.sheets[0]).unwrap_err().to_string().contains("accepted revision"));
}

#[test]
fn export_preserves_a_frozen_detail_after_rebuild_and_reopen() {
    use soliddraft_drawing::display::Owner;

    let k = BoxKernel::new();
    let old = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let new = box_part(&k, "block", Vec3::new(60.0, 30.0, 10.0));
    let mut drawing = drawing(&old);
    let mut stack = UndoStack::new();
    let detail = corner_detail(&mut drawing, &old, &mut stack, FRONT, 0);
    let detail_item =
        |item: &Item| matches!(item, Item::Path(p) if matches!(p.owner, Owner::Curve(id, _) if id == detail));
    let mut earlier = layout_sheet(&drawing, 0);
    earlier.items.retain(detail_item);
    assert!(!earlier.items.is_empty());
    let expected = export::svg(&earlier);
    drawing.view_mut(detail).unwrap().excluded_from_update = true;
    let list = layout_sheet(&drawing, 0);
    let written = String::from_utf8(export::bytes(&list, export::Format::Svg, "Box").unwrap()).unwrap();
    for path in expected.lines().filter(|line| line.starts_with("<path ")) {
        assert!(written.lines().any(|line| line == path), "missing saved detail path {path}");
    }
    run(&mut stack, &mut drawing, &new, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    update_from_model(&mut drawing, &new).unwrap();
    for view in &mut drawing.sheets[0].views {
        view.geometry.catallax = true;
    }

    for saved in [&drawing, &drawing_roundtrip(&drawing)] {
        assert_ne!(saved.view(detail).unwrap().geometry.source_digest, saved.model.pinned.as_str());
        let list = layout_sheet(saved, 0);
        assert_eq!(
            list.printable().filter(|item| detail_item(item)).collect::<Vec<_>>(),
            earlier.items.iter().collect::<Vec<_>>()
        );
        for format in export::Format::ALL {
            assert!(!export::bytes(&list, format, "Box").unwrap().is_empty());
        }
    }
}

#[test]
fn detail_uses_its_own_tangents_and_seams_inside_the_parent_crop() {
    use soliddraft_drawing::{CurveKind, TangentEdges};

    let mut k = BoxKernel::new();
    k.detail_strokes = true;
    let part = box_part(&k, "block", Vec3::new(40.0, 30.0, 10.0));
    let mut drawing = drawing(&part);
    let mut stack = UndoStack::new();
    let detail = corner_detail(&mut drawing, &part, &mut stack, FRONT, 0);
    let flags = |d: &Drawing, id| {
        let curves = &d.view(id).unwrap().geometry.curves;
        (curves.iter().any(|c| c.tangent), curves.iter().any(|c| c.kind == CurveKind::Seam))
    };
    let set = |d: &mut Drawing, stack: &mut UndoStack, view, enabled| {
        let mut display = d.view(view).unwrap().display;
        display.tangent = if enabled { TangentEdges::Phantom } else { TangentEdges::Removed };
        display.seams = enabled;
        run(stack, d, &part, Command::SetViewDisplay { view, display });
    };
    set(&mut drawing, &mut stack, detail, true);
    assert_eq!(flags(&drawing, FRONT), (false, false));
    assert_eq!(flags(&drawing, detail), (true, true));
    run(&mut stack, &mut drawing, &part, Command::SetViewCrop { view: FRONT, crop: Some(left_half()) });
    assert_eq!(flags(&drawing, detail), (false, false));
    run(&mut stack, &mut drawing, &part, Command::SetViewCrop { view: FRONT, crop: None });
    set(&mut drawing, &mut stack, FRONT, true);
    set(&mut drawing, &mut stack, detail, false);
    assert_eq!(flags(&drawing, FRONT), (true, true));
    assert_eq!(flags(&drawing, detail), (false, false));
}
