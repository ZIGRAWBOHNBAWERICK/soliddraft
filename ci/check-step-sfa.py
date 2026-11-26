#!/usr/bin/env python3
"""Reads a STEP file with the NIST STEP File Analyzer and checks its manufacturing information.

usage: check-step-sfa.py install <dir>
       check-step-sfa.py <sfa-cl.exe> <file.step> <expected.json>
       check-step-sfa.py <sfa-cl.exe> <file.step> --print

The STEP File Analyzer and Viewer (SFA) is NIST's reader for the CAx-IF recommended
practices. Its command-line version runs only on Windows. Its PMI and validation property
reports need Microsoft Excel, which hosted runners lack, so this check uses what SFA gives
without Excel: its syntax checker, and one CSV file per entity type it read.

The check passes when SFA reports no syntax error or warning, and its reading equals
<expected.json>: the rows of each entity type the expectation lists, and the items SFA
finds by name with the entity types it reads each one as. Entity numbers are left out, so
the reading does not depend on the file's numbering. `--print` writes the whole reading in
the form the check compares; an expectation keeps the manufacturing information types.

`install <dir>` downloads the pinned SFA, checks its checksum, puts sfa-cl.exe in <dir>,
and installs SFA's IFCsvr STEP toolkit without a window when it is missing.

sfa-cl.exe ends every running Excel before it reads a file, so this check refuses to run
while Excel is open. This checker belongs to CI. The application never runs SFA.
"""

import csv
import hashlib
import io
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import zipfile

VERSION = "5.54"
URL = ("https://raw.githubusercontent.com/usnistgov/SFA/"
       "1236017676c0fa1f24f0fb4642fb088437a406f7/Release/SFA-5.54.zip")
SHA256 = "796b5c162c7751d6eb5a5ed3b4866bd41bfac8b22fae5213d9eb3066df6c1582"
# The toolkit version SFA 5.54 requires, from its source (sfa-proc.tcl, installIFCsvr).
IFCSVR_VERSION = 20260820
IFCSVR_KEY = (r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"
              r"\{3C8CE0A4-803B-48A6-96A0-A3DDD5AE5596}")
IFCSVR_MSI = "ifcsvrr300_setup_1008_en-update.msi"

# Options for the command-line version, read in place of the user's saved options:
# every entity category and the syntax checker, never Excel and never the Viewer.
OPTIONS = """set opt(debugNOXL) 1
set opt(xlFormat) CSV
set opt(outputOpen) 0
set opt(logFile) 1
set opt(syntaxChecker) 1
set opt(viewPart) 0
set opt(viewPMI) 0
set opt(viewTessPart) 0
set opt(viewFEA) 0
set gen(View) 0
set opt(INVERSE) 0
set opt(xlMaxRows) 1048576
foreach category {stepAP242 stepCOMM stepCOMP stepCPNT stepFEAT stepGEOM stepHOLE stepKINE stepMEAS stepOTHR stepPRES stepQUAL stepREPR stepSHAP stepTOLR} {set opt($category) 1}
"""

# A syntax checker finding: "<file>(<line>): error: <text>" or "... warning: ...".
FINDING = re.compile(r"^\s*\S+\((\d+)\): (error|warning): (.*)$")
# A count line at the top of each CSV file: "datum (3)" or "(a)(b) (2)".
COUNT = re.compile(r"^(.*) \((\d+)\)$")


def findings(output):
    """The syntax checker's errors and warnings, and SFA's own error lines."""
    found = []
    for line in output.splitlines():
        match = FINDING.match(line)
        if match:
            found.append(f"line {match[1]}: {match[2]}: {match[3]}")
        elif line.lstrip().startswith("*") or "Error processing" in line or "while executing" in line:
            found.append(line.strip())
    return found


