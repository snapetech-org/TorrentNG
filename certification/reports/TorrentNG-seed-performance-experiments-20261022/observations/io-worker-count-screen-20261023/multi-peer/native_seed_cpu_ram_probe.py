#!/usr/bin/env python3
"""Local whole-client CPU/RAM probe for private, verified torrent seeding.

Runs isolated native TorrentNG/rTorrent processes against the same deterministic
single-file BEP 3 fixture. A bounded TCP leecher requests 16 KiB blocks in a
32-block window, paces offered load to 4 MiB/s, and verifies the full SHA-256.
Completion time is recorded as a validity check, not a performance target.
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.util
import hashlib
import json
import os
import random
import re
import signal
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

BLOCK = 16 * 1024
PIECE = 1024 * 1024
RATE = 4 * 1024 * 1024
WINDOW = 32
PROFILE_FREQUENCY_HZ = 1000
MAX_STORAGE_IO_QUEUE_DEPTH = 16_384
TOKEN = "native-seed-probe-local-token"
PEER_ID = b"-TNG0001-CPU-RAM0001"
CONTROL_ENV_EXACT = {
    "TOKIO_WORKER_THREADS", "RUST_LOG", "RUST_LOG_STYLE", "GLIBC_TUNABLES",
    "LD_PRELOAD", "LD_AUDIT", "LD_LIBRARY_PATH", "LD_PROFILE", "LD_DEBUG",
    "LD_BIND_NOW", "CPUPROFILE", "CPUPROFILE_FREQUENCY", "HEAPPROFILE",
    "HEAP_PROFILE_ALLOCATION_INTERVAL", "LLVM_PROFILE_FILE", "GCOV_PREFIX",
    "PERF_RECORD", "PYTHONMALLOC", "MALLOC_ARENA_MAX", "MALLOC_CONF",
    "MALLOC_TRIM_THRESHOLD_", "MALLOC_MMAP_THRESHOLD_", "MALLOC_PERTURB_",
}
CONTROL_ENV_PREFIXES = ("MALLOC_", "JEMALLOC_", "MIMALLOC_", "TNG_", "TORRENTNG")
SAFE_CONTROL_VALUES = {
    "TOKIO_WORKER_THREADS", "MALLOC_ARENA_MAX", "MALLOC_CONF",
    "MALLOC_TRIM_THRESHOLD_", "MALLOC_MMAP_THRESHOLD_", "MALLOC_PERTURB_",
    "GLIBC_TUNABLES", "LD_PRELOAD", "RUST_LOG",
}
DIAGNOSTIC_METRICS = {
    "torrentng_storage_peer_read_cache_entries",
    "torrentng_storage_peer_read_cache_bytes",
    "torrentng_storage_peer_read_cache_global_backing_bytes",
    "torrentng_storage_peer_read_cache_global_budget_bytes",
    "torrentng_storage_peer_read_cache_hits_total",
    "torrentng_storage_peer_read_cache_misses_total",
    "torrentng_storage_peer_read_cache_evictions_total",
    "torrentng_storage_frame_bytes_in_use",
    "torrentng_storage_frame_bytes_retained",
    "torrentng_storage_frame_bytes_cap",
}


def bencode(v):
    if isinstance(v, int): return b"i" + str(v).encode() + b"e"
    if isinstance(v, bytes): return str(len(v)).encode() + b":" + v
    if isinstance(v, list): return b"l" + b"".join(map(bencode, v)) + b"e"
    if isinstance(v, dict): return b"d" + b"".join(bencode(k) + bencode(v[k]) for k in sorted(v)) + b"e"
    raise TypeError(v)


def make_fixture(folder: Path, size: int, seed: int):
    folder.mkdir()
    payload = folder / "payload.bin"
    key, digest, hashes, pending = seed.to_bytes(8, "big"), hashlib.sha256(), bytearray(), bytearray()
    remaining = size
    counter = 0
    with payload.open("wb") as out:
        while remaining:
            data = hashlib.shake_256(b"native-seed-cpu-ram-v1\0" + key + counter.to_bytes(8, "big")).digest(min(PIECE, remaining))
            out.write(data); digest.update(data); pending.extend(data)
            remaining -= len(data); counter += 1
            while len(pending) >= PIECE:
                # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BEP 3 mandates SHA-1 for torrent piece hashes; this is protocol identity, not a security digest.
                hashes.extend(hashlib.sha1(pending[:PIECE], usedforsecurity=False).digest())
                del pending[:PIECE]
    if pending:
        # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BEP 3 mandates SHA-1 for torrent piece hashes; this is protocol identity, not a security digest.
        hashes.extend(hashlib.sha1(pending, usedforsecurity=False).digest())
    info = {b"length": size, b"name": b"payload.bin", b"piece length": PIECE,
            b"pieces": bytes(hashes), b"private": 1}
    torrent = bencode({b"info": info})
    (folder / "fixture.torrent").write_bytes(torrent)
    # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BEP 3 defines the torrent info-hash as SHA-1.
    info_hash = hashlib.sha1(bencode(info), usedforsecurity=False).digest()
    return payload, torrent, info_hash, digest.hexdigest()


def recv_exact(sock, count, deadline):
    buf = bytearray()
    while len(buf) < count:
        if time.monotonic() >= deadline:
            raise TimeoutError("peer response deadline expired")
        try:
            data = sock.recv(count - len(buf))
        except socket.timeout:
            continue
        if not data: raise RuntimeError("peer closed connection")
        buf.extend(data)
    return bytes(buf)


def send_msg(sock, msg_id, payload=b""):
    sock.sendall(struct.pack(">I", len(payload) + 1) + bytes((msg_id,)) + payload)


def update_available_pieces(body, pieces, available):
    if not body: return available
    if body[0] == 5:
        bits = body[1:]
        if len(bits) == (pieces + 7) // 8:
            available = {piece for piece in range(pieces)
                         if bits[piece // 8] & (0x80 >> (piece % 8))}
    elif body[0] == 4 and len(body) == 5:
        piece = struct.unpack(">I", body[1:])[0]
        if 0 <= piece < pieces: available.add(piece)
    return available


def pacing_deadline(first_payload_time, bytes_received):
    return first_payload_time + bytes_received / RATE


def pacing_delay(deadline, now):
    return max(0.0, deadline - now)


def build_profile_preload(output):
    """Build a temporary preload that permits this probe and its children to ptrace the client."""
    profiler = ctypes.util.find_library("profiler")
    if profiler is None:
        raise RuntimeError("gperftools libprofiler was not found")
    helper = output / "profile-ptrace.so"
    source = r"""
