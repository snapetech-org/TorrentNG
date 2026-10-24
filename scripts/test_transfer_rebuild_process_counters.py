"""Unit tests for diagnostic process and transfer counters."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from transfer_rebuild.process_counters import (
    ContextSwitchSnapshot,
    add_verified_gib_normalization,
    parse_context_switch_status,
    parse_perf_stat,
    parse_strace_summary,
    snapshot_process_context_switches,
)


class ProcessCounterParserTests(unittest.TestCase):
    def test_context_switch_status_parser_requires_both_counters(self) -> None:
        self.assertEqual(
            parse_context_switch_status(
                "Name:\tclient\nvoluntary_ctxt_switches:\t12\n"
                "nonvoluntary_ctxt_switches:\t3\n"
            ),
            (12, 3),
        )
        self.assertIsNone(parse_context_switch_status("voluntary_ctxt_switches: 12\n"))

    def test_context_switch_snapshot_sums_all_threads_in_process_tree(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            self._write_thread(proc, 100, 100, 4, 1, "101")
            self._write_thread(proc, 100, 110, 5, 0, "")
            self._write_thread(proc, 101, 101, 7, 2, "")

            snapshot = snapshot_process_context_switches(100, proc)

        self.assertEqual(snapshot.process_ids, (100, 101))
        self.assertEqual(snapshot.thread_ids, (100, 101, 110))
        self.assertEqual(snapshot.voluntary, 16)
        self.assertEqual(snapshot.involuntary, 3)
        self.assertEqual(snapshot.readable_threads, 3)

    def test_context_switch_delta_is_null_when_process_tree_changes(self) -> None:
        first = ContextSwitchSnapshot((100,), (100, 110), 15, 2, 2)
        later = ContextSwitchSnapshot((100,), (100, 110, 120), 30, 4, 3)

        delta = first.delta(later)

        self.assertIsNone(delta["voluntary"])
        self.assertIsNone(delta["involuntary"])
        self.assertFalse(delta["stable_process_tree"])

    def test_perf_stat_parser_extracts_supported_events(self) -> None:
        output = (
            "123456,,instructions:u,100000,100.00,,\n"
            "654321,,cycles:u,100000,100.00,,\n"
            "12,,context-switches,100000,100.00,,\n"
            "3,,cpu-migrations,100000,100.00,,\n"
        )

        self.assertEqual(parse_perf_stat(output), {
            "instructions": 123456,
            "cycles": 654321,
            "perf_context_switches": 12,
            "cpu_migrations": 3,
        })

    def test_perf_stat_parser_keeps_unavailable_counts_null(self) -> None:
        self.assertEqual(
            parse_perf_stat("<not counted>,,instructions:u,0,0.00,,\n"),
            {"instructions": None},
        )

    def test_strace_summary_parser_extracts_total_calls_and_errors(self) -> None:
        output = (
            "% time     seconds  usecs/call     calls    errors syscall\n"
            "------ ----------- ----------- --------- --------- ----------------\n"
            "  100.0    0.001234           0      2038      1600 total\n"
        )

        self.assertEqual(parse_strace_summary(output), {"syscalls": 2038, "errors": 1600})

    def test_strace_summary_parser_accepts_zero_error_column_elision(self) -> None:
        output = (
            "% time     seconds  usecs/call     calls    errors syscall\n"
            "------ ----------- ----------- --------- --------- ----------------\n"
            "100.00    0.001744           2       861 total\n"
        )

        self.assertEqual(parse_strace_summary(output), {"syscalls": 861, "errors": 0})

    def test_normalization_uses_only_verified_gib(self) -> None:
        one_gib = add_verified_gib_normalization({
            "instructions": 3_000_000_000,
            "syscalls": 12_000,
            "voluntary_context_switches": 90,
            "involuntary_context_switches": None,
        }, 1024 ** 3)
        unverified = add_verified_gib_normalization({"instructions": 3_000_000_000}, 0)

        self.assertEqual(one_gib["instructions_per_verified_gib"], 3_000_000_000)
        self.assertEqual(one_gib["syscalls_per_verified_gib"], 12_000)
        self.assertEqual(one_gib["voluntary_context_switches_per_verified_gib"], 90)
        self.assertIsNone(one_gib["involuntary_context_switches_per_verified_gib"])
        self.assertIsNone(unverified["instructions_per_verified_gib"])

    @staticmethod
    def _write_thread(
        proc: Path,
        pid: int,
        tid: int,
        voluntary: int,
        involuntary: int,
        children: str,
    ) -> None:
        thread = proc / str(pid) / "task" / str(tid)
        thread.mkdir(parents=True)
        (thread / "status").write_text(
            f"Name:\tclient\nvoluntary_ctxt_switches:\t{voluntary}\n"
            f"nonvoluntary_ctxt_switches:\t{involuntary}\n"
        )
        (thread / "children").write_text(children)


if __name__ == "__main__":
    unittest.main()
