#!/usr/bin/env python3
import concurrent.futures
import hashlib
import json
import os
import random
import signal
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[6]
SCRATCH = Path(os.environ.get('TNG_WORKER_SCREEN_SCRATCH', '/tmp/torrentng-seed-goal-20261022'))
sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(1, str(REPO / 'scripts'))
import native_seed_cpu_ram_probe as probe

SIZE = 64 * 1024 * 1024
PEERS = 3
ROUNDS = 1
SEED = 20261022
BATCH8 = REPO / 'target/release/torrentngd'
RT = SCRATCH / 'rtorrent-rootfs/usr/local/bin/rtorrent'
RT_LIBS = SCRATCH / 'rtorrent-libs'
RT_ROOT_LIBS = SCRATCH / 'rtorrent-rootfs/usr/local/lib'
OUT = Path(os.environ.get('TNG_WORKER_SCREEN_OUTPUT', str(SCRATCH / 'io-worker-count-3peer-64m-replay-20261023')))

variants = [
    ('control', 'torrentngd', str(BATCH8)),
    ('one_worker', 'torrentngd', str(BATCH8)),
    ('rt', 'rtorrent', str(RT)),
]
if OUT.exists():
    raise SystemExit(f'refusing to overwrite prior output: {OUT}')
OUT.mkdir(parents=True)
fixture = OUT / 'fixture'
payload, torrent, ih, digest = probe.make_fixture(fixture, SIZE, SEED)
rng = random.Random(int(os.environ.get('TNG_WORKER_SCREEN_ORDER_SEED', '20261029')))
order = list(variants)
rng.shuffle(order)
report = {
    'status': 'running', 'method': 'three simultaneous loopback leechers per one seed daemon; each receives the same deterministic fixture at 4 MiB/s; process CPU normalized over total verified bytes',
    'fixture_bytes_per_peer': SIZE, 'peer_count': PEERS, 'total_payload_bytes': SIZE * PEERS,
    'fixture_sha256': digest, 'info_hash': ih.hex(), 'binary_hashes': {},
    'leecher_rate_bytes_per_second': probe.RATE, 'randomized_order': [v[0] for v in order],
    'runs': []
}
(OUT / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')

try:
    for round_number, client in enumerate(order, 1):
        name, kind, binary = client
        env = ({'TNG_STORAGE_BACKEND': 'pread'} if name in ('control', 'one_worker') else
               {'LD_LIBRARY_PATH': f'{RT_LIBS}:{RT_ROOT_LIBS}'})
        run_dir = OUT / f'run-{round_number:02d}-{name}'
        run_dir.mkdir()
        item = probe.start_client(client, run_dir, fixture, torrent, ih, 120, env)
        stop = threading.Event()
        active_samples = []
        sampler_error = []
        def sample_loop():
            while not stop.is_set():
                try:
                    active_samples.append(probe.process_sample(item['proc_pid']))
                except Exception as exc:
                    sampler_error.append(f'{type(exc).__name__}: {exc}')
                    return
                stop.wait(1.0)
        try:
            time.sleep(2)
            start_sample = probe.process_sample(item['proc_pid'])
            sampler = threading.Thread(target=sample_loop, daemon=True)
            sampler.start()
            started = time.monotonic()
            register = None
            if kind == 'torrentngd':
                register = lambda address: probe.add_torrent_peer(item['api_port'], ih.hex(), address)
            def fetch(peer_index):
                return probe.leecher(
                    item['peer_port'], ih, SIZE, run_dir / f'downloaded-{peer_index}.bin', digest,
                    180, lambda: probe.process_sample(item['proc_pid']), register_peer=register)
            with concurrent.futures.ThreadPoolExecutor(max_workers=PEERS) as pool:
                transfers = list(pool.map(fetch, range(PEERS)))
            elapsed = time.monotonic() - started
            end_sample = probe.process_sample(item['proc_pid'])
            stop.set(); sampler.join()
            all_samples = [start_sample, end_sample, *active_samples]
            for t in transfers:
                all_samples.extend([t['connection_start_sample'], t['first_payload_sample'],
                                    t['payload_complete_sample'], t['post_leecher_sample']])
            cpu_seconds = probe.cpu_seconds_between(start_sample, end_sample,
                os.sysconf(os.sysconf_names['SC_CLK_TCK']))
            total_bytes = sum(t['bytes'] for t in transfers)
            record = {
                'round': round_number, 'name': name, 'kind': kind, 'binary': binary,
                'binary_sha256': item['binary_sha256'], 'environment_overrides': env,
                'verified_peers': sum(bool(t['verified']) for t in transfers),
                'peer_transfers': [{'bytes': t['bytes'], 'sha256': t['sha256'],
                                    'payload_elapsed_seconds': t['payload_elapsed_seconds'],
                                    'elapsed_seconds': t['elapsed_seconds']} for t in transfers],
                'total_verified_bytes': total_bytes, 'elapsed_wall_seconds': elapsed,
                'process_cpu_seconds': cpu_seconds,
                'cpu_seconds_per_gib': cpu_seconds / (total_bytes / 1024**3),
                'peak_pss_bytes': max(s['pss_bytes'] for s in all_samples),
                'peak_anonymous_bytes': max(s['anonymous_bytes'] for s in all_samples),
                'peak_rss_bytes': max(s['rss_bytes'] for s in all_samples),
                'samples_count': len(all_samples), 'sampler_error': sampler_error,
            }
            report['binary_hashes'][name] = item['binary_sha256']
            report['runs'].append(record)
            report['status'] = 'running'
            (OUT / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
            # Remove only this successful run's known generated downloads and seed copy.
            for p in [run_dir / f'downloaded-{i}.bin' for i in range(PEERS)]:
                if p.is_file() and not p.is_symlink(): p.unlink()
            probe.cleanup_run_payloads(run_dir, name)
        finally:
            stop.set()
            try:
                os.killpg(item['process'].pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                item['process'].wait(timeout=15)
            except Exception:
                try: os.killpg(item['process'].pid, signal.SIGKILL)
                except ProcessLookupError: pass
                item['process'].wait()
            item['log'].close()
    report['status'] = 'complete'
except BaseException as exc:
    report['status'] = 'failed'
    report['error'] = f'{type(exc).__name__}: {exc}'
    (OUT / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
    raise
(OUT / 'summary.json').write_text(json.dumps(report, indent=2) + '\n')
print(OUT / 'summary.json')
