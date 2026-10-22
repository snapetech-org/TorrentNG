#!/usr/bin/env python3
"""Attach /proc per-thread CPU snapshots to the native seed probe.

This is a diagnostic wrapper. It imports the existing probe and monkeypatches
only its process_sample function; it does not modify the repository harness.
"""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import sys
import time


HERE = Path(__file__).resolve().parent
PROBE_PATH = HERE.parent / "TorrentNG" / "scripts" / "native_seed_cpu_ram_probe.py"


def read_thread_stat(path: Path) -> dict[str, int]:
    """Read /proc stat fields robustly despite spaces or ')' in comm."""
    raw = path.read_text()
    close = raw.rfind(")")
    if close < 0:
        raise ValueError("stat line has no comm terminator")
    fields = raw[close + 2 :].split()
    # fields[0] is field 3 (state); utime/stime/starttime are fields 14/15/22.
    if len(fields) <= 19:
        raise ValueError("stat line is too short")
    return {
        "utime_ticks": int(fields[11]),
        "stime_ticks": int(fields[12]),
        "starttime_ticks": int(fields[19]),
    }


def capture_pid_threads(pid: int) -> tuple[list[dict], list[str]]:
    threads: list[dict] = []
    errors: list[str] = []
    task_dir = Path(f"/proc/{pid}/task")
    try:
        task_entries = list(task_dir.iterdir())
    except OSError as error:
        return threads, [f"pid {pid}: {type(error).__name__}: {error}"]

    for task in task_entries:
        if not task.name.isdigit():
            continue
        tid = int(task.name)
        try:
            stat = read_thread_stat(task / "stat")
        except (OSError, ValueError) as error:
            # Threads may exit between listing /task and reading stat.
            errors.append(f"pid {pid} tid {tid}: {type(error).__name__}: {error}")
            continue

        try:
            comm = (task / "comm").read_text().strip()
        except OSError:
            comm = "<exited-before-comm-read>"

        schedstat = None
        try:
            values = (task / "schedstat").read_text().split()
            if len(values) >= 3:
                schedstat = {
                    "runtime_ns": int(values[0]),
                    "runqueue_wait_ns": int(values[1]),
                    "timeslices": int(values[2]),
                }
        except (OSError, ValueError):
            # schedstat is optional; stat CPU ticks remain the primary source.
            pass

        threads.append({
            "pid": pid,
            "tid": tid,
            "comm": comm,
            **stat,
            "schedstat": schedstat,
        })
    return threads, errors