#define _GNU_SOURCE
#include <sys/prctl.h>
#include <linux/prctl.h>
#include <unistd.h>
__attribute__((constructor)) static void allow_probe_debugger(void) {
    (void)prctl(PR_SET_PTRACER, getppid(), 0, 0, 0);
}
"""
    result = subprocess.run(
        ["cc", "-shared", "-fPIC", "-O2", "-x", "c", "-o", str(helper), "-"],
        input=source,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"could not build temporary profile helper: {result.stderr.strip()}")
    return f"{profiler}:{helper}"


def gdb_profile_control(pid, action, profile_path, log_path):
    """Start/stop gperftools in the client, with GDB attaching only at transfer boundaries."""
    if action == "start":
        expression = f"call (int)ProfilerStart({json.dumps(str(profile_path))})"
    elif action == "stop":
        expression = "call (void)ProfilerStop()"
    else:
        raise ValueError(f"unsupported profile action: {action}")
    result = subprocess.run(
        ["gdb", "--quiet", "--batch", "--nx", "-iex", "set debuginfod enabled off",
         "--pid", str(pid), "--ex", "set confirm off", "--ex", expression],
        text=True,
        capture_output=True,
        timeout=30,
        check=False,
    )
    log_path.write_text(result.stdout + result.stderr)
    if result.returncode != 0:
        raise RuntimeError(f"GDB could not {action} transfer-window profile: {result.stdout}{result.stderr}")
    if action == "start" and not re.search(r"\$\d+\s*=\s*1\b", result.stdout):
        raise RuntimeError(f"gperftools did not confirm profile start: {result.stdout}{result.stderr}")


def write_pgo_profile_window_script(script_path, pid_path):
    """Run an instrumented daemon under GDB and reset counters at first upload batch."""
    script = f'''set confirm off
set pagination off
set debuginfod enabled off
handle SIGTERM nostop noprint pass
rbreak rt_engine::torrent_task::start_upload_reads
commands 1
silent
printf "TORRENTNG_PGO_PROFILE_RESET\\n"
call (void)__llvm_profile_reset_counters()
disable 1
continue
end
starti
python
import gdb
open({json.dumps(str(pid_path))}, "w").write(str(gdb.selected_inferior().pid))
end
continue
'''
    script_path.write_text(script)


def leecher(port, ih, size, target, expected, timeout, sample_process,
            diagnostic_callback=None, register_peer=None, peer_state_callback=None,
            cpu_profile_callback=None):
    started, deadline = time.monotonic(), time.monotonic() + timeout
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    with sock:
        sock.settimeout(10)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        sock.bind(("127.0.0.1", 0))
        leecher_address = sock.getsockname()
        if register_peer is not None:
            register_peer(leecher_address)
        sock.connect(("127.0.0.1", port))
        with target.open("w+b") as output:
            connection_started = time.monotonic()
            connection_sample = sample_process()
            first_payload_time = None
            first_payload_sample = None
            payload_complete_sample = None
            bytes_received = 0
            mid_transfer_diagnostics = None
            output.truncate(size)
            sock.settimeout(1)
            client_messages = []
            handshake = b"\x13BitTorrent protocol" + bytes(8) + ih + PEER_ID
            sock.sendall(handshake)
            client_messages.append({"type": "handshake", "bytes_hex": handshake.hex()})
            hello = recv_exact(sock, 68, deadline)
            if hello[:20] != b"\x13BitTorrent protocol" or hello[28:48] != ih:
                raise RuntimeError("peer handshake mismatch")
            blocks = (size + BLOCK - 1) // BLOCK
            pieces = (size + PIECE - 1) // PIECE
            empty_bitfield = bytes((pieces + 7) // 8)
            bitfield_frame = struct.pack(">I", len(empty_bitfield) + 1) + b"\x05" + empty_bitfield
            interested_frame = b"\0\0\0\x01\x02"
            sock.sendall(bitfield_frame + interested_frame)
            client_messages.extend((
                {"type": "bitfield", "bytes_hex": bitfield_frame.hex()},
                {"type": "interested", "bytes_hex": interested_frame.hex()},
            ))
            outstanding, done, available = {}, set(), set()
            next_block, unchoked, full_bitfield = 0, False, False
            server_messages = []
            peer_state_snapshots = []
            while len(done) < blocks:
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"transfer stopped after {len(done)} blocks; unchoked={unchoked}; "
                        f"available_pieces={len(available)}/{pieces}; server_message_ids={server_messages[:32]}")
                if unchoked and full_bitfield:
                    while len(outstanding) < WINDOW and next_block < blocks:
                        absolute = next_block * BLOCK
                        piece, offset = divmod(absolute, PIECE)
                        length = min(BLOCK, size - absolute)
                        send_msg(sock, 6, struct.pack(">III", piece, offset, length))
                        outstanding[(piece, offset)] = (absolute, length)
                        next_block += 1
                try:
                    length = struct.unpack(">I", recv_exact(sock, 4, deadline))[0]
                except TimeoutError as error:
                    if peer_state_callback is not None:
                        peer_state_snapshots.append(peer_state_callback())
                    raise TimeoutError(f"no peer message after {len(done)} blocks; unchoked={unchoked}; "
                        f"available_pieces={len(available)}/{pieces}; server_message_ids={server_messages[:32]}; "
                        f"client_messages={client_messages}; "
                        f"rtorrent_state={json.dumps(peer_state_snapshots, separators=(',', ':'))}") from error
                if length == 0: continue
                if length > BLOCK + 9: raise RuntimeError(f"oversized peer message: {length}")
                body = recv_exact(sock, length, deadline)
                server_messages.append(body[0])
                if peer_state_callback is not None and not peer_state_snapshots:
                    peer_state_snapshots.append(peer_state_callback())
                if body[0] == 1: unchoked = True
                elif body[0] == 0: unchoked = False
                elif body[0] in (4, 5):
                    available = update_available_pieces(body, pieces, available)
                    full_bitfield = len(available) == pieces
                elif body[0] == 7:
                    piece, offset = struct.unpack(">II", body[1:9])
                    item = outstanding.pop((piece, offset), None)
                    if item is None: continue
                    absolute, expected_len = item
                    data = body[9:]
                    if len(data) != expected_len: raise RuntimeError("short piece block")
                    now = time.monotonic()
                    if first_payload_time is None:
                        first_payload_time = now
                        if cpu_profile_callback is not None:
                            cpu_profile_callback("start")
                        first_payload_sample = sample_process()
                    output.seek(absolute); output.write(data)
                    done.add(absolute // BLOCK)
                    bytes_received += len(data)
                    wait_until = pacing_deadline(first_payload_time, bytes_received)
                    delay = pacing_delay(wait_until, time.monotonic())
                    if delay: time.sleep(delay)
                    if (diagnostic_callback is not None and mid_transfer_diagnostics is None
                            and time.monotonic() >= first_payload_time + 8):
                        mid_transfer_diagnostics = diagnostic_callback("after-first-payload-8s")
            payload_complete_time = time.monotonic()
            if cpu_profile_callback is not None:
                cpu_profile_callback("stop")
            payload_complete_sample = sample_process()
    post_leecher_sample = sample_process()
    digest = hashlib.sha256()
    with target.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    actual = digest.hexdigest()
    if actual != expected: raise RuntimeError("download SHA-256 differs from fixture")
    return {"bytes": size, "sha256": actual, "verified": True,
            "elapsed_seconds": time.monotonic() - started,
            "first_payload_delay_seconds": first_payload_time - connection_started,
            "payload_elapsed_seconds": payload_complete_time - first_payload_time,
            "offered_rate_bytes_per_second": RATE,
            "connection_start_sample": connection_sample,
            "leecher_address": list(leecher_address),
            "first_payload_sample": first_payload_sample,
            "payload_complete_sample": payload_complete_sample,
            "post_leecher_sample": post_leecher_sample,
            "initial_client_messages": client_messages,
            "peer_state_snapshots": peer_state_snapshots,
            "mid_transfer_diagnostics": mid_transfer_diagnostics}


def process_sample(root_pid):
    parents = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit(): continue
        try:
            status = (entry / "status").read_text().splitlines()
            parents[int(entry.name)] = int(next(line.split()[1] for line in status
                                                if line.startswith("PPid:")))
        except (OSError, StopIteration, ValueError):
            continue
    if root_pid not in parents:
        raise RuntimeError(f"resolved client proc PID {root_pid} disappeared during sampling")
    pids, todo = set(), [root_pid]
    while todo:
        pid = todo.pop()
        if pid in pids: continue
        pids.add(pid)
        todo.extend(child for child, parent in parents.items() if parent == pid and child not in pids)
    cpu = rss = pss = anon = 0
    for pid in pids:
        try:
            raw = Path(f"/proc/{pid}/stat").read_text(); f = raw[raw.rfind(")") + 2:].split()
            cpu += int(f[11]) + int(f[12])
            status = Path(f"/proc/{pid}/status").read_text().splitlines()
            rss += next(int(x.split()[1]) for x in status if x.startswith("VmRSS:")) * 1024
            lines = Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
            vals = {}
            for line in lines:
                key, separator, value = line.partition(":")
                if separator and key in {"Pss", "Anonymous"}:
                    vals[key] = int(value.split()[0]) * 1024
            pss += vals.get("Pss", 0); anon += vals.get("Anonymous", 0)
        except (OSError, StopIteration, ValueError) as error:
            if pid == root_pid:
                raise RuntimeError(f"cannot read resolved client proc PID {pid}: {error}") from error
    precise_cpu_ns = process_tree_cpu_time_ns(root_pid, pids)
    return {"time": time.time(), "monotonic": time.monotonic(), "pids": sorted(pids), "cpu_ticks_user_system": cpu,
            "cpu_time_user_system_ns": precise_cpu_ns,
            "rss_bytes": rss, "pss_bytes": pss, "anonymous_bytes": anon}


def process_tree_cpu_time_ns(root_pid, pids):
    """Read higher-resolution process CPU time, falling back to proc ticks if unavailable."""
    total = 0
    sampled = False
    for pid in pids:
        try:
            clock_id = process_cpu_clock_id(pid)
            if clock_id is None:
                continue
            total += time.clock_gettime_ns(clock_id)
            sampled = True
        except OSError:
            if pid == root_pid:
                return None
    return total if sampled else None


def process_cpu_clock_id(pid):
    """Resolve a PID's CPU clock through Python when exposed, otherwise glibc."""
    clock_getcpuclockid = getattr(time, "clock_getcpuclockid", None)
    if clock_getcpuclockid is not None:
        return clock_getcpuclockid(pid)
    try:
        clock_getcpuclockid = ctypes.CDLL(None).clock_getcpuclockid
    except (AttributeError, OSError):
        return None
    clock_id = ctypes.c_int()
    error = clock_getcpuclockid(pid, ctypes.byref(clock_id))
    if error:
        raise OSError(error, os.strerror(error))
    return clock_id.value


