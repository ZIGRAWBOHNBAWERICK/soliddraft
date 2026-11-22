//! Tables on a sheet. The revision table lists the drawing's revisions, oldest first,
//! with the zones their revision symbols are in.

use soliddraft_kernel_api::Point2;

use super::{Context, Item, Owner, PathItem, Role, TextItem, VAlign, line, rect, to_sheet, zone_at};
use crate::Drawing;
use crate::annotation::AnnotationKind;
use crate::library::{self, Align, Weight};
use crate::sheet::{Sheet, Table, TableKind};

/// The revision table's columns: heading and width in millimetres.
pub const REVISION_COLUMNS: [(&str, f64); 5] =
    [("ZONE", 16.0), ("REV", 12.0), ("DESCRIPTION", 70.0), ("DATE", 24.0), ("APPROVED", 24.0)];

const HOLE_COLUMNS: [(&str, f64); 4] = [("ID", 12.0), ("X (mm)", 19.0), ("Y (mm)", 19.0), ("DIA (mm)", 23.0)];

/// Height of each row of a table, in millimetres.
const ROW: f64 = 7.0;

/// Width and height of a revision table with `rows` revisions: a title row, a heading
/// row and one row per revision.
pub fn revision_table_size(rows: usize) -> (f64, f64) {
    (REVISION_COLUMNS.iter().map(|(_, w)| w).sum(), ROW * (rows as f64 + 2.0))
}

/// Where a sheet's revision table goes when it is made: its top-left corner in the
/// top-right corner of the drawing area, or, when views are there, just above the
/// title block, or in the top-left corner; the first that covers no view.
pub fn revision_table_home(sheet: &Sheet) -> Point2 {
    let f = library::template(sheet.template.as_str());
    let [l, _, r, t] = f.frame();
    let band = f.zones.band;
    let (w, h) = revision_table_size(0);
    let o = f.title_block_origin();
    let candidates = [
        Point2::new(r - band - w, t - band),
        Point2::new(r - band - w, o.v + f.title_block.height + GAP + h),
        Point2::new(l + band, t - band),
    ];
    let clear = |p: &Point2| {
        let rect = [p.u, p.v - h, p.u + w, p.v];
        sheet.views.iter().all(|v| {
            let [a, b, c, d] = crate::project::view_bounds(v);
            rect[2] <= a - GAP || c + GAP <= rect[0] || rect[3] <= b - GAP || d + GAP <= rect[1]
        })
    };
    candidates.iter().copied().find(clear).unwrap_or(candidates[0])
}

/// Room kept between a table and the views or title block around it.
const GAP: f64 = 3.0;

/// Moves a revision table up when its rows would reach into the title block below it.
pub(crate) fn keep_above_title_block(sheet: &mut Sheet) {
    let f = library::template(sheet.template.as_str());
    let o = f.title_block_origin();
    let top = o.v + f.title_block.height + GAP;
    for table in sheet.tables.iter_mut().filter(|t| t.kind == TableKind::Revision) {
        let (w, h) = revision_table_size(table.revisions.len());
        let over = table.position.u < o.u + f.title_block.width && table.position.u + w > o.u;
        if over && table.position.v - h < top && table.position.v > o.v {
            table.position.v = top + h;
        }
    }
}

/// `[left, bottom, right, top]` of every table on a sheet, in sheet millimetres.
pub fn table_bounds(sheet: &Sheet) -> Vec<[f64; 4]> {
    sheet
        .tables
        .iter()
        .map(|t| {
            let (w, h) = if t.kind == TableKind::Hole {
                (HOLE_COLUMNS.iter().map(|(_, w)| w).sum(), ROW * (t.holes.len() as f64 + 2.0))
            } else {
                revision_table_size(t.revisions.len())
            };
            [t.position.u, t.position.v - h, t.position.u + w, t.position.v]
        })
        .collect()
}

/// The zones of the revision symbols that carry `revision`, joined.
fn symbol_zones(sheet: &Sheet, revision: &str, ctx: &Context<'_>) -> String {
    let mut zones: Vec<String> = sheet
        .annotations
        .iter()
        .filter(|a| matches!(&a.body, AnnotationKind::RevisionSymbol { revision: r } if r == revision))
        .map(|a| zone_at(ctx.format, to_sheet(sheet, a.view, a.placement)))
        .collect();
    zones.sort();
    zones.dedup();
    zones.join(", ")
}

