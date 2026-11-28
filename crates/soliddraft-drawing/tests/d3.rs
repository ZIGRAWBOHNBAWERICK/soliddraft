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
                anchors: vec![soliddraft_drawing::Anchor {
                    entity: d.view(view).unwrap().geometry.curves[0].source.clone(),
                    cached: Point2::default(),
                    attach: soliddraft_drawing::Attach::Mid,
                }],
                placement: Point2::new(0.0, 0.0),
                style: Default::default(),
                state: AnnotationState::Ok,
                source: Some(soliddraft_drawing::PmiSource::Item { item: "removed requirement".into() }),
            };
            apply(&mut d, &Command::AddAnnotation { annotation: note.clone() }).unwrap();
            saved.push(note);
        }
        let markup = soliddraft_drawing::review::MarkupTarget { sheet: target, annotation: saved[0].id };
        assert!(soliddraft_drawing::review::find_place(&d, &markup, "removed requirement").is_some());
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
        assert!(soliddraft_drawing::review::place_of_item(&d, "removed requirement").is_none());
        assert!(soliddraft_drawing::review::find_place(&d, &markup, "removed requirement").is_none());
        assert!(!d.views_match_pin());
        for rebuilt in d.sheets[0].views.iter().filter(|v| v.id != id) {
            assert_eq!(rebuilt.geometry.source_digest, changed.digest.as_str());
        }
        let error = export::check_sheet(&d, &d.sheets[target]).unwrap_err().to_string();
        assert!(error.contains(&failure.name), "{error}");
        assert!(error.contains("does not match the drawing's accepted revision"), "{error}");

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
    let mut k = BoxKernel::new();
    k.sections = true;
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

    let half =
        soliddraft_drawing::HalfSection { axis: [Vec3::new(20.0, 0.0, 5.0), Vec3::new(20.0, 30.0, 5.0)], flip: false };
    let attempt = stack.run(&mut d, &Command::SetHalfSection { view: id, half: Some(half) }, Some(&part));
    assert!(matches!(attempt, Err(soliddraft_drawing::DrawingError::BadRequest(_))));
    assert!(d.view(id).unwrap().half.is_none());

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
    let xf = project::ViewTransform::new(frame, 1.0, Point2::new(0.0, 0.0));
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

#[test]
fn a_detail_of_either_break_axis_keeps_its_model_points_after_rescale() {
    let k = BoxKernel::new();
    let part = box_part(&k, "block", Vec3::new(120.0, 30.0, 120.0));
    let index = PartIndex::build(&part).unwrap();
    for (axis, from, to) in [
        (soliddraft_drawing::AlignAxis::Horizontal, Vec3::new(30.0, 0.0, 0.0), Vec3::new(80.0, 0.0, 0.0)),
        (soliddraft_drawing::AlignAxis::Vertical, Vec3::new(0.0, 0.0, 30.0), Vec3::new(0.0, 0.0, 80.0)),
    ] {
        let mut drawing = drawing(&part);
        let mut stack = UndoStack::new();
        run(
            &mut stack,
            &mut drawing,
            &part,
            Command::SetViewBreaks { view: FRONT, breaks: vec![soliddraft_drawing::Break { from, to, axis }] },
        );
        let before = project::view_scale(&drawing, FRONT).unwrap();
        let parent = derived::context(&drawing, &part, &index, FRONT).unwrap();
        let [gap] = parent.xf.gaps()[..] else { panic!("one break") };
        let along = axis == soliddraft_drawing::AlignAxis::Horizontal;
        let [u0, v0, _, _] = parent.projected.geometry.bbox;
        let centre = if along { Point2::new(gap.at - 4.0, v0) } else { Point2::new(u0, gap.at - 4.0) };
        let (detail, ctx) =
            derived::new_detail(&drawing, &part, &index, 0, FRONT, centre, 12.0, Vec::new(), 2.0, None).unwrap();
        let reach = |geometry: &soliddraft_drawing::ViewGeometry| {
            let [g] = geometry.breaks[..] else { panic!("the detail inherits the gap") };
            let max = geometry
                .curves
                .iter()
                .flat_map(|c| c.path.elements())
                .filter_map(|el| el.end_point())
                .map(|p| if along { p.x } else { p.y })
                .fold(f64::NEG_INFINITY, f64::max);
            max - (g.at + g.gap)
        };
        assert!((reach(&ctx.projected.geometry) - 4.0).abs() < 1e-3, "{axis:?}: 2 mm on the other side at 2×");
        let id = detail.id;
        run(&mut stack, &mut drawing, &part, Command::PlaceView { sheet: 0, view: detail });
        run(
            &mut stack,
            &mut drawing,
            &part,
            Command::SetViewScale {
                view: FRONT,
                scale: soliddraft_drawing::ViewScale::Custom { factor: before * 0.5 },
            },
        );
        let fresh = derived::context(&drawing, &part, &index, FRONT).unwrap();
        let saved = &drawing.view(FRONT).unwrap().geometry;
        assert_eq!(saved.breaks[0].gap, 6.0);
        assert!((saved.breaks[0].at - fresh.xf.gaps()[0].at).abs() < 1e-6);
        for (cached, rebuilt) in saved.bbox.into_iter().zip(fresh.projected.geometry.bbox) {
            assert!((cached - rebuilt).abs() < 1e-6, "{axis:?} {cached} != {rebuilt}");
        }
        let saved_detail = &drawing.view(id).unwrap().geometry;
        assert!((reach(saved_detail) - 2.0).abs() < 1e-3, "{axis:?}");
        let rebuilt = derived::context(&drawing, &part, &index, id).unwrap();
        assert!((reach(&rebuilt.projected.geometry) - 2.0).abs() < 2e-2, "{axis:?}");
    }
}

