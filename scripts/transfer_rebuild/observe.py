#!/usr/bin/env python3
"""Independent Linux process-tree and cgroup resource observer.

The measured client does not provide these counters. This observer samples
/proc and cgroup v2 from a separate process, writes raw samples to CSV, and
emits a peak summary JSON for later inclusion in a transfer-trial record.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, TextIO


FIELDS = (
    "elapsed_ns",
    "processes",
    "rss_bytes",
    "pss_bytes",
    "cgroup_anon_bytes",
    "cgroup_file_bytes",
    "cgroup_reclaimable_cache_bytes",
    "cgroup_current_bytes",
    "cpu_time_ns",
    "threads",
    "fds",
    "block_read_bytes",
    "block_write_bytes",
    "network_namespace_inode",
    "network_rx_bytes",
    "network_tx_bytes",
)
NETWORK_INTERFACE_RE = re.compile(r"^[A-Za-z0-9_.-]{1,15}$")


def _read_text(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except (FileNotFoundError, PermissionError, ProcessLookupError, OSError):
        return None


def process_tree(root_pid: int, proc_root: Path = Path("/proc")) -> list[int]:
    """Return the live process tree rooted at ``root_pid`` (including root)."""
    todo = [root_pid]
    seen: set[int] = set()
    while todo:
        pid = todo.pop()
        if pid in seen:
            continue
        if not (proc_root / str(pid)).is_dir():
            continue
        seen.add(pid)
        task_root = proc_root / str(pid) / "task"
        try:
            task_ids = [entry.name for entry in task_root.iterdir() if entry.is_dir()]
        except (FileNotFoundError, PermissionError, ProcessLookupError, OSError):
            task_ids = [str(pid)]
        for task_id in task_ids:
            children = _read_text(task_root / task_id / "children")
            if children:
                for child in children.split():
                    try:
                        todo.append(int(child))
                    except ValueError:
                        continue
    return sorted(seen)


def _status_kib(pid: int, key: str, proc_root: Path) -> int | None:
    status = _read_text(proc_root / str(pid) / "status")
    if status is None:
        return None
    prefix = f"{key}:"
    for line in status.splitlines():
        if line.startswith(prefix):
            try:
                return int(line[len(prefix) :].strip().split()[0])
            except (IndexError, ValueError):
                return None
    return None


def _smaps_pss_kib(pid: int, proc_root: Path) -> int | None:
    rollup = _read_text(proc_root / str(pid) / "smaps_rollup")
    if rollup is None:
        return None
    for line in rollup.splitlines():
        if line.startswith("Pss:"):
            try:
                return int(line.split()[1])
            except (IndexError, ValueError):
                return None
    return None


def _cpu_ticks(pid: int, proc_root: Path) -> int | None:
    stat = _read_text(proc_root / str(pid) / "stat")
    if stat is None:
        return None
    end = stat.rfind(")")
    if end < 0:
        return None
    fields = stat[end + 2 :].split()  # starts at field 3 (state)
    try:
        return int(fields[11]) + int(fields[12])  # fields 14 and 15
    except (IndexError, ValueError):
        return None


def _io_bytes(pid: int, proc_root: Path) -> tuple[int | None, int | None]:
    data = _read_text(proc_root / str(pid) / "io")
    if data is None:
        return None, None
    values: dict[str, int] = {}
    for line in data.splitlines():
        key, separator, raw = line.partition(":")
        if separator:
            try:
                values[key.strip()] = int(raw.strip())
            except ValueError:
                continue
    return values.get("read_bytes"), values.get("write_bytes")


def _sum_available(values: list[int | None]) -> int | None:
    present = [value for value in values if value is not None]
    return sum(present) if present and len(present) == len(values) else None


def _cgroup_v2_dir(pid: int, proc_root: Path, cgroup_root: Path) -> Path | None:
    content = _read_text(proc_root / str(pid) / "cgroup")
    if content is None:
        return None
    for line in content.splitlines():
        hierarchy, controllers, path = line.partition(":")
        if hierarchy == "0" and controllers.startswith(":"):
            relative = path.lstrip("/")
            try:
                root = cgroup_root.resolve()
                candidate = (root / relative).resolve()
                candidate.relative_to(root)
            except (OSError, ValueError):
                return None
            return candidate
    return None


def _read_integer(path: Path) -> int | None:
    raw = _read_text(path)
    if raw is None:
        return None
    raw = raw.strip()
    if raw == "max":
        return None
    try:
        return int(raw)
    except ValueError:
        return None


def _cgroup_stat(
    path: Path | None,
) -> tuple[int | None, int | None, int | None, int | None]:
    if path is None:
        return None, None, None, None
    current = _read_integer(path / "memory.current")
    stat = _read_text(path / "memory.stat")
    if stat is None:
        return None, None, None, current
    values: dict[str, int] = {}
    for line in stat.splitlines():
        fields = line.split(maxsplit=1)
        if len(fields) != 2:
            continue
        try:
            values[fields[0]] = int(fields[1])
        except ValueError:
            continue
    return values.get("anon"), values.get("file"), values.get("inactive_file"), current


def _network_counters(
    pid: int,
    interface: str | None,
    proc_root: Path,
) -> tuple[int | None, int | None, int | None]:
    """Read one interface's counters in the observed process's net namespace."""
    if interface is None:
        return None, None, None
    if NETWORK_INTERFACE_RE.fullmatch(interface) is None:
        raise ValueError("network interface must be a simple Linux interface name")

    namespace_path = proc_root / str(pid) / "ns" / "net"
    try:
        before = namespace_path.stat().st_ino
    except OSError:
        return None, None, None
    data = _read_text(proc_root / str(pid) / "net" / "dev")
    if data is None:
        return None, None, None

    matches: list[tuple[int, int]] = []
    for line in data.splitlines():
        name, separator, raw = line.partition(":")
        if not separator or name.strip() != interface:
            continue
        values = raw.split()
        if len(values) < 9:
            return None, None, None
        try:
            received = int(values[0])
            transmitted = int(values[8])
        except ValueError:
            return None, None, None
        if received < 0 or transmitted < 0:
            return None, None, None
        matches.append((received, transmitted))

    try:
        after = namespace_path.stat().st_ino
    except OSError:
        return None, None, None
    if before != after or len(matches) != 1:
        return None, None, None
    received, transmitted = matches[0]
    return before, received, transmitted


def _create_exclusive_text(path: Path, *, newline: str | None = None) -> TextIO:
    """Create a private text artifact without replacing a prior observation."""
    flags = (
        os.O_WRONLY
        | os.O_CREAT
        | os.O_EXCL
        | getattr(os, "O_CLOEXEC", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )
    descriptor = os.open(path, flags, 0o600)
    return os.fdopen(descriptor, "w", encoding="utf-8", newline=newline)


@dataclass
class Sample:
    elapsed_ns: int
    processes: int
    rss_bytes: int | None
    pss_bytes: int | None
    cgroup_anon_bytes: int | None
    cgroup_file_bytes: int | None
    cgroup_reclaimable_cache_bytes: int | None
    cgroup_current_bytes: int | None
    cpu_time_ns: int | None
    threads: int | None
    fds: int | None
    block_read_bytes: int | None
    block_write_bytes: int | None
    network_namespace_inode: int | None = None
    network_rx_bytes: int | None = None
    network_tx_bytes: int | None = None

    def as_row(self) -> dict[str, int | None]:
        return {field: getattr(self, field) for field in FIELDS}


def sample_process_tree(
    root_pid: int,
    start_ns: int,
    proc_root: Path = Path("/proc"),
    cgroup_root: Path = Path("/sys/fs/cgroup"),
    network_interface: str | None = None,
) -> Sample:
    pids = process_tree(root_pid, proc_root)
    rss = [_status_kib(pid, "VmRSS", proc_root) for pid in pids]
    pss = [_smaps_pss_kib(pid, proc_root) for pid in pids]
    threads = [_status_kib(pid, "Threads", proc_root) for pid in pids]
    fds: list[int | None] = []
    for pid in pids:
        try:
            fds.append(len(list((proc_root / str(pid) / "fd").iterdir())))
        except (FileNotFoundError, PermissionError, ProcessLookupError, OSError):
            fds.append(None)
    io_values = [_io_bytes(pid, proc_root) for pid in pids]
    read_values = [value[0] for value in io_values]
    write_values = [value[1] for value in io_values]
    ticks = _sum_available([_cpu_ticks(pid, proc_root) for pid in pids])
    ticks_per_second = os.sysconf("SC_CLK_TCK")
    cgroup_dirs = {_cgroup_v2_dir(pid, proc_root, cgroup_root) for pid in pids}
    cgroup_dir = next(iter(cgroup_dirs)) if len(cgroup_dirs) == 1 else None
    anon, file_backed, reclaimable_cache, current = _cgroup_stat(cgroup_dir)
    network_namespace_inode, network_rx_bytes, network_tx_bytes = _network_counters(
        root_pid,
        network_interface,
        proc_root,
    )
    rss_kib = _sum_available(rss)
    pss_kib = _sum_available(pss)
    return Sample(
        elapsed_ns=time.monotonic_ns() - start_ns,
        processes=len(pids),
        rss_bytes=None if rss_kib is None else rss_kib * 1024,
        pss_bytes=None if pss_kib is None else pss_kib * 1024,
        cgroup_anon_bytes=anon,
        cgroup_file_bytes=file_backed,
        cgroup_reclaimable_cache_bytes=reclaimable_cache,
        cgroup_current_bytes=current,
        cpu_time_ns=None if ticks is None else ticks * 1_000_000_000 // ticks_per_second,
        threads=_sum_available(threads),
        fds=_sum_available(fds),
        block_read_bytes=_sum_available(read_values),
        block_write_bytes=_sum_available(write_values),
        network_namespace_inode=network_namespace_inode,
        network_rx_bytes=network_rx_bytes,
        network_tx_bytes=network_tx_bytes,
    )


class PeakSummary:
    """Online resource high-water marks with constant memory use."""

    def __init__(self, network_interface: str | None = None) -> None:
        self.sample_count = 0
        self.peaks: dict[str, int | None] = {}
        self.network_interface = network_interface
        self._network_valid = network_interface is not None
        self._network_observation_count = 0
        self._network_namespace_inode: int | None = None
        self._network_first: tuple[int, int] | None = None
        self._network_last: tuple[int, int] | None = None

    def observe(self, sample: Sample) -> None:
        self.sample_count += 1
        for field in FIELDS:
            if field in {
                "elapsed_ns",
                "processes",
                "network_namespace_inode",
                "network_rx_bytes",
                "network_tx_bytes",
            }:
                continue
            value = getattr(sample, field)
            if value is None:
                continue
            previous = self.peaks.get(field)
            self.peaks[field] = value if previous is None else max(previous, value)

        if self.network_interface is None:
            return
        if (
            sample.network_namespace_inode is None
            or sample.network_rx_bytes is None
            or sample.network_tx_bytes is None
        ):
            self._network_valid = False
            return
        self._network_observation_count += 1
        if self._network_namespace_inode is None:
            self._network_namespace_inode = sample.network_namespace_inode
            self._network_first = (sample.network_rx_bytes, sample.network_tx_bytes)
            self._network_last = self._network_first
            return
        if (
            sample.network_namespace_inode != self._network_namespace_inode
            or self._network_last is None
            or sample.network_rx_bytes < self._network_last[0]
            or sample.network_tx_bytes < self._network_last[1]
        ):
            self._network_valid = False
            return
        self._network_last = (sample.network_rx_bytes, sample.network_tx_bytes)

    def as_json(self) -> dict[str, Any]:
        peaks = {
            field: self.peaks.get(field)
            for field in FIELDS
            if field
            not in {
                "elapsed_ns",
                "processes",
                "network_namespace_inode",
                "network_rx_bytes",
                "network_tx_bytes",
            }
        }
        network_observed = (
            self.network_interface is not None
            and self._network_valid
            and self._network_observation_count >= 2
            and self._network_first is not None
            and self._network_last is not None
        )
        return {
            "observer_id": "transfer-proc-cgroup-v2",
            "sample_count": self.sample_count,
            "sample_interval_ms": None,
            "peaks": peaks,
            "counter_note": "RSS/PSS are summed across the observed process tree; cgroup values are separate domains.",
            "network_note": (
                "Network byte counters were not requested."
                if self.network_interface is None
                else (
                    "RX/TX count all traffic on the named interface in the root "
                    "process's network namespace; treat them as one client's bytes "
                    "only when that namespace isolates exactly one client."
                )
            ),
            "network_interface": self.network_interface,
            "network_namespace_inode": (
                self._network_namespace_inode if network_observed else None
            ),
            "network_rx_bytes": (
                self._network_last[0] - self._network_first[0]
                if network_observed
                else None
            ),
            "network_tx_bytes": (
                self._network_last[1] - self._network_first[1]
                if network_observed
                else None
            ),
        }


def peak_summary(
    samples: list[Sample],
    network_interface: str | None = None,
) -> dict[str, Any]:
    summary = PeakSummary(network_interface)
    for sample in samples:
        summary.observe(sample)
    return summary.as_json()


def observe(
    root_pid: int,
    csv_path: Path,
    summary_path: Path,
    interval_ms: int,
    duration_seconds: float | None,
    stop_file: Path | None,
    network_interface: str | None = None,
) -> int:
    if interval_ms < 1:
        raise ValueError("interval_ms must be at least 1")
    if duration_seconds is None and stop_file is None:
        raise ValueError("provide duration_seconds or stop_file")
    if duration_seconds is not None and duration_seconds <= 0:
        raise ValueError("duration_seconds must be positive")
    if (
        network_interface is not None
        and NETWORK_INTERFACE_RE.fullmatch(network_interface) is None
    ):
        raise ValueError("network interface must be a simple Linux interface name")
    start_ns = time.monotonic_ns()
    if csv_path.resolve() == summary_path.resolve():
        raise ValueError("CSV and summary outputs must be different files")
    deadline_ns = (
        None
        if duration_seconds is None
        else start_ns + int(duration_seconds * 1_000_000_000)
    )
    summary = PeakSummary(network_interface)
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    with _create_exclusive_text(csv_path, newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS)
        writer.writeheader()
        while True:
            if not process_tree(root_pid):
                break
            sample = sample_process_tree(
                root_pid,
                start_ns,
                network_interface=network_interface,
            )
            summary.observe(sample)
            writer.writerow(sample.as_row())
            stream.flush()
            if stop_file is not None and stop_file.exists():
                break
            if deadline_ns is not None and time.monotonic_ns() >= deadline_ns:
                break
            time.sleep(interval_ms / 1000)
        stream.flush()
        os.fsync(stream.fileno())
    payload = summary.as_json()
    payload["root_pid"] = root_pid
    payload["sample_interval_ms"] = interval_ms
    payload["started_monotonic_ns"] = start_ns
    payload["finished_monotonic_ns"] = time.monotonic_ns()
    payload["samples_csv"] = csv_path.name
    with _create_exclusive_text(summary_path) as stream:
        stream.write(json.dumps(payload, indent=2) + "\n")
        stream.flush()
        os.fsync(stream.fileno())
    if summary.sample_count == 0:
        print(f"process {root_pid} was not observable", file=sys.stderr)
        return 2
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", required=True, type=int)
    parser.add_argument("--csv", required=True, type=Path)
    parser.add_argument("--summary", required=True, type=Path)
    parser.add_argument("--interval-ms", type=int, default=250)
    parser.add_argument("--duration-seconds", type=float)
    parser.add_argument("--stop-file", type=Path)
    parser.add_argument(
        "--network-interface",
        help="attribute bytes to this interface in the observed process's network namespace",
    )
    args = parser.parse_args()
    try:
        return observe(
            args.pid,
            args.csv,
            args.summary,
            args.interval_ms,
            args.duration_seconds,
            args.stop_file,
            args.network_interface,
        )
    except (OSError, ValueError) as error:
        print(f"observer: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
