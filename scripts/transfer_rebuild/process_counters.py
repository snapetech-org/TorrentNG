"""Attach diagnostic process counters to a controlled benchmark transfer.

The counter collector is deliberately separate from the regular resource
observer: ``perf`` and especially ``strace`` can change the workload, so these
records are diagnostic evidence and are not used to score transfer CPU or
throughput.
"""

from __future__ import annotations

import csv
import os
import shutil
import signal
import subprocess
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

from transfer_rebuild.observe import process_tree


GIB = 1024**3
PERF_EVENTS = "instructions:u,cycles:u,context-switches,cpu-migrations"
PERF_EVENT_KEYS = {
    "instructions:u": "instructions",
    "cycles:u": "cycles",
    "context-switches": "perf_context_switches",
    "cpu-migrations": "cpu_migrations",
}
NORMALIZED_COUNTERS = {
    "instructions",
    "cycles",
    "perf_context_switches",
    "cpu_migrations",
    "syscalls",
    "errors",
    "voluntary_context_switches",
    "involuntary_context_switches",
}


def parse_context_switch_status(status_text: str) -> tuple[int, int] | None:
    """Parse the voluntary and involuntary fields from one proc status file."""
    values: dict[str, int] = {}
    for line in status_text.splitlines():
        key, separator, raw = line.partition(":")
        if not separator:
            continue
        key = key.strip()
        if key not in ("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches"):
            continue
        try:
            values[key] = int(raw.strip())
        except (OverflowError, ValueError):
            return None
    if len(values) != 2:
        return None
    return values["voluntary_ctxt_switches"], values["nonvoluntary_ctxt_switches"]


@dataclass(frozen=True)
class ContextSwitchSnapshot:
    process_ids: tuple[int, ...]
    thread_ids: tuple[int, ...]
    voluntary: int
    involuntary: int
    readable_threads: int

    def delta(self, later: "ContextSwitchSnapshot") -> dict[str, int | bool | None]:
        stable = (
            self.process_ids == later.process_ids
            and self.thread_ids == later.thread_ids
            and self.readable_threads == len(self.thread_ids)
            and later.readable_threads == len(later.thread_ids)
        )
        voluntary = later.voluntary - self.voluntary if stable else None
        involuntary = later.involuntary - self.involuntary if stable else None
        if voluntary is not None and voluntary < 0:
            voluntary = None
            stable = False
        if involuntary is not None and involuntary < 0:
            involuntary = None
            stable = False
        return {
            "voluntary": voluntary,
            "involuntary": involuntary,
            "stable_process_tree": stable,
            "process_ids_start": list(self.process_ids),
            "process_ids_stop": list(later.process_ids),
            "thread_ids_start": list(self.thread_ids),
            "thread_ids_stop": list(later.thread_ids),
            "readable_threads_start": self.readable_threads,
            "readable_threads_stop": later.readable_threads,
        }


def snapshot_process_context_switches(
    root_pid: int,
    proc_root: Path = Path("/proc"),
) -> ContextSwitchSnapshot:
    """Sum context-switch counters for every thread in a process tree."""
    process_ids = tuple(process_tree(root_pid, proc_root))
    thread_ids: set[int] = set()
    voluntary = 0
    involuntary = 0
    readable_threads = 0
    for pid in process_ids:
        task_root = proc_root / str(pid) / "task"
        try:
            tasks = [entry for entry in task_root.iterdir() if entry.is_dir()]
        except OSError:
            tasks = []
        for task in tasks:
            try:
                tid = int(task.name)
            except ValueError:
                continue
            thread_ids.add(tid)
            try:
                status = (task / "status").read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            counters = parse_context_switch_status(status)
            if counters is None:
                continue
            voluntary += counters[0]
            involuntary += counters[1]
            readable_threads += 1
    return ContextSwitchSnapshot(
        process_ids=process_ids,
        thread_ids=tuple(sorted(thread_ids)),
        voluntary=voluntary,
        involuntary=involuntary,
        readable_threads=readable_threads,
    )


def _parse_count(raw: str) -> int | None:
    value = raw.strip()
    if not value or value.startswith("<") or value.lower() in {"not counted", "not supported"}:
        return None
    try:
        return int(value)
    except ValueError:
        try:
            return int(float(value))
        except (OverflowError, ValueError):
            return None


def parse_perf_stat(output: str) -> dict[str, int | None]:
    """Parse ``perf stat -x,`` output, retaining unsupported events as null."""
    parsed: dict[str, int | None] = {}
    for row in csv.reader(output.splitlines()):
        if len(row) < 3:
            continue
        event = row[2].strip()
        key = PERF_EVENT_KEYS.get(event)
        if key is None:
            continue
        count = _parse_count(row[0])
        if key in parsed and parsed[key] is not None and count is not None:
            parsed[key] += count
        else:
            parsed[key] = count
    return parsed


def parse_strace_summary(output: str) -> dict[str, int | None]:
    """Read the aggregate calls and errors row from ``strace -c`` output."""
    for line in output.splitlines():
        fields = line.split()
        if len(fields) >= 5 and fields[-1] == "total":
            try:
                if len(fields) == 5:
                    calls = int(fields[-2])
                    errors = 0
                else:
                    calls = int(fields[-3])
                    errors = int(fields[-2])
            except ValueError:
                return {"syscalls": None, "errors": None}
            return {"syscalls": calls, "errors": errors}
    return {"syscalls": None, "errors": None}


