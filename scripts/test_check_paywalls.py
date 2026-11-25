"""Self-test for scripts/check-paywalls.py: the current tree passes, and a planted gate fails.

    python3 -I -m unittest discover -s scripts -p test_check_paywalls.py
"""

import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_paywalls", HERE / "check-paywalls.py")
cp = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = cp
spec.loader.exec_module(cp)


def run(*argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = cp.main(list(argv))
    return code, out.getvalue()


class PlantedTree(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.allow = self.root / "allow.txt"
        self.allow.write_text("# nothing allowed\n")

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def check(self):
        return run("--root", str(self.root), "--allow", str(self.allow))

    def test_a_planted_swift_gate_fails(self):
        self.write("apps/macos/Whimbrel/UI/Export.swift",
                   "func export() {\n    guard plan.isPlus else { return showUpgrade() }\n    writeAll()\n}\n")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("apps/macos/Whimbrel/UI/Export.swift:2:", out)

    def test_planted_gates_in_other_languages_fail(self):
        self.write("apps/windows/ui/src/export.ts", 'if (view.plan.plan === "pro") { exportAll(); }\n')
        self.write("core/api/src/export.rs", 'if plan.plan_tier.as_deref() == Some("plus") { export_all() }\n')
        self.write("apps/mobile/android/app/src/main/java/Export.kt", "if (account.isPaid) exportAll()\n")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("3 plan gate(s)", out)

    def test_a_gate_without_an_if_fails(self):
        self.write("apps/mobile/ios/Whimbrel/Rows.swift", "Rows(showsExport: isCloud, showsHelp: true)\n")
        self.write("apps/mobile/android/app/src/main/java/Rows.kt", "val paid = plan.isPlus\n")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("2 plan gate(s)", out)

    def test_tests_and_comments_are_ignored(self):
        self.write("apps/macos/WhimbrelTests/ExportTests.swift", "XCTAssertTrue(plan.isPlus == true)\n")
        self.write("apps/windows/ui/src/export.test.ts", 'expect(plan.plan === "plus").toBe(true);\n')
        self.write("core/api/src/export.rs", '// if plan.is_plus() { export_all() }\nfn export_all() {}\n')
        self.assertEqual(self.check()[0], 0)

    def test_an_allowlisted_file_passes_and_a_narrow_entry_only_covers_its_lines(self):
        self.write("apps/macos/Whimbrel/Support/Billing.swift",
                   'var isPlus: Bool { planTier == "plus" }\nif plan.isPlus { unlockExport() }\n')
        self.allow.write_text("apps/macos/Whimbrel/Support/Billing.swift :: planTier ==  # decodes the server's answer\n")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("Billing.swift:2:", out)
        self.assertNotIn("Billing.swift:1:", out)
        self.allow.write_text("apps/macos/Whimbrel/Support/Billing.swift  # decodes the server's answer\n")
        self.assertEqual(self.check()[0], 0)

    def test_a_known_violation_passes_but_is_printed(self):
        self.write("apps/mobile/ios/Whimbrel/Feature.swift", "guard isCloud else { return [] }\n")
        self.allow.write_text("! apps/mobile/ios/Whimbrel/Feature.swift  # undecided\n")
        code, out = self.check()
        self.assertEqual(code, 0)
        self.assertIn("KNOWN:", out)

    def test_an_entry_needs_a_reason(self):
        self.allow.write_text("apps/macos/Whimbrel/Support/Billing.swift\n")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("needs a reason", out)

    def test_a_stale_entry_fails(self):
        self.allow.write_text("apps/gone.swift  # was here once\n")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("does not exist", out)


class CurrentTree(unittest.TestCase):
    def test_the_current_tree_passes(self):
        code, out = run()
        self.assertEqual(code, 0, out)


if __name__ == "__main__":
    unittest.main()
