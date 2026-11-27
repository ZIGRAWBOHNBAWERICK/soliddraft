import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preview  # noqa: E402

METAL = {'name': 'Apple M3', 'backend': 'Metal', 'device_type': 'IntegratedGpu'}


def step(bundle, number, counts, adapter=METAL, calls=()):
    folder = bundle / 'commands' / f'{number:04}'
    folder.mkdir(parents=True)
    (folder / 'display.json').write_text(json.dumps({'revision': number}))
    if counts is not None:
        (folder / 'display.gpu.json').write_text(json.dumps({**counts, 'revision': number, 'adapter': adapter}))
    (folder / 'command.json').write_text(json.dumps({'revision': number}))
    (folder / 'kernel.json').write_text(json.dumps({'calls': [{'operation': q, 'wall_ms': ms} for q, ms in calls]}))
    (folder / 'display.kernel.json').write_text(json.dumps({'calls': []}))


class SummarizeTest(unittest.TestCase):
    def counts(self, **change):
        counts = {'status': 'GPU preview on backend: Metal', 'gpu_previews': 2, 'cpu_previews': 0, 'refused_faces': 0}
        counts.update(change)
        return counts

    def test_a_hardware_run_with_gpu_previews_is_accepted_and_sums_checked_calls(self):
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, self.counts(gpu_previews=1), calls=[('tessellate', 12.5), ('preview', 3)])
            step(bundle, 2, self.counts(), calls=[('tessellate', 7.5), ('tessellate_edges', 1)])
            record = preview.summarize('box', bundle, 0)
        self.assertTrue(record['accepted'], record['problems'])
        self.assertFalse(record['strict'], 'a preview call is in the kernel log')
        self.assertEqual(record['kernel_previews'], 1)
        self.assertEqual(record['gpu_previews'], 2)
        self.assertEqual(record['checked_ms'], {'1': 12.5, '2': 8.5})
        self.assertEqual(record['checked_total_ms'], 21.0)

    def test_a_cpu_preview_or_a_software_adapter_fails(self):
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, self.counts(cpu_previews=1))
            self.assertIn('1 CPU previews for the display', preview.summarize('box', bundle, 0)['problems'])
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, self.counts(), adapter={**METAL, 'name': 'llvmpipe', 'device_type': 'Cpu'})
            self.assertFalse(preview.summarize('box', bundle, 0)['accepted'])

    def test_the_cpu_preview_route_fails(self):
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, self.counts(status='GPU preview off, CPU preview kept', gpu_previews=0))
            self.assertFalse(preview.summarize('box', bundle, 0)['accepted'])

    def test_a_step_without_a_gpu_delivery_record_fails(self):
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, None)
            record = preview.summarize('box', bundle, 0)
        self.assertFalse(record['accepted'])
        self.assertIn('GPU preview not on: not installed, no display.gpu.json', record['problems'])


if __name__ == '__main__':
    unittest.main()