def add_verified_gib_normalization(
    values: dict[str, object],
    verified_bytes: int,
) -> dict[str, object]:
    """Add per-GiB counters only when the payload passed independent hashing."""
    result = dict(values)
    scale = GIB / verified_bytes if verified_bytes > 0 else None
    for key in NORMALIZED_COUNTERS:
        value = values.get(key)
        result[f"{key}_per_verified_gib"] = (
            value * scale
            if scale is not None and isinstance(value, (int, float))
            else None
        )
    result["verified_bytes"] = verified_bytes if verified_bytes > 0 else 0
    return result


def _sudo_prefix() -> list[str]:
    if os.geteuid() == 0:
        return []
    sudo = shutil.which("sudo")
    if sudo is None:
        return []
    return [sudo, "-n"]


class ProcessCounterCapture:
    """Capture perf or strace counters for an already-running process tree."""

    def __init__(self, root_pid: int, mode: str, *, proc_root: Path = Path("/proc")):
        if mode not in ("perf", "strace"):
            raise ValueError("counter mode must be perf or strace")
        self.root_pid = int(root_pid)
        self.mode = mode
        self.proc_root = proc_root
        self.process_ids: tuple[int, ...] = ()
        self.start_snapshot: ContextSwitchSnapshot | None = None
        self.process: subprocess.Popen[str] | None = None
        self.temp_dir: tempfile.TemporaryDirectory[str] | None = None
        self.counter_file: Path | None = None
        self.stderr_file = None
        self.started_ns: int | None = None

    def start(self) -> None:
        self.process_ids = tuple(process_tree(self.root_pid, self.proc_root))
        if not self.process_ids:
            raise RuntimeError(f"measured process {self.root_pid} is not present")
        tool = shutil.which(self.mode)
        if tool is None:
            raise RuntimeError(f"counter mode {self.mode} requires {self.mode}")
        self.start_snapshot = snapshot_process_context_switches(self.root_pid, self.proc_root)
        self.temp_dir = tempfile.TemporaryDirectory(prefix="torrentng-process-counters-")
        directory = Path(self.temp_dir.name)
        self.counter_file = directory / "counters.txt"
        error_path = directory / "collector.stderr"
        self.stderr_file = error_path.open("w", encoding="utf-8")
        pids = ",".join(str(pid) for pid in self.process_ids)
        if self.mode == "perf":
            command = _sudo_prefix() + [
                tool,
                "stat",
                "-x,",
                "-e",
                PERF_EVENTS,
                "-p",
                pids,
                "-o",
                str(self.counter_file),
            ]
        else:
            command = _sudo_prefix() + [tool, "-f", "-c"]
            for pid in self.process_ids:
                command.extend(("-p", str(pid)))
            command.extend(("-o", str(self.counter_file)))
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=self.stderr_file,
            text=True,
            start_new_session=True,
        )
        self.started_ns = time.monotonic_ns()
        time.sleep(0.1)
        if self.process.poll() is not None:
            stderr = self._read_stderr()
            self._cleanup_files()
            raise RuntimeError(
                f"{self.mode} counter collector exited during attach "
                f"(status {self.process.returncode}): {stderr.strip()}"
            )

    def stop(self) -> dict[str, object]:
        if self.process is None or self.start_snapshot is None:
            raise RuntimeError("counter capture was not started")
        try:
            os.killpg(self.process.pid, signal.SIGINT)
        except ProcessLookupError:
            pass
        try:
            return_code = self.process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(self.process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                return_code = self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                return_code = self.process.wait(timeout=5)
        finish_ns = time.monotonic_ns()
        later = snapshot_process_context_switches(self.root_pid, self.proc_root)
        switch_delta = self.start_snapshot.delta(later)
        raw = self.counter_file.read_text(encoding="utf-8", errors="replace") if (
            self.counter_file and self.counter_file.exists()
        ) else ""
        stderr = self._read_stderr()
        values: dict[str, object]
        if self.mode == "perf":
            values = parse_perf_stat(raw)
        else:
            values = parse_strace_summary(raw)
        values.update({
            "mode": self.mode,
            "root_pid": self.root_pid,
            "attached_process_ids": list(self.process_ids),
            "collector_exit_code": return_code,
            "collector_stderr": stderr.strip() or None,
            "collector_output": raw.strip() or None,
            "collector_elapsed_seconds": (
                (finish_ns - self.started_ns) / 1_000_000_000
                if self.started_ns is not None else None
            ),
            "voluntary_context_switches": switch_delta["voluntary"],
            "involuntary_context_switches": switch_delta["involuntary"],
            "stable_process_tree": switch_delta["stable_process_tree"],
            "process_ids_start": switch_delta["process_ids_start"],
            "process_ids_stop": switch_delta["process_ids_stop"],
            "thread_ids_start": switch_delta["thread_ids_start"],
            "thread_ids_stop": switch_delta["thread_ids_stop"],
        })
        if return_code not in (0, -signal.SIGINT, 128 + signal.SIGINT):
            values["collector_error"] = f"{self.mode} exited with status {return_code}"
        self._cleanup_files()
        return values

    def _read_stderr(self) -> str:
        if self.stderr_file is None:
            return ""
        self.stderr_file.flush()
        try:
            return Path(self.stderr_file.name).read_text(encoding="utf-8", errors="replace")
        except OSError:
            return ""

    def _cleanup_files(self) -> None:
        if self.stderr_file is not None:
            self.stderr_file.close()
            self.stderr_file = None
        if self.temp_dir is not None:
            self.temp_dir.cleanup()
            self.temp_dir = None
