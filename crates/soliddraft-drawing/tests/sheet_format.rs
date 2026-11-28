//! A sheet's format and title block layout change through undoable commands and are
//! saved with the drawing, with no kernel: the bracket fixture's sheet goes to ISO A0 and
//! back, and its owner field moves cells and back.

use soliddraft_drawing::display::{self, Item};
use soliddraft_drawing::{Command, DrawingError, TemplateId, TitleCell, UndoStack, file, fixture};

fn owner_at(drawing: &soliddraft_drawing::Drawing) -> soliddraft_kernel_api::Point2 {
    display::layout_sheet(drawing, 0)
        .items
        .iter()
        .find_map(|i| match i {
            Item::Text(t) if t.text == "Rig Shop" => Some(t.pos),
            _ => None,
        })
        .expect("the owner's text is on the sheet")
}

#[test]
fn sheet_format_and_title_layout_undo_redo_and_reopen() {
    let mut drawing = fixture::bracket_a3();
    let mut stack = UndoStack::new();
    stack
        .run(&mut drawing, &Command::SetTitleField { sheet: 0, key: "owner".into(), value: "Rig Shop".into() }, None)
        .unwrap();
    let origin = |d: &soliddraft_drawing::Drawing| {
        soliddraft_drawing::library::template(d.sheets[0].template.as_str()).title_block_origin()
    };
    let at = owner_at(&drawing);
    let o = origin(&drawing);
    assert_eq!((at.u - o.u, at.v - o.v), (1.5, 10.5), "the template's owner cell is at 0, 9");

    // A larger sheet: the paper, zones and title block follow; the views stay.
    let views = drawing.sheets[0].views.clone();
    stack.run(&mut drawing, &Command::SetSheetFormat { sheet: 0, template: TemplateId::from("iso-a0") }, None).unwrap();
    assert_eq!(display::layout_sheet(&drawing, 0).paper, (1189.0, 841.0));
    assert_eq!(drawing.sheets[0].size.width_mm, 1189.0);
    assert_eq!(drawing.sheets[0].zones.columns, 24);
    assert_eq!(drawing.sheets[0].views, views);

    let moved = TitleCell { cell: [4.0, 9.0, 54.0, 9.0] };
    stack
        .run(&mut drawing, &Command::SetTitleLayout { sheet: 0, key: "owner".into(), layout: Some(moved) }, None)
        .unwrap();
    let (at, o) = (owner_at(&drawing), origin(&drawing));
    assert_eq!((at.u - o.u, at.v - o.v), (5.5, 10.5));

    // A cell outside the block or a field the block lacks is refused, with no change.
    let before = drawing.clone();
    for (key, cell) in [
        ("owner", [170.0, 30.0, 20.0, 9.0]),
        ("owner", [60.0, 9.0, 60.0, 9.0]),
        ("owner", [0.0, 9.0, 0.5, 9.0]),
        ("owner", [0.0, 9.0, 60.0, 2.0]),
        ("nope", [0.0, 0.0, 9.0, 9.0]),
    ] {
        let command = Command::SetTitleLayout { sheet: 0, key: key.into(), layout: Some(TitleCell { cell }) };
        assert!(matches!(stack.run(&mut drawing, &command, None), Err(DrawingError::BadRequest(_))), "{key} {cell:?}");
        assert_eq!(drawing, before);
    }

    // Saved and reopened as it is.
    let path = std::env::temp_dir().join(format!("soliddraft-sheet-format-{}.dwdrw", std::process::id()));
    let meta = file::FileMeta::new("test", drawing.meta.units);
    file::write(&path, &drawing, &meta).unwrap();
    let (reopened, _) = file::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(reopened.sheets[0].template.as_str(), "iso-a0");
    assert_eq!(reopened.sheets[0].title_layout.get("owner"), Some(&moved));
    assert_eq!(owner_at(&reopened), owner_at(&drawing));

    // Undo puts the cell back, then the A3 paper; redo makes both again.
    assert_eq!(stack.undo(&mut drawing), Some("Title block layout"));
    let (at, o) = (owner_at(&drawing), origin(&drawing));
    assert_eq!((at.u - o.u, at.v - o.v), (1.5, 10.5));
    assert_eq!(stack.undo(&mut drawing), Some("Sheet format"));
    assert_eq!(display::layout_sheet(&drawing, 0).paper, (420.0, 297.0));
    assert_eq!(stack.undo(&mut drawing), Some("Title block"));
    assert_eq!(drawing.sheets[0].format_overrides["owner"], "SolidDraft workshop");
    assert_eq!(stack.redo(&mut drawing), Some("Title block"));
    assert_eq!(owner_at(&drawing).u - origin(&drawing).u, 1.5);
    assert_eq!(stack.redo(&mut drawing), Some("Sheet format"));
    assert_eq!(stack.redo(&mut drawing), Some("Title block layout"));
    assert_eq!(drawing, reopened);
}

