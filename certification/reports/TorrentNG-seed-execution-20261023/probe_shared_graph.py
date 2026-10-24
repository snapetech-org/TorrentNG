#!/usr/bin/env python3
"""Diagnostic graph comparison with activated facade and explicit API CPU work."""
import http.client
import json
import os
from pathlib import Path
import statistics
import sys
import time

sys.path.insert(0, str(Path(__file__).parent / 'TorrentNG/scripts'))
import native_seed_cpu_ram_probe as probe

original = probe.collect_diagnostics


def collect(api_port, root_pid, directory, label):
    result = original(api_port, root_pid, directory, label)
    if label not in ('before-torrent-add', 'after-transfer'):
        return result
    count = 11 if label == 'before-torrent-add' else 10000
    connection = http.client.HTTPConnection('127.0.0.1', api_port, timeout=10)
    before = probe.process_sample(root_pid)
    timings = []
    for index in range(count):
        alias = '/api/qb/v2' if index % 2 else '/api/v2'
        start = time.perf_counter_ns()
        connection.request('GET', alias + '/app/version',
                           headers={'Authorization': 'Bearer ' + probe.TOKEN})
        response = connection.getresponse()
        body = response.read()
        if response.status != 200 or not body:
            raise RuntimeError('qBit alias activation failed')
        timings.append(time.perf_counter_ns() - start)
    after = probe.process_sample(root_pid)
    connection.close()
    activated = original(api_port, root_pid, directory, label + '-after-api-loop')
    observation = {
        'diagnostic_only': True,
        'requests': count,
        'request_pattern': 'alternating aliases, app/version, one persistent HTTP connection',
        'first_request_ns': timings[0],
        'latency_median_ns': statistics.median(timings[1:]),
        'latency_p95_ns': sorted(timings[1:])[int((count - 1) * .95)],
        'cpu_seconds': probe.cpu_seconds_between(before, after, os.sysconf('SC_CLK_TCK')),
        'before_sample': before,
        'after_sample': after,
        'activated_diagnostics': activated,
    }
    result['facade_graph_diagnostic'] = observation
    (Path(directory) / (label + '-facade-loop.json')).write_text(json.dumps(observation, indent=2) + '\n')
    return result


if '--diagnostics' not in sys.argv:
    raise SystemExit('graph experiments require --diagnostics; no qualification from this data')
probe.collect_diagnostics = collect
probe.main()
