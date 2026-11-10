"""Required checks reject incomplete, duplicated or unsuccessful partition reports."""
import importlib.util
import json
import pathlib
import tempfile
import unittest
import xml.etree.ElementTree as ET

from balance import inventory_digest

spec = importlib.util.spec_from_file_location('merge_shards', pathlib.Path(__file__).with_name('merge-shards.py'))
shards = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shards)


def reports(directory, count, active=None):
    active = set(range(1, count + 1)) if active is None else active
    inventory = [f'bin\ttest_{i}' for i in sorted(active)]
    for i in range(1, count + 1):
        folder = directory / f'workspace-partition-linux-{i}'
        folder.mkdir()
        selected = [f'bin\ttest_{i}'] if i in active else []
        metadata = {'format': 'soliddraft-partition/1', 'partition': i, 'partitions': count,
                    'inventory_count': len(inventory), 'inventory_sha256': inventory_digest(inventory),
                    'selected_count': len(selected), 'selected_sha256': inventory_digest(selected),
                    'returncode': 0, 'finished_at': f'2026-01-01T00:00:{i:02d}+00:00'}
        (folder / 'inventory.json').write_text(json.dumps(metadata))
        if selected:
            root = ET.Element('testsuites', tests='1', failures='0', errors='0', skipped='0',
                              time='10', timestamp=metadata['finished_at'])
            suite = ET.SubElement(root, 'testsuite', name='bin', tests='1')
            ET.SubElement(suite, 'testcase', classname='bin', name=f'test_{i}', time='1')
            ET.ElementTree(root).write(folder / 'junit.xml')


class Coverage(unittest.TestCase):
    def test_four_required_reports_cover_all_execution_partitions_once(self):
        for count in (4, 8, 12):
            with self.subTest(count=count), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                reports(root, count)
                groups = [shards.merge(root, f'{i}/4', count) for i in range(1, 5)]
                names = [c.get('name') for group in groups for c in group.iter('testcase')]
                self.assertEqual(set(names), {f'test_{i}' for i in range(1, count + 1)})
                self.assertEqual(len(names), count)
                if count == 12:
                    self.assertEqual(float(groups[0].get('time')), 18)

    def test_empty_partitions_keep_the_no_tests_pass_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            reports(root, 12, active={1})
            groups = [shards.merge(root, f'{i}/4', 12) for i in range(1, 5)]
            self.assertEqual([int(g.get('tests')) for g in groups], [1, 0, 0, 0])

    def test_duplicates_and_omissions_cannot_pass(self):
        for mode in ('duplicate', 'omitted'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                reports(root, 8)
                folder = root / 'workspace-partition-linux-2'
                metadata = json.loads((folder / 'inventory.json').read_text())
                if mode == 'duplicate':
                    tree = ET.parse(folder / 'junit.xml')
                    next(tree.getroot().iter('testcase')).set('name', 'test_1')
                    tree.write(folder / 'junit.xml')
                    metadata['selected_sha256'] = inventory_digest(['bin\ttest_1'])
                else:
                    (folder / 'junit.xml').unlink()
                    metadata.update(selected_count=0, selected_sha256=inventory_digest([]))
                (folder / 'inventory.json').write_text(json.dumps(metadata))
                with self.assertRaisesRegex(ValueError, 'exactly once'):
                    shards.merge(root, '1/4', 8)

    def test_missing_or_failed_execution_is_not_hidden_by_other_groups(self):
        for mode in ('missing', 'failed'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                reports(root, 8)
                path = root / 'workspace-partition-linux-8/inventory.json'
                if mode == 'missing':
                    path.unlink()
                    error = FileNotFoundError
                else:
                    metadata = json.loads(path.read_text())
                    metadata['returncode'] = 100
                    path.write_text(json.dumps(metadata))
                    error = ValueError
                with self.assertRaises(error):
                    shards.merge(root, '1/4', 8)


if __name__ == '__main__':
    unittest.main()