def cpu_seconds_between(start, end, ticks_per_second):
    """Use precise process CPU clocks when available; retain /proc-tick fallback."""
    start_ns = start.get("cpu_time_user_system_ns")
    end_ns = end.get("cpu_time_user_system_ns")
    if start_ns is not None and end_ns is not None and end_ns >= start_ns:
        return (end_ns - start_ns) / 1_000_000_000
    return (end["cpu_ticks_user_system"] - start["cpu_ticks_user_system"]) / ticks_per_second


def collect_diagnostics(api_port, root_pid, directory, label):
    """Write detailed process maps and the selected TorrentNG metrics outside CPU windows."""
    directory.mkdir(parents=True, exist_ok=True)
    result = {"label": label, "captured_unix": time.time(),
              "process": process_sample(root_pid), "smaps": [], "metrics": {}}
    smaps_totals = {}
    for pid in result["process"]["pids"]:
        source = Path(f"/proc/{pid}/smaps")
        try:
            raw = source.read_text(errors="replace")
        except OSError as error:
            result["smaps"].append({"pid": pid, "error": str(error)})
            continue
        destination = directory / f"{label}-pid-{pid}.smaps"
        destination.write_text(raw)
        totals = {key: 0 for key in ("Size", "Rss", "Pss", "Anonymous", "Private_Clean",
                                      "Private_Dirty", "Shared_Clean", "Shared_Dirty", "Swap")}
        for line in raw.splitlines():
            key, separator, value = line.partition(":")
            if separator and key in totals:
                parts = value.split()
                try: totals[key] += int(parts[0]) * (1024 if len(parts) > 1 and parts[1] == "kB" else 1)
                except (IndexError, ValueError): pass
        smaps_totals[pid] = totals
        result["smaps"].append({"pid": pid, "path": str(destination), "totals_bytes": totals})
    result["smaps_totals_bytes"] = {
        key: sum(item[key] for item in smaps_totals.values())
        for key in ("Size", "Rss", "Pss", "Anonymous", "Private_Clean", "Private_Dirty",
                    "Shared_Clean", "Shared_Dirty", "Swap")
    }
    metrics_path = directory / f"{label}.metrics.prom"
    try:
        request = urllib.request.Request(f"http://127.0.0.1:{api_port}/metrics",
            headers={"Authorization": f"Bearer {TOKEN}"})
        # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected -- URL host is a generated loopback API port and the route and scheme are fixed.
        with urllib.request.urlopen(request, timeout=15) as response:
            metrics_text = response.read().decode("utf-8", errors="replace")
        metrics_path.write_text(metrics_text)
        result["metrics"] = parse_selected_metrics(metrics_text)
        result["metrics_path"] = str(metrics_path)
    except (OSError, urllib.error.URLError, UnicodeError) as error:
        result["metrics_error"] = str(error)
    return result