/// Every table of a sheet as items.
pub(super) fn table_items(_drawing: &Drawing, sheet: &Sheet, ctx: &Context<'_>, items: &mut Vec<Item>) {
    let mut next_prefix = "A".to_owned();
    for (i, table) in sheet.tables.iter().enumerate() {
        let owner = Owner::Table(i);
        match table.kind {
            TableKind::Revision => {
                let rows = table.revisions.iter().map(|row| {
                    let zone =
                        if row.zone.is_empty() { symbol_zones(sheet, &row.revision, ctx) } else { row.zone.clone() };
                    [zone, row.revision.clone(), row.description.clone(), row.date.clone(), row.approved.clone()]
                });
                grid(table, ("REVISIONS", &REVISION_COLUMNS), rows, owner, ctx, items);
            }
            TableKind::Hole => {
                let Some(view) = table.hole_view.and_then(|id| sheet.view(id)) else { continue };
                let prefix = next_prefix;
                next_prefix = crate::next_letter(&prefix);
                let number = |n| crate::measure::format_value(n, crate::measure::precision_for(n));
                let rows = table.holes.iter().enumerate().map(|(i, h)| {
                    [format!("{prefix}{}", i + 1), number(h.coordinates.u), number(h.coordinates.v), number(h.diameter)]
                });
                grid(table, ("HOLE TABLE · MODEL ORIGIN", &HOLE_COLUMNS), rows, owner, ctx, items);

                let scale = crate::project::effective_scale(sheet, view);
                let mut tags = table.holes.iter().enumerate().peekable();
                while let Some((i, h)) = tags.next() {
                    let mut label = format!("{prefix}{}", i + 1);
                    let mut diameter = h.diameter;
                    while tags.peek().is_some_and(|(_, next)| next.coordinates == h.coordinates) {
                        let (j, next) = tags.next().expect("peeked");
                        label.push_str(&format!(", {prefix}{}", j + 1));
                        diameter = next.diameter;
                    }
                    let pos = Point2::new(
                        view.position.u + (h.centre.u + diameter / 2.0) * scale + 1.0,
                        view.position.v + h.centre.v * scale + 1.0,
                    );
                    items.push(Item::Text(TextItem {
                        pos,
                        height: 2.5,
                        angle: 0.0,
                        text: label,
                        halign: Align::Left,
                        valign: VAlign::Bottom,
                        weight: Weight::Medium,
                        role: Role::Text,
                        owner,
                    }));
                }
            }
            TableKind::Bom => {}
        }
    }
}

fn grid<const N: usize>(
    table: &Table,
    (title, columns): (&str, &[(&str, f64); N]),
    rows: impl ExactSizeIterator<Item = [String; N]>,
    owner: Owner,
    ctx: &Context<'_>,
    items: &mut Vec<Item>,
) {
    let lines = &ctx.format.lines;
    let w: f64 = columns.iter().map(|(_, w)| w).sum();
    let h = ROW * (rows.len() as f64 + 2.0);
    let (x0, top) = (table.position.u, table.position.v);
    let mut path = |p: kurbo::BezPath, width: f64, role: Role| {
        items.push(Item::Path(PathItem { path: p, width, dash: None, role, owner, fill: false, accuracy: None }));
    };
    path(rect(x0, top - h, x0 + w, top), lines.border, Role::Border);
    for r in 1..(rows.len() + 2) {
        let y = top - ROW * r as f64;
        path(line(Point2::new(x0, y), Point2::new(x0 + w, y)), lines.thin, Role::Thin);
    }
    let mut x = x0;
    for (i, (_, cw)) in columns.iter().enumerate() {
        if i > 0 {
            path(line(Point2::new(x, top - ROW), Point2::new(x, top - h)), lines.thin, Role::Thin);
        }
        x += cw;
    }
    let mut text = |pos: Point2, text: String, height: f64, halign: Align, weight: Weight, role: Role| {
        items.push(Item::Text(TextItem {
            pos,
            height,
            angle: 0.0,
            text,
            halign,
            valign: VAlign::Centre,
            weight,
            role,
            owner,
        }));
    };
    text(Point2::new(x0 + w / 2.0, top - ROW / 2.0), title.into(), 2.5, Align::Centre, Weight::Medium, Role::Text);
    let mut x = x0;
    for (heading, cw) in columns {
        text(
            Point2::new(x + cw / 2.0, top - 1.5 * ROW),
            (*heading).into(),
            1.8,
            Align::Centre,
            Weight::Regular,
            Role::Label,
        );
        x += cw;
    }
    for (r, cells) in rows.enumerate() {
        let y = top - ROW * (r as f64 + 2.5);
        let mut x = x0;
        for (cell, (label, cw)) in cells.into_iter().zip(columns) {
            let (pos, align) = if *label == "DESCRIPTION" {
                (Point2::new(x + 1.5, y), Align::Left)
            } else {
                (Point2::new(x + cw / 2.0, y), Align::Centre)
            };
            if !cell.is_empty() {
                text(pos, cell, 2.5, align, Weight::Regular, Role::Text);
            }
            x += cw;
        }
    }
}