def tables(directory):
    """Each CSV file SFA wrote, as its entity type, its stated count and its rows."""
    out = {}
    for path in sorted(pathlib.Path(directory).glob("*.csv")):
        if path.stem == "Header":
            continue
        with open(path, encoding="utf-8", errors="replace", newline="") as file:
            rows = list(csv.reader(file))
        match = COUNT.match(rows[0][0]) if rows and rows[0] else None
        if match is None:
            raise SystemExit(f"{path}: no entity count on its first line")
        out[path.stem] = {"names": match[1], "count": int(match[2]), "header": rows[1], "rows": rows[2:]}
    return out


def unnumbered(cell, types):
    """A cell with the entity numbers after each referenced entity type left out.

    A type is known from SFA's own CSV files, or by its underscore when it has no file.
    """
    cell = re.sub(r"\b([a-z0-9]+(?:_[a-z0-9]+)+)(?: \d+)+\b", r"\1", cell)
    cell = re.sub(r"\b(" + "|".join(sorted(types, key=len, reverse=True)) + r")(?: \d+)+\b", r"\1", cell)
    return re.sub(r"(\([a-z0-9_]+\))(?: \d+)+\b", r"\1", cell)


def reading(directory):
    """What SFA read, in the form the check compares."""
    found = tables(directory)
    types = set()
    for table in found.values():
        types.update(re.findall(r"[a-z0-9_]+", table["names"]))
    entities, problems = {}, []
    for name, table in found.items():
        if table["count"] != len(table["rows"]):
            problems.append(f"{name}: SFA counted {table['count']} and wrote {len(table['rows'])} rows")
        header = table["header"][1:]
        rows = [[unnumbered(cell, types) for cell in row[1:]] for row in table["rows"]]
        entities[name] = {"columns": header, "rows": sorted(rows)}
    items = {}

    def item(name, entity):
        items.setdefault(name, set()).add(entity)

    for row in found.get("id_attribute", {}).get("rows", []):
        # A part of an item carries the item's name and a slash, "Slot/width".
        item(row[1].split("/")[0], row[2].split(" ")[0])
    for name, table in found.items():
        if "toleranced_shape_aspect" in table["header"]:
            column = table["header"].index("name")
            for row in table["rows"]:
                item(row[column], name)
    return {"entities": entities, "items": {k: sorted(v) for k, v in sorted(items.items())}}, problems


def compare(found, expected):
    """The parts of the reading that differ from the expectation."""
    failures = []
    for name, want in sorted(expected["entities"].items()):
        got = found["entities"].get(name)
        if got != want:
            failures.append(name)
            print(f"{name}: SFA read", json.dumps(got, ensure_ascii=False))
            print(f"{name}: expected", json.dumps(want, ensure_ascii=False))
    if found["items"] != expected["items"]:
        failures.append("items")
        print("items: SFA read", json.dumps(found["items"], ensure_ascii=False))
        print("items: expected", json.dumps(expected["items"], ensure_ascii=False))
    return failures


def excel_running():
    listing = subprocess.run(["tasklist", "/FI", "IMAGENAME eq EXCEL.EXE", "/NH"],
                             capture_output=True, text=True, errors="replace")
    return "excel.exe" in listing.stdout.lower()


def analyse(sfa, step, work):
    """Runs SFA on a copy of the file in <work>; returns its output and the directory of its CSV files."""
    if excel_running():
        raise SystemExit("Excel is running; sfa-cl.exe would end it. Close Excel first.")
    work = pathlib.Path(work)
    copy = work / pathlib.Path(step).name
    shutil.copyfile(step, copy)
    options = work / "options.dat"
    options.write_text(OPTIONS)
    run = subprocess.run([str(pathlib.Path(sfa).resolve()), str(copy), "csv", "noopen", str(options)],
                         cwd=work, capture_output=True, timeout=600)
    output = (run.stdout + run.stderr).decode("utf-8", errors="replace")
    print(output)
    if run.returncode != 0:
        raise SystemExit(f"SFA exited with {run.returncode} on {step}")
    directory = work / f"{copy.stem}-sfa-csv"
    if not directory.is_dir():
        raise SystemExit(f"SFA wrote no CSV files for {step}")
    return output, directory


