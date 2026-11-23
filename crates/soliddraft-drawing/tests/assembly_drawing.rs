//! Drawings of assemblies: boxes from a kernel of boxes, placed, drawn with a bill of
//! materials and balloons, saved and read back, exported, and brought up to date after a
//! part moves and another goes; then projected, section and detail views of them.

mod support;

use soliddraft_drawing::assembly::{
    AssemblyModel, BomRow, NewAssemblyDrawing, make_assembly_drawing, new_assembly_detail, new_assembly_projected,
    new_assembly_section, update_assembly_drawing,
};
use soliddraft_drawing::display::{Item, Owner, layout_sheet};
use soliddraft_drawing::file::{FileMeta, from_bytes, to_bytes};
use soliddraft_drawing::placed::{PlacedPart, component_of};
use soliddraft_drawing::project;
use soliddraft_drawing::{
    AlignAxis, AnnotationKind, AnnotationState, Boundary, Command, ContentHash, CurveKind, DocKind, DocRef, DocumentId,
    Drawing, DrawingMeta, FoldDirection, Rigid, Scale, TableKind, Template, Units, ViewKind, apply,
};
use soliddraft_kernel_api::Vec3;
use soliddraft_kernel_api::drawing::StandardView;
use support::BoxKernel;

/// A box of `size` with a corner at the origin, placed with that corner at `at`.
fn cuboid<'k>(k: &'k BoxKernel, path: &str, at: Vec3, size: Vec3) -> PlacedPart<'k> {
    let body = k.add("f1", Vec3::ZERO, size);
    PlacedPart {
        kernel: k,
        body,
        placement: Rigid { translation: at, ..Rigid::IDENTITY },
        component: path.into(),
        label: format!("Part<{path}>"),
    }
}

fn model<'k>(revision: &[u8], parts: Vec<(PlacedPart<'k>, u32)>) -> AssemblyModel<'k> {
    let reference = DocRef::new(DocumentId::new(), DocKind::Assembly, "rig.dwasm", ContentHash::of_bytes(revision));
    let mut bom = vec![
        BomRow { item: 1, name: "Base".into(), file: "parts/base.dwprt".into(), quantity: 1 },
        BomRow { item: 2, name: "Post".into(), file: "parts/post.dwprt".into(), quantity: 0 },
    ];
    bom[1].quantity = parts.iter().filter(|(_, i)| *i == 2).count() as u32;
    let items = parts.iter().map(|(_, i)| *i).collect();
    AssemblyModel {
        reference: Some(reference),
        parts: parts.into_iter().map(|(p, _)| p).collect(),
        items,
        exploded: Vec::new(),
        bom,
    }
}

fn base(k: &BoxKernel) -> (PlacedPart<'_>, u32) {
    (cuboid(k, "2", Vec3::ZERO, Vec3::new(100.0, 60.0, 10.0)), 1)
}

fn post<'k>(k: &'k BoxKernel, path: &str, x: f64) -> (PlacedPart<'k>, u32) {
    (cuboid(k, path, Vec3::new(x, 20.0, 10.0), Vec3::new(20.0, 20.0, 60.0)), 2)
}

