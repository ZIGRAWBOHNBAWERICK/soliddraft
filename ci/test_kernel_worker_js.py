"""Runs the browser kernel worker's message test (`kernel-worker.test.mjs`) under Node."""

import pathlib
import shutil
import subprocess
import unittest

HERE = pathlib.Path(__file__).resolve().parent


class KernelWorkerMessagesTest(unittest.TestCase):
    @unittest.skipIf(shutil.which("node") is None, "Node is not installed")
    def test_worker_messages_reach_their_handlers(self):
        result = subprocess.run(["node", "--test", str(HERE / "kernel-worker.test.mjs")],
                                capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