def load_probe():
    spec = importlib.util.spec_from_file_location("native_seed_cpu_ram_probe", PROBE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import probe at {PROBE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def thread_cpu_summary(run: dict, tick_hz: int) -> dict:
    transfer = run.get("transfer") or {}
    start = transfer.get("first_payload_sample") or {}
    end = transfer.get("payload_complete_sample") or {}
    start_snapshot = start.get("thread_cpu") or {}
    end_snapshot = end.get("thread_cpu") or {}

    def index(snapshot):
        result = {}
        for row in snapshot.get("threads", []):
            key = (row.get("pid"), row.get("tid"), row.get("starttime_ticks"))
            result[key] = row
        return result

    starts, ends = index(start_snapshot), index(end_snapshot)
    matching = sorted(set(starts) & set(ends))
    rows = []
    for key in matching:
        before, after = starts[key], ends[key]
        user_delta = after["utime_ticks"] - before["utime_ticks"]
        system_delta = after["stime_ticks"] - before["stime_ticks"]
        if user_delta < 0 or system_delta < 0:
            continue
        row = {
            "pid": key[0],
            "tid": key[1],
            "starttime_ticks": key[2],
            "comm_start": before.get("comm"),
            "comm_end": after.get("comm"),
            "user_ticks": user_delta,
            "system_ticks": system_delta,
            "cpu_seconds": (user_delta + system_delta) / tick_hz,
        }
        before_sched, after_sched = before.get("schedstat"), after.get("schedstat")
        if before_sched and after_sched:
            row["schedstat_delta"] = {
                key: after_sched[key] - before_sched[key]
                for key in ("runtime_ns", "runqueue_wait_ns", "timeslices")
            }
        rows.append(row)

    process_ticks = None
    process_seconds = None
    if start.get("cpu_ticks_user_system") is not None and end.get("cpu_ticks_user_system") is not None:
        process_ticks = end["cpu_ticks_user_system"] - start["cpu_ticks_user_system"]
        process_seconds = process_ticks / tick_hz
    matched_ticks = sum(row["user_ticks"] + row["system_ticks"] for row in rows)
    return {
        "boundary": "first_payload_sample_to_payload_complete_sample",
        "tick_hz": tick_hz,
        "process_tree_cpu_ticks": process_ticks,
        "process_tree_cpu_seconds": process_seconds,
        "matched_thread_cpu_ticks": matched_ticks,
        "matched_thread_cpu_seconds": matched_ticks / tick_hz,
        # Signed difference: it can be negative if thread snapshots race with
        # process-tree membership or sample at slightly different instants.
        "process_minus_matched_thread_ticks": (
            process_ticks - matched_ticks if process_ticks is not None else None
        ),
        "matched_thread_count": len(rows),
        "missing_at_payload_complete": [list(key) for key in sorted(set(starts) - set(ends))],
        "new_at_payload_complete": [list(key) for key in sorted(set(ends) - set(starts))],
        "start_snapshot_errors": start_snapshot.get("errors", []),
        "end_snapshot_errors": end_snapshot.get("errors", []),
        "threads": rows,
    }


def self_test() -> None:
    import tempfile

    sample = "42 (worker ) with parens) R 1 2 3 4 5 6 7 8 9 10 120 30 16 17 18 19 20 21 98765 22 23"
    with tempfile.TemporaryDirectory() as directory:
        stat_path = Path(directory) / "stat"
        stat_path.write_text(sample)
        assert read_thread_stat(stat_path) == {
            "utime_ticks": 120,
            "stime_ticks": 30,
            "starttime_ticks": 98765,
        }

    def snap(rows, process_ticks):
        return {"cpu_ticks_user_system": process_ticks,
                "thread_cpu": {"threads": rows, "errors": []}}

    base = {"pid": 10, "tid": 11, "starttime_ticks": 100, "comm": "worker",
            "utime_ticks": 20, "stime_ticks": 3}
    done = {**base, "utime_ticks": 27, "stime_ticks": 5}
    changed = {**base, "starttime_ticks": 101, "utime_ticks": 900, "stime_ticks": 0}
    run = {"transfer": {
        "first_payload_sample": snap([base], 25),
        "payload_complete_sample": snap([done, changed], 33),
    }}
    summary = thread_cpu_summary(run, 100)
    assert summary["matched_thread_count"] == 1
    assert summary["process_tree_cpu_ticks"] == 8
    assert summary["matched_thread_cpu_ticks"] == 9
    assert summary["new_at_payload_complete"] == [[10, 11, 101]]
    assert not summary["missing_at_payload_complete"]

    missing = {"transfer": {
        "first_payload_sample": snap([base], 25),
        "payload_complete_sample": snap([], 30),
    }}
    missing_summary = thread_cpu_summary(missing, 100)
    assert missing_summary["matched_thread_count"] == 0
    assert missing_summary["missing_at_payload_complete"] == [[10, 11, 100]]
    print("proc thread CPU wrapper self-test passed")


def main() -> None:
    if sys.argv[1:] == ["--self-test"]:
        self_test()
        return
    if not any(arg == "--output" or arg.startswith("--output=") for arg in sys.argv[1:]):
        raise SystemExit("diagnostic wrapper requires --output DIR")

    probe = load_probe()
    original_sample = probe.process_sample
    tick_hz = os.sysconf(os.sysconf_names["SC_CLK_TCK"])

    def process_sample_with_threads(root_pid: int) -> dict:
        sample = original_sample(root_pid)
        threads, errors = [], []
        for pid in sample.get("pids", [root_pid]):
            pid_threads, pid_errors = capture_pid_threads(pid)
            threads.extend(pid_threads)
            errors.extend(pid_errors)
        sample["thread_cpu"] = {
            "captured_monotonic": sample.get("monotonic", time.monotonic()),
            "tick_hz": tick_hz,
            "threads": threads,
            "errors": errors,
        }
        return sample

    probe.process_sample = process_sample_with_threads
    probe.main()

    output_arg = next(
        (sys.argv[i + 1] for i, arg in enumerate(sys.argv[:-1]) if arg == "--output"),
        next((arg.split("=", 1)[1] for arg in sys.argv[1:] if arg.startswith("--output=")), None),
    )
    if output_arg is None:
        raise SystemExit("could not resolve --output directory")
    summary_path = Path(output_arg).resolve() / "summary.json"
    report = json.loads(summary_path.read_text())
    for run in report.get("runs", []):
        run["per_thread_payload_cpu"] = thread_cpu_summary(run, tick_hz)
    report["thread_cpu_wrapper"] = {
        "source": str(Path(__file__).resolve()),
        "proc_stat_fields": {"utime": 14, "stime": 15, "starttime": 22},
        "thread_identity": "pid+tid+starttime_ticks",
        "schedstat": "runtime_ns, runqueue_wait_ns and timeslices when available",
        "diagnostic_data_not_clean_performance_evidence": True,
    }
    temporary = summary_path.with_suffix(".thread-cpu.tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n")
    temporary.replace(summary_path)
    print(summary_path)


if __name__ == "__main__":
    main()