#[test]
fn long_title_values_fit_their_cells_without_losing_the_saved_text() {
    let mut drawing = fixture::bracket_a3();
    let owner = "Intercontinental precision components and fabrication company";
    let mut stack = UndoStack::new();
    for command in [
        Command::SetTitleLayout {
            sheet: 0,
            key: "owner".into(),
            layout: Some(TitleCell { cell: [30.0, 9.0, 30.0, 9.0] }),
        },
        Command::SetTitleField { sheet: 0, key: "owner".into(), value: owner.into() },
    ] {
        stack.run(&mut drawing, &command, None).unwrap();
    }
    assert_eq!(drawing.sheets[0].format_overrides["owner"], owner);
    drawing.sheets[0].views.clear();
    drawing.sheets[0].annotations.clear();
    let list = display::layout_sheet(&drawing, 0);
    let pdf = soliddraft_drawing::export::pdf(&[list], &Default::default()).unwrap();
    let pdf = lopdf::Document::load_mem(&pdf).unwrap();
    let page = *pdf.get_pages().values().next().unwrap();
    let fonts = pdf.get_page_fonts(page).unwrap();
    let font = fonts.values().next().unwrap();
    let encoding = font.get_font_encoding(&pdf).unwrap();
    let cid = font.get(b"DescendantFonts").unwrap().as_array().unwrap()[0].as_reference().unwrap();
    let cid = pdf.get_dictionary(cid).unwrap();
    let descriptor = pdf.get_dictionary(cid.get(b"FontDescriptor").unwrap().as_reference().unwrap()).unwrap();
    let embedded = pdf
        .get_object(descriptor.get(b"FontFile2").unwrap().as_reference().unwrap())
        .unwrap()
        .as_stream()
        .unwrap()
        .get_plain_content()
        .unwrap();
    let face = ttf_parser::Face::parse(&embedded, 0).unwrap();
    let operations = lopdf::content::Content::decode(&pdf.get_page_content(page)).unwrap().operations;
    let mut origin = 0.0;
    let mut size = 0.0;
    let mut scale = 1.0;
    let right = operations
        .iter()
        .find_map(|op| {
            let number = |i: usize| f64::from(op.operands[i].as_float().unwrap());
            match op.operator.as_str() {
                "cm" => scale = number(0),
                "Tm" => origin = number(4),
                "Tf" => size = number(1),
                "Tj" => {
                    let bytes = op.operands[0].as_str().unwrap();
                    if lopdf::Document::decode_text(&encoding, bytes).unwrap() == owner {
                        let mut x = 0.0;
                        let mut right = 0.0_f64;
                        for glyph in bytes.as_chunks::<2>().0 {
                            let id = ttf_parser::GlyphId(u16::from_be_bytes(*glyph));
                            if let Some(box_) = face.glyph_bounding_box(id) {
                                right = right.max(x + f64::from(box_.x_max));
                            }
                            x += f64::from(face.glyph_hor_advance(id).unwrap());
                        }
                        return Some((origin + right * size / f64::from(face.units_per_em())) * scale);
                    }
                }
                _ => {}
            }
            None
        })
        .expect("the owner text is present in the PDF");
    let title_boundary = (420.0 - 10.0 - 180.0 + 60.0) * 72.0 / 25.4;
    assert!(right <= title_boundary + 0.01, "{right} pt exceeds {title_boundary} pt");
}

#[test]
fn an_override_must_affect_title_text() {
    let mut drawing = fixture::bracket_a3();
    let before = drawing.clone();
    // A typo, the projection symbol, a property no field is keyed by, and clearing a
    // field that does not exist are all refused.
    for (key, value) in [("owenr", "third-angle"), ("projection", "third-angle"), ("sheets", "99"), ("owenr", "")] {
        let command = Command::SetTitleField { sheet: 0, key: key.into(), value: value.into() };
        assert!(
            matches!(soliddraft_drawing::apply(&mut drawing, &command), Err(DrawingError::BadRequest(_))),
            "{key}={value}"
        );
        assert_eq!(drawing, before);
    }
    // The sheet field takes its text by its own key.
    let field = soliddraft_drawing::library::template("iso-a3").title_block.fields.iter().find(|f| f.key == "sheet");
    let field = field.expect("the block has a sheet field");
    assert_eq!(display::field_value(&drawing, 0, field), "1 / 1");
    let command = Command::SetTitleField { sheet: 0, key: "sheet".into(), value: "2 / 9".into() };
    soliddraft_drawing::apply(&mut drawing, &command).unwrap();
    assert_eq!(display::field_value(&drawing, 0, field), "2 / 9");
    let shown =
        display::layout_sheet(&drawing, 0).items.iter().any(|i| matches!(i, Item::Text(t) if t.text == "2 / 9"));
    assert!(shown, "the sheet draws the new text");
}