fn texts(drawing: &soliddraft_drawing::Drawing) -> Vec<String> {
    layout_sheet(drawing, 0)
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_assembly_drawing_has_views_a_bom_and_balloons_that_follow_the_assembly() {
    let template = Template::builtin().remove(0);
    let new = NewAssemblyDrawing {
        template,
        meta: DrawingMeta { title: "Rig".into(), ..DrawingMeta::default() },
        views: vec![StandardView::Isometric, StandardView::Front],
        scale: None,
        balloons: true,
        exploded: false,
    };
    let k = BoxKernel::new();
    let first = model(b"r1", vec![base(&k), post(&k, "3", 10.0), post(&k, "4", 70.0)]);
    let mut drawing = make_assembly_drawing(&first, &new).unwrap();
    assert_eq!(drawing.model.kind, DocKind::Assembly);
    assert!(drawing.views_match_pin());

    // Both views have line work, each curve owned by an occurrence; the posts hide part
    // of the base's far edges in the front view, so some line work is hidden.
    let sheet = &drawing.sheets[0];
    assert_eq!(sheet.views.len(), 2);
    for v in &sheet.views {
        assert!(!v.geometry.curves.is_empty());
        assert!(
            v.geometry
                .curves
                .iter()
                .all(|c| component_of(&c.source.feature).is_some_and(|o| ["2", "3", "4"].contains(&o)))
        );
        assert!(v.geometry.catallax, "every part's line work is its kernel's own");
    }
    let front = &sheet.views[1].geometry;
    assert!(front.curves.iter().any(|c| !c.visible), "the posts hide some of the base");

    // The BOM lists both parts with their quantities, and each row has a balloon.
    let bom = sheet.tables.iter().find(|t| t.kind == TableKind::Bom).unwrap();
    assert_eq!(bom.bom.iter().map(|r| (r.item, r.quantity)).collect::<Vec<_>>(), vec![(1, 1), (2, 2)]);
    assert_eq!(bom.bom.iter().map(|r| r.file.as_str()).collect::<Vec<_>>(), ["base.dwprt", "post.dwprt"]);
    let balloons: Vec<(u32, String)> = sheet
        .annotations
        .iter()
        .filter_map(|a| match &a.body {
            AnnotationKind::Balloon { item, occurrence } => Some((*item, occurrence.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(balloons, vec![(1, "2".to_owned()), (2, "3".to_owned())]);
    let shown = texts(&drawing);
    for t in ["BILL OF MATERIALS", "Base", "Post", "base.dwprt", "1", "2"] {
        assert!(shown.iter().any(|s| s == t), "{t} is on the sheet: {shown:?}");
    }
    // Balloons sit outside the view and clear of each other.
    let iso = &sheet.views[0];
    let ps: Vec<_> = sheet.annotations.iter().filter(|a| a.view == iso.id).map(|a| a.placement).collect();
    let [x0, y0, x1, y1] = iso.geometry.bbox;
    for p in &ps {
        assert!(p.u < x0 || p.u > x1 || p.v < y0 || p.v > y1, "{p:?} outside {:?}", iso.geometry.bbox);
    }
    assert!(((ps[0].u - ps[1].u).powi(2) + (ps[0].v - ps[1].v).powi(2)).sqrt() > 9.0);

    // Saved and read back, the drawing is the same; it exports with its table.
    let meta = FileMeta::new("boxes", Units::Millimetre);
    let bytes = to_bytes(&drawing, &meta).unwrap();
    let (back, _) = from_bytes(&bytes).unwrap();
    assert_eq!(back, drawing);

    let mut older = serde_json::to_value(&drawing).unwrap();
    older["sheets"][0]["tables"][0]["bom"][0]["file"] = serde_json::json!(r"C:\private\base.dwprt");
    let loaded = Drawing::from_json(&older.to_string()).unwrap();
    assert_eq!(loaded.sheets[0].tables[0].bom, bom.bom);

    soliddraft_drawing::export::check_sheet(&back, &back.sheets[0]).expect("the kernels' line work exports");
    let svg = soliddraft_drawing::export::svg::svg(&layout_sheet(&back, 0));
    assert!(svg.contains("BILL OF MATERIALS"));

    // A later change: the second post moves, the first goes, a third part arrives.
    let mut parts = vec![base(&k), post(&k, "4", 40.0)];
    parts.push((cuboid(&k, "5", Vec3::new(0.0, 0.0, 70.0), Vec3::new(100.0, 60.0, 5.0)), 3));
    let mut second = model(b"r2", parts);
    second.reference.as_mut().unwrap().id = first.reference.as_ref().unwrap().id;
    second.bom.push(BomRow { item: 3, name: "Lid".into(), file: "parts/lid.dwprt".into(), quantity: 1 });
    let updated = update_assembly_drawing(&drawing, &second).unwrap();
    apply(&mut drawing, &Command::UpdateFromAssembly { drawing: Box::new(updated) }).unwrap();
    assert!(drawing.views_match_pin());
    let sheet = &drawing.sheets[0];
    let state_of = |occurrence: &str| {
        sheet
            .annotations
            .iter()
            .find(|a| matches!(&a.body, AnnotationKind::Balloon { occurrence: o, .. } if o == occurrence))
            .map(|a| a.state.clone())
    };
    assert!(matches!(state_of("3"), Some(AnnotationState::Dangling { .. })), "the post it pointed at is gone");
    assert_eq!(state_of("5"), Some(AnnotationState::Ok), "the new part gets a balloon");
    assert_eq!(sheet.tables[0].bom.len(), 3);
    assert_eq!(
        sheet.tables[0].bom.iter().map(|row| row.file.as_str()).collect::<Vec<_>>(),
        ["base.dwprt", "post.dwprt", "lid.dwprt"]
    );
    let (reopened, _) = from_bytes(&to_bytes(&drawing, &meta).unwrap()).unwrap();
    assert_eq!(reopened.sheets[0].tables[0].bom, sheet.tables[0].bom);
    assert!(texts(&drawing).iter().any(|t| t == "Lid"));
}

/// Hatched areas of a view, in view square millimetres, largest first.
fn hatched(view: &soliddraft_drawing::View) -> Vec<f64> {
    let ring = |ps: &[soliddraft_kernel_api::Point2]| {
        let n = ps.len();
        ((0..n).map(|i| ps[i].u * ps[(i + 1) % n].v - ps[(i + 1) % n].u * ps[i].v).sum::<f64>() / 2.0).abs()
    };
    let mut areas: Vec<f64> =
        view.geometry.hatch.iter().map(|h| ring(&h.outer) - h.holes.iter().map(|l| ring(l)).sum::<f64>()).collect();
    areas.sort_by(|a, b| b.total_cmp(a));
    areas
}

fn owners(view: &soliddraft_drawing::View, kind: CurveKind) -> Vec<String> {
    let mut out: Vec<String> = view
        .geometry
        .curves
        .iter()
        .filter(|c| c.kind == kind)
        .filter_map(|c| component_of(&c.source.feature).map(str::to_owned))
        .collect();
    out.sort();
    out.dedup();
    out
}

#[test]
fn assembly_drawings_take_projected_section_and_detail_views_that_follow_the_assembly() {
    let template = Template::builtin().remove(0);
    let new = NewAssemblyDrawing {
        template,
        meta: DrawingMeta { title: "Rig".into(), ..DrawingMeta::default() },
        views: vec![StandardView::Front],
        // Full size leaves room beside the front for the derived views.
        scale: Some(Scale::FULL),
        balloons: true,
        exploded: false,
    };
    let mut k = BoxKernel::new();
    k.sections = true;
    let first = model(b"r1", vec![base(&k), post(&k, "3", 10.0), post(&k, "4", 70.0)]);
    let mut drawing = make_assembly_drawing(&first, &new).unwrap();
    let front = drawing.sheets[0].views[0].id;
    let scale = project::view_scale(&drawing, front).unwrap();

    // A projected view folded to the right of the front, aligned with it.
    let side = new_assembly_projected(&drawing, &first, 0, front, FoldDirection::Right).unwrap();
    assert!(matches!(side.kind, ViewKind::Projected { parent, .. } if parent == front));
    assert_eq!(side.alignment.map(|a| a.axis), Some(AlignAxis::Horizontal));
    assert!(side.position.u > drawing.sheets[0].views[0].position.u);
    assert_eq!(owners(&side, CurveKind::Edge), ["2", "3", "4"]);
    let side_id = side.id;
    apply(&mut drawing, &Command::PlaceView { sheet: 0, view: side }).unwrap();

    // Section A-A: a vertical cut through the first post, at x = 20. It cuts the base
    // (60 by 10) and the post (20 by 60), hatched one region each, and draws their cut
    // outlines; the second post is whole.
    let section = new_assembly_section(&drawing, &first, 0, front, Vec3::new(20.0, 30.0, 40.0), false, false).unwrap();
    assert!(matches!(&section.kind, ViewKind::Section { label, .. } if label == "A"));
    let areas = hatched(&section);
    assert_eq!(areas.len(), 2, "{areas:?}");
    let s2 = scale * scale;
    assert!((areas[0] - 1200.0 * s2).abs() < 1e-6 * areas[0], "{areas:?} at scale {scale}");
    assert!((areas[1] - 600.0 * s2).abs() < 1e-6 * areas[1], "{areas:?}");
    assert_eq!(owners(&section, CurveKind::Section), ["2", "3"]);
    let section_id = section.id;
    apply(&mut drawing, &Command::PlaceView { sheet: 0, view: section }).unwrap();

    // Detail B: the foot of the first post's right side, twice the front's size, inside its circle.
    // The circle reaches 6 model millimetres round a point 4 short of the post's side.
    let reach = 6.0 * scale;
    let detail = new_assembly_detail(&drawing, &first, 0, front, Vec3::new(26.0, 30.0, 10.0), reach, 2.0).unwrap();
    assert!(matches!(&detail.kind, ViewKind::Detail { label, .. } if label == "B"));
    let ratio = project::scale_in(&drawing, 0, &detail) / scale;
    assert!((ratio - 2.0).abs() < 1e-12);
    let Some(Boundary::Circle { center, radius }) = detail.geometry.outline.clone() else { panic!("a circle") };
    assert!(!detail.geometry.curves.is_empty());
    for c in &detail.geometry.curves {
        for el in c.path.elements() {
            if let Some(p) = el.end_point() {
                assert!((p.x - center.u).hypot(p.y - center.v) <= radius + 1e-6, "inside the circle");
            }
        }
    }
    assert_eq!(owners(&detail, CurveKind::Edge), ["2", "3"]);
    let detail_id = detail.id;
    apply(&mut drawing, &Command::PlaceView { sheet: 0, view: detail }).unwrap();

    // A cut that meets no part is refused by name.
    let err = new_assembly_section(&drawing, &first, 0, front, Vec3::new(500.0, 30.0, 40.0), false, false).unwrap_err();
    assert!(err.to_string().contains("no part"), "{err}");

    // Saved and read back, the views are the same.
    let meta = FileMeta::new("boxes", Units::Millimetre);
    let (mut drawing, _) = from_bytes(&to_bytes(&drawing, &meta).unwrap()).unwrap();
    assert_eq!(drawing.sheets[0].views.len(), 4);

    // The first post moves clear of the cut: the section now cuts the base alone, the
    // detail shows the base alone and the projected view follows.
    let mut second = model(b"r2", vec![base(&k), post(&k, "3", 40.0), post(&k, "4", 70.0)]);
    second.reference.as_mut().unwrap().id = first.reference.as_ref().unwrap().id;
    let updated = update_assembly_drawing(&drawing, &second).unwrap();
    apply(&mut drawing, &Command::UpdateFromAssembly { drawing: Box::new(updated) }).unwrap();
    assert!(drawing.views_match_pin());
    let view = |id| drawing.view(id).unwrap().clone();
    assert_eq!(hatched(&view(section_id)).len(), 1);
    assert_eq!(owners(&view(section_id), CurveKind::Section), ["2"]);
    assert_eq!(owners(&view(detail_id), CurveKind::Edge), ["2"]);
    assert_eq!(owners(&view(side_id), CurveKind::Edge), ["2", "3", "4"]);

    // Later work on top: a detail of the section keeps the hatch, clipped to its circle.
    let more = new_assembly_detail(&drawing, &second, 0, section_id, Vec3::new(20.0, 30.0, 5.0), 10.0, 2.0).unwrap();
    assert!(!more.geometry.hatch.is_empty());
    assert!(more.geometry.hatch.iter().all(|h| h.clip.is_some()));
    apply(&mut drawing, &Command::PlaceView { sheet: 0, view: more }).unwrap();
    let svg = soliddraft_drawing::export::svg::svg(&layout_sheet(&drawing, 0));
    assert!(svg.contains("SECTION A-A") || svg.contains("A-A"), "the section is labelled");
}

/// A drawing of `model` on `template` with the views asked for, ballooned.
fn drawing_of(
    model: &AssemblyModel<'_>,
    template: &str,
    views: Vec<StandardView>,
    scale: Option<Scale>,
) -> soliddraft_drawing::Drawing {
    let new = NewAssemblyDrawing {
        template: Template::by_id(template).unwrap(),
        meta: DrawingMeta { title: "Rig".into(), ..DrawingMeta::default() },
        views,
        scale,
        balloons: true,
        exploded: false,
    };
    make_assembly_drawing(model, &new).unwrap()
}

/// The same assembly at a later revision.
fn later<'k>(first: &AssemblyModel<'_>, revision: &[u8], parts: Vec<(PlacedPart<'k>, u32)>) -> AssemblyModel<'k> {
    let mut next = model(revision, parts);
    next.reference.as_mut().unwrap().id = first.reference.as_ref().unwrap().id;
    next
}

#[test]
fn a_derived_view_refuses_an_assembly_revision_its_drawing_does_not_show() {
    let k = BoxKernel::new();
    let first = model(b"r1", vec![base(&k), post(&k, "3", 10.0)]);
    let drawing = drawing_of(&first, "iso-a3", vec![StandardView::Front], Some(Scale::FULL));
    let front = drawing.sheets[0].views[0].id;
    // The assembly changed since the drawing was made: its views still show r1.
    let second = later(&first, b"r2", vec![base(&k), post(&k, "3", 60.0)]);
    let at = Vec3::new(20.0, 30.0, 40.0);
    for refused in [
        new_assembly_projected(&drawing, &second, 0, front, FoldDirection::Right).map(|_| ()),
        new_assembly_section(&drawing, &second, 0, front, at, false, false).map(|_| ()),
        new_assembly_detail(&drawing, &second, 0, front, at, 6.0, 2.0).map(|_| ()),
    ] {
        let err = refused.expect_err("a view of r2 beside views of r1 is refused");
        assert!(err.to_string().contains("Rebuild"), "{err}");
    }
    // At the revision it shows, the same views come.
    new_assembly_projected(&drawing, &first, 0, front, FoldDirection::Right).unwrap();
}

#[test]
fn a_rebuild_leaves_a_view_excluded_from_updates_as_it_was() {
    let mut k = BoxKernel::new();
    k.sections = true;
    let first = model(b"r1", vec![base(&k), post(&k, "3", 10.0), post(&k, "4", 70.0)]);
    let mut drawing = drawing_of(&first, "iso-a3", vec![StandardView::Front], Some(Scale::FULL));
    let front = drawing.sheets[0].views[0].id;
    // A section through the first post, cut from the front: computing it reads the front.
    let at = Vec3::new(20.0, 30.0, 40.0);
    let section = new_assembly_section(&drawing, &first, 0, front, at, false, false).unwrap();
    let section_id = section.id;
    apply(&mut drawing, &Command::PlaceView { sheet: 0, view: section }).unwrap();
    drawing.sheets[0].views[0].excluded_from_update = true;
    let kept = drawing.sheets[0].views[0].clone();
    let balloons: Vec<_> = drawing.sheets[0].annotations.iter().filter(|a| a.view == front).cloned().collect();
    assert!(!balloons.is_empty());

    // The first post moves; the front keeps its line work and its balloons.
    let second = later(&first, b"r2", vec![base(&k), post(&k, "3", 40.0), post(&k, "4", 70.0)]);
    let updated = update_assembly_drawing(&drawing, &second).unwrap();
    assert_eq!(updated.view(front).unwrap(), &kept, "the excluded view is not rewritten");
    let after: Vec<_> = updated.sheets[0].annotations.iter().filter(|a| a.view == front).cloned().collect();
    assert_eq!(after, balloons, "nor are its balloons");
    // The section cut from it follows the assembly; the excluded view keeps the old revision.
    assert_eq!(updated.view(section_id).unwrap().geometry.source_digest, second.digest().as_str());
    assert_eq!(updated.view(front).unwrap().geometry.source_digest, first.digest().as_str());
}

#[test]
fn a_balloon_whose_part_went_claims_no_item_and_the_new_row_gets_its_own() {
    let k = BoxKernel::new();
    let first = model(b"r1", vec![base(&k), post(&k, "3", 10.0)]);
    let mut drawing = drawing_of(&first, "iso-a3", vec![StandardView::Isometric], None);
    let gone = drawing.sheets[0]
        .annotations
        .iter()
        .find(|a| matches!(&a.body, AnnotationKind::Balloon { occurrence, .. } if occurrence == "2"))
        .map(|a| a.id)
        .unwrap();

    // The base goes and a lid takes its place as item 1; the post stays item 2.
    let lid = (cuboid(&k, "5", Vec3::new(0.0, 0.0, 70.0), Vec3::new(100.0, 60.0, 5.0)), 1);
    let mut second = later(&first, b"r2", vec![lid, post(&k, "3", 10.0)]);
    second.bom[0] = BomRow { item: 1, name: "Lid".into(), file: "parts/lid.dwprt".into(), quantity: 1 };
    let updated = update_assembly_drawing(&drawing, &second).unwrap();
    apply(&mut drawing, &Command::UpdateFromAssembly { drawing: Box::new(updated) }).unwrap();
    let sheet = &drawing.sheets[0];
    let lid_balloon = sheet
        .annotations
        .iter()
        .find(|a| matches!(&a.body, AnnotationKind::Balloon { occurrence, .. } if occurrence == "5"));
    let lid_balloon = lid_balloon.expect("the lid's row gets a balloon of its own");
    assert!(matches!(lid_balloon.body, AnnotationKind::Balloon { item: 1, .. }));
    assert_eq!(lid_balloon.state, AnnotationState::Ok);
    let dangling = sheet.annotations.iter().find(|a| a.id == gone).unwrap();
    assert!(matches!(dangling.state, AnnotationState::Dangling { .. }));
    // On the sheet the dangling balloon shows no item number.
    let shown: Vec<String> = layout_sheet(&drawing, 0)
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Text(t) if t.owner == Owner::Annotation(gone) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(shown, ["?"], "the balloon of a part that went claims no BOM item");
}

/// The distance from `p` to the nearest point of `path`.
fn distance_to(path: &kurbo::BezPath, p: soliddraft_kernel_api::Point2) -> f64 {
    use kurbo::ParamCurveNearest;
    path.segments()
        .map(|s| s.nearest(kurbo::Point::new(p.u, p.v), 1e-9).distance_sq.sqrt())
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_balloon_leader_lands_on_a_visible_stretch_of_its_edge() {
    let k = BoxKernel::new();
    // A narrow block in front of the base's middle hides the middle of its long front
    // edges in the front view; their ends stay visible.
    let cover = (cuboid(&k, "3", Vec3::new(40.0, -20.0, -5.0), Vec3::new(20.0, 10.0, 20.0)), 2);
    let first = model(b"r1", vec![base(&k), cover]);
    let drawing = drawing_of(&first, "iso-a3", vec![StandardView::Front], Some(Scale::FULL));
    let view = &drawing.sheets[0].views[0];
    assert!(
        view.geometry.curves.iter().any(|c| !c.visible && component_of(&c.source.feature) == Some("2")),
        "the cover hides part of the base"
    );
    let balloon = drawing.sheets[0]
        .annotations
        .iter()
        .find(|a| matches!(&a.body, AnnotationKind::Balloon { occurrence, .. } if occurrence == "2"))
        .unwrap();
    let at = balloon.anchors[0].cached;
    let on_visible = view
        .geometry
        .curves
        .iter()
        .filter(|c| c.visible && component_of(&c.source.feature) == Some("2"))
        .map(|c| distance_to(&c.path, at))
        .fold(f64::INFINITY, f64::min);
    assert!(on_visible < 1e-6, "the leader lands {on_visible} mm from the base's visible line work, at {at:?}");
}

/// The base and a post, with `extra` more rows in the BOM.
fn with_rows<'k>(k: &'k BoxKernel, revision: &[u8], extra: u32) -> AssemblyModel<'k> {
    let mut m = model(revision, vec![base(k), post(k, "3", 10.0)]);
    for item in 3..3 + extra {
        m.bom.push(BomRow { item, name: format!("Spare {item}"), file: format!("parts/s{item}.dwprt"), quantity: 1 });
    }
    m
}

#[test]
fn a_bom_too_tall_for_the_sheet_is_refused() {
    let k = BoxKernel::new();
    // Thirty rows make a 224 mm table; an A4 sheet is 210 mm tall.
    let tall = with_rows(&k, b"r1", 28);
    assert_eq!(tall.bom.len(), 30);
    let new = NewAssemblyDrawing {
        template: Template::by_id("iso-a4").unwrap(),
        meta: DrawingMeta::default(),
        views: vec![StandardView::Isometric],
        scale: None,
        balloons: true,
        exploded: false,
    };
    let err = make_assembly_drawing(&tall, &new).expect_err("the BOM does not fit an A4 sheet");
    assert!(err.to_string().contains("BOM"), "{err}");

    // A rebuild that grows the BOM past the sheet is refused too, and changes nothing.
    let first = with_rows(&k, b"r1", 0);
    let drawing = drawing_of(&first, "iso-a3", vec![StandardView::Isometric], None);
    let mut grown = with_rows(&k, b"r2", 45);
    grown.reference.as_mut().unwrap().id = first.reference.as_ref().unwrap().id;
    let err = update_assembly_drawing(&drawing, &grown).expect_err("47 rows do not fit an A3 sheet");
    assert!(err.to_string().contains("BOM"), "{err}");
}
