//! Drawings of assemblies: two boxes, one standing in front of the other, drawn with a
//! bill of materials and balloons, saved and read back, refused by exports, and brought up to date
//! after a part moves and another goes.

use soliddraft_drawing::assembly::{
    AssemblyModel, BomRow, NewAssemblyDrawing, PlacedPart, make_assembly_drawing, update_assembly_drawing,
};
use soliddraft_drawing::display::{Item, layout_sheet};
use soliddraft_drawing::file::{FileMeta, from_bytes, to_bytes};
use soliddraft_drawing::{
    AnnotationKind, AnnotationState, Command, ContentHash, DocKind, DocRef, DocumentId, DrawingMeta, TableKind,
    Template, Units, apply,
};
use soliddraft_kernel_api::Vec3;
use soliddraft_kernel_api::drawing::StandardView;

/// A box from `lo` to `hi`: twelve triangles on six faces and its twelve edges.
fn cuboid(path: &str, item: u32, lo: Vec3, hi: Vec3) -> PlacedPart {
    let c = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    let positions: Vec<Vec3> = (0..8).map(c).collect();
    // Faces as quads, counter-clockwise from outside.
    let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
    let mut triangles = Vec::new();
    let mut faces = Vec::new();
    for (f, q) in quads.iter().enumerate() {
        triangles.push([q[0], q[1], q[2]]);
        triangles.push([q[0], q[2], q[3]]);
        faces.extend([f as u32, f as u32]);
    }
    let pairs = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    let edges = pairs.iter().map(|(a, b)| vec![positions[*a], positions[*b]]).collect();
    PlacedPart { path: path.into(), item, positions, triangles, faces, edges }
}

fn model(revision: &[u8], parts: Vec<PlacedPart>) -> AssemblyModel {
    let reference = DocRef::new(DocumentId::new(), DocKind::Assembly, "rig.dwasm", ContentHash::of_bytes(revision));
    let mut bom = vec![
        BomRow { item: 1, name: "Base".into(), file: "parts/base.dwprt".into(), quantity: 1 },
        BomRow { item: 2, name: "Post".into(), file: "parts/post.dwprt".into(), quantity: 0 },
    ];
    bom[1].quantity = parts.iter().filter(|p| p.item == 2).count() as u32;
    AssemblyModel { reference: Some(reference), parts, exploded: Vec::new(), bom }
}

fn base() -> PlacedPart {
    cuboid("2", 1, Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 10.0))
}

fn post(path: &str, x: f64) -> PlacedPart {
    cuboid(path, 2, Vec3::new(x, 20.0, 10.0), Vec3::new(x + 20.0, 40.0, 70.0))
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
    let first = model(b"r1", vec![base(), post("3", 10.0), post("4", 70.0)]);
    let mut drawing = make_assembly_drawing(&first, &new).unwrap();
    assert_eq!(drawing.model.kind, DocKind::Assembly);
    assert!(drawing.views_match_pin());

    // Both views have line work, each curve owned by an occurrence; the posts hide part
    // of the base's far edges in the front view, so some line work is hidden.
    let sheet = &drawing.sheets[0];
    assert_eq!(sheet.views.len(), 2);
    for v in &sheet.views {
        assert!(!v.geometry.curves.is_empty());
        assert!(v.geometry.curves.iter().all(|c| ["2", "3", "4"].contains(&c.source.feature.as_str())));
    }
    let front = &sheet.views[1].geometry;
    assert!(front.curves.iter().any(|c| !c.visible), "the posts hide some of the base");

    // The BOM lists both parts with their quantities, and each row has a balloon.
    let bom = sheet.tables.iter().find(|t| t.kind == TableKind::Bom).unwrap();
    assert_eq!(bom.bom.iter().map(|r| (r.item, r.quantity)).collect::<Vec<_>>(), vec![(1, 1), (2, 2)]);
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

    // Saved and read back, the drawing is the same. Exports draw Catallax line work only,
    // so they refuse its mesh-drawn views by name; the sheet still lays out its table.
    let meta = FileMeta::new("display meshes", Units::Millimetre);
    let bytes = to_bytes(&drawing, &meta).unwrap();
    let (back, _) = from_bytes(&bytes).unwrap();
    assert_eq!(back, drawing);
    let refused = soliddraft_drawing::export::check_sheet(&back, &back.sheets[0]).unwrap_err().to_string();
    assert!(refused.contains("display meshes"), "{refused}");
    let svg = soliddraft_drawing::export::svg::svg(&layout_sheet(&back, 0));
    assert!(svg.contains("BILL OF MATERIALS"));

    // A later change: the second post moves, the first goes, a third part arrives.
    let mut parts = vec![base(), post("4", 40.0)];
    parts.push(cuboid("5", 3, Vec3::new(0.0, 0.0, 70.0), Vec3::new(100.0, 60.0, 75.0)));
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
    assert!(texts(&drawing).iter().any(|t| t == "Lid"));
}
