import base64
import importlib.util
import io
import json
from pathlib import Path
import sqlite3
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("feedback_sync", ROOT / "sync" / "sync.py")
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)


class SyncTests(unittest.TestCase):
    def setUp(self):
        self.row = {"id": "12345678-1234-1234-1234-123456789012", "created_at": "2026-11-05T00:00:00Z"}
        self.payload = {"source": "app", "version": "0.1.0", "commit": "abcdef0", "kind": "bug",
                        "message": "@someone ```\nbody\n```", "email": "person@example.org", "context": None}
        self.row["payload"] = json.dumps(self.payload)
        self.db = sqlite3.connect(":memory:")
        self.db.executescript((ROOT / "worker" / "schema.sql").read_text())
        self.db.execute("INSERT INTO feedback VALUES (?, ?, ?, NULL, NULL)", tuple(self.row.values()))

    def tearDown(self):
        self.db.close()

    def d1(self, sql, config=None):
        self.db.row_factory = sqlite3.Row
        return [dict(r) for r in self.db.execute(sql).fetchall()]

    def test_decodes_axi_output_without_losing_json_structure(self):
        encoded = base64.b64encode(b'{"private":true}').decode()
        with patch.object(sync, "run", return_value=encoded) as run:
            self.assertEqual(sync.github(""), {"private": True})
            self.assertIn("@base64", run.call_args.args[0])

    def test_issue_body_keeps_text_literal_and_context_collapsed(self):
        body = sync.issue_body(self.row, self.payload)
        self.assertTrue(body.startswith(sync.marker(self.row["id"])))
        self.assertIn("````\n@someone ```", body)
        self.assertIn("<details><summary>Diagnostic context</summary>", body)
        self.assertIn("```json\nnull\n```", body)

    def test_copy_then_mark_and_no_work_on_second_run(self):
        issues = []
        labels = []

        def github(path, method="GET", body=None):
            if not path:
                return {"private": True}
            if path.startswith("/issues?"):
                return issues
            if path.startswith("/labels?"):
                return [{"name": label} for label in labels]
            if path == "/labels":
                labels.append(body["name"])
                return body
            if path == "/issues":
                issue = {**body, "html_url": "https://github.com/example/private/issues/1"}
                issues.append(issue)
                return issue
            self.fail(path)

        with patch.object(sync, "github", side_effect=github), patch.object(sync, "d1", side_effect=self.d1), patch("sys.stdout", new=io.StringIO()):
            sync.sync()
            sync.sync()
        self.assertEqual(len(issues), 1)
        self.assertEqual(issues[0]["labels"], ["source:app", "kind:bug", "version:0.1.0"])
        self.assertEqual(self.db.execute("SELECT issue_url FROM feedback").fetchone()[0], issues[0]["html_url"])

    def test_retry_after_issue_creation_does_not_duplicate(self):
        existing = {"body": sync.issue_body(self.row, self.payload), "html_url": "https://github.com/example/private/issues/1"}

        def github(path, method="GET", body=None):
            self.assertEqual(method, "GET")
            return {"private": True} if not path else ([existing] if path.startswith("/issues?") else [])

        with patch.object(sync, "github", side_effect=github), patch.object(sync, "d1", side_effect=self.d1), patch("sys.stdout", new=io.StringIO()):
            sync.sync()
        self.assertEqual(self.db.execute("SELECT issue_url FROM feedback").fetchone()[0], existing["html_url"])

    def test_refuses_public_repository_before_reading_feedback(self):
        with patch.object(sync, "github", return_value={"private": False}), patch.object(sync, "d1") as d1:
            with self.assertRaises(RuntimeError):
                sync.sync()
            d1.assert_not_called()


if __name__ == "__main__":
    unittest.main()
