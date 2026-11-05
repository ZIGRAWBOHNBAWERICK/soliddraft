//! Drawing edits keep picking data and attached measurements with their linework.

use soliddraft_app::document::{Command as PartCommand, Document, ExtrudeParams, ProfileKind};
use soliddraft_app::drawing::DrawingDoc;
use soliddraft_app::drawing::part::PartContext;
use soliddraft_drawing::measure::{Pick, propose};
use soliddraft_drawing::{
    Annotation, AnnotationKind, AnnotationState, Command, Dimension, DimensionKind, LinearOrientation, PartIndex,
    Scale, ViewId, ViewKind, ViewScale, annotate, derived, project,
};
use soliddraft_kernel_api::{Point2, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use std::sync::Arc;

fn part(profile: ProfileKind) -> Document {
    let mut doc = Document::new("Drawing edits", Arc::new(CatallaxKernel::new()));
    doc.apply(PartCommand::AddExtrude {
        params: ExtrudeParams { profile, depth: 10.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    doc
}

fn drawing(doc: &Document) -> DrawingDoc {
    let ctx = PartContext::new(doc).unwrap();
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
        scale: Some(Scale::FULL),
    };
    let made = soliddraft_drawing::make_drawing(&ctx.certified_part(), &new).unwrap();
    let mut drawing = DrawingDoc::new(made.drawing);
    for (id, p) in &made.projections {
        drawing.remember(*id, p);
    }
    drawing
}

#[test]
fn circle_picks_follow_sheet_view_and_inherited_scales_through_history() {
    let doc = part(ProfileKind::Circle { diameter: 10.0 });
    let original = drawing(&doc);
    let top = ViewId(2);
    assert!(!original.circles[&top].is_empty());
    for command in [
        Command::SetSheetScale { sheet: 0, scale: Scale::new(1, 2) },
        Command::SetViewScale { view: top, scale: ViewScale::Custom { factor: 0.5 } },
        Command::SetViewScale { view: ViewId(1), scale: ViewScale::Custom { factor: 0.5 } },
    ] {
        let mut drawing = drawing(&doc);
        drawing.apply(command, None).unwrap();
        for expected_scale in [0.5, 1.0, 0.5] {
            let view = drawing.drawing.view(top).unwrap();
            let scale = project::view_scale(&drawing.drawing, top).unwrap();
            assert_eq!(scale, expected_scale);
            let circles = &drawing.circles[&top];
            let curve = view
                .geometry
                .curves
                .iter()
                .position(|c| c.source.kernel_hint == circles[0].entity.kernel_hint)
                .unwrap();
            let pick = Pick { view: top, curve, end: None };
            let proposal = propose(view, &[pick], circles).unwrap();
            assert_eq!(proposal.kind, DimensionKind::Diameter);
            assert!(
                (proposal.radius * 2.0 / scale - 10.0).abs() < 1e-6,
                "cached radius {} at scale {scale}",
                proposal.radius
            );
            let xf = drawing.transforms[&top];
            assert_eq!(xf.scale, scale);
            assert_eq!(proposal.centre, xf.point(circles[0].model_centre));
            if expected_scale == 0.5 && drawing.can_undo() {
                drawing.undo().unwrap();
            } else if drawing.can_redo() {
                drawing.redo().unwrap();
            }
        }
    }
}

#[test]
fn round_dimension_repairs_accept_circles_and_remeasure_the_value() {
    let doc = part(ProfileKind::Circle { diameter: 10.0 });
    let ctx = PartContext::new(&doc).unwrap();
    let part = ctx.certified_part();
    for (kind, value) in [(DimensionKind::Diameter, 10.0), (DimensionKind::Radius, 5.0)] {
        let mut d = drawing(&doc);
        let view = ViewId(2);
        let circle = d.circles[&view][0].clone();
        let mut entity = circle.entity.clone();
        entity.feature = "deleted_hole".into();
        let a = Annotation {
            id: d.drawing.next_annotation_id(),
            view,
            body: AnnotationKind::Dimension(Dimension::measured(kind, 42.0, 0)),
            anchors: vec![soliddraft_drawing::Anchor {
                entity,
                attach: soliddraft_drawing::Attach::Centre,
                cached: circle.centre,
            }],
            placement: Point2::new(20.0, 20.0),
            style: Default::default(),
            state: AnnotationState::Dangling { lost: vec![0], cause: "hole was deleted".into() },
        };
        let id = a.id;
        d.apply(Command::AddDimension { annotation: a }, None).unwrap();
        soliddraft_drawing::apply_with(
            &mut d.drawing,
            &Command::Reattach { annotation: id, end: 0, entity: circle.entity },
            &part,
        )
        .unwrap();
        soliddraft_drawing::update_from_model(&mut d.drawing, &part).unwrap();
        let a = d.drawing.annotation(id).unwrap();
        assert_eq!(a.state, AnnotationState::Ok);
        let AnnotationKind::Dimension(dim) = &a.body else { panic!() };
        assert_eq!(dim.value, soliddraft_drawing::DimValue::Measured { value });
        let reopened = DrawingDoc::from_bytes(&d.to_bytes("catallax", false).unwrap()).unwrap();
        assert_eq!(reopened.drawing, d.drawing);
    }
}

#[test]
fn off_origin_circle_centres_follow_the_transform_after_rescaling() {
    let doc = soliddraft_app::command_tools::board_bracket();
    let mut d = drawing(&doc);
    let before = d.circles.clone();
    assert!(before.values().flatten().any(|c| c.centre.u.abs() + c.centre.v.abs() > 1.0));
    d.apply(Command::SetSheetScale { sheet: 0, scale: Scale::new(1, 2) }, None).unwrap();
    for (id, circles) in &before {
        let xf = d.transforms[id];
        for (old, now) in circles.iter().zip(&d.circles[id]) {
            assert_eq!(now.model_radius, old.model_radius);
            assert!((now.radius - old.radius / 2.0).abs() < 1e-9);
            assert_eq!(now.centre, xf.point(now.model_centre));
        }
    }
}

#[test]
fn section_flip_and_redraw_refresh_anchors_values_and_exports_atomically() {
    let doc = part(ProfileKind::Rectangle { width: 40.0, height: 30.0 });
    let ctx = PartContext::new(&doc).unwrap();
    let part = ctx.certified_part();
    let index = PartIndex::build(&part).unwrap();
    let mut d = drawing(&doc);
    let top = ViewId(2);
    let line = derived::cutting_line(&d.drawing, &part, &index, top, Vec3::ZERO, true).unwrap();
    let (view, section) = derived::new_section(&d.drawing, &part, &index, 0, top, line, false, None).unwrap();
    let id = view.id;
    d.apply(Command::PlaceView { sheet: 0, view }, Some(&ctx)).unwrap();
    d.remember(id, &section.projected);
    let points = [Vec3::new(-20.0, -15.0, 0.0), Vec3::new(20.0, 15.0, 10.0)];
    let anchors = points.map(|p| annotate::anchor_near(&index, &section.xf, p).unwrap()).to_vec();
    let annotation = Annotation {
        id: d.drawing.next_annotation_id(),
        view: id,
        body: AnnotationKind::Dimension(Dimension::measured(
            DimensionKind::Linear { orientation: LinearOrientation::Horizontal },
            40.0,
            0,
        )),
        anchors,
        placement: Point2::new(0.0, 20.0),
        style: Default::default(),
        state: AnnotationState::Ok,
    };
    let aid = annotation.id;
    d.apply(Command::AddDimension { annotation }, None).unwrap();
    let mut history = soliddraft_drawing::UndoStack::new();
    for redraw in [false, true] {
        let before = d.drawing.clone();
        let mut kind = d.drawing.view(id).unwrap().kind.clone();
        let ViewKind::Section { flip, line, model_line, .. } = &mut kind else { panic!() };
        if redraw {
            *line = derived::cutting_line(&d.drawing, &part, &index, top, Vec3::ZERO, false).unwrap().to_vec();
            let parent = derived::context(&d.drawing, &part, &index, top).unwrap();
            *model_line = line.iter().map(|p| parent.xf.unproject(*p)).collect();
        } else {
            *flip = !*flip;
        }
        history.run(&mut d.drawing, &Command::EditView { view: id, kind }, Some(&part)).unwrap();
        let expected = project::ViewContext::compute(&d.drawing, &part, &index, id).unwrap();
        let a = d.drawing.annotation(aid).unwrap();
        for (anchor, point) in a.anchors.iter().zip(points) {
            assert_eq!(anchor.cached, expected.xf.point(point));
        }
        let AnnotationKind::Dimension(dim) = &a.body else { panic!() };
        let value = if redraw { 10.0 } else { 40.0 };
        assert_eq!(dim.value, soliddraft_drawing::DimValue::Measured { value });
        if redraw {
            assert_eq!(a.state, AnnotationState::Changed { previous: "40".into() });
        }
        let after = d.drawing.clone();
        history.undo(&mut d.drawing).unwrap();
        assert_eq!(d.drawing, before);
        history.redo(&mut d.drawing).unwrap();
        assert_eq!(d.drawing, after);
        let bytes = d.to_bytes("catallax", false).unwrap();
        let reopened = DrawingDoc::from_bytes(&bytes).unwrap();
        assert_eq!(reopened.drawing, after);
        let list = soliddraft_drawing::display::layout_sheet(&reopened.drawing, 0);
        assert_eq!(list.dimensions.iter().find(|r| r.owner == aid).unwrap().value, value);
        assert!(soliddraft_drawing::export::svg(&list).contains(&format!(">{value}</text>")));
        soliddraft_drawing::export::dxf(&list).unwrap();
    }
}