#[test]
fn a_break_must_shorten_the_view_and_keep_both_ends() {
    let kernel = BoxKernel::new();
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 120.0));
    let mut drawing = drawing(&part);
    apply(
        &mut drawing,
        &Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Custom { factor: 1.0 } },
    )
    .unwrap();
    apply(&mut drawing, &Command::AddSheet { name: "Reduced".into(), template: "iso-a3".into() }).unwrap();
    let reduced = soliddraft_drawing::Scale::new(1, 2);
    apply(&mut drawing, &Command::SetSheetScale { sheet: 0, scale: soliddraft_drawing::Scale::FULL }).unwrap();
    apply(&mut drawing, &Command::SetSheetScale { sheet: 1, scale: reduced }).unwrap();
    let mut stack = UndoStack::new();
    for axis in [soliddraft_drawing::AlignAxis::Horizontal, soliddraft_drawing::AlignAxis::Vertical] {
        let point = |x| match axis {
            soliddraft_drawing::AlignAxis::Horizontal => Vec3::new(x, 0.0, 0.0),
            soliddraft_drawing::AlignAxis::Vertical => Vec3::new(0.0, 0.0, x),
        };
        let interrupt = |a, b| Command::SetViewBreaks {
            view: FRONT,
            breaks: vec![soliddraft_drawing::Break { from: point(a), to: point(b), axis }],
        };
        let before = drawing.clone();
        for cmd in [interrupt(40.0, 42.0), interrupt(-2.0, 122.0)] {
            assert!(stack.run(&mut drawing, &cmd, Some(&part)).is_err(), "{axis:?}");
            assert_eq!(drawing, before);
        }
        run(&mut stack, &mut drawing, &part, interrupt(50.0, 60.0));
        let before = drawing.clone();
        let too_small =
            Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Custom { factor: 0.5 } };
        assert!(apply(&mut drawing, &too_small).is_err(), "{axis:?}");
        assert_eq!(drawing, before);
        let parent_scale = Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Parent };
        apply(&mut drawing, &parent_scale).unwrap();
        let before = drawing.clone();
        for cmd in [
            Command::SetSheetScale { sheet: 0, scale: reduced },
            Command::MoveViewToSheet { view: FRONT, sheet: 1, to: Point2::new(100.0, 100.0) },
        ] {
            assert!(apply(&mut drawing, &cmd).is_err(), "{axis:?}");
            assert_eq!(drawing, before);
        }
    }
}

#[test]
fn dxf_preserves_the_length_across_a_small_break() {
    use dxf::entities::EntityType;
    let kernel = BoxKernel::new();
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 50.0));
    let mut drawing = drawing(&part);
    apply(
        &mut drawing,
        &Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Custom { factor: 1.0 } },
    )
    .unwrap();
    let mut stack = UndoStack::new();
    run(
        &mut stack,
        &mut drawing,
        &part,
        Command::SetViewBreaks {
            view: FRONT,
            breaks: vec![soliddraft_drawing::Break {
                from: Vec3::new(50.0, 0.0, 0.0),
                to: Vec3::new(56.005, 0.0, 0.0),
                axis: soliddraft_drawing::AlignAxis::Horizontal,
            }],
        },
    );
    let index = PartIndex::build(&part).unwrap();
    let ctx = derived::context(&drawing, &part, &index, FRONT).unwrap();
    let annotation = annotate::dimension_set(
        &drawing,
        &index,
        &ctx,
        SetKind::Baseline,
        LinearOrientation::Horizontal,
        &[Vec3::ZERO, Vec3::new(120.0, 0.0, 0.0)],
        10.0,
    )
    .unwrap();
    run(&mut stack, &mut drawing, &part, Command::AddAnnotation { annotation });
    let bytes = export::dxf(&layout_sheet(&drawing, 0)).unwrap();
    let dxf = dxf::Drawing::load(&mut bytes.as_slice()).unwrap();
    let dim = dxf
        .entities()
        .find_map(|e| match &e.specific {
            EntityType::RotatedDimension(dim) => Some(dim),
            _ => None,
        })
        .unwrap();
    let scale = dxf
        .dim_styles()
        .find(|s| s.name == dim.dimension_base.dimension_style_name)
        .unwrap()
        .dimension_linear_measurement_scale_factor;
    let drawn = (dim.definition_point_3.x - dim.definition_point_2.x).abs();
    assert!((drawn - 119.995).abs() < 1e-5);
    assert!((drawn * scale - 120.0).abs() < 1e-5);
}

