//! AS9102C first article inspection forms 1, 2 and 3 as an XLSX workbook, with the field
//! numbers of the [public IAQG form layout](https://iaqg.org/standards/forms).
//!
//! The workbook states design requirements; it is not an inspection result. Results,
//! dispositions, approvals and identifiers that the part does not state stay blank. The
//! document name is the part name, not a part number: the part number is the caller's.

use std::fmt::Display;
use std::io::{self, Cursor, Write};

use zip::{ZipWriter, write::SimpleFileOptions};

use super::{InspectionPlan, InspectionUnit, json};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml";
const SHEETS: [&str; 3] = ["Form 1", "Form 2", "Form 3"];

/// Form 3 columns with their widths: the AS9102 fields 5 to 12, a disposition, then the
/// plan's trace back to the PMI item, its faces and its datum frame.
const CHARACTERISTICS: [(&str, u8); 23] = [
    ("5. Char. No.", 36),
    ("6. Reference Location", 20),
    ("7. Characteristic Designator", 16),
    ("8. Requirement", 40),
    ("9. Results", 16),
    ("10. Designed / Qualified Tooling", 20),
    ("11. Nonconformance Number", 18),
    ("12. Additional Data / Comments", 36),
    ("Disposition", 14),
    ("Group ID", 28),
    ("PMI Item", 24),
    ("Member", 9),
    ("Kind", 26),
    ("Applicability", 13),
    ("Nominal", 11),
    ("Unit", 6),
    ("Tolerance", 12),
    ("Fit Class", 9),
    ("Lower Limit", 11),
    ("Upper Limit", 11),
    ("Datum Order (JSON)", 30),
    ("Locating Datum", 9),
    ("Face References (JSON)", 48),
];

/// The Form 3 row that names the columns; the rows above it stay in view.
const HEADER_ROW: usize = 7;

/// Cell formats in `xl/styles.xml`, by index.
#[derive(Clone, Copy)]
enum Style {
    Plain = 0,
    Heading = 1,
    Wrapped = 2,
}

const STYLES: &str = concat!(
    r#"<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font>"#,
    r#"<font><b/><sz val="11"/><name val="Calibri"/></font></fonts>"#,
    r#"<fills count="2"><fill><patternFill patternType="none"/></fill>"#,
    r#"<fill><patternFill patternType="gray125"/></fill></fills>"#,
    r#"<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>"#,
    r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#,
    r#"<cellXfs count="3"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>"#,
    r#"<xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>"#,
    r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0" applyAlignment="1">"#,
    r#"<alignment vertical="top" wrapText="1"/></xf></cellXfs>"#,
    r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>"#,
);