def download():
    for attempt in range(1, 6):
        try:
            request = urllib.request.Request(URL, headers={"User-Agent": "soliddraft-ci"})
            with urllib.request.urlopen(request, timeout=300) as response:
                return response.read()
        except (OSError, urllib.error.URLError) as error:
            if attempt == 5:
                raise SystemExit(f"SFA download failed after 5 attempts: {error}")
            time.sleep(5 * 2 ** (attempt - 1))


def ifcsvr_version():
    import winreg

    try:
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, IFCSVR_KEY) as key:
            text = winreg.QueryValueEx(key, "DisplayVersion")[0]
    except OSError:
        return 0
    digits = re.sub(r"\D", "", text[text.find("20"):]) if "20" in text else ""
    return int(digits) if digits else 0


def install(directory):
    directory = pathlib.Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    payload = download()
    if hashlib.sha256(payload).hexdigest() != SHA256:
        raise SystemExit("SFA checksum mismatch")
    with zipfile.ZipFile(io.BytesIO(payload)) as archive:
        (directory / "sfa-cl.exe").write_bytes(archive.read("sfa-cl.exe"))
    print(f"SFA {VERSION} command-line version in {directory}")
    if ifcsvr_version() >= IFCSVR_VERSION:
        print(f"IFCsvr {ifcsvr_version()} is installed")
        return
    # On its first run SFA copies the toolkit's installer to its temporary folder, opens it
    # in a window and exits. Close that window and install the toolkit without one.
    probe = directory / "first-run.step"
    probe.write_text("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n")
    subprocess.run([str(directory / "sfa-cl.exe"), str(probe)], cwd=directory, timeout=300)
    time.sleep(5)
    subprocess.run(["taskkill", "/F", "/IM", "msiexec.exe"], capture_output=True)
    candidates = [pathlib.Path(os.environ.get("LOCALAPPDATA", "")) / "Temp" / "SFA" / IFCSVR_MSI,
                  pathlib.Path(os.environ.get("TEMP", "")) / "SFA" / IFCSVR_MSI,
                  pathlib.Path(os.environ.get("USERPROFILE", "")) / "Documents" / IFCSVR_MSI,
                  directory / IFCSVR_MSI]
    msi = next((path for path in candidates if path.is_file()), None)
    if msi is None:
        raise SystemExit(f"SFA did not write {IFCSVR_MSI}; looked in {', '.join(map(str, candidates))}")
    log = directory / "ifcsvr-install.log"
    result = subprocess.run(["msiexec", "/i", str(msi), "/qn", "/norestart", "/l*v", str(log)], timeout=600)
    if result.returncode not in (0, 3010) or ifcsvr_version() < IFCSVR_VERSION:
        if log.exists():
            print(log.read_text(encoding="utf-16", errors="replace")[-4000:])
        raise SystemExit(f"IFCsvr installation failed: msiexec {result.returncode}, version {ifcsvr_version()}")
    print(f"IFCsvr {ifcsvr_version()} installed")


def main(arguments):
    if len(arguments) == 2 and arguments[0] == "install":
        install(arguments[1])
        return
    if len(arguments) != 3:
        raise SystemExit(__doc__)
    sfa, step, expectation = arguments
    with tempfile.TemporaryDirectory(prefix="sfa-") as work:
        output, directory = analyse(sfa, step, work)
        found, mismatched = reading(directory)
    problems = findings(output)
    problems += mismatched
    if expectation == "--print":
        json.dump(found, sys.stdout, indent=1, sort_keys=True, ensure_ascii=False)
        print()
    else:
        with open(expectation, encoding="utf-8") as file:
            expected = json.load(file)
        problems += [f"{part} differs from {expectation}" for part in compare(found, expected)]
    for problem in problems:
        print(f"{step}: {problem}")
    if problems:
        raise SystemExit(f"{step}: SFA found {len(problems)} problems")
    names = ", ".join(found["items"])
    print(f"{step}: SFA {VERSION} read {len(found['items'])} items with no syntax finding: {names}")


if __name__ == "__main__":
    main(sys.argv[1:])

