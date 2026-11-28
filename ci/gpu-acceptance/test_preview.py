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


class ControlTest(unittest.TestCase):
    def test_the_control_expects_the_cpu_preview(self):
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            off = {'status': 'GPU preview off, CPU preview kept (preview.gpu-disabled: SOLIDDRAFT_GPU_PREVIEW is off)',
                   'gpu_previews': 0, 'cpu_previews': 3, 'refused_faces': 0}
            step(bundle, 1, off, calls=[('preview', 2.0)])
            self.assertTrue(preview.summarize('box', bundle, 0, control=True)['accepted'])
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, None, calls=[('preview', 2.0)])
            self.assertTrue(preview.summarize('box', bundle, 0, control=True)['accepted'],
                            'no delivery record is written without a GPU preview')
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, None)
            self.assertFalse(preview.summarize('box', bundle, 0, control=True)['accepted'])
        with tempfile.TemporaryDirectory() as folder:
            bundle = Path(folder)
            step(bundle, 1, {'status': 'GPU preview on backend: Metal', 'gpu_previews': 1, 'cpu_previews': 0})
            self.assertFalse(preview.summarize('box', bundle, 0, control=True)['accepted'])


ADAPTER = {'backend': 'Metal', 'device_type': 'IntegratedGpu', 'name': 'Apple M3'}


def part(**change):
    value = {'status': 'GPU preview on backend: Metal, device_type: IntegratedGpu, name: "Apple M3"', 'route': 'gpu',
             'counts': {'gpu_previews': 1, 'cpu_previews': 0, 'refused_faces': 0, 'refusals': {}},
             'checked_route': True, 'problems': [], 'bodies': [{'route': 'gpu', 'refused': []}]}
    value.update(change)
    return value


class JudgePartTest(unittest.TestCase):
    def test_a_read_back_gpu_preview_on_metal_is_accepted(self):
        self.assertEqual(preview.judge_part(part(), ADAPTER), [])

    def test_harness_problems_a_software_adapter_and_a_cpu_route_fail(self):
        self.assertIn('crack', preview.judge_part(part(problems=['crack']), ADAPTER))
        self.assertTrue(preview.judge_part(part(), {**ADAPTER, 'device_type': 'Cpu'}))
        cpu = part(route='cpu', bodies=[{'route': 'cpu', 'why': 'the kernel has no GPU description of the body'}])
        self.assertIn('no GPU preview shown (route cpu): the kernel has no GPU description of the body',
                      preview.judge_part(cpu, ADAPTER))
        self.assertTrue(preview.judge_part(part(counts={'gpu_previews': 1, 'cpu_previews': 2}), ADAPTER))
        self.assertEqual(preview.judge_part(part(checked_route=False), ADAPTER), [])

    def test_refused_faces_are_listed_with_their_reason(self):
        refused = part(bodies=[{'body': 'BodyId(3)', 'refused': [
            {'face': 'FaceId(7)', 'reason': 'preview.unsupported-trim', 'cpu_drawn': True}]}])
        self.assertEqual(preview.judge_part(refused, ADAPTER), [])
        self.assertEqual(preview.fallback_faces(refused), [
            {'face': 'FaceId(7)', 'reason': 'preview.unsupported-trim', 'cpu_drawn': True, 'body': 'BodyId(3)'}])

    def test_empty_parts_and_field_bodies_are_outside_the_corpus(self):
        empty = part(route='none', live_bodies=0, bodies=[])
        self.assertEqual(preview.excluded(empty), 'the part has no body to display')
        field = part(route='cpu', bodies=[{'route': 'cpu', 'field': True}])
        self.assertTrue(preview.readback_record(field, ADAPTER, False)['excluded'])
        self.assertTrue(preview.readback_record(field, ADAPTER, False)['accepted'])
        brep = part(route='cpu', bodies=[{'route': 'cpu', 'field': False, 'why': 'no description'}])
        self.assertIsNone(preview.excluded(brep))
        self.assertFalse(preview.readback_record(brep, ADAPTER, False)['accepted'])

    def test_the_control_needs_the_cpu_route(self):
        off = part(status='GPU preview off, CPU preview kept (preview.gpu-disabled)', route='cpu',
                   counts={'gpu_previews': 0, 'cpu_previews': 1})
        self.assertEqual(preview.judge_part(off, ADAPTER, control=True), [])
        self.assertTrue(preview.judge_part(part(), ADAPTER, control=True))


if __name__ == '__main__':
    unittest.main()
