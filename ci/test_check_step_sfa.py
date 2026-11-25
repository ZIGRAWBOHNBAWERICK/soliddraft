"""The NIST analyzer check must read SFA's output the way SFA writes it, and fail on findings."""
import importlib.util
import json
import pathlib
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('check_step_sfa', ROOT / 'ci/check-step-sfa.py')
sfa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sfa)

# Lines from SFA 5.54 on the P1 bracket, and on a copy with a short entity and an unknown one.
CLEAN = """Generating STEP Entity CSV files
datum (3)
ROSE: warning: No system schema path. Set $ROSE_RUNTIME or $ROSE_SCHEMA.
p1-bracket-pmi(7): "SetOfgeometric_tolerance_modifier": Best-fit class is "RoseEnumSet".
CSV files written to:
"""
BROKEN = CLEAN + """broken(9292): error: Expecting 3 attributes, found 2 instead.
broken(9491): warning: Unknown type 'NOT_AN_ENTITY', entity ignored.
"""

# CSV files as SFA writes them: a count line, the columns, then one row per entity.
FILES = {
    'datum_system.csv': 'datum_system (1)\r\nID,name,constituents\r\n'
                        '9294,A|B|C,(3) datum_reference_compartment 9295 9296 9297\r\n',
    'id_attribute.csv': 'id_attribute (3)\r\nID,attribute_value,identified_item\r\n'
                        '9291,Bore finish,property_definition 9284\r\n'
                        '9366,Slot/length,dimensional_location 9359\r\n'
                        '9387,Slot/end radius,dimensional_size 9384\r\n',
    'flatness_tolerance.csv': 'flatness_tolerance (1)\r\n'
                              'ID,name,description,magnitude,toleranced_shape_aspect\r\n'
                              '9400,Flatness of A,,0.05 (length_measure_with_unit 9401),datum_feature 9260\r\n',
    'measure_representation_item.csv': 'measure_representation_item (1)\r\n'
                                       'ID,name,value_component,unit_component\r\n'
                                       '9286,Ra,1.6,(length_unit) (si_unit) 9287\r\n',
    'general_property.csv': 'general_property (1)\r\nID,id,name,description\r\n'
                            '9483,note 1,note 1,user defined attribute\r\n',
    'Header.csv': 'Name,p1-bracket-pmi\r\n',
}


class Output(unittest.TestCase):
    def test_a_clean_run_has_no_findings(self):
        self.assertEqual(sfa.findings(CLEAN), [])

    def test_syntax_errors_and_unknown_entities_are_findings(self):
        self.assertEqual(sfa.findings(BROKEN), [
            'line 9292: error: Expecting 3 attributes, found 2 instead.',
            "line 9491: warning: Unknown type 'NOT_AN_ENTITY', entity ignored."])

    def test_a_crash_and_sfa_error_lines_are_findings(self):
        self.assertEqual(len(sfa.findings('*** Error processing #12=DATUM\n    while executing\n')), 2)


class Reading(unittest.TestCase):
    def read(self, files):
        with tempfile.TemporaryDirectory() as directory:
            for name, text in files.items():
                pathlib.Path(directory, name).write_bytes(text.encode())
            return sfa.reading(directory)

    def test_entity_numbers_are_left_out_and_text_keeps_its_digits(self):
        found, problems = self.read(FILES)
        self.assertEqual(problems, [])
        self.assertEqual(found['entities']['datum_system']['rows'],
                         [['A|B|C', '(3) datum_reference_compartment']])
        self.assertEqual(found['entities']['flatness_tolerance']['rows'],
                         [['Flatness of A', '', '0.05 (length_measure_with_unit)', 'datum_feature']])
        self.assertEqual(found['entities']['measure_representation_item']['rows'],
                         [['Ra', '1.6', '(length_unit) (si_unit)']])
        self.assertEqual(found['entities']['general_property']['rows'],
                         [['note 1', 'note 1', 'user defined attribute']])
        self.assertNotIn('Header', found['entities'])

    def test_items_are_named_by_id_attribute_and_tolerance_names(self):
        found, _ = self.read(FILES)
        self.assertEqual(found['items'], {
            'Bore finish': ['property_definition'],
            'Flatness of A': ['flatness_tolerance'],
            'Slot': ['dimensional_location', 'dimensional_size'],
        })

    def test_a_count_that_differs_from_the_rows_is_a_problem(self):
        files = dict(FILES, **{'datum.csv': 'datum (3)\r\nID,name,identification\r\n9262,,A\r\n'})
        _, problems = self.read(files)
        self.assertEqual(problems, ['datum: SFA counted 3 and wrote 1 rows'])


class Expectation(unittest.TestCase):
    def setUp(self):
        self.expected = json.loads((ROOT / 'ci/fixtures/pmi/p1-bracket-sfa.json').read_text())

    def test_the_bracket_expectation_names_its_fourteen_items(self):
        self.assertEqual(len(self.expected['items']), 14)
        self.assertEqual(self.expected['items']['Bore finish'], ['property_definition'])
        self.assertEqual(self.expected['entities']['surface_texture_representation']['rows'],
                         [['', '(1) descriptive_representation_item   (1) measure_representation_item',
                           '(geometric_representation_context) (global_uncertainty_assigned_context) '
                           '(global_unit_assigned_context)']])

    def test_a_reading_equal_to_the_expectation_passes_and_a_changed_row_fails(self):
        found = json.loads(json.dumps(self.expected))
        with mock.patch('builtins.print'):
            self.assertEqual(sfa.compare(found, self.expected), [])
            found['entities']['datum']['rows'][0][-1] = 'D'
            del found['items']['Bore finish']
            self.assertEqual(sfa.compare(found, self.expected), ['datum', 'items'])

    def test_the_check_refuses_to_run_beside_excel(self):
        with mock.patch.object(sfa, 'excel_running', return_value=True):
            with self.assertRaises(SystemExit) as stop:
                sfa.analyse('sfa-cl.exe', 'bracket.step', '.')
        self.assertIn('Excel is running', str(stop.exception))


if __name__ == '__main__':
    unittest.main()