def parse_selected_metrics(metrics_text):
    selected = {}
    selected_backends = set()
    for line in metrics_text.splitlines():
        if not line or line.startswith("#"): continue
        fields = line.split()
        if len(fields) < 2: continue
        metric_name, separator, labels = fields[0].partition("{")
        if metric_name == "torrentng_storage_scheduler_backend_selected":
            try:
                active = float(fields[1]) > 0
            except ValueError:
                active = False
            if active and separator:
                for label in labels.rstrip("}").split(","):
                    key, equals, value = label.partition("=")
                    if key == "backend" and equals:
                        selected_backends.add(value.strip('"'))
        if metric_name in DIAGNOSTIC_METRICS:
            try: selected[metric_name] = float(fields[1])
            except ValueError: selected[metric_name] = fields[1]
    if selected_backends:
        selected["storage_scheduler_selected_backends"] = sorted(selected_backends)
    return selected


def free_port():
    with socket.socket() as sock: sock.bind(("127.0.0.1", 0)); return sock.getsockname()[1]


def host_proc_pid(namespace_pid, executable):
    """Map a child PID from this process namespace to host-visible /proc."""
    self_status = Path("/proc/self/status").read_text()
    host_self = int(os.readlink("/proc/self"))
    namespace = os.readlink("/proc/self/ns/pid")
    expected = os.path.realpath(executable)
    matches = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit(): continue
        pid = int(entry.name)
        try:
            status = Path(f"/proc/{pid}/status").read_text().splitlines()
            parent_pid = int(next(line.split()[1] for line in status if line.startswith("PPid:")))
            if parent_pid != host_self: continue
            nspid = next(line.split()[1:] for line in status if line.startswith("NSpid:"))
            child_ns = os.readlink(f"/proc/{pid}/ns/pid")
            child_executable = os.path.realpath(f"/proc/{pid}/exe")
            command_line = (Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")[0])
            command_binary = os.path.realpath(os.fsdecode(command_line))
        except (OSError, StopIteration):
            continue
        if (nspid and int(nspid[-1]) == namespace_pid and child_ns == namespace
                and child_executable == expected and command_binary == expected):
            matches.append(pid)
    if len(matches) != 1:
        raise RuntimeError(f"expected one host /proc PID for child {namespace_pid} ({expected}); found {matches}")
    return matches[0]


def scgi(endpoint, method, *params):
    # The SCGI peer returns XML-RPC data. Use the shared defused parser so a
    # malformed local service response cannot expand XML entity payloads.
    # nosemgrep: python.lang.security.use-defused-xmlrpc.use-defused-xmlrpc -- this adapter invokes the shared wrapper after its defusedxml monkey patch is active.
    from transfer_rebuild.xmlrpc_safety import xmlrpc_client
    body = xmlrpc_client.dumps(params, methodname=method, allow_none=True).encode()
    headers = scgi_headers(len(body))
    packet = str(len(headers)).encode() + b":" + headers + b"," + body
    family = socket.AF_UNIX if isinstance(endpoint, (str, Path)) else socket.AF_INET
    with socket.socket(family, socket.SOCK_STREAM) as sock:
        sock.settimeout(10)
        sock.connect(str(endpoint) if family == socket.AF_UNIX else endpoint)
        sock.sendall(packet)
        response = bytearray()
        while True:
            data = sock.recv(65536)
            if not data: break
            response.extend(data)
    values, _ = xmlrpc_client.loads(bytes(response).split(b"\r\n\r\n", 1)[-1])
    return values[0] if values else None


def scgi_headers(body_length):
    return (b"CONTENT_LENGTH\0" + str(body_length).encode() + b"\0SCGI\0" + b"1\0"
            + b"REQUEST_METHOD\0POST\0REQUEST_URI\0/RPC2\0CONTENT_TYPE\0text/xml\0")


def add_torrent(api_port, torrent, save_path):
    boundary = "native-seed-probe-boundary"
    parts = [("savepath", save_path.encode(), None), ("paused", b"false", None),
             ("torrents", torrent, "fixture.torrent")]
    body = bytearray()
    for name, value, filename in parts:
        body.extend(f"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"".encode())
        if filename: body.extend(f"; filename=\"{filename}\"\r\nContent-Type: application/x-bittorrent".encode())
        body.extend(b"\r\n\r\n" + value + b"\r\n")
    body.extend(f"--{boundary}--\r\n".encode())
    req = urllib.request.Request(f"http://127.0.0.1:{api_port}/api/qb/v2/torrents/add", data=body,
        headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": f"multipart/form-data; boundary={boundary}"})
    # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected -- URL host is the generated loopback API port; route and scheme are fixed.
    with urllib.request.urlopen(req, timeout=30) as response:
        if response.status >= 300: raise RuntimeError(f"torrent add HTTP {response.status}")


def add_torrent_peer(api_port, info_hash, peer_address):
    body = urllib.parse.urlencode({"hashes": info_hash, "peers": f"{peer_address[0]}:{peer_address[1]}"}).encode()
    request = urllib.request.Request(f"http://127.0.0.1:{api_port}/api/qb/v2/torrents/addPeers", data=body,
        headers={"Authorization": f"Bearer {TOKEN}", "Content-Type": "application/x-www-form-urlencoded"})
    # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected -- URL host is the generated loopback API port and route and scheme are fixed.
    with urllib.request.urlopen(request, timeout=15) as response:
        if response.status >= 300: raise RuntimeError(f"manual peer add HTTP {response.status}")


def api_json(api_port, path):
    request = urllib.request.Request(f"http://127.0.0.1:{api_port}{path}",
        headers={"Authorization": f"Bearer {TOKEN}"})
    # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected -- host and route callers are confined to the probe's local API.
    with urllib.request.urlopen(request, timeout=15) as response:
        return json.loads(response.read())


def torrentng_seed_ready(row):
    if not isinstance(row, dict): return False
    try: complete = float(row.get("progress", 0)) >= 1.0
    except (TypeError, ValueError): return False
    state = str(row.get("state", "")).strip().lower()
    return complete and state in {"uploading", "seeding", "stalledup", "forcedup"}


def rtorrent_seed_ready(row):
    if not isinstance(row, list) or len(row) != 5: return False
    try:
        complete, active, size, received = (int(value) for value in row[1:])
    except (TypeError, ValueError): return False
    return complete == 1 and active == 1 and size > 0 and received >= size


def rtorrent_debug_snapshot(endpoint, info_hash):
    """Read-only SCGI state for explaining an idle/unchoked peer handshake."""
    result = {"captured_unix": time.time(), "info_hash": info_hash}

    def read(method, *params):
        try:
            return scgi(endpoint, method, *params)
        except Exception as error:
            return {"error": f"{type(error).__name__}: {error}"}

    for method in ("d.connection_current", "d.up.max", "d.up.rate", "d.peers_connected",
                   "d.peers_not_connected", "d.group"):
        result[method] = read(method, info_hash)
    group = result.get("d.group")
    if isinstance(group, int):
        for method in ("choke_group.up.max", "choke_group.up.total", "choke_group.up.queued",
                       "choke_group.up.unchoked"):
            result[method] = read(method, group)
    for method in ("throttle.max_uploads", "throttle.max_uploads.global",
                   "throttle.max_uploads.div", "throttle.global_up.max_rate"):
        result[method] = read(method)
    result["peers"] = read("p.multicall", info_hash, "", "p.id=", "p.address=", "p.port=",
                           "p.client_version=", "p.is_incoming=", "p.completed_percent=",
                           "p.up_rate=", "p.down_rate=", "p.is_snubbed=", "p.is_preferred=")
    return result


def rtorrent_config_text(download, session, peer_port, scgi_port):
    return (f"directory.default.set = {download}\n"
            f"session.path.set = {session}\n"
            "network.bind_address.ipv4.set = 127.0.0.1\n"
            f"network.listen.port.range.set = {peer_port}-{peer_port}\n"
            "network.listen.port.random.set = no\n"
            "dht.mode.set = disable\n"
            "protocol.pex.set = no\n"
            f"network.scgi.open_port = 127.0.0.1:{scgi_port}\n")


def wait_seed_ready(kind, proc, api_port, scgi_path, ih, timeout):
    if kind == "torrentngd":
        def ready():
            rows = api_json(api_port, f"/api/qb/v2/torrents/info?hashes={ih.hex()}")
            row = next((item for item in rows if str(item.get("hash", "")).lower() == ih.hex()), None)
            return row if torrentng_seed_ready(row) else None
    else:
        def ready():
            rows = scgi(scgi_path, "d.multicall2", "", "main", "d.hash=", "d.complete=",
                        "d.is_active=", "d.size_bytes=", "d.completed_bytes=") or []
            row = next((item for item in rows if str(item[0]).lower() == ih.hex()), None)
            return row if rtorrent_seed_ready(row) else None
    wait_for(ready, proc, timeout, description=f"{kind} fully checked and seeding")


def wait_for(check, proc, timeout, description):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if proc.poll() is not None: raise RuntimeError(f"client exited ({proc.returncode}); inspect client.log")
        try:
            result = check()
            if result:
                if hasattr(result, "close"): result.close()
                return result
        except (OSError, urllib.error.URLError): pass
        time.sleep(.2)
    raise TimeoutError(f"client not ready: {description}")


def parse_client(value):
    name, spec = value.split("=", 1); kind, binary = spec.split(":", 1)
    if kind not in {"torrentngd", "rtorrent"}: raise argparse.ArgumentTypeError("kind must be torrentngd or rtorrent")
    binary = str(Path(binary).resolve())
    if not Path(binary).is_file() or not os.access(binary, os.X_OK): raise argparse.ArgumentTypeError(f"not executable: {binary}")
    return name, kind, binary


def parse_client_env(value):
    label, separator, assignment = value.partition(":")
    key, equals, contents = assignment.partition("=")
    if not separator or not label or not equals or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
        raise argparse.ArgumentTypeError("expected LABEL:ENV_NAME=VALUE")
    return label, key, contents


def client_env_overrides(entries, labels):
    known_labels, overrides = set(labels), {}
    for label, key, value in entries:
        if label not in known_labels:
            raise ValueError(f"--client-env references unknown client label: {label}")
        per_client = overrides.setdefault(label, {})
        if key in per_client:
            raise ValueError(f"duplicate --client-env for {label}:{key}")
        per_client[key] = value
    return overrides


def apply_client_environment(env, overrides):
    env.update(overrides)
    return env


def is_measurement_control_env(key):
    return key in CONTROL_ENV_EXACT or key.startswith(CONTROL_ENV_PREFIXES)


def inherited_measurement_controls(environment):
    names = sorted(key for key in environment if is_measurement_control_env(key))
    values = {key: environment[key] for key in names if key in SAFE_CONTROL_VALUES}
    return {"names": names, "safe_values": values}


def sanitize_client_environment(base_environment, overrides=None):
    """Strip inherited measurement controls, then apply explicit per-client settings."""
    inherited = inherited_measurement_controls(base_environment)
    child_environment = {key: value for key, value in base_environment.items()
                         if not is_measurement_control_env(key)}
    apply_client_environment(child_environment, overrides or {})
    return child_environment, inherited


def validate_client_count(client_count, repeats, allow_single):
    if client_count < 2 and not (client_count == 1 and allow_single and repeats == 1):
        raise ValueError("need at least two clients, or one client with --allow-single-client and --repeats 1")


def validate_pgo_profile_window(clients, repeats, cpu_profile, env_overrides):
    if len(clients) != 1 or clients[0][1] != "torrentngd" or repeats != 1:
        raise ValueError("--pgo-profile-window requires one TorrentNG client and --repeats 1")
    if cpu_profile:
        raise ValueError("--pgo-profile-window cannot be combined with --cpu-profile")
    if "LLVM_PROFILE_FILE" in env_overrides.get(clients[0][0], {}):
        raise ValueError("--pgo-profile-window controls LLVM_PROFILE_FILE automatically")


def storage_io_queue_depth_line(queue_depth):
    if queue_depth is None:
        return ""
    if not 1 <= queue_depth <= MAX_STORAGE_IO_QUEUE_DEPTH:
        raise ValueError(
            f"storage I/O queue depth must be 1–{MAX_STORAGE_IO_QUEUE_DEPTH}"
        )
    return f"io_queue_depth = {queue_depth}\n"


def cleanup_run_payloads(run_dir, client_name):
    """Remove generated payload copies, refusing paths that escape this run directory."""
    root = Path(run_dir).resolve(strict=True)
    candidates = (root / "downloaded.bin", root / client_name / "download" / "payload.bin")
    removed = []
    for path in candidates:
        resolved = path.resolve(strict=False)
        if not resolved.is_relative_to(root):
            raise ValueError(f"refusing to clean a payload outside run directory: {path}")
        if path.is_symlink():
            raise ValueError(f"refusing to clean a symlink payload path: {path}")
        if path.is_file():
            path.unlink()
            removed.append(path.relative_to(root).as_posix())
    return removed


def start_client(client, run_dir, fixture_dir, torrent, ih, ready_timeout, env_overrides=None,
                 profile_preload=None, profile_path=None, pgo_profile_path=None,
                 storage_io_queue_depth=None):
    name, kind, binary = client
    work = run_dir / name; work.mkdir()
    download = work / "download"; download.mkdir()
    shutil.copyfile(fixture_dir / "payload.bin", download / "payload.bin")
    peer_port, api_port = free_port(), free_port()
    env, inherited_controls = sanitize_client_environment(os.environ, env_overrides)
    if pgo_profile_path is not None:
        if kind != "torrentngd":
            raise ValueError("transfer-window LLVM profiling requires a TorrentNG client")
        if "LLVM_PROFILE_FILE" in (env_overrides or {}):
            raise ValueError("--pgo-profile-window controls LLVM_PROFILE_FILE automatically")
        env["LLVM_PROFILE_FILE"] = str(pgo_profile_path)
    if profile_preload is not None and kind == "torrentngd":
        if "LD_PRELOAD" in (env_overrides or {}):
            raise ValueError("--cpu-profile cannot be combined with an explicit LD_PRELOAD override")
        env["LD_PRELOAD"] = profile_preload
        env["CPUPROFILE_FREQUENCY"] = str(PROFILE_FREQUENCY_HZ)
    config = work / ("config.toml" if kind == "torrentngd" else "rtorrent.rc")
    scgi_endpoint = None
    pgo_profile_pid_path = None
    if kind == "torrentngd":
        token_file = work / "api-token"; token_file.write_text(TOKEN + "\n"); token_file.chmod(0o600)
        storage_queue_depth = storage_io_queue_depth_line(storage_io_queue_depth)
        config.write_text(f'''[daemon]\napi_bind = "127.0.0.1:{api_port}"\nsession_dir = "{work}/session"\nlog_level = "warn"\n\n[network]\nlisten_port = {peer_port}\nmax_peers = 8\nmax_incoming_handshakes = 32\n\n[storage]\nio_worker_threads = {1 if name == "one_worker" else 2}\ndownload_dir = "{download}"\n{storage_queue_depth}\n[dht]\nenabled = false\n\n[auth]\napi_tokens_file = "{token_file}"\n''')
        env["TORRENTNGD_CONFIG"] = str(config)
        if pgo_profile_path is None:
            command = [binary]
        else:
            pgo_profile_pid_path = work / "torrentngd.pid"
            pgo_profile_script = work / "profile-window.gdb"
            write_pgo_profile_window_script(pgo_profile_script, pgo_profile_pid_path)
            command = [shutil.which("gdb"), "--quiet", "--batch", "--nx",
                       "--command", str(pgo_profile_script), "--args", binary]
    else:
        scgi_port = free_port()
        scgi_endpoint = ("127.0.0.1", scgi_port)
        (work / "session").mkdir()
        config.write_text(rtorrent_config_text(download, work / "session", peer_port, scgi_port))
        command = [binary, "-n", "-o", f"import={config}", "-o", "system.daemon.set=true"]
    logfile = (work / "client.log").open("wb")
    proc = subprocess.Popen(command, cwd=work, env=env, stdout=logfile, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        diagnostics_dir = work / "diagnostics"
        pre_add_diagnostics = None
        post_seed_diagnostics = None
        if kind == "torrentngd":
            wait_for(lambda: socket.create_connection(("127.0.0.1", api_port), .2), proc, 60, "TorrentNG API")
            if pgo_profile_pid_path is None:
                proc_pid = host_proc_pid(proc.pid, binary)
            else:
                pid_text = wait_for(
                    lambda: pgo_profile_pid_path.read_text().strip()
                    if pgo_profile_pid_path.is_file() else None,
                    proc,
                    30,
                    "GDB-launched TorrentNG PID",
                )
                proc_pid = int(pid_text)
            pre_add_diagnostics = collect_diagnostics(api_port, proc_pid, diagnostics_dir, "before-torrent-add")
            add_torrent(api_port, torrent, str(download))
            wait_seed_ready(kind, proc, api_port, None, ih, ready_timeout)
            post_seed_diagnostics = collect_diagnostics(api_port, proc_pid, diagnostics_dir, "after-seed-recheck")
        else:
            wait_for(lambda: socket.create_connection(scgi_endpoint, .2), proc, 60, "rTorrent SCGI")
            proc_pid = host_proc_pid(proc.pid, binary)
            scgi(scgi_endpoint, "load.normal", "", str(fixture_dir / "fixture.torrent"))
            scgi(scgi_endpoint, "d.directory.set", ih.hex(), str(download))
            scgi(scgi_endpoint, "d.start", ih.hex())
            wait_seed_ready(kind, proc, None, scgi_endpoint, ih, ready_timeout)
        return {"process": proc, "log": logfile, "peer_port": peer_port, "api_port": api_port,
            "config": str(config), "command": command, "binary": binary,
            "scgi_endpoint": scgi_endpoint,
            "proc_pid": proc_pid,
            "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
            "cpu_profile_path": str(profile_path) if profile_path is not None else None,
            "pgo_profile_pid_path": str(pgo_profile_pid_path) if pgo_profile_pid_path is not None else None,
            "environment_overrides": dict(env_overrides or {}),
            "inherited_measurement_controls": inherited_controls,
            "diagnostics_dir": str(diagnostics_dir), "pre_add_diagnostics": pre_add_diagnostics,
            "post_seed_diagnostics": post_seed_diagnostics}
    except BaseException:
        proc.terminate(); proc.wait(timeout=10); logfile.close(); raise


def parse_args():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--client", action="append", type=parse_client, required=True, metavar="NAME=KIND:BIN",
        help="repeat for each pinned baseline/candidate, e.g. rt=rtorrent:/path/rtorrent")
    p.add_argument("--client-env", action="append", type=parse_client_env, default=[], metavar="LABEL:KEY=VALUE",
        help="set an environment variable for one named client, e.g. verify-oneworker:TOKIO_WORKER_THREADS=1")
    p.add_argument("--allow-single-client", action="store_true",
        help="allow one unpaired client for a recovery run; requires --repeats 1")
    p.add_argument("--keep-payloads", action="store_true",
        help="retain per-run seed and downloaded payload copies after a successful verified transfer")
    p.add_argument("--size", type=int, default=128 * 1024**2, help="payload bytes, 16 MiB to 4 GiB")
    p.add_argument("--seed", type=int, default=20261022)
    p.add_argument("--storage-io-queue-depth", type=int,
        help="set [storage].io_queue_depth on TorrentNG clients for backend sizing screens")
    p.add_argument("--output", help="new result directory; default is a fresh temporary directory")
    p.add_argument("--warmup", type=int, default=5); p.add_argument("--cooldown", type=int, default=5)
    p.add_argument("--timeout", type=int, default=900); p.add_argument("--order-seed", type=int, default=1)
    p.add_argument("--repeats", type=int, default=1, help="paired randomized rounds")
    p.add_argument("--ready-timeout", type=int, default=600, help="seed verification timeout seconds")
    p.add_argument("--diagnostics", action="store_true",
        help="capture an extra TorrentNG /metrics and smaps snapshot 8 seconds into transfer; marks diagnostic runs")
    p.add_argument("--cpu-profile", action="store_true",
        help="opt-in gperftools stack profile for one TorrentNG run, bounded to first payload through completion")
    p.add_argument("--pgo-profile-window", action="store_true",
        help="reset LLVM PGO counters at the first upload batch for one instrumented TorrentNG run")
    p.add_argument("--peer-debug", action="store_true",
        help="on rTorrent runs, capture read-only SCGI/choke/peer state after handshake and at timeout")
    return p.parse_args()


def write_summary(output, report):
    path = output / "summary.json"
    temporary = output / "summary.json.tmp"
    temporary.write_text(json.dumps(report, indent=2) + "\n")
    temporary.replace(path)


def main():
    args = parse_args()
    if not Path("/proc/self/stat").exists(): raise SystemExit("requires Linux /proc")
    if not 16 * 1024**2 <= args.size <= 4 * 1024**3: raise SystemExit("--size must be 16 MiB–4 GiB")
    try:
        storage_io_queue_depth_line(args.storage_io_queue_depth)
    except ValueError as error:
        raise SystemExit(str(error)) from error
    try:
        validate_client_count(len(args.client), args.repeats, args.allow_single_client)
    except ValueError as error:
        raise SystemExit(str(error)) from error
    if args.output:
        output = Path(args.output).resolve()
        output.mkdir(parents=True, exist_ok=False)
    else:
        output = Path(tempfile.mkdtemp(prefix="native-seed-cpu-ram-"))
    if args.repeats < 1 or args.warmup < 0 or args.cooldown < 0:
        raise SystemExit("--repeats must be positive; warmup/cooldown must be nonnegative")
    if args.ready_timeout < 1 or args.timeout < 1:
        raise SystemExit("timeouts must be positive")
    if len({client[0] for client in args.client}) != len(args.client):
        raise SystemExit("--client labels must be unique")
    try:
        env_overrides = client_env_overrides(args.client_env, [client[0] for client in args.client])
    except ValueError as error:
        raise SystemExit(str(error)) from error
    if not 0 <= args.seed < 2**64:
        raise SystemExit("--seed must be an unsigned 64-bit integer")
    fixture_dir = output / "fixture"
    _, torrent, ih, digest = make_fixture(fixture_dir, args.size, args.seed)
    rng, runs, round_orders = random.Random(args.order_seed), [], []
    if args.cpu_profile:
        if len(args.client) != 1 or args.client[0][1] != "torrentngd" or args.repeats != 1:
            raise SystemExit("--cpu-profile requires exactly one TorrentNG client and --repeats 1")
        if shutil.which("gdb") is None or shutil.which("cc") is None:
            raise SystemExit("--cpu-profile requires gdb and cc")
        try:
            profile_preload = build_profile_preload(output)
        except (OSError, RuntimeError) as error:
            raise SystemExit(str(error)) from error
    else:
        profile_preload = None
    if args.pgo_profile_window:
        try:
            validate_pgo_profile_window(args.client, args.repeats, args.cpu_profile, env_overrides)
        except ValueError as error:
            raise SystemExit(str(error)) from error
        if shutil.which("gdb") is None:
            raise SystemExit("--pgo-profile-window requires gdb")
    report = {"format": "native-seed-cpu-ram-v1", "status": "running",
        "cpu_measurement": "process CPU clock nanoseconds where available; /proc user+system ticks fallback",
        "diagnostic_run": bool(args.diagnostics or args.peer_debug or args.cpu_profile or args.pgo_profile_window), "peer_debug": bool(args.peer_debug),
        "cpu_profile": ({"method": "gperftools, started/stopped by GDB at payload boundaries",
                         "sampling_window": "first valid Piece block through final block completion",
                         "frequency_hz": PROFILE_FREQUENCY_HZ,
                         "client": args.client[0][0]} if args.cpu_profile else None),
        "pgo_profile_window": ({"method": "LLVM_PROFILE_FILE with GDB counter reset",
                                 "sampling_window": "first scheduled upload batch through daemon shutdown",
                                 "client": args.client[0][0]} if args.pgo_profile_window else None),
        "fixture_bytes": args.size, "fixture_seed": args.seed,
        "storage_io_queue_depth": args.storage_io_queue_depth,
        "fixture_sha256": digest, "info_hash": ih.hex(), "offered_rate_bytes_per_second": RATE,
        "block_bytes": BLOCK, "request_window": WINDOW, "dht": False, "pex": False,
        "tracker": False, "bind_address": "127.0.0.1", "paired": len(args.client) >= 2,
        "payload_cleanup_policy": "remove per-run seed and downloaded payload copies after a verified transfer and client shutdown",
        "keep_payloads": bool(args.keep_payloads), "randomized_orders": round_orders, "runs": runs}
    report["client_environment_overrides"] = env_overrides
    report["inherited_measurement_controls"] = inherited_measurement_controls(os.environ)
    write_summary(output, report)
    try:
      for round_index in range(args.repeats):
        order = list(range(len(args.client))); rng.shuffle(order)
        round_orders.append([args.client[i][0] for i in order])
        write_summary(output, report)
        for index in order:
            client = args.client[index]; run_dir = output / f"run-{round_index + 1:02d}-{client[0]}"; run_dir.mkdir()
            active, stop = [], threading.Event()
            profile_path = run_dir / "payload-window.prof" if args.cpu_profile else None
            pgo_profile_path = run_dir / "transfer-window.profraw" if args.pgo_profile_window else None
            item = start_client(client, run_dir, fixture_dir, torrent, ih, args.ready_timeout,
                                env_overrides.get(client[0], {}), profile_preload, profile_path,
                                pgo_profile_path, args.storage_io_queue_depth)
            successful_run, run_record = False, None
            def sample_loop():
                while not stop.is_set(): active.append(process_sample(item["proc_pid"])); stop.wait(1)
            try:
                idle_before = []
                for _ in range(args.warmup):
                    idle_before.append(process_sample(item["proc_pid"])); time.sleep(1)
                sampler = threading.Thread(target=sample_loop, daemon=True); sampler.start()
                pre_leecher_sample = process_sample(item["proc_pid"])
                during_transfer_capture = None
                peer_registration = None
                if args.diagnostics and client[1] == "torrentngd":
                    during_transfer_capture = lambda label: collect_diagnostics(
                        item["api_port"], item["proc_pid"], Path(item["diagnostics_dir"]), label)
                if client[1] == "torrentngd":
                    peer_registration = lambda address: add_torrent_peer(item["api_port"], ih.hex(), address)
                profile_control = None
                if profile_path is not None:
                    profile_control = lambda action: gdb_profile_control(
                        item["proc_pid"], action, profile_path,
                        run_dir / f"profile-{action}.gdb.log")
                peer_state_callback = None
                if args.peer_debug and client[1] == "rtorrent":
                    item["peer_state_snapshots"] = []
                    peer_state_path = run_dir / "rtorrent-peer-debug.json"
                    def capture_rtorrent_state():
                        snapshot = rtorrent_debug_snapshot(item["scgi_endpoint"], ih.hex())
                        item["peer_state_snapshots"].append(snapshot)
                        peer_state_path.write_text(json.dumps(item["peer_state_snapshots"], indent=2) + "\n")
                        return snapshot
                    peer_state_callback = capture_rtorrent_state
                transfer = leecher(item["peer_port"], ih, args.size, run_dir / "downloaded.bin", digest,
                                   args.timeout, lambda: process_sample(item["proc_pid"]),
                                   during_transfer_capture, peer_registration, peer_state_callback,
                                   profile_control)
                stop.set(); sampler.join()
                post_transfer_diagnostics = None
                if client[1] == "torrentngd":
                    post_transfer_diagnostics = collect_diagnostics(
                        item["api_port"], item["proc_pid"], Path(item["diagnostics_dir"]), "after-transfer")
                idle_after = []
                for _ in range(args.cooldown): idle_after.append(process_sample(item["proc_pid"])); time.sleep(1)
                hz = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
                connection = transfer["connection_start_sample"]
                payload_start = transfer["first_payload_sample"]
                payload_end = transfer["payload_complete_sample"]
                post_leecher_sample = transfer["post_leecher_sample"]
                resource_samples = active + [pre_leecher_sample, connection, payload_start,
                                             payload_end, post_leecher_sample]
                active_cpu_seconds = cpu_seconds_between(payload_start, payload_end, hz)
                connection_wait_cpu_seconds = cpu_seconds_between(connection, payload_start, hz)
                connection_to_payload_cpu_seconds = cpu_seconds_between(connection, payload_end, hz)
                connection_setup_cpu_seconds = cpu_seconds_between(pre_leecher_sample, connection, hz)
                def peak(samples, field): return max((x[field] for x in samples), default=0)
                run_record = {"round": round_index + 1, "name": client[0], "kind": client[1], "binary": item["binary"],
                    "binary_sha256": item["binary_sha256"], "command": item["command"], "config_path": item["config"],
                    "environment_overrides": item["environment_overrides"],
                    "inherited_measurement_controls": item["inherited_measurement_controls"],
                    "namespace_pid": item["process"].pid, "proc_pid": item["proc_pid"],
                    "transfer": transfer, "pre_leecher_sample": pre_leecher_sample,
                    "cpu_tick_hz": hz,
                    "payload_cpu_seconds_user_system": active_cpu_seconds,
                    "connection_to_payload_cpu_seconds_user_system": connection_to_payload_cpu_seconds,
                    "connection_wait_cpu_seconds": connection_wait_cpu_seconds,
                    "connection_setup_cpu_seconds": connection_setup_cpu_seconds,
                    "active_peak_rss_bytes": peak(resource_samples, "rss_bytes"),
                    "active_peak_pss_bytes": peak(resource_samples, "pss_bytes"),
                    "active_peak_anonymous_bytes": peak(resource_samples, "anonymous_bytes"),
                    "idle_before": idle_before, "active_samples": resource_samples,
                    "idle_after": idle_after,
                    "diagnostic_run": bool(args.diagnostics or args.peer_debug or args.cpu_profile or args.pgo_profile_window),
                    "torrentng_diagnostics": ({
                        "before_torrent_add": item["pre_add_diagnostics"],
                        "after_seed_recheck": item["post_seed_diagnostics"],
                        "during_transfer": transfer["mid_transfer_diagnostics"],
                        "after_transfer": post_transfer_diagnostics,
                    } if client[1] == "torrentngd" else None),
                    "payload_cleanup": ({"policy": "keep_payloads", "status": "retained-by-option"}
                                        if args.keep_payloads else
                                        {"policy": "after-verified-transfer-and-client-shutdown", "status": "pending"})}
                if profile_path is not None:
                    run_record["cpu_profile"] = {"path": profile_path.relative_to(output).as_posix(),
                                                  "bytes": profile_path.stat().st_size}
                runs.append(run_record)
                report["runs"] = runs
                write_summary(output, report)
                successful_run = bool(transfer["verified"])
            finally:
                stop.set()
                try:
                    if pgo_profile_path is not None:
                        # GDB owns the child process; signal the daemon alone so
                        # the debugger can reap it and the profile runtime can flush.
                        os.kill(item["proc_pid"], signal.SIGTERM)
                    else:
                        os.killpg(item["process"].pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try: item["process"].wait(timeout=15)
                except subprocess.TimeoutExpired: os.killpg(item["process"].pid, signal.SIGKILL); item["process"].wait()
                item["log"].close()
                if pgo_profile_path is not None and run_record is not None:
                    log_text = (run_dir / client[0] / "client.log").read_text(errors="replace")
                    if successful_run and "TORRENTNG_PGO_PROFILE_RESET" not in log_text:
                        raise RuntimeError("GDB did not reach the first upload batch to reset LLVM profile counters")
                    if pgo_profile_path.is_file() and pgo_profile_path.stat().st_size > 0:
                        run_record["pgo_profile"] = {
                            "path": pgo_profile_path.relative_to(output).as_posix(),
                            "bytes": pgo_profile_path.stat().st_size,
                        }
                    elif successful_run:
                        raise RuntimeError("instrumented TorrentNG did not write its LLVM PGO profile")
                    write_summary(output, report)
                if successful_run and not args.keep_payloads:
                    run_record["payload_cleanup"] = {
                        "policy": "after-verified-transfer-and-client-shutdown", "status": "deleted",
                        "paths": cleanup_run_payloads(run_dir, client[0])}
                    write_summary(output, report)
    except BaseException as error:
        report["status"] = "failed"
        report["error"] = f"{type(error).__name__}: {error}"
        report["runs"] = runs
        write_summary(output, report)
        raise
    report["status"] = "complete"
    write_summary(output, report)
    print(output / "summary.json")


if __name__ == "__main__": main()
