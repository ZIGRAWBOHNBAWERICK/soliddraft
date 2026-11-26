//! The bracket fixture, serialisation, templates and commands that need no model.

use std::path::PathBuf;

use soliddraft_drawing::display::layout_sheet;
use soliddraft_drawing::export;
use soliddraft_drawing::file::{self, FileMeta};
use soliddraft_drawing::fixture::bracket_a3;
use soliddraft_drawing::{
    AnnotationState, Command, ContentHash, DocKind, DocRef, Drawing, DrawingError, Projection, Template, ViewId,
    ViewKind, apply,
};
use soliddraft_kernel_api::Point2;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/bracket-a3.json")
}

/// The checked-in JSON is what the builder makes. Set `SOLIDDRAFT_BLESS=1` to rewrite it.
#[test]
fn fixture_json_matches_builder() {
    let json = bracket_a3().to_json().unwrap() + "\n";
    let path = fixture_path();
    if std::env::var_os("SOLIDDRAFT_BLESS").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    let on_disk = std::fs::read_to_string(&path).expect("fixture missing; run with SOLIDDRAFT_BLESS=1");
    assert!(on_disk == json, "{} is stale; run with SOLIDDRAFT_BLESS=1", path.display());
}

#[test]
fn json_round_trips() {
    let drawing = bracket_a3();
    let back = Drawing::from_json(&drawing.to_json().unwrap()).unwrap();
    assert_eq!(back, drawing);
    let on_disk = std::fs::read_to_string(fixture_path()).unwrap();
    assert_eq!(Drawing::from_json(&on_disk).unwrap(), drawing);
}

#[test]
fn fixture_has_the_views_and_annotations_a_ui_needs() {
    let d = bracket_a3();
    let sheet = &d.sheets[0];
    assert_eq!(sheet.projection, Projection::FirstAngle);
    assert_eq!(sheet.views.len(), 4);
    assert!(matches!(d.view(ViewId(3)).unwrap().kind, ViewKind::Projected { parent: ViewId(1), .. }));
    assert!(sheet.views.iter().all(|v| v.geometry.curves.iter().any(|c| c.visible)));
    assert!(sheet.views.iter().any(|v| v.geometry.curves.iter().any(|c| !c.visible)));
    let dims = sheet.annotations.iter().filter(|a| matches!(a.body, soliddraft_drawing::AnnotationKind::Dimension(_)));
    assert_eq!(dims.count(), 5);
    let attention: Vec<_> = d.annotations_needing_attention().collect();
    assert_eq!(attention.len(), 2);
    assert!(attention.iter().any(|a| matches!(a.state, AnnotationState::Dangling { .. })));
    assert!(attention.iter().any(|a| matches!(a.state, AnnotationState::Changed { .. })));
    assert_eq!(d.next_view_id(), ViewId(5));
}

/// Every view lies inside the border and clear of the title block.
#[test]
fn fixture_views_fit_the_sheet() {
    let d = bracket_a3();
    let template = Template::by_id("iso-a3").unwrap();
    let (m, size, tb) = (template.margins, template.size, template.title_block);
    for v in &d.sheets[0].views {
        let [x0, y0, x1, y1] = v.geometry.bbox;
        let (x0, y0, x1, y1) = (x0 + v.position.u, y0 + v.position.v, x1 + v.position.u, y1 + v.position.v);
        assert!(x0 > m.left && y0 > m.bottom && x1 < size.width_mm - m.right && y1 < size.height_mm - m.top);
        let clear_of_title = x1 < tb.position.u || y0 > tb.position.v + tb.height;
        assert!(clear_of_title, "view {} overlaps the title block", v.id.0);
    }
}

#[test]
fn builtin_templates() {
    use Projection::{FirstAngle, ThirdAngle};

    let all = Template::builtin();
    let sizes: Vec<_> = all.iter().map(|t| (t.id.as_str(), t.size.width_mm, t.size.height_mm, t.projection)).collect();
    assert_eq!(
        sizes,
        [
            ("iso-a3", 420.0, 297.0, FirstAngle),
            ("iso-a4", 297.0, 210.0, FirstAngle),
            ("ansi-a", 279.4, 215.9, ThirdAngle),
            ("ansi-b", 431.8, 279.4, ThirdAngle),
            ("iso-a2", 594.0, 420.0, FirstAngle),
            ("iso-a1", 841.0, 594.0, FirstAngle),
            ("iso-a0", 1189.0, 841.0, FirstAngle),
            ("ansi-c", 558.8, 431.8, ThirdAngle),
            ("ansi-d", 863.6, 558.8, ThirdAngle),
            ("ansi-e", 1117.6, 863.6, ThirdAngle),
        ]
    );
    assert!(Template::by_id("nope").is_none());
    for t in &all {
        assert!(t.title_block.fields.iter().any(|f| f.link == "${title}"));
    }
}