#[test]
fn clipping_a_hatch_opens_a_hole_into_a_notch_in_the_dxf_region() {
    let mut drawing = soliddraft_drawing::fixture::bracket_a3();
    let points = |ring: &[(f64, f64)]| ring.iter().map(|&(u, v)| Point2::new(u, v)).collect();
    let view = drawing.view_mut(FRONT).unwrap();
    view.geometry.hatch.push(soliddraft_drawing::Polygon {
        outer: points(&[(-10.0, -10.0), (10.0, -10.0), (10.0, 10.0), (-10.0, 10.0)]),
        holes: vec![points(&[(-2.0, -3.0), (2.0, -3.0), (2.0, 3.0), (-2.0, 3.0)])],
        clip: Some(Boundary::rectangle(Point2::new(0.0, -10.0), Point2::new(10.0, 10.0))),
        approximate: false,
    });
    let list = layout_sheet(&drawing, 0);
    let [hatch] = list.hatches.as_slice() else { panic!("one region") };
    let [outer] = hatch.loops.as_slice() else { panic!("the hole opens to the outside") };
    let area =
        outer.iter().zip(outer.iter().cycle().skip(1)).map(|(a, b)| a.u * b.v - b.u * a.v).sum::<f64>().abs() / 2.0;
    assert!((area - 188.0).abs() < 1e-6, "{area}");
    assert_eq!(outer.len(), 8);
}

#[test]
fn exporting_omits_a_display_section_annotations_and_its_marks_on_another_sheet() {
    use soliddraft_drawing::display::Owner;
    let kernel = BoxKernel::new();
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 50.0));
    let mut drawing = drawing(&part);
    apply(&mut drawing, &Command::AddSheet { name: "Section".into(), template: "iso-a3".into() }).unwrap();
    let mut section = drawing.view(FRONT).unwrap().clone();
    section.id = drawing.next_view_id();
    section.kind = ViewKind::Section {
        parent: FRONT,
        line: vec![Point2::new(-10.0, -5.0), Point2::new(10.0, 5.0)],
        flip: false,
        label: "B".into(),
        hatch: None,
        model_line: Vec::new(),
    };
    section.geometry.display_only = true;
    let id = section.id;
    apply(&mut drawing, &Command::PlaceView { sheet: 1, view: section }).unwrap();
    let mut note = soliddraft_drawing::fixture::bracket_a3()
        .sheets
        .remove(0)
        .annotations
        .into_iter()
        .find(|a| matches!(a.body, AnnotationKind::Dimension(_)))
        .unwrap();
    note.view = id;
    note.id = drawing.next_annotation_id();
    let annotation = note.id;
    apply(&mut drawing, &Command::AddDimension { annotation: note }).unwrap();
    let parent = layout_sheet(&drawing, 0);
    assert!(parent.items.iter().any(|i| i.owner() == Owner::View(id) && i.role() == Role::CuttingPlane));
    assert!(parent.printable().all(|i| i.owner() != Owner::View(id)));
    let child = layout_sheet(&drawing, 1);
    assert_eq!(child.dimensions.len(), 1);
    assert!(child.printable().all(|i| i.owner() != Owner::Annotation(annotation)));
    let bytes = export::dxf(&child).unwrap();
    let dxf = dxf::Drawing::load(&mut bytes.as_slice()).unwrap();
    assert!(!dxf.entities().any(|e| e.common.layer == "DIMENSION"));
    drawing.view_mut(FRONT).unwrap().geometry.display_only = true;
    drawing.view_mut(id).unwrap().geometry.display_only = false;
    let parent = layout_sheet(&drawing, 0);
    assert!(parent.items.iter().any(|i| i.owner() == Owner::View(id) && i.role() == Role::CuttingPlane));
    assert!(parent.printable().all(|i| i.owner() != Owner::View(id)));
    let child = layout_sheet(&drawing, 1);
    assert!(child.items.iter().any(|i| matches!(i, Item::Text(t) if t.text.starts_with("FROM SHEET"))));
    assert!(child.printable().any(|i| matches!(i.owner(), Owner::Curve(v, _) if v == id)));
    assert!(child.printable().all(|i| !matches!(i, Item::Text(t) if t.text.starts_with("FROM SHEET"))));
}

