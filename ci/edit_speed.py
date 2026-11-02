#!/usr/bin/env python3
"""Pair the five ordinary edits on a pull request and its base, and compare their answers.

Both revisions run the head's scenario runner on one CPU in alternating order. One warmup
pair is discarded. Answers must match in every run; kernel work may differ, so it is
recorded, never compared. Timing means nothing outside a hosted runner.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys

TARGETS = {"bracket": {27: "edit_dimension", 43: "set_value"},
           "drawing": {24: "edit_extrude"},
           "references": {17: "edit_extrude", 18: "edit_dimension"}}
LABELS = {("bracket", 27): "Bracket dimension", ("bracket", 43): "Bracket value",
          ("drawing", 24): "Drawing extrusion", ("references", 17): "Reference extrusion",
          ("references", 18): "Reference dimension"}
MEASURED_PAIRS = 9
PIN = re.compile(r'pub const CATALLAX_REVISION: &str = "([0-9a-f]{40})";')
PIN_SOURCE = "crates/soliddraft-kernel-catallax/src/lib.rs"


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def checkout(root, sha):
    subprocess.run(["git", "-C", str(root), "checkout", "--quiet", "--detach", sha], check=True)


def build(args):
    """Builds both revisions in one checkout and target, so only their difference recompiles."""
    root, pins = args.app, []
    args.binaries.mkdir(parents=True)
    for revision, sha in (("baseline", os.environ["BASE_SHA"]), ("candidate", os.environ["HEAD_SHA"])):
        checkout(root, sha)
        pins.append(PIN.search((root / PIN_SOURCE).read_text())[1])
        subprocess.run(["cargo", "build", "--release", "--locked", "-p", "soliddraft-cli"], cwd=root, check=True)
        (args.binaries / revision).write_bytes((root / "target/release/soliddraft").read_bytes())
        (args.binaries / revision).chmod(0o755)
    assert pins[0] == pins[1], f"the kernel revision moved: {pins}"


def truncated(source, targets):
    """A scenario cut off after its last edit, with its export and drawing acceptance left out.

    Those run in the application scenarios job and are outside the timed clicks.
    """
    for number, tool in targets.items():
        assert source["steps"][number - 1]["tool"] == tool, f"wrong edit target: {number}"
    steps = source["steps"][:max(targets)]
    for step in steps:
        step.pop("checked_step_export", None)
        step.pop("drawing", None)
    return {**source, "steps": steps}


def compare_answers(comparator, left, right, pins):
    """Fails on any answer that differs. A count of background meshes is delivery, not an answer."""
    for difference in comparator.problems(left, right, pins):
        if re.fullmatch(r"\d{4}/command.json", difference):
            values = [json.loads(comparator.text(root / "commands" / difference, root, pins))
                      for root in (left, right)]
            for value in values:
                value["rebuild"].pop("background_meshes_rebuilt", None)
            if values[0] == values[1]:
                continue
        raise AssertionError(f"application answers differ in {right}: {difference}")


def sample(folder, step):
    """One click's timing and the foreground kernel calls inside it."""
    calls = []
    for name in ("kernel.json", "display.kernel.json"):
        calls.extend(json.loads((folder / name).read_text())["calls"])
    foreground = [c for c in calls if c["on_capture_thread"] and not c.get("nested", False)]
    topology = [c for c in foreground if c["operation"] == "topology"]
    timing = step["timing"]
    return dict(click_ms=timing["elapsed_ms"], split_ms=timing["split_ms"], work=timing["work"],
                kernel_ms=sum(c["wall_ms"] for c in foreground),
                topology_ms=sum(c["wall_ms"] for c in topology),
                topology_builds=sum(1 for c in topology if ((c.get("work") or {}).get("used") or 0) > 0),
                boolean_ms=sum(c["wall_ms"] for c in foreground if c["operation"] == "boolean"),
                carrier_calls=sum(1 for c in foreground if c["operation"] == "face_carriers"))


