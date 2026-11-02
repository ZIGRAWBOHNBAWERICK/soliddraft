"""Tests for the paired edit timing's reading of its samples."""
import importlib.util
import json
import pathlib
import tempfile
import types
import unittest

ROOT = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("edit_speed", ROOT / "edit_speed.py")
edit_speed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(edit_speed)


def row(pair, revision, click, warmup=False, scenario="references", step=17):
    return dict(pair=pair, warmup=warmup, revision=revision, scenario=scenario, step=step, click_ms=click,
                kernel_ms=click / 2, topology_ms=click / 4, boolean_ms=click / 8, topology_builds=1)


def rows_for_every_edit():
    samples = []
    for scenario, step in edit_speed.LABELS:
        samples += [row(0, "baseline", 9999, True, scenario, step), row(0, "candidate", 1, True, scenario, step)]
        for pair, (base, head) in enumerate([(100, 90), (120, 80), (80, 85)], 1):
            samples += [row(pair, "baseline", base, False, scenario, step),
                        row(pair, "candidate", head, False, scenario, step)]
    return samples


class Medians(unittest.TestCase):
    def test_warmup_is_left_out_and_pairs_are_won_by_pair(self):
        result = {r["label"]: r for r in edit_speed.medians(rows_for_every_edit())}
        extrusion = result["Reference extrusion"]
        self.assertEqual(set(result), set(edit_speed.LABELS.values()))
        self.assertEqual((extrusion["baseline"]["click_ms"], extrusion["candidate"]["click_ms"]), (100, 85))
        self.assertEqual((extrusion["faster_pairs"], extrusion["pairs"]), (2, 3))
        self.assertAlmostEqual(extrusion["reduction_percent"], 15)

    def test_the_table_has_one_row_per_edit(self):
        text = edit_speed.table(edit_speed.medians(rows_for_every_edit()))
        self.assertEqual(len(text.splitlines()), 2 + len(edit_speed.LABELS))
        self.assertIn("| Reference extrusion | 100.000 | 85.000 | 15.00% | 2/3 |", text)


class Truncation(unittest.TestCase):
    def test_a_scenario_stops_after_its_last_edit_without_export_acceptance(self):
        source = dict(steps=[dict(tool="sketch", drawing={}), dict(tool="edit_extrude", checked_step_export={}),
                             dict(tool="save")])
        self.assertEqual(edit_speed.truncated(source, {2: "edit_extrude"})["steps"],
                         [dict(tool="sketch"), dict(tool="edit_extrude")])

    def test_a_moved_edit_is_refused(self):
        with self.assertRaises(AssertionError):
            edit_speed.truncated(dict(steps=[dict(tool="sketch")]), {1: "edit_extrude"})

    def test_the_named_edits_are_where_the_scenarios_have_them(self):
        for name, targets in edit_speed.TARGETS.items():
            source = json.loads((ROOT / "scenarios" / f"{name}.json").read_text())
            self.assertEqual(len(edit_speed.truncated(source, targets)["steps"]), max(targets))


class Answers(unittest.TestCase):
    def bundle(self, root, background):
        folder = root / "commands" / "0001"
        folder.mkdir(parents=True)
        command = dict(rebuild=dict(background_meshes_rebuilt=background), summary=dict(faces=6))
        (folder / "command.json").write_text(json.dumps(command))
        return root

    def comparator(self, differences):
        return types.SimpleNamespace(problems=lambda *_: differences, text=lambda path, *_: path.read_text())

    def test_only_a_background_mesh_count_may_differ(self):
        with tempfile.TemporaryDirectory() as temp:
            left = self.bundle(pathlib.Path(temp) / "left", 1)
            right = self.bundle(pathlib.Path(temp) / "right", 2)
            edit_speed.compare_answers(self.comparator(["0001/command.json"]), left, right, None)
            with self.assertRaisesRegex(AssertionError, "0001/recipe.json"):
                edit_speed.compare_answers(self.comparator(["0001/recipe.json"]), left, right, None)

    def test_another_command_difference_is_refused(self):
        with tempfile.TemporaryDirectory() as temp:
            left = self.bundle(pathlib.Path(temp) / "left", 1)
            right = self.bundle(pathlib.Path(temp) / "right", 1)
            text = (right / "commands/0001/command.json").read_text().replace('"faces": 6', '"faces": 7')
            (right / "commands/0001/command.json").write_text(text)
            with self.assertRaisesRegex(AssertionError, "0001/command.json"):
                edit_speed.compare_answers(self.comparator(["0001/command.json"]), left, right, None)


if __name__ == "__main__":
    unittest.main()
