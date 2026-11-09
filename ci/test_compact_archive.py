"""Archive compaction must preserve payloads and leave usable originals on failure."""
import hashlib
import importlib.util
import pathlib
import random
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('compact_archive', pathlib.Path(__file__).with_name('compact-archive.py'))
archive = importlib.util.module_from_spec(spec)
spec.loader.exec_module(archive)


def digest_file(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


@unittest.skipUnless(shutil.which('zstd'), 'zstd is needed for the real codec fixture')
class Codec(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        directory = tempfile.TemporaryDirectory()
        cls.addClassCleanup(directory.cleanup)
        root = pathlib.Path(directory.name)
        raw = root / 'raw'
        block = random.Random(0).randbytes(12 * 1024 * 1024)
        digest = hashlib.sha256()
        with raw.open('wb') as stream:
            for _ in range(4):
                stream.write(block)
                digest.update(block)
        cls.digest = digest.hexdigest()
        cls.original = root / 'original.tar.zst'
        subprocess.run(['zstd', '-q', '-3', '-T1', str(raw), '-o', str(cls.original)], check=True)

    def test_a_smaller_archive_contains_the_exact_original_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'archive.tar.zst'
            shutil.copyfile(self.original, path)
            report = archive.compact(path, threads=1, minimum=0)
            self.assertTrue(report['changed'], report)
            self.assertLess(report['after_bytes'], report['before_bytes'] / 2)
            self.assertEqual(report['payload_sha256'], self.digest)
            self.assertEqual(list(path.parent.iterdir()), [path])

    def test_timeout_retains_the_original_and_removes_partial_output(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'archive.tar.zst'
            shutil.copyfile(self.original, path)
            before = digest_file(path)
            report = archive.compact(path, threads=1, minimum=0, timeout=0)
            self.assertFalse(report['changed'], report)
            self.assertEqual(report['reason'], 'timeout; kept original')
            self.assertEqual(digest_file(path), before)
            self.assertEqual(list(path.parent.iterdir()), [path])


class Availability(unittest.TestCase):
    def test_production_archive_reduction_enables_eight_partitions(self):
        for before, after in ((4789764825, 2939618032), (4748681358, 2915748579)):
            self.assertEqual(archive.execution_partitions({'before_bytes': before, 'after_bytes': after}), 8)
        for remaining, expected in ((201, 4), (200, 8), (101, 8), (100, 12)):
            self.assertEqual(archive.execution_partitions({'before_bytes': 300, 'after_bytes': remaining}), expected)

    def test_execution_partitions_bound_transfer_overhead(self):
        for remaining in range(1, 1001):
            count = archive.execution_partitions({'before_bytes': 1000, 'after_bytes': remaining})
            self.assertIn(count, (4, 8, 12))
            self.assertLessEqual(3 * count * remaining, 16 * 1000)

    def test_a_missing_codec_keeps_the_original(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'archive.tar.zst'
            path.write_bytes(b'original')
            with mock.patch.object(archive.shutil, 'which', return_value=None):
                report = archive.compact(path, minimum=0)
            self.assertFalse(report['changed'])
            self.assertEqual(report['reason'], 'zstd unavailable')
            self.assertEqual(path.read_bytes(), b'original')


if __name__ == '__main__':
    unittest.main()
