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