#[test]
fn binding_a_title_property_preserves_the_sheet_override() {
    let mut drawing = fixture::bracket_a3();
    drawing.source_properties.insert("Description".into(), "Bracket arm".into());
    let title = soliddraft_drawing::library::template("iso-a3").title_block.fields.iter().find(|f| f.key == "title");
    let title = title.expect("the block has a title field");
    let mut stack = UndoStack::new();
    let set = Command::SetTitleField { sheet: 0, key: "title".into(), value: "Release".into() };
    stack.run(&mut drawing, &set, None).unwrap();
    assert_eq!(display::field_value(&drawing, 0, title), "Release");
    let bind = Command::BindTitleProperty { sheet: 0, key: "title".into(), property: "Description".into() };
    stack.run(&mut drawing, &bind, None).unwrap();
    assert_eq!(display::field_value(&drawing, 0, title), "Release");
    let clear = Command::SetTitleField { sheet: 0, key: "title".into(), value: String::new() };
    stack.run(&mut drawing, &clear, None).unwrap();
    assert_eq!(display::field_value(&drawing, 0, title), "Bracket arm");
    stack.undo(&mut drawing).unwrap();
    assert_eq!(display::field_value(&drawing, 0, title), "Release");
}

#[test]
fn the_projection_symbol_stays_inside_its_cell_below_the_caption() {
    use soliddraft_drawing::Projection;
    let mut drawing = fixture::bracket_a3();
    drawing.sheets[0].views.clear();
    drawing.sheets[0].annotations.clear();
    let mut stack = UndoStack::new();
    stack.run(&mut drawing, &Command::SetSheetFormat { sheet: 0, template: TemplateId::from("ansi-e") }, None).unwrap();
    let block = soliddraft_drawing::library::template("ansi-e");
    let o = block.title_block_origin();
    let field = block.title_block.fields.iter().find(|f| f.key == "projection").expect("a projection cell");
    for layout in [None, Some(TitleCell { cell: [0.0, 0.0, 20.6, 8.5] }), Some(TitleCell { cell: field.cell })] {
        let command = Command::SetTitleLayout { sheet: 0, key: "projection".into(), layout };
        stack.run(&mut drawing, &command, None).unwrap();
        let [x, y, w, h] = layout.map_or(field.cell, |c| c.cell);
        for projection in [Projection::FirstAngle, Projection::ThirdAngle] {
            stack.run(&mut drawing, &Command::SetSheetProjection { sheet: 0, projection }, None).unwrap();
            let list = display::layout_sheet(&drawing, 0);
            let at = list
                .items
                .iter()
                .position(|i| matches!(i, Item::Text(t) if t.text == field.label))
                .expect("the caption is drawn");
            let Item::Text(caption) = &list.items[at] else { unreachable!() };
            let caption_bottom = caption.pos.v - caption.height;
            // The trapezoid, two circles and three centrelines follow the caption.
            let parts: Vec<_> = list.items[at + 1..at + 7]
                .iter()
                .map(|i| match i {
                    Item::Path(p) => kurbo::Shape::bounding_box(&p.path),
                    _ => panic!("the symbol's line work follows its caption"),
                })
                .collect();
            if layout.is_none() {
                // The template's own cell keeps the full-size symbol where the template
                // puts it, beside the caption.
                let (u, v) = (o.u + x + w / 2.0, o.v + y + 3.2);
                let (cone, circles) = match projection {
                    Projection::FirstAngle => (u - 4.5, u + 4.5),
                    Projection::ThirdAngle => (u + 4.5, u - 4.5),
                };
                let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
                let (trapezoid, outer) = (parts[0], parts[1]);
                let what = format!("{projection:?}: {trapezoid:?} {outer:?}");
                assert!(near(trapezoid.x0, cone - 3.0) && near(trapezoid.x1, cone + 3.0), "{what}");
                assert!(near(trapezoid.y0, v - 2.25) && near(trapezoid.y1, v + 2.25), "{what}");
                assert!(near(outer.x0, circles - 2.25) && near(outer.x1, circles + 2.25), "{what}");
                assert!(near(outer.y0, v - 2.25) && near(outer.y1, v + 2.25), "{what}");
                continue;
            }
            for b in parts {
                let what = format!("{projection:?} in {:?}: {b:?}", [x, y, w, h]);
                assert!(b.x0 >= o.u + x - 1e-9 && b.x1 <= o.u + x + w + 1e-9, "{what}");
                assert!(b.y0 >= o.v + y - 1e-9 && b.y1 <= o.v + y + h + 1e-9, "{what}");
                assert!(b.y1 < caption_bottom, "{what} reaches the caption at {caption_bottom}");
            }
        }
    }
}

