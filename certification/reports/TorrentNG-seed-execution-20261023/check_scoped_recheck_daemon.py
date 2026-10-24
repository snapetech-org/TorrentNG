#!/usr/bin/env python3
"""Isolated daemon recheck witness, NOT a deterministic timing/performance test.

Exit 0: completion and a strictly mid-scan cancellation were witnessed, with
fresh frame counters settled at zero after each. Exit 3: cancellation could not
be observed before hashing finished (INCONCLUSIVE). Exit 1: validation error.
Uses only a new /tmp fixture/session and localhost API. Terminal job evidence is
read from that private SQLite database in mode=ro because /jobs omits terminals.
No host knobs, injected sleeps in the daemon, or runtime changes. Do not run
alongside performance scoring or PGO training. Fixture disk usage is128MiB.
"""
import argparse
import hashlib
import importlib
import json
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request


class Inconclusive(Exception):
    pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--repo', type=Path, default=Path('/workspace/scratch/ca998092d7e5/TorrentNG'))
    parser.add_argument('--config', type=Path, help='Optional TOML containing only [storage] scalar overrides; download_dir is always isolated')
    parser.add_argument('--cpus', help='Optional daemon CPU affinity; no system-wide settings')
    parser.add_argument('--attempts', type=int, default=3, choices=range(1, 6))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.output = args.output.resolve()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    sys.path.insert(0, str(args.repo.resolve() / 'scripts'))
    probe = importlib.import_module('native_seed_cpu_ram_probe')
    # All HTTP in this process is loopback; avoid inherited proxy routing.
    urllib.request.install_opener(urllib.request.build_opener(urllib.request.ProxyHandler({})))
    storage_extra = ''
    if args.config:
        supplied = tomllib.loads(args.config.read_text())
        if set(supplied) - {'storage'} or not isinstance(supplied.get('storage', {}), dict):
            parser.error('--config accepts only a [storage] table')
        for key, value in supplied.get('storage', {}).items():
            if key == 'download_dir':
                parser.error('--config cannot override isolated download_dir')
            if not all(c.isalnum() or c in '_-' for c in key):
                parser.error('invalid storage key')
            if type(value) not in (bool, int, str):
                parser.error('storage overrides must be boolean, integer or string')
            storage_extra += f'{key} = {json.dumps(value)}\n'
    report = {'status': 'ERROR', 'scope': 'functional lifecycle witness; no performance claim',
              'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'fixture_bytes': 128 * 1024 * 1024, 'cancel_attempts': [],
              'phase': 'initializing', 'job_observations': [], 'mutation_requests': [],
              'expected_job_kind': 'recheck_torrent'}
    proc = None
    log = None
    exit_code = 1
    with tempfile.TemporaryDirectory(prefix='tng-scoped-recheck-', dir='/tmp') as temp:
        work = Path(temp)
        try:
            fixture = work / 'fixture'
            _, torrent, info_hash, digest = probe.make_fixture(fixture, report['fixture_bytes'], 924)
            report['fixture_sha256'] = digest
            ih = info_hash.hex()
            port, peer_port = probe.free_port(), probe.free_port()
            db_path = work / 'state.db'
            token = work / 'api-token'
            token.write_text(probe.TOKEN + '\n')
            token.chmod(0o600)
            config = work / 'config.toml'
            config.write_text(f'''[daemon]
api_bind = "127.0.0.1:{port}"
session_dir = "{work}/session"
log_level = "warn"
[db]
path = "{db_path}"
[network]
listen_port = {peer_port}
max_peers = 8
[storage]
download_dir = "{fixture}"
{storage_extra}[dht]
enabled = false
[auth]
api_tokens_file = "{token}"
''')
            env, controls = probe.sanitize_client_environment(os.environ, {'TNG_STORAGE_ANON_LARGE_FRAMES': '1'})
            env['TORRENTNGD_CONFIG'] = str(config)
            report['removed_inherited_controls'] = controls
            report['generated_config'] = config.read_text()
            command = [str(binary)]
            if args.cpus:
                command = ['taskset', '--cpu-list', args.cpus, *command]
            log = (work / 'daemon.log').open('wb')
            proc = subprocess.Popen(command, cwd=work, env=env, stdout=log, stderr=subprocess.STDOUT)

            def request(route, method='GET', timeout=5, raw=False):
                observation = None
                if method == 'POST':
                    observation = {'route': route, 'started_monotonic': time.monotonic(), 'status': None}
                    report['mutation_requests'].append(observation)
                req = urllib.request.Request(f'http://127.0.0.1:{port}{route}',
                    data=b'' if method == 'POST' else None, method=method,
                    headers={'Authorization': f'Bearer {probe.TOKEN}'})
                with urllib.request.urlopen(req, timeout=timeout) as response:
                    if observation is not None:
                        observation['status'] = response.status
                    data = response.read().decode()
                    return data if raw else (json.loads(data) if data else None)

            def jobs():
                # Each query ends its read transaction promptly. No DB writes.
                with sqlite3.connect(db_path.as_uri() + '?mode=ro', uri=True, timeout=1) as db:
                    db.row_factory = sqlite3.Row
                    all_rows = [dict(row) for row in db.execute('SELECT * FROM jobs ORDER BY created_at, job_id')]
                selected = [row for row in all_rows if row['kind'] == 'recheck_torrent'
                            and ih in json.loads(row['affected_torrents'])]
                report['last_database_jobs'] = all_rows
                report['last_selected_jobs'] = selected
                compact = [{key: row[key] for key in ('job_id', 'state', 'total', 'done', 'verified_bytes', 'finished_at')}
                           for row in selected]
                observations = report['job_observations']
                if not observations or observations[-1]['rows'] != compact:
                    observations.append({'monotonic': time.monotonic(), 'phase': report['phase'], 'rows': compact})
                    del observations[:-512]
                return selected

            def job(job_id):
                return next(row for row in jobs() if row['job_id'] == job_id)

            def wait(check, timeout=30):
                end = time.monotonic() + timeout
                while time.monotonic() < end:
                    if proc.poll() is not None:
                        raise RuntimeError(f'daemon exited {proc.returncode}')
                    result = check()
                    if result:
                        return result
                    time.sleep(.002)
                raise TimeoutError('required state was not observed during phase ' + report['phase'])

            def begin_recheck():
                previous = {row['job_id'] for row in jobs()}
                report['phase'] += ':request'
                request(f'/api/v1/torrents/{ih}/recheck', 'POST')
                report['phase'] = report['phase'].removesuffix(':request') + ':discover-job'
                return wait(lambda: next((row for row in jobs() if row['job_id'] not in previous), None))

            def settle_frames():
                samples = []
                deadline = time.monotonic() + 15
                streak = 0
                while time.monotonic() < deadline:
                    text, freshness = probe.collect_fresh_metrics(
                        lambda timeout: request('/metrics', timeout=timeout, raw=True), timeout=3)
                    metrics = probe.parse_selected_metrics(text)
                    samples.append({'freshness': freshness, 'metrics': metrics})
                    if freshness['status'] != 'fresh':
                        raise RuntimeError('cannot validate frame release using stale/unsupported counters')
                    zero = (metrics.get('torrentng_storage_frame_bytes_in_use') == 0
                            and metrics.get('torrentng_storage_frame_bytes_retained') == 0
                            and metrics.get('torrentng_storage_prepared_active_jobs') == 0)
                    streak = streak + 1 if zero else 0
                    if streak == 3:
                        return samples
                    time.sleep(.1)
                raise RuntimeError('frame/prepared-job counters did not settle at zero')

            report['phase'] = 'wait-api'
            probe.wait_for(lambda: socket.create_connection(('127.0.0.1', port), .2), proc, 30, 'API')
            report['phase'] = 'initial-add'
            probe.add_torrent(port, torrent, str(fixture))
            probe.wait_seed_ready('torrentngd', proc, port, None, info_hash, 60)
            report['phase'] = 'initial-add:frame-settle'
            report['after_initial_add'] = settle_frames()
            report['phase'] = 'explicit-completion'
            started = begin_recheck()
            report['phase'] = 'explicit-completion:wait-terminal'
            completed = wait(lambda: (row if row['finished_at'] is not None else None)
                             if (row := job(started['job_id'])) else None)
            if not (completed['state'] == 'completed' and completed['done'] == completed['total']
                    and completed['verified_bytes'] == report['fixture_bytes']):
                raise RuntimeError(f'explicit completion verification failed: {completed}')
            report['completed_recheck'] = completed
            report['phase'] = 'explicit-completion:frame-settle'
            report['after_completion'] = settle_frames()

            witnessed = False
            for attempt in range(args.attempts):
                report['phase'] = f'cancel-attempt-{attempt + 1}'
                started = begin_recheck()
                evidence = {'job_id': started['job_id'], 'observed_partial': None}
                report['cancel_attempts'].append(evidence)
                report['phase'] = f'cancel-attempt-{attempt + 1}:observe-partial'
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    row = job(started['job_id'])
                    if row['finished_at'] is not None:
                        evidence['terminal'] = row
                        if row['state'] != 'completed':
                            raise RuntimeError(f'unexpected pre-cancel terminal state: {row}')
                        break
                    if row['state'] == 'running' and 0 < row['done'] < row['total']:
                        rows = request('/api/qb/v2/torrents/info?hashes=' + ih)
                        torrent_row = next((r for r in rows if r.get('hash', '').lower() == ih), {})
                        if str(torrent_row.get('state', '')).lower().startswith('checking'):
                            evidence['observed_partial'] = {'job': row, 'torrent': torrent_row}
                            report['phase'] = f'cancel-attempt-{attempt + 1}:request-cancel'
                            try:
                                request('/api/v1/jobs/' + urllib.parse.quote(row['job_id'], safe='') + '/cancel', 'POST')
                            except urllib.error.HTTPError as error:
                                if error.code != 400:
                                    raise
                                evidence['cancel_error'] = error.read().decode()
                            report['phase'] = f'cancel-attempt-{attempt + 1}:wait-terminal'
                            final = wait(lambda: (current if current['finished_at'] is not None else None)
                                         if (current := job(row['job_id'])) else None)
                            evidence['terminal'] = final
                            if final['state'] not in ('cancelled', 'completed'):
                                raise RuntimeError(f'unexpected cancel terminal state: {final}')
                            # Actor can label a late cancellation cancelled AFTER full hashing;
                            # only a strictly partial terminal checkpoint proves interruption.
                            witnessed = (final['state'] == 'cancelled' and 0 < final['done'] < final['total']
                                         and 0 < final['verified_bytes'] < report['fixture_bytes'])
                            break
                    time.sleep(.002)
                else:
                    raise TimeoutError('recheck never reached a terminal/observable partial state')
                report['phase'] = f'cancel-attempt-{attempt + 1}:frame-settle'
                evidence['settled_frames'] = settle_frames()
                if witnessed:
                    break
            if not witnessed:
                raise Inconclusive('no strictly mid-recheck cancellation witnessed; completed/late races are not passes')
            report['phase'] = 'complete'
            report['status'] = 'PASS'
            exit_code = 0
        except Inconclusive as error:
            report.update(status='INCONCLUSIVE', reason=str(error))
            exit_code = 3
        except Exception as error:
            report.update(status='ERROR', reason=f'{type(error).__name__}: {error}')
        finally:
            if proc is not None and proc.poll() is None and report['status'] != 'PASS':
                # Preserve evidence before disposing this private session. Never
                # replace the original error if a diagnostic request also fails.
                for label, capture in (
                    ('database_jobs', lambda: jobs()),
                    ('api_jobs', lambda: request('/api/v1/jobs')),
                    ('torrent_state', lambda: request('/api/qb/v2/torrents/info?hashes=' + ih)),
                    ('metrics_unfenced', lambda: request('/metrics', raw=True)),
                ):
                    try:
                        report.setdefault('failure_evidence', {})[label] = capture()
                    except Exception as error:
                        report.setdefault('failure_evidence', {})[label] = {'error': repr(error)}
            if proc is not None and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
            if log is not None:
                log.close()
                shutil.copyfile(work / 'daemon.log', args.output.with_suffix('.daemon.log'))
            args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'status': report['status'], 'output': str(args.output), 'reason': report.get('reason')}))
    return exit_code


if __name__ == '__main__':
    raise SystemExit(main())