#[test]
fn large_and_small_presets_reopen_and_export_at_paper_size() {
    for (id, width, height) in [("iso-a0", 1189.0, 841.0), ("ansi-e", 1117.6, 863.6), ("iso-a4", 297.0, 210.0)] {
        let model = DocRef::unbound(DocKind::Part, "pump.json", ContentHash::of_bytes(b"pump"));
        let mut drawing = Drawing::new(model, &Template::by_id(id).unwrap());
        let title = "Pump housing";
        apply(&mut drawing, &Command::SetTitleField { sheet: 0, key: "title".into(), value: title.into() }).unwrap();
        let bytes = file::to_bytes(&drawing, &FileMeta::new("paper test", drawing.meta.units)).unwrap();
        let (back, _) = file::from_bytes(&bytes).unwrap();
        let sheet = &back.sheets[0];
        assert_eq!(sheet.template.as_str(), id);
        assert_eq!((sheet.size.width_mm, sheet.size.height_mm), (width, height), "{id}");
        assert_eq!(sheet.format_overrides["title"], title);

        let list = layout_sheet(&back, 0);
        let svg = export::svg(&list);
        assert!(svg.contains(&format!(r#"width="{width}mm" height="{height}mm""#)), "{id}");
        assert!(svg.contains(&format!(">{title}</text>")), "{id}");

        let pdf = export::pdf(std::slice::from_ref(&list), &export::PdfInfo::default()).unwrap();
        let pdf = lopdf::Document::load_mem(&pdf).unwrap();
        let page = pdf.get_pages()[&1];
        let media_box = pdf.get_dictionary(page).unwrap().get(b"MediaBox").unwrap().as_array().unwrap();
        assert_eq!(media_box.len(), 4);
        for (value, mm) in media_box[2..].iter().zip([width, height]) {
            let points = f64::from(value.as_float().unwrap());
            assert!((points - mm * 72.0 / 25.4).abs() < 0.01, "{id}: {points} pt");
        }
        assert!(pdf.extract_text(&[1]).unwrap().contains(title), "{id}");

        let dxf = export::dxf(&list).unwrap();
        let dxf = dxf::Drawing::load(&mut dxf.as_slice()).unwrap();
        let extents = &dxf.header.maximum_drawing_extents;
        assert_eq!((extents.x, extents.y), (width, height), "{id}");
        assert!(dxf.entities().any(|e| matches!(&e.specific, dxf::entities::EntityType::Text(t) if t.value == title)));
        assert!(dxf.entities().any(|e| matches!(&e.specific, dxf::entities::EntityType::Line(_))));
    }
}

#[test]
fn new_drawing_has_one_empty_sheet() {
    let model = DocRef::unbound(DocKind::Part, "p.json", ContentHash::of_bytes(b"d"));
    let d = Drawing::new(model, &Template::by_id("ansi-b").unwrap());
    assert_eq!(d.sheets.len(), 1);
    assert_eq!(d.sheets[0].projection, Projection::ThirdAngle);
    assert_eq!(d.sheets[0].next_view_id(), ViewId(1));
    assert!(d.settings.automatic_update);
}

#[test]
fn apply_moves_views_and_asks_for_the_model_where_it_needs_one() {
    let mut d = bracket_a3();
    let to = Point2::new(120.0, 200.0);
    apply(&mut d, &Command::MoveView { view: ViewId(1), to }).unwrap();
    assert_eq!(d.view(ViewId(1)).unwrap().position, to);
    let missing = apply(&mut d, &Command::MoveView { view: ViewId(99), to });
    assert!(matches!(missing, Err(DrawingError::UnknownView(ViewId(99)))));
    let update = apply(&mut d, &Command::UpdateFromModel);
    assert!(matches!(update, Err(DrawingError::NeedsModel(_))));
}

#[test]
fn commands_serialise_with_a_type_tag() {
    let c = Command::BreakAlignment { view: ViewId(2) };
    let json = serde_json::to_string(&c).unwrap();
    assert_eq!(json, r#"{"type":"break_alignment","view":2}"#);
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), c);
}
