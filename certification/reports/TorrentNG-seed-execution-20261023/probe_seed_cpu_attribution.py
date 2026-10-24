#!/usr/bin/env python3
"""Sample the existing first-payload/completion window; diagnostic data only."""
from pathlib import Path
import sys
import time

root = Path(__file__).parent
sys.path.insert(0, str(root / 'TorrentNG/scripts'))
import native_seed_cpu_ram_probe as probe


def control(action):
    code = {'start': 'S', 'stop': 'E'}[action]
    with (root / 'seed-profile-control.fifo').open('w') as request:
        request.write(code)
    ack = root / 'seed-profile-ack.txt'
    deadline = time.monotonic() + 10
    while True:
        if ack.exists() and ack.read_text().strip() == code + ' 1':
            return
        if time.monotonic() >= deadline:
            raise RuntimeError('CPU profiler did not confirm ' + action)
        time.sleep(.01)


original = probe.leecher


def leecher(*args, **kwargs):
    args = list(args)
    args[10] = control
    return original(*args, **kwargs)


if '--diagnostics' not in sys.argv:
    raise SystemExit('CPU attribution requires --diagnostics and is not qualification')
probe.leecher = leecher
probe.main()