#[test]
fn a_smaller_format_keeps_the_revision_table_on_the_page() {
    use soliddraft_drawing::{RevisionRow, apply};
    use soliddraft_kernel_api::Point2;
    let mut drawing = fixture::bracket_a3();
    let sheet = &mut drawing.sheets[0];
    sheet.views.clear();
    sheet.annotations.clear();
    sheet.tables.clear();
    let format = |id: &str| Command::SetSheetFormat { sheet: 0, template: id.into() };
    apply(&mut drawing, &format("ansi-e")).unwrap();
    apply(
        &mut drawing,
        &Command::AddRevision { sheet: 0, row: RevisionRow { revision: "B".into(), ..Default::default() } },
    )
    .unwrap();
    let before = drawing.clone();
    let error = apply(&mut drawing, &format("iso-a3")).unwrap_err().to_string();
    assert!(error.contains("revision table"), "{error}");
    assert_eq!(drawing, before);
    apply(&mut drawing, &Command::MoveTable { sheet: 0, table: 0, to: Point2::new(200.0, 250.0) }).unwrap();
    apply(&mut drawing, &format("iso-a3")).unwrap();
    let list = display::layout_sheet(&drawing, 0);
    assert!(list.printable().any(|item| matches!(item, Item::Text(t) if t.text == "REVISIONS" && t.pos.v > 200.0)));
}

#[test]
fn a_sheet_annotation_kept_as_a_note_uses_its_own_sheets_style() {
    use soliddraft_drawing::{
        Annotation, AnnotationId, AnnotationKind, Dimension, DimensionKind, LinearOrientation, SHEET,
    };
    let mut drawing = fixture::bracket_a3();
    let mut second = drawing.sheets[0].clone();
    second.views.clear();
    second.annotations.clear();
    second.tables.clear();
    second.template = TemplateId::from("ansi-e");
    drawing.sheets.push(second);
    let dimension = Dimension::measured(DimensionKind::Linear { orientation: LinearOrientation::Horizontal }, 120.0, 0);
    let note = |id: u32| Annotation {
        id: AnnotationId(id),
        view: SHEET,
        body: AnnotationKind::Dimension(dimension.clone()),
        anchors: Vec::new(),
        placement: soliddraft_kernel_api::Point2::new(100.0, 100.0),
        style: Default::default(),
        state: Default::default(),
        source: None,
    };
    drawing.sheets[0].annotations.push(note(900));
    drawing.sheets[1].annotations.push(note(901));
    for (id, text) in [(900, "120 (ref)"), (901, "(120) (ref)")] {
        soliddraft_drawing::apply(&mut drawing, &Command::FreezeAsNote { annotation: AnnotationId(id) }).unwrap();
        let body = &drawing.annotation(AnnotationId(id)).unwrap().body;
        assert!(matches!(body, AnnotationKind::Note { text: t, .. } if t == text), "{id}: {body:?}");
    }
}

#[test]
fn changing_to_a_smaller_paper_names_the_views_that_would_be_off_sheet() {
    use soliddraft_drawing::{ViewId, apply, project};
    use soliddraft_kernel_api::Point2;
    let mut drawing = fixture::bracket_a3();
    let format = |id: &str| Command::SetSheetFormat { sheet: 0, template: id.into() };
    apply(&mut drawing, &format("ansi-e")).unwrap();
    let view = ViewId(1);
    apply(&mut drawing, &Command::MoveView { view, to: Point2::new(650.0, 400.0) }).unwrap();
    let sheet = &drawing.sheets[0];
    let name = project::view_label_in(&drawing, sheet, sheet.view(view).unwrap());
    let before = drawing.clone();
    let error = apply(&mut drawing, &format("ansi-a")).unwrap_err().to_string();
    assert!(error.contains(&name), "{error}");
    assert_eq!(drawing, before);
}