def medians(samples):
    """Per-edit medians over measured pairs, with how many pairs the candidate won."""
    out = []
    for (scenario, step), label in LABELS.items():
        rows = {revision: {s["pair"]: s for s in samples if s["scenario"] == scenario and s["step"] == step
                           and s["revision"] == revision and not s["warmup"]}
                for revision in ("baseline", "candidate")}
        middle = {revision: {key: statistics.median(row[key] for row in rows[revision].values())
                             for key in ("click_ms", "kernel_ms", "topology_ms", "boolean_ms", "topology_builds")}
                  for revision in rows}
        before, after = middle["baseline"]["click_ms"], middle["candidate"]["click_ms"]
        out.append(dict(label=label, scenario=scenario, step=step, pairs=len(rows["candidate"]),
                        baseline=middle["baseline"], candidate=middle["candidate"],
                        reduction_percent=100 * (before - after) / before,
                        faster_pairs=sum(rows["candidate"][pair]["click_ms"] < rows["baseline"][pair]["click_ms"]
                                         for pair in rows["candidate"])))
    return out


def table(rows):
    lines = ["| Edit | Base click, ms | Head click, ms | Reduction | Faster pairs | "
             "Topology, ms, base → head | Topology builds, base → head |",
             "|---|---:|---:|---:|---:|---:|---:|"]
    for row in rows:
        base, head = row["baseline"], row["candidate"]
        lines.append(f"| {row['label']} | {base['click_ms']:.3f} | {head['click_ms']:.3f} | "
                     f"{row['reduction_percent']:.2f}% | {row['faster_pairs']}/{row['pairs']} | "
                     f"{base['topology_ms']:.3f} → {head['topology_ms']:.3f} | "
                     f"{base['topology_builds']:g} → {head['topology_builds']:g} |")
    return "\n".join(lines) + "\n"


def run(args):
    args.out.mkdir(parents=True)
    core = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {core})
    comparator = load(args.app / "ci/compare-scenario-bundles.py", "compare_bundles")
    pins = os.environ["BASE_SHA"], os.environ["HEAD_SHA"]
    scenarios = args.out / "inputs"
    scenarios.mkdir()
    for name, targets in TARGETS.items():
        source = json.loads((args.app / f"ci/scenarios/{name}.json").read_text())
        (scenarios / f"{name}.json").write_text(json.dumps(truncated(source, targets), indent=2) + "\n")
    samples, references = [], {}
    for pair in range(MEASURED_PAIRS + 1):
        for revision in (("baseline", "candidate") if pair % 2 == 0 else ("candidate", "baseline")):
            for name, targets in TARGETS.items():
                dest = args.out / f"{pair}-{revision}" / name
                dest.parent.mkdir(exist_ok=True)
                load_before = os.getloadavg()
                command = [sys.executable, str(args.app / "ci/run-scenarios.py"), str(scenarios / f"{name}.json"),
                           "--out", str(dest), "--soliddraft", str(args.binaries / revision),
                           "--catallax-verify", str(args.verifier)]
                with (dest.parent / f"{name}.log").open("w") as log:
                    result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT)
                assert result.returncode == 0, f"scenario failed: {dest.parent / (name + '.log')}"
                report = json.loads((dest / "result.json").read_text())
                assert report["pass"], f"incomplete or failed scenario: {dest}"
                if name in references:
                    compare_answers(comparator, references[name], dest, pins)
                else:
                    references[name] = dest
                for number in targets:
                    row = dict(pair=pair, warmup=pair == 0, revision=revision, scenario=name, step=number,
                               load_before=load_before, load_after=os.getloadavg(),
                               **sample(dest / "commands" / f"{number:04}", report["steps"][number - 1]))
                    samples.append(row)
                    print(json.dumps(row), flush=True)
                (args.out / "samples.json").write_text(json.dumps(samples, indent=2) + "\n")
    rows = medians(samples)
    report = dict(base=pins[0], head=pins[1], core=core, measured_pairs=MEASURED_PAIRS,
                  binary_sha256={r: hashlib.sha256((args.binaries / r).read_bytes()).hexdigest()
                                 for r in ("baseline", "candidate")},
                  comparison="identical part files, recipes, kernel questions, views and command answers in every run",
                  medians=rows, samples=samples)
    (args.out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(table(rows))
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as out:
            out.write(f"Base `{pins[0]}`, head `{pins[1]}`, {MEASURED_PAIRS} measured pairs.\n\n" + table(rows))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("build", "run"))
    parser.add_argument("--app", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--binaries", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--verifier", type=lambda value: Path(value).resolve())
    parser.add_argument("--out", type=lambda value: Path(value).resolve())
    arguments = parser.parse_args()
    {"build": build, "run": run}[arguments.mode](arguments)