impl InspectionPlan {
    /// Render the AS9102C part number, product and characteristic accountability forms.
    /// `part_number` is the manufacturing part number as the user states it; without one
    /// the field stays blank. Results, dispositions and approvals are always blank.
    pub fn as9102_xlsx(&self, part_number: Option<&str>) -> io::Result<Vec<u8>> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        part(&mut zip, "[Content_Types].xml")?;
        write!(
            zip,
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="{MIME}.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="{MIME}.styles+xml"/>"#
        )?;
        for n in 1..=SHEETS.len() {
            write!(zip, r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="{MIME}.worksheet+xml"/>"#)?;
        }
        zip.write_all(b"</Types>")?;

        part(&mut zip, "_rels/.rels")?;
        write!(
            zip,
            r#"<Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
        )?;
        part(&mut zip, "xl/workbook.xml")?;
        write!(zip, r#"<workbook xmlns="{MAIN}" xmlns:r="{REL}"><bookViews><workbookView/></bookViews><sheets>"#)?;
        for (n, name) in (1..).zip(SHEETS) {
            write!(zip, r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#)?;
        }
        zip.write_all(b"</sheets></workbook>")?;

        part(&mut zip, "xl/_rels/workbook.xml.rels")?;
        write!(zip, r#"<Relationships xmlns="{PACKAGE}">"#)?;
        for n in 1..=SHEETS.len() {
            write!(zip, r#"<Relationship Id="rId{n}" Type="{REL}/worksheet" Target="worksheets/sheet{n}.xml"/>"#)?;
        }
        let styles = SHEETS.len() + 1;
        write!(zip, r#"<Relationship Id="rId{styles}" Type="{REL}/styles" Target="styles.xml"/></Relationships>"#)?;

        part(&mut zip, "xl/styles.xml")?;
        write!(zip, r#"<styleSheet xmlns="{MAIN}">{STYLES}</styleSheet>"#)?;

        part(&mut zip, "xl/worksheets/sheet1.xml")?;
        self.part_accountability(&mut Sheet::new(&mut zip, &[45, 40, 23, 23], None)?, part_number)?;
        part(&mut zip, "xl/worksheets/sheet2.xml")?;
        self.product_accountability(&mut Sheet::new(&mut zip, &[35, 26, 24, 25, 32, 37], None)?, part_number)?;
        part(&mut zip, "xl/worksheets/sheet3.xml")?;
        let widths = CHARACTERISTICS.map(|(_, width)| width);
        self.characteristic_accountability(&mut Sheet::new(&mut zip, &widths, Some(HEADER_ROW))?, part_number)?;
        Ok(zip.finish()?.into_inner())
    }

    /// Fields 1 to 4, which head every form.
    fn form_heading(&self, sheet: &mut Sheet<'_>, title: &str, part_number: Option<&str>) -> io::Result<()> {
        sheet.row(Style::Heading, &[title])?;
        sheet.row(Style::Heading, &["1. Part Number", "2. Part Name", "3. Serial Number", "4. FAIR Identifier"])?;
        sheet.row(Style::Plain, &[part_number.unwrap_or_default(), &self.part])
    }

    fn part_accountability(&self, sheet: &mut Sheet<'_>, part_number: Option<&str>) -> io::Result<()> {
        self.form_heading(sheet, "AS9102C Form 1 \u{2014} Part Number Accountability", part_number)?;
        for (label, value) in [
            ("5. Part Revision Level", self.revision.as_deref().unwrap_or_default()),
            ("6. Drawing Number", ""),
            ("7. Drawing Revision Level", ""),
            ("8. Additional Changes", ""),
            ("9. Manufacturing Process Reference", ""),
            ("10. Organization Name", ""),
            ("11. Supplier Code", ""),
            ("12. Purchase Order Number", ""),
            ("13. Detail / Assembly", ""),
            ("14. Full / Partial FAI", ""),
            ("Baseline Part Number and Revision", ""),
            ("Reason for Full / Partial FAI", ""),
        ] {
            sheet.row(Style::Plain, &[label, value])?;
        }
        sheet.row(Style::Heading, &["Assembly Index"])?;
        sheet.row(Style::Heading, &["15. Part Number", "16. Part Name", "17. Part Type", "18. FAIR Identifier"])?;
        sheet.row(Style::Plain, &[])?;
        for label in [
            "19. Documented Nonconformances?",
            "20. FAIR Verified By",
            "21. Verification Date",
            "22. FAIR Reviewed / Approved By",
            "23. Review / Approval Date",
            "24. Customer Approval",
            "25. Customer Approval Date",
            "26. Comments",
        ] {
            sheet.row(Style::Plain, &[label])?;
        }
        sheet.row(Style::Plain, &["General Tolerance", self.general_tolerance.as_deref().unwrap_or_default()])?;
        sheet.finish()
    }

    fn product_accountability(&self, sheet: &mut Sheet<'_>, part_number: Option<&str>) -> io::Result<()> {
        self.form_heading(sheet, "AS9102C Form 2 \u{2014} Product Accountability", part_number)?;
        sheet.row(Style::Heading, &["Materials and Special Processes"])?;
        sheet.row(
            Style::Heading,
            &[
                "5. Material or Process Name",
                "6. Specification Number",
                "7. Code",
                "8. Supplier",
                "9. Customer Approval Verification",
                "10. Certificate of Conformance Number",
            ],
        )?;
        sheet.row(Style::Plain, &[self.material.as_deref().unwrap_or_default()])?;
        sheet.row(Style::Plain, &[])?;
        sheet.row(Style::Heading, &["Functional Testing"])?;
        sheet.row(
            Style::Heading,
            &["11. Functional Test Procedure Number", "12. Acceptance Report Number", "13. Comments"],
        )?;
        sheet.finish()
    }

    fn characteristic_accountability(&self, sheet: &mut Sheet<'_>, part_number: Option<&str>) -> io::Result<()> {
        self.form_heading(
            sheet,
            "AS9102C Form 3 \u{2014} Characteristic Accountability, Verification and Compatibility Evaluation",
            part_number,
        )?;
        sheet.row(Style::Plain, &["Part Revision Level", self.revision.as_deref().unwrap_or_default()])?;
        sheet.row(
            Style::Plain,
            &["Design requirements only. Reference and unresolved rows are not inspection results."],
        )?;
        sheet.row(Style::Plain, &[])?;
        sheet.row(Style::Heading, &CHARACTERISTICS.map(|(name, _)| name))?;
        for group in &self.groups {
            for row in &group.rows {
                sheet.start_row(Style::Wrapped)?;
                sheet.text(b'A', &row.id)?;
                sheet.text(b'D', &group.callout)?;
                sheet.text(b'H', group.problem.as_deref().unwrap_or_default())?;
                sheet.text(b'J', &group.id)?;
                sheet.text(b'K', group.item_id.as_deref().unwrap_or_default())?;
                sheet.number(b'L', row.member)?;
                sheet.text(b'M', row.kind.key())?;
                sheet.text(b'N', row.applicability(group).key())?;
                sheet.number(b'O', row.nominal.filter(|value| value.is_finite()))?;
                sheet.text(b'P', row.unit.map_or("", InspectionUnit::symbol))?;
                let (tolerance, fit) = row.tolerance();
                sheet.text(b'Q', tolerance)?;
                if let Some(fit) = fit {
                    sheet.text(b'R', fit.as_str())?;
                }
                sheet.number(b'S', row.lower_limit.filter(|value| value.is_finite()))?;
                sheet.number(b'T', row.upper_limit.filter(|value| value.is_finite()))?;
                let datums = row.datums(group);
                if !datums.is_empty() {
                    sheet.text(b'U', &json(datums)?)?;
                }
                if let Some(index) = row.locating_datum {
                    sheet.text(b'V', &group.datums[index].letter)?;
                }
                let faces = row.faces(group);
                if !faces.is_empty() {
                    sheet.text(b'W', &json(faces)?)?;
                }
                sheet.end_row()?;
            }
        }
        sheet.finish()
    }
}

fn part(zip: &mut ZipWriter<Cursor<Vec<u8>>>, name: &str) -> io::Result<()> {
    zip.start_file(name, SimpleFileOptions::default())?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#)
}

/// One worksheet, written row by row with inline strings.
struct Sheet<'a> {
    out: &'a mut dyn Write,
    row: usize,
    style: Style,
}

impl<'a> Sheet<'a> {
    fn new(out: &'a mut dyn Write, widths: &[u8], freeze: Option<usize>) -> io::Result<Self> {
        // Printed landscape, one page wide.
        write!(out, r#"<worksheet xmlns="{MAIN}"><sheetPr><pageSetUpPr fitToPage="1"/></sheetPr>"#)?;
        if let Some(n) = freeze {
            write!(
                out,
                r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="{n}" topLeftCell="A{}" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>"#,
                n + 1
            )?;
        }
        out.write_all(b"<cols>")?;
        for (n, width) in (1..).zip(widths) {
            write!(out, r#"<col min="{n}" max="{n}" width="{width}" customWidth="1"/>"#)?;
        }
        out.write_all(b"</cols><sheetData>")?;
        Ok(Self { out, row: 0, style: Style::Plain })
    }

    fn start_row(&mut self, style: Style) -> io::Result<()> {
        self.row += 1;
        if self.row > 1_048_576 {
            return Err(io::Error::other("the inspection plan exceeds the XLSX row limit"));
        }
        self.style = style;
        write!(self.out, r#"<row r="{}">"#, self.row)
    }

    fn end_row(&mut self) -> io::Result<()> {
        self.out.write_all(b"</row>")
    }

    fn row(&mut self, style: Style, cells: &[&str]) -> io::Result<()> {
        self.start_row(style)?;
        for (column, value) in (b'A'..).zip(cells) {
            self.text(column, value)?;
        }
        self.end_row()
    }

    fn cell(&mut self, column: u8, kind: &str) -> io::Result<()> {
        write!(self.out, r#"<c r="{}{}" s="{}"{kind}>"#, char::from(column), self.row, self.style as u8)
    }

    /// An inline string; an empty one writes no cell.
    fn text(&mut self, column: u8, value: &str) -> io::Result<()> {
        if value.is_empty() {
            return Ok(());
        }
        if value.encode_utf16().count() > 32_767 {
            return Err(io::Error::other("an inspection workbook cell exceeds the XLSX text limit"));
        }
        self.cell(column, r#" t="inlineStr""#)?;
        self.out.write_all(br#"<is><t xml:space="preserve">"#)?;
        let mut start = 0;
        for (index, c) in value.char_indices() {
            let escaped: &[u8] = match c {
                '&' => b"&amp;",
                '<' => b"&lt;",
                '>' => b"&gt;",
                '\r' => b"&#13;",
                '\u{0}'..='\u{8}' | '\u{b}'..='\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => {
                    return Err(io::Error::other("an inspection workbook cell contains invalid XML text"));
                }
                // Spreadsheet readers decode _xHHHH_ as a character; escape the underscore.
                '_' if value.as_bytes().get(index..index + 7).is_some_and(|bytes| {
                    bytes[1] == b'x' && bytes[2..6].iter().all(u8::is_ascii_hexdigit) && bytes[6] == b'_'
                }) =>
                {
                    b"_x005F_"
                }
                _ => continue,
            };
            self.out.write_all(&value.as_bytes()[start..index])?;
            self.out.write_all(escaped)?;
            start = index + c.len_utf8();
        }
        self.out.write_all(&value.as_bytes()[start..])?;
        self.out.write_all(b"</t></is></c>")
    }

    fn number(&mut self, column: u8, value: Option<impl Display>) -> io::Result<()> {
        if let Some(value) = value {
            self.cell(column, "")?;
            write!(self.out, "<v>{value}</v></c>")?;
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        self.out.write_all(
            concat!(
                r#"</sheetData><pageMargins left="0.4" right="0.4" top="0.5" bottom="0.5" header="0.3" footer="0.3"/>"#,
                r#"<pageSetup orientation="landscape" fitToWidth="1" fitToHeight="0"/></worksheet>"#
            )
            .as_bytes(),
        )
    }
}

#[cfg(test)]
pub(super) mod tests;
