"""Reject release notes that would misidentify the packaged kernel."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class SourceRevisionsTests(unittest.TestCase):
    def test_rejects_inconsistent_kernel_sources(self):
        kernel, other = "a" * 40, "b" * 40
        cases = (
            (other, kernel, kernel, "dependencies"),
            (kernel, other, kernel, "backend identity"),
            (kernel, kernel, other, "Locked"),
        )
        for dependency, declared, locked, error in cases:
            with self.subTest(error=error), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                adapter = root / "crates/soliddraft-kernel-catallax"
                (adapter / "src").mkdir(parents=True)
                (adapter / "Cargo.toml").write_text(
                    f'[dependencies]\ncatallax-app = {{ rev = "{kernel}" }}\n'
                    f'catallax-mesh = {{ rev = "{dependency}" }}\n')
                (adapter / "src/lib.rs").write_text(
                    f'pub const CATALLAX_REVISION: &str = "{declared}";\n')
                (root / "Cargo.lock").write_text(
                    '[[package]]\nname = "catallax-app"\n'
                    f'source = "git+https://github.com/BohnBawerick/catallax?rev={kernel}#{locked}"\n')
                with patch.object(release, "ROOT", root), patch.object(
                        release.subprocess, "check_output", return_value="c" * 40 + "\n"):
                    with self.assertRaisesRegex(ValueError, error):
                        release.source_revisions()


class ReleaseSourceTests(unittest.TestCase):
    def test_publication_requires_matching_release_branch_ancestry(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n')

            def git(*args):
                subprocess.run(["git", *args], cwd=root, check=True,
                               stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

            git("init")
            git("add", "Cargo.toml")
            git("-c", "user.name=Release test", "-c", "user.email=release@example.invalid",
                "commit", "-m", "candidate")
            git("update-ref", "refs/remotes/origin/main", "HEAD")
            git("update-ref", "refs/remotes/origin/release/0.2.0", "HEAD")
            with patch.object(release, "ROOT", root):
                with self.assertRaises(subprocess.CalledProcessError):
                    release.check_source("v0.1.0")
                git("update-ref", "refs/remotes/origin/release/0.1.0", "HEAD")
                self.assertEqual(release.check_source("v0.1.0"), "release/0.1.0")
                (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n# release-only pin\n')
                git("add", "Cargo.toml")
                git("-c", "user.name=Release test", "-c", "user.email=release@example.invalid",
                    "commit", "-m", "release-only metadata")
                git("update-ref", "refs/remotes/origin/release/0.1.0", "HEAD")
                self.assertEqual(release.check_source("v0.1.0"), "release/0.1.0")
                with self.assertRaisesRegex(ValueError, "workspace version"):
                    release.check_source("v0.2.0")
                (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n# later work\n')
                git("add", "Cargo.toml")
                git("-c", "user.name=Release test", "-c", "user.email=release@example.invalid",
                    "commit", "-m", "unreleased work")
                git("update-ref", "refs/remotes/origin/main", "HEAD")
                with self.assertRaises(subprocess.CalledProcessError):
                    release.check_source("v0.1.0")


class ReleaseHistoryTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Release test")
        self.git("config", "user.email", "release@example.invalid")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.2.0"\n')
        kind = self.root / "crates/soliddraft-document/src/kind.rs"
        kind.parent.mkdir(parents=True)
        kind.write_text('DocKind::Part => "soliddraft-part/1"\n'
                        'DocKind::Assembly => "soliddraft-assembly/1"\n'
                        'DocKind::Drawing => "soliddraft-drawing/1"\n')
        self.base = self.commit("base")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True,
                                       stderr=subprocess.PIPE).strip()

    def commit(self, message):
        (self.root / "change").write_text(message)
        self.git("add", ".")
        self.git("commit", "-m", message)
        return self.git("rev-parse", "HEAD")

    def test_previous_published_version_need_not_be_an_ancestor(self):
        self.git("checkout", "-b", "old-release")
        self.commit("previous release")
        self.git("tag", "v0.1.0")
        self.git("checkout", "main")
        self.commit("current release")
        for tag in ("v0.0.9", "v0.2.0", "v0.3.0", "v0.1.9"):
            self.git("tag", tag)
        releases = self.root / "releases.json"
        releases.write_text(json.dumps(["v0.3.0", "v0.0.9", "v0.2.0", "v0.1.0", "v0.2.0-rc1"]))
        with patch.object(release, "ROOT", self.root):
            previous = release.previous_release(releases)
            self.assertEqual(previous, "v0.1.0")
            pulls = self.root / "pulls.json"
            pulls.write_text("[[]]")
            with patch.object(release, "source_revisions", return_value=("a", "b")), \
                    patch.object(release, "known_gap_notes", return_value=[]):
                release.notes(pulls, previous, False)
            notes = (self.root / "dist/release-notes.md").read_text()
            self.assertEqual(notes.count("(unchanged)"), 3)
            self.assertNotIn("first release", notes)
            releases.write_text(json.dumps(["v0.2.0", "v0.3.0", "invalid"]))
            self.assertEqual(release.previous_release(releases), "")
            releases.write_text(json.dumps(["v0.0.9", "v0.0.10"]))
            self.assertEqual(release.previous_release(releases), "v0.0.10")

    def test_cherry_picked_pulls_are_included_once_and_previous_pulls_excluded(self):
        old = self.commit("already shipped")
        new = self.commit("new fix")
        self.git("checkout", "-b", "old-release", self.base)
        self.git("cherry-pick", "-x", old)
        self.git("tag", "v0.1.0")
        self.git("checkout", "-b", "candidate", old)
        self.git("cherry-pick", "-x", new)
        direct = self.commit("direct merge")
        prs = [dict(number=number, title=f"Fix {number}", user={"login": "author"},
                    html_url=f"https://example.invalid/pull/{number}",
                    merged_at="2026-11-01T00:00:00Z", merge_commit_sha=sha)
               for number, sha in ((1, old), (2, new), (3, direct), (4, "f" * 40))]
        prs.append(dict(prs[2], number=5, merged_at=None))
        pulls = self.root / "pulls.json"
        pulls.write_text(json.dumps([prs[2:], prs[:2]]))
        with patch.object(release, "ROOT", self.root):
            text = release.pull_request_notes(pulls, "v0.1.0")
            self.assertNotIn(prs[0]["html_url"], text)
            self.assertEqual(text.count(prs[1]["html_url"]), 1)
            self.assertEqual(text.count(prs[2]["html_url"]), 1)
            self.assertNotIn(prs[3]["html_url"], text)
            self.assertNotIn("Fix 5", text)
            self.assertLess(text.index("Fix 2"), text.index("Fix 3"))
            first = release.pull_request_notes(pulls, None)
            self.assertIn(prs[0]["html_url"], first)
            self.assertIn(prs[1]["html_url"], first)
            pulls.write_text("[[]]")
            self.assertIn("No merged pull requests", release.pull_request_notes(pulls, None))



class DeferredWindowsTests(unittest.TestCase):
    NAMES = ("linux-x86_64.AppImage", "linux-x86_64.deb", "linux-x86_64.tar.gz", "macos-universal.dmg", "web.zip")
    WINDOWS = ("windows-x64.msi", "windows-x64.zip")

    def inventory(self, names):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n')
        (root / "dist").mkdir()
        for name in ("release-notes.md", "soliddraft-0.1.0.spdx.json", *(f"soliddraft-0.1.0-{n}" for n in names)):
            (root / "dist" / name).write_text(name)
        return root

    def test_a_package_run_has_no_windows_cell_unless_asked(self):
        self.assertEqual([cell["platform"] for cell in release.desktop(False)], ["linux", "macos"])
        self.assertEqual(release.desktop(True), [
            {"platform": "linux", "os": "ubuntu-22.04", "target": "x86_64-unknown-linux-gnu", "formats": "appimage,deb"},
            {"platform": "windows", "os": "windows-2022", "target": "x86_64-pc-windows-msvc", "formats": "wix"},
            {"platform": "macos", "os": "macos-26", "target": "universal-apple-darwin", "formats": "dmg"}])

    def test_inventory_needs_windows_files_exactly_when_windows_ran(self):
        for names, windows, words in ((self.NAMES, False, None), (self.NAMES + self.WINDOWS, True, None),
                                      (self.NAMES, True, "missing"), (self.NAMES + self.WINDOWS[:1], True, "missing"),
                                      (self.NAMES + self.WINDOWS, False, "extra")):
            with self.subTest(names=names, windows=windows):
                root = self.inventory(names)
                with patch.object(release, "ROOT", root):
                    if words:
                        with self.assertRaisesRegex(ValueError, words):
                            release.checksums(windows)
                        self.assertFalse((root / "dist/SHA256SUMS").exists())
                    else:
                        release.checksums(windows)
                        self.assertEqual("windows" in (root / "dist/SHA256SUMS").read_text(), windows)

    def test_notes_say_when_a_release_has_no_windows_packages(self):
        for windows, present, absent in ((False, "no Windows packages", "SmartScreen"),
                                         (True, "SmartScreen", "no Windows packages")):
            with self.subTest(windows=windows):
                root = self.inventory(())
                with patch.object(release, "ROOT", root), patch.object(release, "formats", return_value={}), \
                        patch.object(release, "source_revisions", return_value=("a", "b")), \
                        patch.object(release, "known_gap_notes", return_value=[]), \
                        patch.object(release, "pull_request_notes", return_value=""):
                    release.notes(root / "pulls.json", None, windows)
                text = (root / "dist/release-notes.md").read_text()
                self.assertIn(present, text)
                self.assertNotIn(absent, text)
                self.assertIn("macOS Gatekeeper", text)
                self.assertIn("[SolidDraft website](https://zigrawbohnbawerick.github.io/soliddraft/)", text)

    def test_pr_inventory_omits_only_macos_and_still_rejects_incomplete_packages(self):
        pr_names = tuple(name for name in self.NAMES if name != 'macos-universal.dmg')
        for windows in (False, True):
            expected = pr_names + (self.WINDOWS if windows else ())
            for names, macos, error in ((expected, False, None), (expected, True, 'missing'),
                                        (expected + ('macos-universal.dmg',), False, 'extra'),
                                        (expected[1:], False, 'missing')):
                with self.subTest(windows=windows, names=names, macos=macos):
                    root = self.inventory(names)
                    with patch.object(release, 'ROOT', root):
                        if error:
                            with self.assertRaisesRegex(ValueError, error):
                                release.checksums(windows, macos)
                            self.assertFalse((root / 'dist/SHA256SUMS').exists())
                        else:
                            release.checksums(windows, macos)
                            sums = (root / 'dist/SHA256SUMS').read_text()
                            self.assertNotIn('macos', sums)
                            self.assertEqual('windows' in sums, windows)


class KnownGapNotesTests(unittest.TestCase):
    def test_lists_every_known_gap_by_scenario(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "ci").mkdir()
            (root / "ci/known-over-budget.json").write_text(
                '{"format": "soliddraft-known-over-budget/1", "follow_up": "https://example.invalid/27", '
                '"steps": [{"scenario": "bracket", "step": 19, "tool": "edit_extrude", "last_measured_ms": 1}, '
                '{"scenario": "drawing", "step": 24, "tool": "edit_extrude", "last_measured_ms": 1}, '
                '{"scenario": "drawing", "step": 35, "tool": "edit_extrude", "last_measured_ms": 1}]}')
            with patch.object(release, "ROOT", root):
                text = "\n".join(release.known_gap_notes())
        self.assertIn("## Known limitations", text)
        self.assertIn("- bracket: steps 19 (edit_extrude).", text)
        self.assertIn("- drawing: steps 24 (edit_extrude), 35 (edit_extrude).", text)
        self.assertIn("https://example.invalid/27", text)

    def test_current_release_discloses_strict_failures_and_export_timeouts(self):
        with patch.object(release, "version", return_value="0.5.0"):
            text = "\n".join(release.known_gap_notes())
        self.assertIn("13 scenarios passed functional checks but exceeded strict timing budgets", text)
        for scenario in ("graded-fields", "loft", "p5-flanged-housing", "p5-placed-housing",
                         "ring-fillet", "rod-cut-28deg", "skew-corner-fillet", "sphere-revolve",
                         "spline-sweep", "torus-revolve", "canonical-p1", "canonical-p5", "loft-edge-fillet"):
            self.assertIn(f"- `{scenario}`.", text)
        self.assertIn("STL export timed out after 600 seconds in 2 scenarios: `gyroid`, `lattice`", text)
        self.assertIn("did not pass the full strict scenario verdict", text)
        self.assertIn("Timing budgets remain unchanged", text)
        self.assertIn("not quiet timing measurements", text)
        self.assertIn("https://github.com/ZIGRAWBOHNBAWERICK/soliddraft/issues/27", text)
        self.assertNotIn("https://github.com/BohnBawerick/soliddraft/issues/", text)

    def test_release_limitations_do_not_leak_into_another_version(self):
        with patch.object(release, "version", return_value="0.5.1"):
            text = "\n".join(release.known_gap_notes())
        self.assertIn("- bracket: steps", text)
        self.assertNotIn("### 0.5.0 validation", text)
        self.assertNotIn("STL export timed out", text)

if __name__ == "__main__":
    unittest.main()
