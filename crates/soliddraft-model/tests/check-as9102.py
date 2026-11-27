"""Check AS9102 workbook/CSV pairs with openpyxl, a reader independent of SolidDraft.

Generate the pairs with SOLIDDRAFT_PMI_PROOF_DIR=<directory> and
cargo test --locked -p soliddraft-model --lib as9102 (P1 from its STEP fixture), and
cargo test --locked -p soliddraft-app --test pmi_inspection (the P1 package, and a part with
the newer PMI kinds, from the package and from the plan).
Then run: python3 crates/soliddraft-model/tests/check-as9102.py <directory>
Every NAME.xlsx with a NAME.csv beside it is checked.
"""

import csv
import json
import sys
from pathlib import Path

from openpyxl import load_workbook


FORMS = ["Form 1", "Form 2", "Form 3"]
HEADER_ROW = 7
TEXT = {
    "8. Requirement": "pmi_callout",
    "12. Additional Data / Comments": "problem",
    "Group ID": "group_id",
    "PMI Item": "pmi_item",
    "Kind": "kind",
    "Applicability": "applicability",
    "Unit": "unit",
    "Tolerance": "tolerance",
    "Fit Class": "fit_class",
    "Locating Datum": "locating_datum",
}
NUMBERS = {"Member": "member", "Nominal": "nominal", "Lower Limit": "lower_limit", "Upper Limit": "upper_limit"}
JSON = {"Datum Order (JSON)": "datum_order", "Face References (JSON)": "face_refs"}
RESULTS = ("9. Results", "10. Designed / Qualified Tooling", "11. Nonconformance Number", "Disposition")


def check(workbook_path):
    with workbook_path.with_suffix(".csv").open(encoding="utf-8", newline="") as source:
        records = list(csv.DictReader(source))
    workbook = load_workbook(workbook_path, read_only=True)
    try:
        assert workbook.sheetnames == FORMS, workbook.sheetnames
        form1, form2, form3 = workbook.worksheets
        part = records[0]["part"] if records else None
        revision = records[0]["revision"] if records else None
        for sheet in workbook.worksheets:
            assert sheet["A2"].value == "1. Part Number" and sheet["B2"].value == "2. Part Name"
            assert part is None or sheet["B3"].value == part, (sheet.title, sheet["B3"].value)
            assert sheet["C3"].value is None and sheet["D3"].value is None, "serial number and FAIR identifier"
        assert form1["A4"].value == "5. Part Revision Level"
        assert (form1["B4"].value or "") == (revision or ""), form1["B4"].value
        for row in range(19, 27):
            assert form1[f"B{row}"].value is None, f"Form 1 B{row} is an approval, not a design value"
        assert all(form2[f"{column}6"].value is None for column in "BCDEF"), "no certificates or approvals"

        headers = [cell.value for cell in next(form3.iter_rows(min_row=HEADER_ROW, max_row=HEADER_ROW))]
        assert headers[0] == "5. Char. No.", headers
        seen = []
        for cells in form3.iter_rows(min_row=HEADER_ROW + 1, max_col=len(headers)):
            values = dict(zip(headers, (cell.value for cell in cells)))
            assert len(values) == len(headers), values
            if values.get("5. Char. No.") is None:
                continue
            seen.append(values)
        assert len(seen) == len(records), (len(seen), len(records))
        for values, record in zip(seen, records):
            key = record["characteristic_id"]
            assert values["5. Char. No."] == key, (values["5. Char. No."], key)
            assert record["actual"] == "" and record["disposition"] == "", key
            for name in RESULTS:
                assert values[name] is None, (key, name)
            for name, field in TEXT.items():
                assert (values[name] or "") == record[field], (key, name, values[name], record[field])
            for name, field in NUMBERS.items():
                actual, intended = values[name], record[field]
                if intended == "":
                    assert actual is None, (key, name, actual)
                else:
                    assert isinstance(actual, (int, float)) and abs(actual - float(intended)) < 1e-12, (
                        key, name, actual, intended)
            for name, field in JSON.items():
                assert json.loads(values[name] or "[]") == json.loads(record[field]), (key, name)
        kinds = sorted({values["Kind"] for values in seen})
        print(f"{workbook_path.name}: openpyxl read {len(FORMS)} forms and {len(seen)} characteristics "
              f"matching the CSV, with blank results; kinds: {', '.join(kinds)}")
    finally:
        workbook.close()


if __name__ == "__main__":
    directory = Path(sys.argv[1])
    pairs = sorted(path for path in directory.glob("*.xlsx") if path.with_suffix(".csv").is_file())
    assert pairs, f"no workbook with a CSV beside it in {directory}"
    for path in pairs:
        check(path)
