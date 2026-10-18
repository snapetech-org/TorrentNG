"""Shared timing, start-barrier, and unique-payload accounting primitives."""

from __future__ import annotations

import bisect
import math
import os
import re
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable


_PARTICIPANT = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$")


@dataclass(frozen=True)
class BarrierResult:
    wait_started_ns: int
    released_observed_ns: int

    @property
    def wait_ns(self) -> int:
        if self.released_observed_ns < self.wait_started_ns:
            raise ValueError("barrier clock moved backwards")
        return self.released_observed_ns - self.wait_started_ns


class FileStartBarrier:
    """A bounded cross-process start gate backed by task-owned scratch files.

    The caller creates a fresh directory for each trial and removes it after
    collecting the run. Participant IDs are fixed up front so a typo cannot
    silently lower the barrier count.
    """

    def __init__(self, directory: Path, participants: list[str]) -> None:
        if directory.is_symlink() or not directory.is_dir():
            raise ValueError("barrier directory must be an existing, non-symlink directory")
        if not participants or len(set(participants)) != len(participants):
            raise ValueError("barrier participants must be nonempty and unique")
        if any(_PARTICIPANT.fullmatch(participant) is None for participant in participants):
            raise ValueError("barrier participant IDs must be simple filename-safe labels")

        self.directory = directory
        self.participants = tuple(sorted(participants))
        self.release_path = directory / "released"

    def wait(
        self,
        participant: str,
        *,
        timeout_seconds: float,
        poll_interval_seconds: float = 0.005,
        clock_ns: Callable[[], int] = time.perf_counter_ns,
        clock: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ) -> BarrierResult:
        if participant not in self.participants:
            raise ValueError(f"unknown barrier participant: {participant}")
        if (
            not math.isfinite(timeout_seconds)
            or not math.isfinite(poll_interval_seconds)
            or timeout_seconds <= 0
            or poll_interval_seconds <= 0
        ):
            raise ValueError("barrier timeout and poll interval must be positive")
        if self.release_path.exists():
            raise ValueError("barrier release marker already exists; use a fresh trial directory")

        wait_started = clock_ns()
        ready_path = self.directory / f"{participant}.ready"
        descriptor = os.open(ready_path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(descriptor)
        deadline = clock() + timeout_seconds
        ready_paths = [self.directory / f"{name}.ready" for name in self.participants]

        while True:
            if all(path.is_file() for path in ready_paths):
                try:
                    self.release_path.mkdir(mode=0o700)
                except FileExistsError:
                    pass
            if self.release_path.is_dir():
                return BarrierResult(wait_started, clock_ns())

            remaining = deadline - clock()
            if remaining <= 0:
                raise TimeoutError(f"start barrier timed out for participant {participant}")
            sleep(min(poll_interval_seconds, remaining))


class UsefulRangeLedger:
    """Count unique accepted payload ranges and duplicate bytes by file offset.

    Call this with validated payload ranges only. Protocol overhead is outside
    the ledger, and BEP 47 padding ranges are ignored.
    """

    def __init__(self) -> None:
        self._ranges: dict[int, list[tuple[int, int]]] = {}
        self.useful_bytes = 0
        self.duplicate_bytes = 0

    def accept(self, file_index: int, offset: int, length: int, *, pad: bool = False) -> tuple[int, int]:
        if file_index < 0 or offset < 0 or length < 0:
            raise ValueError("file index, offset, and length must be nonnegative")
        if pad or length == 0:
            return 0, 0
        end = offset + length
        if end < offset:
            raise ValueError("payload range overflow")

        intervals = self._ranges.setdefault(file_index, [])
        first = bisect.bisect_left(intervals, (offset, offset))
        if first and intervals[first - 1][1] >= offset:
            first -= 1

        merged_start = offset
        merged_end = end
        overlap = 0
        last = first
        while last < len(intervals) and intervals[last][0] <= merged_end:
            current_start, current_end = intervals[last]
            overlap += max(0, min(end, current_end) - max(offset, current_start))
            merged_start = min(merged_start, current_start)
            merged_end = max(merged_end, current_end)
            last += 1

        unique = length - overlap
        intervals[first:last] = [(merged_start, merged_end)]
        self.useful_bytes += unique
        self.duplicate_bytes += overlap
        return unique, overlap


class TrialTimeline:
    """Capture separate monotonic intervals required by transfer-trial-v1."""

    _PHASES = {
        "barrier_wait": ("barrier_wait_started", "barrier_released"),
        "transfer": ("first_useful_byte", "last_useful_byte"),
        "flush": ("flush_started", "flush_completed"),
        "validation": ("validation_started", "validation_completed"),
        "shutdown": ("shutdown_started", "shutdown_completed"),
    }
    _MARKERS = {marker for pair in _PHASES.values() for marker in pair}
    _ORDERED_MARKERS = (
        "barrier_wait_started",
        "barrier_released",
        "first_useful_byte",
        "last_useful_byte",
        "flush_started",
        "flush_completed",
        "validation_started",
        "validation_completed",
        "shutdown_started",
        "shutdown_completed",
    )

    def __init__(self, clock_ns: Callable[[], int] = time.perf_counter_ns) -> None:
        self._clock_ns = clock_ns
        self._markers: dict[str, int] = {}

    def mark(self, name: str, timestamp_ns: int | None = None) -> int:
        if name not in self._MARKERS:
            raise ValueError(f"unknown trial timing marker: {name}")
        if name in self._markers:
            raise ValueError(f"trial timing marker already recorded: {name}")
        value = self._clock_ns() if timestamp_ns is None else timestamp_ns
        if value < 0:
            raise ValueError("monotonic timestamps must be nonnegative")
        self._markers[name] = value
        return value

    def durations_ns(self) -> dict[str, int]:
        missing = [name for name in self._ORDERED_MARKERS if name not in self._markers]
        if missing:
            raise ValueError(f"missing timing markers: {', '.join(missing)}")
        ordered = [self._markers[name] for name in self._ORDERED_MARKERS]
        if any(left > right for left, right in zip(ordered, ordered[1:])):
            raise ValueError("trial timing phases overlap or are out of order")

        durations: dict[str, int] = {}
        for phase, (start_name, end_name) in self._PHASES.items():
            if start_name not in self._markers or end_name not in self._markers:
                raise ValueError(f"missing timing markers for {phase}")
            duration = self._markers[end_name] - self._markers[start_name]
            if duration < 0:
                raise ValueError(f"timing markers are reversed for {phase}")
            if phase == "transfer" and duration == 0:
                raise ValueError("transfer duration must be positive for goodput")
            durations[phase] = duration
        return durations