#[test]
fn a_diagonal_half_section_clips_both_sides_of_a_break_in_model_space() {
    let mut kernel = BoxKernel::new();
    kernel.sections = true;
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 120.0));
    let mut drawing = drawing(&part);
    let mut stack = UndoStack::new();
    run(
        &mut stack,
        &mut drawing,
        &part,
        Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Custom { factor: 1.0 } },
    );
    run(
        &mut stack,
        &mut drawing,
        &part,
        Command::SetHalfSection {
            view: FRONT,
            half: Some(soliddraft_drawing::HalfSection {
                axis: [Vec3::new(0.0, 25.0, 0.0), Vec3::new(120.0, 25.0, 120.0)],
                flip: false,
            }),
        },
    );
    run(
        &mut stack,
        &mut drawing,
        &part,
        Command::SetViewBreaks {
            view: FRONT,
            breaks: vec![soliddraft_drawing::Break {
                from: Vec3::new(40.0, 0.0, 0.0),
                to: Vec3::new(80.0, 0.0, 0.0),
                axis: soliddraft_drawing::AlignAxis::Horizontal,
            }],
        },
    );
    let view = drawing.view(FRONT).unwrap();
    let mut areas: Vec<f64> = view
        .geometry
        .hatch
        .iter()
        .map(|p| {
            p.outer.iter().zip(p.outer.iter().cycle().skip(1)).map(|(a, b)| a.u * b.v - b.u * a.v).sum::<f64>().abs()
                / 2.0
        })
        .collect();
    areas.sort_by(f64::total_cmp);
    assert_eq!(areas.len(), 2);
    assert!((areas[0] - 800.0).abs() < 1e-4 && (areas[1] - 4000.0).abs() < 1e-4, "{areas:?}");
    let list = layout_sheet(&drawing, 0);
    let centre_lines: Vec<_> = list
        .printable()
        .filter_map(|i| match i {
            Item::Path(p) if p.role == Role::Boundary && p.dash.is_some() => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(centre_lines.len(), 2);
    for line in centre_lines {
        let [kurbo::PathEl::MoveTo(a), kurbo::PathEl::LineTo(b)] = line.path.elements() else {
            panic!("one line per side");
        };
        assert!(((b.x - a.x).abs() - (b.y - a.y).abs()).abs() < 1e-4);
    }
}

#[test]
fn aligned_section_dimensions_keep_the_second_leg_on_rebuild() {
    use soliddraft_drawing::measure::{Pick, propose};
    use soliddraft_drawing::{Attach, DimValue, Dimension, DimensionKind, Entity, Resolution};
    let mut kernel = BoxKernel::new();
    kernel.sections = true;
    let part = box_part(&kernel, "block", Vec3::new(120.0, 100.0, 50.0));
    let mut drawing = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let top = drawing.sheets[0]
        .views
        .iter()
        .find(|v| project::view_label_in(&drawing, &drawing.sheets[0], v) == "Top view")
        .unwrap()
        .id;
    let context = derived::context(&drawing, &part, &index, top).unwrap();
    let line = [Vec3::new(0.0, 30.0, 0.0), Vec3::new(60.0, 30.0, 0.0), Vec3::new(60.0, 100.0, 0.0)]
        .map(|p| context.xf.point(p));
    let (section, ctx) = derived::new_section_along(&drawing, &part, &index, 0, top, &line, false, None).unwrap();
    let (edge, _) = section
        .geometry
        .curves
        .iter()
        .enumerate()
        .find(|(_, c)| {
            let Resolution::Found(Entity::Edge(id)) = index.resolve(&c.source) else { return false };
            let Some(curve) = index.edge_curve(id) else { return false };
            let delta = curve.end() - curve.start();
            c.source.section_leg == 1 && delta.y.abs() > 99.0 && c.ends.is_some_and(|p| (p[1].u - p[0].u).abs() > 1.0)
        })
        .expect("the second leg unfolds a 100 mm model edge along the sheet's X");
    let proposal =
        propose(&section, &[Pick { view: section.id, curve: edge, end: None }], &ctx.projected.circles).unwrap();
    assert_eq!(proposal.anchors.iter().map(|a| a.attach).collect::<Vec<_>>(), [Attach::Start, Attach::End]);
    let source = &section.geometry.curves[edge].source;
    let Resolution::Found(Entity::Edge(model_edge)) = index.resolve(source) else { panic!("model edge") };
    let curve = index.edge_curve(model_edge).unwrap();
    let point = curve.point_at(if curve.start().y > curve.end().y { 0.1 } else { 0.9 });
    let selected = annotate::view_anchor_near(&index, &ctx, point).expect("aligned occurrence");
    assert_eq!(selected.entity.kernel_hint, source.kernel_hint, "selected {selected:?} at {point:?}");
    assert_eq!(selected.entity.section_leg, 1, "selected {selected:?} at {point:?}");
    let id = section.id;
    let mut stack = UndoStack::new();
    run(&mut stack, &mut drawing, &part, Command::PlaceView { sheet: 0, view: section });
    let annotation = soliddraft_drawing::Annotation {
        id: drawing.next_annotation_id(),
        view: id,
        body: AnnotationKind::Dimension(Dimension::measured(
            DimensionKind::Linear { orientation: LinearOrientation::Horizontal },
            100.0,
            0,
        )),
        anchors: proposal.anchors,
        placement: Point2::new(0.0, -30.0),
        style: Default::default(),
        state: AnnotationState::Ok,
        source: None,
    };
    let annotation_id = annotation.id;
    run(&mut stack, &mut drawing, &part, Command::AddDimension { annotation });
    let file = soliddraft_drawing::file::to_bytes(
        &drawing,
        &soliddraft_drawing::file::FileMeta::new("test", drawing.meta.units),
    )
    .unwrap();
    let (mut drawing, _) = soliddraft_drawing::file::from_bytes(&file).unwrap();
    soliddraft_drawing::regenerate_view(&mut drawing, &part, id).unwrap();
    let ann = drawing.annotation(annotation_id).unwrap();
    assert!(ann.anchors.iter().all(|anchor| anchor.entity.section_leg == 1));
    assert!(matches!(&ann.body, AnnotationKind::Dimension(d) if d.value == DimValue::Measured { value: 100.0 }));
}

#[test]
fn measuring_the_same_two_lines_does_not_depend_on_edge_direction() {
    use soliddraft_drawing::measure::{AnchorGeometry, measure};
    use soliddraft_drawing::{Attach, DimType, DimensionKind};
    use soliddraft_kernel_api::drawing::{Curve3, ViewSpec};
    let xf = project::ViewTransform::new(ViewSpec::new(Vec3::Z, Vec3::Y).frame().unwrap(), 1.0, Point2::default());
    let horizontal = Curve3::Line { a: Vec3::ZERO, b: Vec3::X };
    let diagonal = Vec3::new(-1.0, 1.0, 0.0);
    for (a, b) in [(Vec3::ZERO, diagonal), (diagonal, Vec3::ZERO)] {
        let other = Curve3::Line { a, b };
        let anchors = [&horizontal, &other].map(|curve| AnchorGeometry {
            curve: Some(curve),
            point: Vec3::ZERO,
            attach: Attach::Mid,
        });
        let angle = measure(&DimensionKind::Angular, DimType::Projected, &anchors, &xf).unwrap();
        assert!((angle - 45.0).abs() < 1e-9, "{angle}");
    }
}

#[test]
fn an_axial_section_refuses_when_only_its_mesh_cut_succeeds() {
    let mut kernel = BoxKernel::new();
    kernel.sections = true;
    kernel.refuse_exact_section = true;
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 50.0));
    let drawing = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let xf = derived::context(&drawing, &part, &index, FRONT).unwrap().xf;
    let trace = [Vec3::new(0.0, 0.0, 20.0), Vec3::new(120.0, 0.0, 20.0)].map(|p| xf.point(p));
    let error = derived::new_section(&drawing, &part, &index, 0, FRONT, trace, false, None).unwrap_err();
    assert!(matches!(error, soliddraft_drawing::DrawingError::ViewRefused { .. }));
    assert!(error.to_string().contains("test exact section refused"));

    let preview = Part { preview: true, ..part };
    let (_, context) = derived::new_section(&drawing, &preview, &index, 0, FRONT, trace, false, None).unwrap();
    assert!(!context.projected.geometry.hatch.is_empty());
}

