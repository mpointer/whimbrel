"""Self-test for scripts/public-review.py, the private-material scan run before a publish.

    python3 -I -m unittest discover -s scripts -p test_public_review.py
"""

import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("public_review", HERE / "public-review.py")
pr = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = pr
spec.loader.exec_module(pr)


class Scan(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.tree = Path(self.tmp.name) / "tree"
        self.tree.mkdir()
        self.allow = Path(self.tmp.name) / "allow.txt"
        self.allow.write_text("")

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, rel, text):
        path = self.tree / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def scan(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = pr.main([str(self.tree), "--allow", str(self.allow)])
        return code, out.getvalue()

    def test_a_clean_tree_passes(self):
        self.write("README.md", "# Hello\nprivate let key = PrivateKey()\nPRIVATE_REPO=x\n")
        self.assertEqual(self.scan()[0], 0)

    def test_placeholders_pass(self):
        self.write("src/a.rs", 'let k = "sk-ant-api03-abcdefghijklmnopqrstuv1234"; let a = "AKIAIOSFODNN7EXAMPLE";\n')
        self.assertEqual(self.scan()[0], 0)

    def test_a_real_looking_key_fails_without_printing_it(self):
        key = "ghp_" + "Q7rT2mVb9LkWz4XnP8sYc3HdJ6fGa1Ue5Ri0"
        self.write("docs/note.md", f"token: {key}\n")
        code, out = self.scan()
        self.assertEqual(code, 1)
        self.assertIn("GitHub token", out)
        self.assertNotIn(key, out)

    def test_markers_and_secret_files_fail(self):
        # Split, so this file does not trip the scan it tests when it is published.
        self.write("docs/plan.md", "<!-- INTERNAL" + " ONLY -->\n")
        self.write("server/.env", "X=1\n")
        self.write("server/.env.example", "X=\n")
        code, out = self.scan()
        self.assertEqual(code, 1)
        self.assertIn("docs/plan.md:1", out)
        self.assertIn("server/.env:", out)
        self.assertNotIn(".env.example", out)

    def test_every_shape_of_the_private_label_fails(self):
        label = "PRIV" + "ATE"
        self.write("docs/a.md", f"<!-- {label} -->\n")
        self.write("docs/b.md", f"**{label}**\n")
        self.write("docs/c.md", f"{label}\n")
        self.write(f"docs/{label}-notes.md", "hello\n")
        code, out = self.scan()
        self.assertEqual(code, 1)
        self.assertIn("4 private-material hit(s)", out)

    def test_the_allowlist_needs_a_reason_and_silences_a_file(self):
        self.write("docs/plan.md", "CONFIDEN" + "TIAL\n")
        self.allow.write_text("docs/plan.md # a quoted example\n")
        self.assertEqual(self.scan()[0], 0)
        self.allow.write_text("docs/plan.md\n")
        with self.assertRaises(SystemExit):
            self.scan()


if __name__ == "__main__":
    unittest.main()
