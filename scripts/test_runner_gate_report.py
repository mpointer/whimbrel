"""Tests for runner-gate-report.py. Run: python3 -I scripts/test_runner_gate_report.py"""
import importlib.util
import io
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("gate", os.path.join(HERE, "runner-gate-report.py"))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def line(day, ran=("daily_brief",), left=(), rows=100, ms=60000, extra=""):
    ran_json = str(list(ran)).replace("'", '"')
    left_json = str(list(left)).replace("'", '"')
    return (f'{day}T05:00:00Z {{"ran": {ran_json}, "skipped_left_for_computer": {left_json}, '
            f'"rows_pulled": {rows}, "duration_ms": {ms}{extra}}}')


class ReportTests(unittest.TestCase):
    def test_normal_runs(self):
        lines = [line("2026-11-01", ms=60000, rows=100), line("2026-11-02", ms=120000, rows=300),
                 line("2026-11-03", ms=90000, rows=200, left=["eod_recap"])]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["runs"], 3)
        self.assertEqual(r["days_with_run"], 3)
        self.assertEqual(r["longest_streak"], 3)
        self.assertEqual(r["left_for_computer_runs"], 1)
        self.assertEqual(r["duration_ms"]["max"], 120000)
        self.assertEqual(r["duration_ms"]["p50"], 90000)
        self.assertEqual(r["rows_pulled"]["max"], 300)
        self.assertEqual(r["suggested_ready_by_p95_ms"], 120000)
        self.assertEqual(r["suggested_ready_by_margin_ms"], 60000)

    def test_gap_day_breaks_streak(self):
        lines = [line("2026-11-01"), line("2026-11-02"), line("2026-11-04"), line("2026-11-05"), line("2026-11-06")]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["days_with_run"], 5)
        self.assertEqual(r["longest_streak"], 3)

    def test_runs_per_day_and_two_runs_one_day(self):
        lines = [line("2026-11-01"), line("2026-11-01"), line("2026-11-02")]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["runs_per_day"], {"2026-11-01": 2, "2026-11-02": 1})
        self.assertEqual(r["longest_streak"], 2)

    def test_run_with_nothing_ran_is_not_a_successful_day(self):
        lines = [line("2026-11-01", ran=[]), line("2026-11-02")]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["runs"], 2)
        self.assertEqual(r["days_with_run"], 1)
        self.assertEqual(r["longest_streak"], 1)

    def test_other_routines_do_not_count_toward_the_brief_streak(self):
        lines = [line("2026-11-01"), line("2026-11-02", ran=["eod_recap"]), line("2026-11-03", ran=["eod_recap", "daily_brief"])]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["days_with_run"], 2)
        self.assertEqual(r["longest_streak"], 1)
        self.assertEqual(r["days_with_any_routine"], 3)
        self.assertEqual(r["longest_streak_any_routine"], 3)
        out = io.StringIO()
        gate.print_report(r, out)
        self.assertIn("any routine: 3 days", out.getvalue())

    def test_noise_lines_ignored(self):
        lines = ["starting up", "", "{not json", '{"other": 1}', "[1,2]", line("2026-11-01"), "error: x"]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["runs"], 1)

    def test_unknown_fields_tolerated(self):
        lines = [line("2026-11-01", extra=', "future": {"a": 1}')]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["runs"], 1)

    def test_date_field_and_bare_json(self):
        lines = ['{"ran": ["daily_brief"], "skipped_left_for_computer": [], "rows_pulled": 5, "duration_ms": 10, "date": "2026-11-09"}']
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["days_with_run"], 1)

    def test_default_date_from_file_name(self):
        lines = ['{"ran": ["daily_brief"], "skipped_left_for_computer": [], "rows_pulled": 5, "duration_ms": 10}']
        r = gate.summarize(gate.parse_lines(lines, default_day="2026-11-10"))
        self.assertEqual(r["runs_per_day"], {"2026-11-10": 1})

    def test_undated_runs_counted_but_not_days(self):
        lines = ['{"ran": ["daily_brief"], "skipped_left_for_computer": [], "rows_pulled": 5, "duration_ms": 10}']
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["runs"], 1)
        self.assertEqual(r["undated_runs"], 1)
        self.assertEqual(r["days_with_run"], 0)

    def test_p95_rounds_up_to_next_second(self):
        lines = [line("2026-11-01", ms=61234)]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["suggested_ready_by_p95_ms"], 62000)

    def test_empty_input(self):
        r = gate.summarize(gate.parse_lines([]))
        self.assertEqual(r["runs"], 0)
        self.assertEqual(r["longest_streak"], 0)
        self.assertIsNone(r["suggested_ready_by_p95_ms"])
        out = io.StringIO()
        gate.print_report(r, out)
        self.assertIn("No runs", out.getvalue())

    def test_report_text_names_the_constants_to_replace(self):
        r = gate.summarize(gate.parse_lines([line("2026-11-01")]))
        out = io.StringIO()
        gate.print_report(r, out)
        self.assertIn("READY_BY_P95_MS", out.getvalue())


class RobustnessTests(unittest.TestCase):
    def test_invalid_and_missing_fields_are_skipped_and_counted(self):
        base = '{"ran": ["daily_brief"], "skipped_left_for_computer": [], '
        lines = [base + '"rows_pulled": 1, "duration_ms": NaN}', base + '"rows_pulled": 1, "duration_ms": Infinity}',
                 base + '"rows_pulled": 1, "duration_ms": -5}', base + '"rows_pulled": 1}', base + '"duration_ms": 5}',
                 base + '"rows_pulled": "x", "duration_ms": 5}', line("2026-11-01")]
        skipped = {}
        r = gate.summarize(gate.parse_lines(lines, skipped=skipped), skipped)
        self.assertEqual(r["runs"], 1)
        self.assertEqual(r["skipped_incomplete"], 6)
        out = io.StringIO()
        gate.print_report(r, out)
        self.assertIn("Skipped/incomplete report lines: 6", out.getvalue())

    def test_twenty_or_more_samples_use_real_p95_rank_and_no_warning(self):
        lines = [line("2026-11-01", ms=i * 1000) for i in range(1, 21)]
        r = gate.summarize(gate.parse_lines(lines))
        self.assertEqual(r["duration_ms"]["p95"], 19000)
        out = io.StringIO()
        gate.print_report(r, out)
        self.assertNotIn("WARNING", out.getvalue())

    def test_low_sample_warning(self):
        r = gate.summarize(gate.parse_lines([line("2026-11-01")]))
        out = io.StringIO()
        gate.print_report(r, out)
        self.assertIn("WARNING: only 1 runs", out.getvalue())

    def test_margin_above_120_seconds(self):
        r = gate.summarize(gate.parse_lines([line("2026-11-01", ms=300000)]))
        self.assertEqual(r["suggested_ready_by_p95_ms"], 300000)
        self.assertEqual(r["suggested_ready_by_margin_ms"], 150000)
        self.assertGreater(r["suggested_ready_by_margin_ms"], 120000)

    def test_margin_floor(self):
        r = gate.summarize(gate.parse_lines([line("2026-11-01", ms=10000)]))
        self.assertEqual(r["suggested_ready_by_margin_ms"], 60000)

    def test_z_prefix_gives_the_utc_date(self):
        r = gate.summarize(gate.parse_lines([line("2026-11-01")]))
        self.assertIn("2026-11-01", r["runs_per_day"])


if __name__ == "__main__":
    unittest.main()