#[test]
fn editing_a_break_preserves_the_material_inside_the_detail_callout() {
    let kernel = BoxKernel::new();
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 120.0));
    let index = PartIndex::build(&part).unwrap();
    for axis in [soliddraft_drawing::AlignAxis::Horizontal, soliddraft_drawing::AlignAxis::Vertical] {
        let mut drawing = drawing(&part);
        let mut stack = UndoStack::new();
        let station = |x| match axis {
            soliddraft_drawing::AlignAxis::Horizontal => Vec3::new(x, 0.0, 0.0),
            soliddraft_drawing::AlignAxis::Vertical => Vec3::new(0.0, 0.0, x),
        };
        run(
            &mut stack,
            &mut drawing,
            &part,
            Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Custom { factor: 1.0 } },
        );
        let parent = derived::context(&drawing, &part, &index, FRONT).unwrap();
        let callout = parent.xf.point(station(105.0));
        let (detail, _) =
            derived::new_detail(&drawing, &part, &index, 0, FRONT, callout, 15.0, Vec::new(), 2.0, None).unwrap();
        let id = detail.id;
        run(&mut stack, &mut drawing, &part, Command::PlaceView { sheet: 0, view: detail });
        for (interval, lower) in [(Some((10.0, 100.0)), 100.0), (Some((20.0, 98.0)), 98.0), (None, 90.0)] {
            let breaks = interval
                .into_iter()
                .map(|(a, b)| soliddraft_drawing::Break { from: station(a), to: station(b), axis })
                .collect();
            run(&mut stack, &mut drawing, &part, Command::SetViewBreaks { view: FRONT, breaks });
            let detail = derived::context(&drawing, &part, &index, id).unwrap();
            let mut points = detail
                .projected
                .geometry
                .curves
                .iter()
                .flat_map(|c| c.path.elements())
                .filter_map(|e| e.end_point())
                .map(|p| detail.xf.unproject(Point2::new(p.x, p.y)));
            let coordinate = |p: Vec3| match axis {
                soliddraft_drawing::AlignAxis::Horizontal => p.x,
                soliddraft_drawing::AlignAxis::Vertical => p.z,
            };
            let min = points.by_ref().map(coordinate).fold(f64::INFINITY, f64::min);
            assert!((min - lower).abs() < 1e-3, "{axis:?} {interval:?}: {min}");
        }
    }
}

