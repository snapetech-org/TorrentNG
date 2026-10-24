#!/usr/bin/env python3
"""Diagnostic only: reclaim free allocator pages after the completed recheck."""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'TorrentNG/scripts'))
import native_seed_cpu_ram_probe as probe

original = probe.collect_diagnostics


def collect(api_port, root_pid, directory, label):
    result = original(api_port, root_pid, directory, label)
    if label != 'after-seed-recheck':
        return result
    import time
    base = Path(__file__).parent
    with (base / 'recheck-trim-command.fifo').open('w') as fifo:
        fifo.write('T')
    observation_path = base / 'recheck-trim-observation.json'
    deadline = time.monotonic() + 10
    while not observation_path.exists():
        if time.monotonic() >= deadline:
            raise RuntimeError('allocator diagnostic did not respond')
        time.sleep(.01)
    observation = json.loads(observation_path.read_text())
    after = original(api_port, root_pid, directory, 'after-recheck-allocator-trim')
    result['allocator_retention_diagnostic'] = {
        'scored': False,
        'action': 'one diagnostic-preload malloc_trim(0) after the completed seed recheck',
        'allocator': observation,
        'before': result['smaps_totals_bytes'],
        'after': after['smaps_totals_bytes'],
        'after_diagnostics': after,
    }
    return result


probe.collect_diagnostics = collect
if '--diagnostics' not in sys.argv:
    raise SystemExit('this experiment requires --diagnostics and is never scored')
probe.main()
