//! Where a review comment sits on a drawing. A comment names the annotation that shows
//! an item of the part's manufacturing information, so it follows the annotation when the
//! annotation moves.

use kurbo::Shape;
use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::Point2;

use crate::annotation::{Annotation, PmiSource};
use crate::display::{self, Item, Owner};
use crate::{AnnotationId, Drawing, Sheet};

/// A place on a drawing for a review comment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarkupTarget {
    /// Index of the sheet.
    pub sheet: usize,
    /// The annotation the comment is on.
    pub annotation: AnnotationId,
}

/// The resolved place of a comment on an open sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkupPlace {
    /// Index of the sheet.
    pub sheet: usize,
    /// The annotation the comment is on.
    pub annotation: AnnotationId,
    /// The centre of the annotation's lines and text on the sheet, in millimetres.
    pub at: Point2,
}

/// Where the annotation that shows the part's item `item` is: the annotation made from
/// the item itself, or else one made from it such as a basic distance, on the first sheet
/// that has one. `None` when no annotation shows it.
pub fn place_of_item(drawing: &Drawing, item: &str) -> Option<MarkupTarget> {
    let shows = |a: &Annotation, own: bool| match &a.source {
        Some(PmiSource::Item { item: i }) => i == item,
        Some(source) => !own && source.item() == Some(item),
        None => false,
    };
    [true, false].into_iter().find_map(|own| {
        drawing.sheets.iter().enumerate().find_map(|(sheet, s)| {
            let a = s.annotations.iter().find(|a| shows(a, own) && current(drawing, s, a))?;
            Some(MarkupTarget { sheet, annotation: a.id })
        })
    })
}

fn current(drawing: &Drawing, sheet: &Sheet, a: &Annotation) -> bool {
    sheet.view(a.view).is_some_and(|v| v.geometry.source_digest == drawing.model.pinned.as_str())
}

/// Resolves the current annotation showing the named item and finds its position.
pub fn find_place(drawing: &Drawing, place: &MarkupTarget, item: &str) -> Option<MarkupPlace> {
    let sheet = drawing.sheets.get(place.sheet)?;
    let a = sheet.annotations.iter().find(|a| a.id == place.annotation)?;
    if !current(drawing, sheet, a) || a.source.as_ref().and_then(PmiSource::item) != Some(item) {
        return None;
    }
    Some(MarkupPlace { sheet: place.sheet, annotation: a.id, at: centre(drawing, place.sheet, a.id)? })
}

/// The centre of the box around what the sheet draws for annotation `id`.
fn centre(drawing: &Drawing, sheet: usize, id: AnnotationId) -> Option<Point2> {
    let list = display::layout_sheet(drawing, sheet);
    let mut bounds: Option<kurbo::Rect> = None;
    for item in &list.items {
        if !matches!(item.owner(), Owner::Annotation(a) | Owner::AnnotationEnd(a, _) if a == id) {
            continue;
        }
        let r = match item {
            Item::Path(p) => p.path.bounding_box(),
            Item::Text(t) => kurbo::Rect::from_points((t.pos.u, t.pos.v), (t.pos.u, t.pos.v)),
        };
        bounds = Some(bounds.map_or(r, |b| b.union(r)));
    }
    let c = bounds?.center();
    Some(Point2::new(c.x, c.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;

    #[test]
    fn a_place_follows_its_annotation_and_refuses_another_item() {
        let mut drawing = fixture::bracket_a3();
        let sheet = &mut drawing.sheets[0];
        let index = sheet.annotations.iter().position(|a| a.view != crate::SHEET).expect("an annotation in a view");
        sheet.annotations[index].source = Some(PmiSource::Item { item: "Bore diameter".into() });
        let id = sheet.annotations[index].id;
        let target = place_of_item(&drawing, "Bore diameter").expect("the item is on the drawing");
        let place = find_place(&drawing, &target, "Bore diameter").unwrap();
        assert_eq!((place.sheet, place.annotation), (0, id));
        assert!(place_of_item(&drawing, "Slot").is_none());

        let placement = &mut drawing.sheets[0].annotations[index].placement;
        placement.u += 10.0;
        placement.v += 10.0;
        let moved = find_place(&drawing, &target, "Bore diameter").expect("still there");
        let shift = (moved.at.u - place.at.u).hypot(moved.at.v - place.at.v);
        assert!(shift > 1.0, "{moved:?} follows the annotation from {place:?}");
        assert!(find_place(&drawing, &target, "Slot").is_none());
        let view = drawing.sheets[0].annotations[index].view;
        drawing.view_mut(view).unwrap().geometry.source_digest = "older".into();
        assert!(place_of_item(&drawing, "Bore diameter").is_none());
        assert!(find_place(&drawing, &target, "Bore diameter").is_none());
    }
}