#[test]
fn a_break_must_shorten_the_visible_cropped_and_inherited_spans() {
    let kernel = BoxKernel::new();
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 50.0));
    let index = PartIndex::build(&part).unwrap();
    let mut drawing = drawing(&part);
    let mut stack = UndoStack::new();
    let apply = |stack: &mut UndoStack, d: &mut Drawing, command| stack.run(d, &command, Some(&part));
    apply(
        &mut stack,
        &mut drawing,
        Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Custom { factor: 1.0 } },
    )
    .unwrap();
    let parent = derived::context(&drawing, &part, &index, FRONT).unwrap();
    let p = |x, z| parent.xf.point(Vec3::new(x, 0.0, z));
    let crop = Boundary::rectangle(p(75.0, -10.0), p(125.0, 60.0));
    apply(&mut stack, &mut drawing, Command::SetViewCrop { view: FRONT, crop: Some(crop) }).unwrap();
    let command = Command::SetViewBreaks {
        view: FRONT,
        breaks: vec![soliddraft_drawing::Break {
            from: Vec3::new(50.0, 0.0, 0.0),
            to: Vec3::new(60.0, 0.0, 0.0),
            axis: soliddraft_drawing::AlignAxis::Horizontal,
        }],
    };
    let before = drawing.clone();
    assert!(apply(&mut stack, &mut drawing, command.clone()).is_err());
    assert_eq!(drawing, before);
    apply(&mut stack, &mut drawing, Command::SetViewCrop { view: FRONT, crop: None }).unwrap();
    apply(&mut stack, &mut drawing, command).unwrap();
    let parent = derived::context(&drawing, &part, &index, FRONT).unwrap();
    let center = parent.xf.point(Vec3::new(55.0, 0.0, 0.0));
    assert!(derived::new_detail(&drawing, &part, &index, 0, FRONT, center, 30.0, Vec::new(), 0.2, None).is_err());
    let (detail, _) =
        derived::new_detail(&drawing, &part, &index, 0, FRONT, center, 30.0, Vec::new(), 2.0, None).unwrap();
    let view = detail.id;
    apply(&mut stack, &mut drawing, Command::PlaceView { sheet: 0, view: detail }).unwrap();
    assert!(
        apply(
            &mut stack,
            &mut drawing,
            Command::SetViewScale { view, scale: soliddraft_drawing::ViewScale::Custom { factor: 0.2 } }
        )
        .is_err()
    );
}

#[test]
fn dimension_proposals_and_commands_use_the_unbroken_model_edge() {
    use soliddraft_drawing::measure::{Pick, propose_model};
    use soliddraft_drawing::{Attach, DimValue, Dimension, DimensionKind, EntityKind};
    let kernel = BoxKernel::new();
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 50.0));
    let index = PartIndex::build(&part).unwrap();
    let mut drawing = drawing(&part);
    let mut stack = UndoStack::new();
    run(
        &mut stack,
        &mut drawing,
        &part,
        Command::SetViewScale { view: FRONT, scale: soliddraft_drawing::ViewScale::Custom { factor: 1.0 } },
    );
    run(
        &mut stack,
        &mut drawing,
        &part,
        Command::SetViewBreaks {
            view: FRONT,
            breaks: vec![soliddraft_drawing::Break {
                from: Vec3::new(40.0, 0.0, 0.0),
                to: Vec3::new(80.0, 0.0, 0.0),
                axis: soliddraft_drawing::AlignAxis::Horizontal,
            }],
        },
    );
    let context = derived::context(&drawing, &part, &index, FRONT).unwrap();
    let view = drawing.view(FRONT).unwrap();
    let edge = view
        .geometry
        .curves
        .iter()
        .position(|c| {
            c.visible && c.source.kind == EntityKind::Edge && c.ends.is_some_and(|[x, y]| (x.u - y.u).abs() > 85.0)
        })
        .unwrap();
    let picks = [Pick { view: FRONT, curve: edge, end: None }];
    let proposal = propose_model(view, &picks, &index, &context.xf).unwrap();
    let distance = proposal.distance(LinearOrientation::Horizontal, 1.0);
    assert!((distance - 120.0).abs() < 1e-6);
    let mut annotation = soliddraft_drawing::Annotation {
        id: drawing.next_annotation_id(),
        view: FRONT,
        body: AnnotationKind::Dimension(Dimension::measured(
            DimensionKind::Linear { orientation: LinearOrientation::Horizontal },
            distance,
            0,
        )),
        anchors: proposal.anchors,
        placement: Point2::new(0.0, -40.0),
        style: Default::default(),
        state: AnnotationState::Ok,
        source: None,
    };
    let id = annotation.id;
    run(&mut stack, &mut drawing, &part, Command::AddDimension { annotation: annotation.clone() });
    assert!(
        matches!(&drawing.annotation(id).unwrap().body, AnnotationKind::Dimension(d) if d.value == DimValue::Measured { value: 120.0 })
    );
    let topology = part.geometry.topology().unwrap();
    let face = index.face_ref(topology.faces[0].id).unwrap();
    annotation.id = drawing.next_annotation_id();
    for (anchor, attach) in annotation.anchors.iter_mut().zip([Attach::Start, Attach::End]) {
        anchor.entity = face.clone();
        anchor.attach = attach;
    }
    assert!(stack.run(&mut drawing, &Command::AddDimension { annotation }, Some(&part)).is_err());
}

#[test]
fn markup_targets_only_live_annotations_of_the_direct_item() {
    use soliddraft_drawing::{PmiSource, review};
    let mut drawing = soliddraft_drawing::fixture::bracket_a3();
    let annotation = drawing.sheets[0]
        .annotations
        .iter_mut()
        .find(|a| !a.anchors.is_empty() && matches!(a.state, AnnotationState::Ok))
        .unwrap();
    let id = annotation.id;
    let target = review::MarkupTarget { sheet: 0, annotation: id };
    let item = "The changed item";
    annotation.source = Some(PmiSource::Located { item: item.into(), datum: "A".into(), micrometres: 10000 });
    assert!(review::place_of_item(&drawing, item).is_none());
    assert!(review::find_place(&drawing, &target, item).is_none());
    drawing.annotation_mut(id).unwrap().source = Some(PmiSource::Item { item: item.into() });
    assert_eq!(review::place_of_item(&drawing, item), Some(target));
    assert!(review::find_place(&drawing, &target, item).is_some());
    let before = drawing.clone();
    drawing.annotation_mut(id).unwrap().state = AnnotationState::Dangling { lost: vec![0], cause: "missing".into() };
    assert!(review::place_of_item(&drawing, item).is_none());
    assert!(review::find_place(&drawing, &target, item).is_none());
    drawing = before;
    apply(&mut drawing, &Command::FreezeAsNote { annotation: id }).unwrap();
    apply(&mut drawing, &Command::EditNote { annotation: id, text: "Unrelated text".into() }).unwrap();
    assert!(drawing.annotation(id).unwrap().source.is_none());
    assert!(review::find_place(&drawing, &target, item).is_none());
}

#[test]
fn an_inclined_section_view_shows_its_mesh_cut_and_never_blocks_an_export() {
    use soliddraft_drawing::display::Owner;
    let mut kernel = BoxKernel::new();
    kernel.sections = true;
    kernel.refuse_exact_section = true;
    kernel.inclined_mesh_cuts = true;
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 50.0));
    let mut drawing = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let top = drawing.sheets[0]
        .views
        .iter()
        .find(|v| project::view_label_in(&drawing, &drawing.sheets[0], v) == "Top view")
        .unwrap()
        .id;
    let xf = derived::context(&drawing, &part, &index, top).unwrap().xf;
    // The plane x = y, square to no model axis.
    let trace = [Vec3::new(0.0, 0.0, 50.0), Vec3::new(50.0, 50.0, 50.0)].map(|p| xf.point(p));
    let (section, _) = derived::new_section(&drawing, &part, &index, 0, top, trace, false, None).unwrap();
    assert!(section.geometry.display_only && !section.geometry.unproved && !section.geometry.catallax);
    assert!(!section.has_saved_catallax_linework());
    assert!(!section.geometry.curves.is_empty(), "the mesh cut shows on screen");
    let id = section.id;
    apply(&mut drawing, &Command::PlaceView { sheet: 0, view: section }).unwrap();
    export::check_sheet(&drawing, &drawing.sheets[0]).unwrap();

    // Left out of updates while the part grows and the drawing rebuilds, then put back
    // without a rebuild: the section keeps the older revision.
    apply(&mut drawing, &Command::SetViewExcluded { view: id, excluded: true }).unwrap();
    let before = drawing.model.pinned.clone();
    let changed = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 60.0));
    let report = update_from_model(&mut drawing, &changed).unwrap();
    assert!(report.dangling_views.is_empty(), "{:?}", report.dangling_views);
    apply(&mut drawing, &Command::SetViewExcluded { view: id, excluded: false }).unwrap();
    assert_ne!(drawing.model.pinned, before);
    let section = drawing.view(id).unwrap();
    assert_eq!(section.geometry.source_digest, before.as_str());
    assert!(!section.accepts_revision(drawing.model.pinned.as_str()));
    export::check_sheet(&drawing, &drawing.sheets[0]).unwrap();
    let list = layout_sheet(&drawing, 0);
    assert!(list.items.iter().any(|i| matches!(i.owner(), Owner::Curve(v, _) if v == id)));
    assert!(list.printable().all(|i| !matches!(i.owner(), Owner::Curve(v, _) | Owner::View(v) if v == id)));
    let svg = String::from_utf8(export::bytes(&list, export::Format::Svg, "Box").unwrap()).unwrap();
    assert!(svg.contains(r#"<g id="VISIBLE""#), "the other views print");
    assert!(!svg.contains(r#"<g id="HATCH""#) && !svg.contains(r#"<g id="CUTTING""#));
}

#[test]
fn inclined_half_and_broken_out_sections_refuse_when_only_their_mesh_cut_succeeds() {
    use soliddraft_drawing::{BrokenOut, HalfSection};
    let mut kernel = BoxKernel::new();
    kernel.sections = true;
    kernel.refuse_exact_section = true;
    kernel.inclined_mesh_cuts = true;
    let part = box_part(&kernel, "block", Vec3::new(120.0, 50.0, 50.0));
    let drawing = drawing(&part);
    let index = PartIndex::build(&part).unwrap();
    let iso = drawing.sheets[0]
        .views
        .iter()
        .find(|v| project::view_spec(&drawing, v.id).is_ok_and(|spec| project::is_pictorial(&spec)))
        .unwrap()
        .id;
    let xf = derived::context(&drawing, &part, &index, iso).unwrap().xf;
    let centre = Vec3::new(60.0, 25.0, 25.0);

    let mut cut = drawing.clone();
    cut.view_mut(iso)
        .unwrap()
        .broken_out
        .push(BrokenOut { boundary: Boundary::Circle { center: xf.point(centre), radius: 15.0 }, depth: 10.0 });
    let error = derived::context(&cut, &part, &index, iso).map(|_| ()).unwrap_err();
    assert!(matches!(error, soliddraft_drawing::DrawingError::ViewRefused { .. }), "{error}");
    assert!(error.to_string().contains("test exact section refused"), "{error}");

    let across = xf.frame.direction.cross(Vec3::Z).normalized().unwrap();
    let mut half = drawing.clone();
    half.view_mut(iso).unwrap().half = Some(HalfSection { axis: [centre, centre + across * 40.0], flip: false });
    let error = derived::context(&half, &part, &index, iso).map(|_| ()).unwrap_err();
    assert!(matches!(error, soliddraft_drawing::DrawingError::ViewRefused { .. }), "{error}");
    assert!(error.to_string().contains("test exact section refused"), "{error}");

    // The view itself still draws from the exact path.
    let view = derived::context(&drawing, &part, &index, iso).unwrap();
    assert!(!view.projected.geometry.display_only && view.projected.geometry.catallax);
}
