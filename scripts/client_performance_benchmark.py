#!/usr/bin/env python3
"""Compare TorrentNG and four reference clients with the same Linux ISO."""

from __future__ import annotations

import base64
import csv
import datetime as dt
import hashlib
import http.cookiejar
import json
import math
import os
import platform
import random
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import xmlrpc.client
from pathlib import Path

from transfer_rebuild.observe import sample_process_tree

ROOT = Path(__file__).resolve().parents[1]
COMPOSE_FILE = ROOT / "deploy/interop/compose.yml"
ALL_CLIENTS = ("torrentngd", "qbittorrent", "transmission", "deluge", "rtorrent")
CLIENTS = tuple(
    item.strip() for item in os.environ.get("TNG_CLIENT_BENCH_CLIENTS", ",".join(ALL_CLIENTS)).split(",")
    if item.strip()
)
if not CLIENTS or set(CLIENTS) - set(ALL_CLIENTS):
    raise SystemExit("TNG_CLIENT_BENCH_CLIENTS must list one or more supported client IDs")
if len(CLIENTS) != len(set(CLIENTS)):
    raise SystemExit("TNG_CLIENT_BENCH_CLIENTS must not repeat client IDs")
LABELS = {
    "torrentngd": "TorrentNG",
    "qbittorrent": "qBittorrent",
    "transmission": "Transmission",
    "deluge": "Deluge",
    "rtorrent": "rTorrent",
}
TOKEN = "interop-backend-token-20260904"
RATE_BPS = int(os.environ.get("TNG_CLIENT_BENCH_RATE_BPS", 32 * 1024 * 1024))
SEED_LEECHER_RATE_BPS = int(os.environ.get(
    "TNG_CLIENT_BENCH_SEED_LEECHER_RATE_BPS", 4 * 1024 * 1024,
))
MAX_PEERS = int(os.environ.get("TNG_CLIENT_BENCH_MAX_PEERS", "32"))
DOWNLOAD_TIMEOUT = int(os.environ.get("TNG_CLIENT_BENCH_TIMEOUT_SECS", "1800"))
SEED_TIMEOUT = int(os.environ.get("TNG_CLIENT_BENCH_SEED_TIMEOUT_SECS", "900"))
SEED_NO_PEER_TIMEOUT = int(os.environ.get("TNG_CLIENT_BENCH_NO_PEER_TIMEOUT_SECS", "90"))
INTERVAL = float(os.environ.get("TNG_CLIENT_BENCH_SAMPLE_SECS", "2"))
ALLOCATOR_METRIC_INTERVAL = float(os.environ.get("TNG_CLIENT_BENCH_ALLOCATOR_SAMPLE_SECS", "5"))
IDLE_SAMPLE_SECONDS = float(os.environ.get("TNG_CLIENT_BENCH_IDLE_SAMPLE_SECS", "8"))
IDLE_PROFILE_WARMUP_SECONDS = float(
    os.environ.get("TNG_CLIENT_BENCH_IDLE_WARMUP_SECS", "30")
)
IDLE_PROFILE_MEASURE_SECONDS = float(
    os.environ.get("TNG_CLIENT_BENCH_IDLE_MEASURE_SECS", "60")
)
IDLE_PROCESS_SAMPLE_SECONDS = float(
    os.environ.get("TNG_CLIENT_BENCH_IDLE_PROCESS_SAMPLE_SECS", "15")
)
IDLE_PROFILES = {
    "isolated": {
        "label": "network_isolated_idle",
        "dht_enabled": False,
        "network_mode": "isolated",
    },
    "dht_off": {
        "label": "dht_disabled_idle",
        "dht_enabled": False,
        "network_mode": "attached",
    },
    "dht_on": {
        "label": "dht_enabled_idle",
        "dht_enabled": True,
        "network_mode": "attached",
    },
}
TORRENT_INDEX = "https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/"
SHA256SUMS_URL = "https://cdimage.debian.org/debian-cd/current/amd64/iso-cd/SHA256SUMS"


def run_command(args, *, env=None, check=True, quiet=False, timeout=None):
    result = subprocess.run(
        args, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, timeout=timeout,
    )
    if check and result.returncode:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(map(str, args))}\n"
            f"{result.stdout[-6000:]}"
        )
    if not quiet and result.stdout:
        print(result.stdout.rstrip(), flush=True)
    return result.stdout, result.returncode


def available_port(used=()):
    reserved = set(used)
    for _ in range(50):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as tcp:
            tcp.bind(("127.0.0.1", 0))
            port = tcp.getsockname()[1]
        if port in reserved:
            continue
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
                udp.bind(("127.0.0.1", port))
        except OSError:
            continue
        return port
    raise RuntimeError("could not allocate an unused TCP/UDP port")


def optional_positive_int(name):
    value = os.environ.get(name, "").strip()
    if not value:
        return None
    try:
        parsed = int(value)
    except ValueError as error:
        raise RuntimeError(f"{name} must be a positive integer") from error
    if parsed < 1:
        raise RuntimeError(f"{name} must be a positive integer")
    return parsed


def optional_nonnegative_int(name):
    value = os.environ.get(name, "").strip()
    if not value:
        return None
    try:
        parsed = int(value)
    except ValueError as error:
        raise RuntimeError(f"{name} must be a non-negative integer") from error
    if parsed < 0:
        raise RuntimeError(f"{name} must be a non-negative integer")
    return parsed


def env_enabled(name):
    return os.environ.get(name, "").strip().lower() in ("1", "true", "yes", "on")


def to_bytes(value):
    match = re.fullmatch(r"\s*([\d.]+)\s*([KMGT]?i?B)?\s*", value, re.I)
    if not match:
        return 0
    number = float(match.group(1))
    unit = (match.group(2) or "B").lower()
    power = {"b": 0, "kb": 1, "kib": 1, "mb": 2, "mib": 2,
             "gb": 3, "gib": 3, "tb": 4, "tib": 4}.get(unit, 0)
    return int(number * (1024 ** power))


def parse_cgroup_io_stat(text):
    """Sum cgroup v2 io.stat counters across the devices in the cgroup."""
    counters = {"read_bytes": 0, "write_bytes": 0, "read_ops": 0, "write_ops": 0}
    names = {"rbytes": "read_bytes", "wbytes": "write_bytes",
             "rios": "read_ops", "wios": "write_ops"}
    for line in text.splitlines():
        fields = line.split()
        for field in fields[1:]:
            key, separator, value = field.partition("=")
            if not separator or key not in names:
                continue
            try:
                counters[names[key]] += int(value)
            except ValueError:
                continue
    return counters


def evict_file_cache(path):
    """Best-effort eviction of clean fixture pages before a client reads them."""
    fadvise = getattr(os, "posix_fadvise", None)
    advice = getattr(os, "POSIX_FADV_DONTNEED", None)
    if fadvise is None or advice is None:
        return False
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
        fadvise(descriptor, 0, 0, advice)
    except OSError:
        return False
    finally:
        os.close(descriptor)
    return True


def parse_cgroup_cpu_usage_usec(text):
    for line in text.splitlines():
        key, separator, value = line.partition(" ")
        if key == "usage_usec" and separator:
            try:
                return int(value.strip())
            except ValueError:
                return None
    return None


def parse_proc_net_dev(text):
    """Sum RX/TX bytes on non-loopback interfaces in one network namespace."""
    received = 0
    transmitted = 0
    found = False
    for line in text.splitlines()[2:]:
        interface, separator, counters = line.partition(":")
        if not separator or interface.strip() == "lo":
            continue
        values = counters.split()
        if len(values) < 9:
            continue
        try:
            rx_bytes, tx_bytes = int(values[0]), int(values[8])
        except ValueError:
            continue
        if rx_bytes < 0 or tx_bytes < 0:
            continue
        received += rx_bytes
        transmitted += tx_bytes
        found = True
    return (received, transmitted) if found else (None, None)


def per_gib(value, payload_bytes, unit_divisor=1):
    if value is None or payload_bytes <= 0:
        return None
    return value * (1024 ** 3) / payload_bytes / unit_divisor


def bencode(value):
    if isinstance(value, int):
        return b"i" + str(value).encode("ascii") + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode("ascii") + b":" + value
    if isinstance(value, str):
        return bencode(value.encode("utf-8"))
    if isinstance(value, (list, tuple)):
        return b"l" + b"".join(bencode(item) for item in value) + b"e"
    if isinstance(value, dict):
        pairs = sorted(value.items(), key=lambda item: item[0])
        return b"d" + b"".join(bencode(key) + bencode(item) for key, item in pairs) + b"e"
    raise TypeError(f"unsupported bencode value: {type(value).__name__}")


def make_private_torrent(path, announce, output):
    piece_size = 1024 * 1024
    piece_hashes = []
    with path.open("rb") as handle:
        while data := handle.read(piece_size):
            piece_hashes.append(hashlib.sha1(data).digest())
    info = {
        b"length": path.stat().st_size,
        b"name": path.name.encode("utf-8"),
        b"piece length": piece_size,
        b"pieces": b"".join(piece_hashes),
        b"private": 1,
    }
    output.write_bytes(bencode({
        b"announce": announce.encode("ascii"),
        b"created by": b"TorrentNG client performance benchmark",
        b"creation date": int(time.time()),
        b"info": info,
    }))
    return hashlib.sha1(bencode(info)).hexdigest()


class HttpClient:
    def __init__(self, base_url):
        self.base_url = base_url
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(self.jar))
        self.transmission_sid = ""
        self.transmission_style = "hyphen"

    def request(self, path, *, data=None, headers=None, timeout=15, method=None):
        request = urllib.request.Request(
            self.base_url + path, data=data, headers=headers or {}, method=method,
        )
        return self.opener.open(request, timeout=timeout)


class Benchmark:
    def __init__(self):
        self.mode = os.environ.get("TNG_CLIENT_BENCH_MODE", "transfer").strip().lower()
        if self.mode not in ("transfer", "idle"):
            raise RuntimeError("TNG_CLIENT_BENCH_MODE must be transfer or idle")
        self.skip_build = env_enabled("TNG_CLIENT_BENCH_SKIP_BUILD")
        allocator = os.environ.get("TNG_CLIENT_BENCH_ALLOCATOR", "system").strip().lower()
        allocator_features = {
            "system": "system-allocator",
            "jemalloc": "jemalloc-allocator",
            "mimalloc": "mimalloc-allocator",
        }
        if allocator not in allocator_features:
            raise RuntimeError("TNG_CLIENT_BENCH_ALLOCATOR must be system, jemalloc, or mimalloc")
        self.allocator = allocator
        self.cargo_features = allocator_features[allocator]
        self.mimalloc_purge_delay = optional_nonnegative_int(
            "TNG_CLIENT_BENCH_MIMALLOC_PURGE_DELAY",
        )
        allow_thp = os.environ.get("TNG_CLIENT_BENCH_MIMALLOC_ALLOW_THP")
        if allow_thp is not None and allow_thp.strip().lower() not in (
            "1", "0", "true", "false", "yes", "no", "on", "off",
        ):
            raise RuntimeError(
                "TNG_CLIENT_BENCH_MIMALLOC_ALLOW_THP must be a boolean value"
            )
        self.mimalloc_allow_thp = (
            allow_thp.strip().lower() in ("1", "true", "yes", "on")
            if allow_thp is not None else None
        )
        if self.allocator != "mimalloc" and (
            self.mimalloc_purge_delay is not None or self.mimalloc_allow_thp is not None
        ):
            raise RuntimeError("MIMALLOC benchmark settings require TNG_CLIENT_BENCH_ALLOCATOR=mimalloc")
        self.prebuilt_image = os.environ.get(
            "TNG_CLIENT_BENCH_PREBUILT_IMAGE", "torrentng/torrentngd:interop",
        ).strip()
        self.tokio_worker_threads = optional_positive_int(
            "TNG_CLIENT_BENCH_TOKIO_WORKER_THREADS",
        )
        self.storage_io_worker_threads = optional_positive_int(
            "TNG_CLIENT_BENCH_STORAGE_IO_WORKER_THREADS",
        )
        self.storage_hash_worker_threads = optional_positive_int(
            "TNG_CLIENT_BENCH_STORAGE_HASH_WORKER_THREADS",
        )
        self.storage_peer_read_readahead_bytes = optional_nonnegative_int(
            "TNG_CLIENT_BENCH_STORAGE_PEER_READ_READAHEAD_BYTES",
        )
        self.storage_peer_read_cache_entries = optional_nonnegative_int(
            "TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_ENTRIES",
        )
        self.storage_peer_read_cache_bytes = optional_nonnegative_int(
            "TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_BYTES",
        )
        self.piece_assembly_cap_mb = optional_nonnegative_int(
            "TNG_CLIENT_BENCH_PIECE_ASSEMBLY_CAP_MB",
        )
        self.peer_buffer_cap_mb = optional_nonnegative_int(
            "TNG_CLIENT_BENCH_PEER_BUFFER_CAP_MB",
        )
        for name, value in (
            ("TNG_CLIENT_BENCH_STORAGE_IO_WORKER_THREADS", self.storage_io_worker_threads),
            ("TNG_CLIENT_BENCH_STORAGE_HASH_WORKER_THREADS", self.storage_hash_worker_threads),
        ):
            if value is not None and value > 64:
                raise RuntimeError(f"{name} must be <= 64")
        for name, value in (
            ("TNG_CLIENT_BENCH_PIECE_ASSEMBLY_CAP_MB", self.piece_assembly_cap_mb),
            ("TNG_CLIENT_BENCH_PEER_BUFFER_CAP_MB", self.peer_buffer_cap_mb),
        ):
            if value is not None and value > 512:
                raise RuntimeError(f"{name} must be <= 512")
        if (self.storage_peer_read_readahead_bytes is not None
                and self.storage_peer_read_readahead_bytes > 64 * 1024 * 1024):
            raise RuntimeError(
                "TNG_CLIENT_BENCH_STORAGE_PEER_READ_READAHEAD_BYTES must be <= 67108864"
            )
        if (self.storage_peer_read_cache_entries is not None
                and self.storage_peer_read_cache_entries > 65_536):
            raise RuntimeError(
                "TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_ENTRIES must be <= 65536"
            )
        if (self.storage_peer_read_cache_bytes is not None
                and self.storage_peer_read_cache_bytes > 64 * 1024 * 1024):
            raise RuntimeError(
                "TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_BYTES must be <= 67108864"
            )
        now = dt.datetime.now(dt.timezone.utc)
        self.stamp = now.strftime("%Y%m%dT%H%M%SZ")
        self.started = now.isoformat(timespec="seconds")
        self.report = Path(os.environ.get(
            "TNG_CLIENT_BENCH_REPORT",
            ROOT / f"certification/reports/client-performance-benchmark-{self.stamp}.md",
        )).expanduser().resolve()
        self.csv_path = self.report.with_name(self.report.stem + "-samples.csv")
        self.memory_profile = self.report.with_name(self.report.stem + "-memory-profile.json")
        artifact_dir = os.environ.get("TNG_CLIENT_BENCH_ARTIFACT_DIR")
        self.artifact_dir = Path(artifact_dir).expanduser().resolve() if artifact_dir else None
        self.artifact_status = "not requested"
        if self.report.exists() or self.csv_path.exists() or self.memory_profile.exists():
            raise RuntimeError(f"refusing to overwrite an existing benchmark report: {self.report}")
        scratch = Path(os.environ.get("TNG_CLIENT_BENCH_SCRATCH_ROOT", "/var/tmp")).resolve()
        if not scratch.is_dir() or not os.access(scratch, os.W_OK | os.X_OK):
            raise RuntimeError(f"scratch root is not writable: {scratch}")
        requested = os.environ.get("TNG_CLIENT_BENCH_WORKDIR")
        if requested:
            candidate = Path(requested).expanduser()
            if candidate.is_symlink():
                raise RuntimeError(f"refusing symlink workdir: {candidate}")
            self.workdir = candidate.resolve()
            if self.workdir.exists():
                raise RuntimeError(f"refusing pre-existing workdir: {self.workdir}")
            self.workdir.mkdir(parents=True, exist_ok=True)
        else:
            self.workdir = Path(tempfile.mkdtemp(
                prefix=f"torrentng-client-bench-{self.stamp}-", dir=scratch,
            ))
        config_root_value = os.environ.get("TNG_CLIENT_BENCH_CONFIG_ROOT")
        if config_root_value:
            config_root = Path(config_root_value).expanduser()
            if config_root.is_symlink():
                raise RuntimeError(f"refusing symlink config root: {config_root}")
            config_root = config_root.resolve()
            if not config_root.is_dir() or not os.access(config_root, os.W_OK | os.X_OK):
                raise RuntimeError(f"config root is not a writable directory: {config_root}")
            self.config_dir = config_root / f"torrentng-client-bench-{self.stamp}"
            if self.config_dir.exists() or self.config_dir.is_symlink():
                raise RuntimeError(f"refusing pre-existing config directory: {self.config_dir}")
        else:
            self.config_dir = self.workdir / "config"
        self.project = "tngbench" + re.sub(r"[^a-z0-9]", "", self.stamp.lower())
        self.image = f"torrentng/torrentngd:benchmark-{self.stamp.lower()}"
        malloc_conf_override = os.environ.get("TNG_CLIENT_BENCH_MALLOC_CONF")
        if self.allocator == "jemalloc":
            self.malloc_conf = (
                malloc_conf_override.strip()
                if malloc_conf_override is not None else "thp:never"
            )
        else:
            if malloc_conf_override is not None:
                raise RuntimeError(
                    "TNG_CLIENT_BENCH_MALLOC_CONF requires TNG_CLIENT_BENCH_ALLOCATOR=jemalloc"
                )
            self.malloc_conf = None
        self.compose_override_path = None
        self.env = os.environ.copy()
        self.ports = {}
        used_ports = set()
        self.sessions = {}
        self.results = []
        self.idle_results = []
        self.idle_client_images = {}
        self.idle_order_seed = int(hashlib.sha256(self.stamp.encode()).hexdigest()[:16], 16)
        self.network_isolated_clients = set()
        self.detached_networks = {}
        self.samples = []
        self.container_ids = {}
        self.container_pids = {}
        self.torrentng_pids = {}
        self.cgroup_paths = {}
        self.engine_memory_metrics = []
        self.process_memory_snapshots = []
        self.last_engine_memory_metrics_at = 0.0
        self.partial_download_rows = {}
        self.partial_seed_rows = {}
        self.partial_seed_elapsed = {}
        self.partial_seed_setup = {}
        self.seed_source = None
        self.seed_cache_eviction = {}
        seed_source = os.environ.get("TNG_CLIENT_BENCH_SEED_SOURCE")
        self.seed_source_override = Path(seed_source).expanduser().resolve() if seed_source else None
        self.seed_leecher_version = "unknown"
        self.seed_leecher_image = "not started"
        self.cleanup_status = "pending"
        self.failure = ""
        self._env("INTEROP_WORKDIR", self.workdir)
        self._env("INTEROP_TORRENTNG_IMAGE", self.image)
        self._env("INTEROP_MAX_PEERS", MAX_PEERS)
        self._env("INTEROP_RATE_BPS", RATE_BPS)
        self._env("INTEROP_SOURCE_REVISION", self._git("rev-parse", "HEAD"))
        self._env("INTEROP_CONFIG_DIR", self.config_dir)
        self._env("INTEROP_TORRENTNG_CONFIG", self.config_dir / "torrentngd.config.toml")
        self._env("INTEROP_RTORRENT_CONFIG", self.config_dir / "rtorrent-benchmark.rc")
        self._env("INTEROP_SEED_QBIT_CONFIG", self.config_dir / "qbittorrent-seed")
        self._env("INTEROP_SEED_QBIT_DOWNLOAD_DIR", self.workdir / "leecher")
        self._env("TMPDIR", self.workdir / "tmp")
        self._env("INTEROP_CLIENT_BIND_ADDR", os.environ.get("TNG_CLIENT_BENCH_PEER_BIND", "0.0.0.0"))
        self._env("INTEROP_RTORRENT_BIND_ADDR", os.environ.get("TNG_CLIENT_BENCH_PEER_BIND", "0.0.0.0"))
        self._env("PUID", os.getuid())
        self._env("PGID", os.getgid())
        self._env("TZ", os.environ.get("TZ", "Etc/UTC"))
        keys = (
            ("torrentngd", "INTEROP_RUST_HOST_PORT"),
            ("qbittorrent", "INTEROP_QBIT_HOST_PORT"),
            ("transmission", "INTEROP_TRANSMISSION_HOST_PORT"),
            ("deluge", "INTEROP_DELUGE_HOST_PORT"),
            ("deluge_daemon", "INTEROP_DELUGE_DAEMON_PORT"),
            ("tracker", "INTEROP_TRACKER_HOST_PORT"),
            ("seed_leecher", "INTEROP_SEED_QBIT_HOST_PORT"),
            ("peer_torrentngd", "INTEROP_RUST_PEER_PORT"),
            ("peer_qbittorrent", "INTEROP_QBIT_PEER_PORT"),
            ("peer_transmission", "INTEROP_TRANSMISSION_PEER_PORT"),
            ("peer_deluge", "INTEROP_DELUGE_PEER_PORT"),
            ("peer_rtorrent", "INTEROP_RTORRENT_PEER_PORT"),
            ("peer_seed_qbittorrent", "INTEROP_SEED_QBIT_PEER_PORT"),
        )
        for key, envname in keys:
            self.ports[key] = available_port(used_ports)
            used_ports.add(self.ports[key])
            self._env(envname, self.ports[key])
        self.load_start = os.getloadavg()
        self.cpu_count = os.cpu_count() or 0
        self.mem_total = self._meminfo("MemTotal")
        self.mem_available = self._meminfo("MemAvailable")
        self.scratch_device, self.scratch_type = self._findmnt(self.workdir)
        self.source_commit = self._git("rev-parse", "HEAD")
        self.source_tree = self._git("rev-parse", "HEAD^{tree}")
        (
            self.source_dirty,
            self.source_patch_sha256,
            self.source_untracked_count,
            self.source_untracked_sha256,
            self.source_fingerprint_sha256,
        ) = self._source_fingerprint()
        self._env("INTEROP_SOURCE_FINGERPRINT", self.source_fingerprint_sha256)
        self._env("INTEROP_TORRENTNG_CARGO_FEATURES", self.cargo_features)
        self.host = socket.gethostname()
        self.kernel = platform.release()
        self.machine = platform.machine()
        self.docker_version = run_command(
            ["docker", "version", "--format", "client={{.Client.Version}} server={{.Server.Version}}"],
            quiet=True, check=False,
        )[0].strip()
        self.compose_version = run_command(["docker", "compose", "version"],
                                           quiet=True, check=False)[0].strip()

    def _env(self, key, value):
        self.env[key] = str(value)

    @staticmethod
    def _meminfo(key):
        try:
            for line in Path("/proc/meminfo").read_text().splitlines():
                if line.startswith(key + ":"):
                    return int(line.split()[1]) * 1024
        except OSError:
            pass
        return 0

    @staticmethod
    def _findmnt(path):
        output, code = run_command(
            ["findmnt", "-n", "-T", str(path), "-o", "SOURCE,FSTYPE"],
            check=False, quiet=True,
        )
        if code == 0 and output.strip():
            return tuple(output.strip().split(maxsplit=1))
        return "unknown", "unknown"

    @staticmethod
    def _git(*args):
        return run_command(["git", *args], quiet=True)[0].strip()

    def _source_fingerprint(self):
        status = subprocess.run(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"],
            cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
        ).stdout
        patch = subprocess.run(
            [
                "git", "diff", "--binary", "--full-index", "--no-ext-diff",
                "--no-textconv", "--no-renames", "--no-color",
                "--diff-algorithm=myers", "--no-indent-heuristic", "HEAD",
            ],
            cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
        ).stdout
        untracked = subprocess.run(
            ["git", "ls-files", "--others", "--exclude-standard", "-z"],
            cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
        ).stdout
        untracked_hash = hashlib.sha256()
        untracked_paths = [raw for raw in untracked.split(b"\0") if raw]
        for raw_path in sorted(untracked_paths):
            path = ROOT / os.fsdecode(raw_path)
            untracked_hash.update(len(raw_path).to_bytes(8, "big"))
            untracked_hash.update(raw_path)
            if path.is_symlink():
                target = os.fsencode(os.readlink(path))
                untracked_hash.update(b"symlink\0")
                untracked_hash.update(target)
            elif path.is_file():
                untracked_hash.update(b"file\0")
                with path.open("rb") as handle:
                    for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                        untracked_hash.update(chunk)
            else:
                untracked_hash.update(b"other\0")
        patch_sha256 = hashlib.sha256(patch).hexdigest()
        untracked_sha256 = untracked_hash.hexdigest()
        fingerprint = hashlib.sha256()
        for value in (
            self.source_commit,
            self.source_tree,
            patch_sha256,
            untracked_sha256,
        ):
            fingerprint.update(value.encode("ascii"))
            fingerprint.update(b"\0")
        return (
            bool(status.strip()), patch_sha256, len(untracked_paths),
            untracked_sha256, fingerprint.hexdigest(),
        )

    def compose(self, *args, check=True, quiet=False, timeout=None):
        command = ["docker", "compose", "-p", self.project, "-f", str(COMPOSE_FILE)]
        if self.compose_override_path is not None:
            command.extend(["-f", str(self.compose_override_path)])
        command.extend(args)
        return run_command(
            command,
            env=self.env, check=check, quiet=quiet, timeout=timeout,
        )

    def service_id(self, client):
        return self.compose("ps", "-q", client, quiet=True)[0].strip()

    def api_url(self, client):
        return "http://127.0.0.1:" + str(self.ports[client])

    def http(self, client, path, *, data=None, headers=None, timeout=15, method=None):
        return self.sessions[client].request(
            path, data=data, headers=headers, timeout=timeout, method=method,
        )

    def json(self, client, path, *, data=None, headers=None):
        with self.http(client, path, data=data, headers=headers) as response:
            body = response.read()
        return json.loads(body) if body else None

    def prepare(self):
        for rel in ("artifacts", "downloads", "leecher", "logs", "tmp", "torrents", "watch/rtorrent"):
            (self.workdir / rel).mkdir(parents=True, exist_ok=True)
        self.config_dir.mkdir(parents=True, exist_ok=True)
        self.config_dir.chmod(0o777)
        for client in CLIENTS:
            directory = self.workdir / "downloads" / client
            directory.mkdir(parents=True, exist_ok=True)
            if client != "rtorrent":
                (directory / "public").mkdir(exist_ok=True)
            directory.chmod(0o777)
        (self.workdir / "downloads").chmod(0o777)
        compose_environment = {}
        if self.malloc_conf is not None:
            compose_environment["MALLOC_CONF"] = self.malloc_conf
        if self.mimalloc_purge_delay is not None:
            compose_environment["MIMALLOC_PURGE_DELAY"] = str(self.mimalloc_purge_delay)
        if self.mimalloc_allow_thp is not None:
            compose_environment["MIMALLOC_ALLOW_THP"] = (
                "1" if self.mimalloc_allow_thp else "0"
            )
        if self.tokio_worker_threads is not None:
            compose_environment["TOKIO_WORKER_THREADS"] = str(self.tokio_worker_threads)
        if compose_environment or self.mode == "idle":
            self.compose_override_path = self.workdir / "compose.memory-profile.yml"
            override_lines = ["services:"]
            if compose_environment:
                override_lines.extend(["  torrentngd:", "    environment:"])
                override_lines.extend(
                    f"      {key}: {json.dumps(value)}"
                    for key, value in compose_environment.items()
                )
            if self.mode == "idle":
                for client in CLIENTS:
                    if client == "torrentngd" and compose_environment:
                        override_lines.extend(["    healthcheck:", "      disable: true"])
                    else:
                        override_lines.extend([
                            f"  {client}:", "    healthcheck:", "      disable: true",
                        ])
            self.compose_override_path.write_text("\n".join(override_lines) + "\n")
        self._prepare_qbit_config()
        self._prepare_torrentng_config(
            dht_enabled=False if self.mode == "idle" else None,
        )
        self._prepare_rtorrent_config(
            dht_enabled=False if self.mode == "idle" else None,
        )
        self.csv_path.parent.mkdir(parents=True, exist_ok=True)
        with self.csv_path.open("w", newline="") as handle:
            csv.writer(handle).writerow([
                "utc", "client", "phase", "elapsed_seconds", "progress",
                "download_bytes_per_second", "upload_bytes_per_second",
                "connected_peers", "container_cpu_percent", "container_memory_bytes",
                "host_load_1m", "cgroup_memory_current_bytes",
                "cgroup_memory_peak_bytes", "cgroup_memory_anon_bytes",
                "cgroup_memory_file_bytes", "cgroup_memory_slab_bytes",
                "torrentng_process_rss_bytes", "torrentng_process_hwm_bytes",
                "torrentng_process_rss_anon_bytes", "torrentng_process_rss_file_bytes",
                "torrentng_process_rss_shmem_bytes", "torrentng_process_vm_data_bytes",
                "torrentng_process_vm_stk_bytes", "torrentng_process_vm_pte_bytes",
                "torrentng_process_threads", "cgroup_memory_sock_bytes",
                "cgroup_memory_kernel_stack_bytes", "cgroup_memory_pagetables_bytes",
                "cgroup_cpu_usage_usec", "cgroup_io_read_bytes",
                "cgroup_io_write_bytes", "cgroup_io_read_ops", "cgroup_io_write_ops",
                "network_rx_bytes", "network_tx_bytes",
                "container_process_tree_rss_bytes", "container_process_tree_pss_bytes",
                "container_process_tree_count", "container_process_tree_threads",
            ])

    def _prepare_qbit_config(self):
        path = self.config_dir / "qbittorrent/qBittorrent/qBittorrent.conf"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            "[BitTorrent]\n"
            "Session\\DefaultSavePath=/downloads/qbittorrent\n"
            "Session\\Port=6882\n\n"
            "[LegalNotice]\nAccepted=true\n\n"
            "[Preferences]\n"
            "WebUI\\AuthSubnetWhitelist=0.0.0.0/0,::/0\n"
            "WebUI\\AuthSubnetWhitelistEnabled=true\n"
            "WebUI\\LocalHostAuth=false\n"
            "WebUI\\Port=8080\n"
        )

    def _prepare_seed_qbit_config(self, client):
        path = self.config_dir / "qbittorrent-seed" / client / "qBittorrent/qBittorrent.conf"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            "[BitTorrent]\n"
            "Session\\DefaultSavePath=/downloads\n"
            "Session\\Port=6886\n\n"
            "[LegalNotice]\nAccepted=true\n\n"
            "[Preferences]\n"
            "WebUI\\AuthSubnetWhitelist=0.0.0.0/0,::/0\n"
            "WebUI\\AuthSubnetWhitelistEnabled=true\n"
            "WebUI\\LocalHostAuth=false\n"
            "WebUI\\Port=8080\n"
        )

    def _prepare_torrentng_config(self, *, dht_enabled=None):
        text = (ROOT / "deploy/interop/torrentngd.config.toml").read_text()
        text = re.sub(r"(?m)^max_peers\s*=\s*\d+\s*$", f"max_peers = {MAX_PEERS}", text)
        text = re.sub(r"(?m)^upload_rate_limit\s*=\s*\d+\s*$",
                      f"upload_rate_limit = {RATE_BPS}", text)
        text = re.sub(r"(?m)^download_rate_limit\s*=\s*\d+\s*$",
                      f"download_rate_limit = {RATE_BPS}", text)
        if dht_enabled is not None:
            section = re.search(r"(?ms)^\[dht\]\s*\n(.*?)(?=^\[|\Z)", text)
            if section is None:
                raise RuntimeError("TorrentNG benchmark config has no [dht] section")
            body, count = re.subn(
                r"(?m)^enabled\s*=\s*(?:true|false)\s*$",
                f"enabled = {'true' if dht_enabled else 'false'}",
                section.group(1), count=1,
            )
            if count != 1:
                raise RuntimeError("TorrentNG benchmark DHT setting is not explicit")
            text = text[:section.start(1)] + body + text[section.end(1):]
        storage_overrides = []
        if self.storage_io_worker_threads is not None:
            storage_overrides.append(
                f"io_worker_threads = {self.storage_io_worker_threads}"
            )
        if self.storage_hash_worker_threads is not None:
            storage_overrides.append(
                f"hash_worker_threads = {self.storage_hash_worker_threads}"
            )
        if self.storage_peer_read_readahead_bytes is not None:
            storage_overrides.append(
                "peer_read_readahead_bytes = "
                f"{self.storage_peer_read_readahead_bytes}"
            )
        if self.storage_peer_read_cache_entries is not None:
            storage_overrides.append(
                "peer_read_cache_entries = "
                f"{self.storage_peer_read_cache_entries}"
            )
        if self.storage_peer_read_cache_bytes is not None:
            storage_overrides.append(
                "peer_read_cache_bytes = "
                f"{self.storage_peer_read_cache_bytes}"
            )
        if storage_overrides:
            text = re.sub(
                r'(?m)^(download_dir\s*=\s*"[^"]+"\s*)$',
                lambda match: match.group(1) + "\n" + "\n".join(storage_overrides),
                text,
                count=1,
            )
        memory_overrides = []
        if self.piece_assembly_cap_mb is not None:
            memory_overrides.append(
                f"piece_assembly_cap_mb = {self.piece_assembly_cap_mb}"
            )
        if self.peer_buffer_cap_mb is not None:
            memory_overrides.append(f"peer_buffer_cap_mb = {self.peer_buffer_cap_mb}")
        if memory_overrides:
            section = re.search(r"(?ms)^\[memory\]\s*\n(.*?)(?=^\[|\Z)", text)
            if section:
                body = section.group(1)
                for key, value in (
                    ("piece_assembly_cap_mb", self.piece_assembly_cap_mb),
                    ("peer_buffer_cap_mb", self.peer_buffer_cap_mb),
                ):
                    if value is None:
                        continue
                    setting = re.compile(rf"(?m)^{key}\s*=\s*\d+\s*$")
                    if setting.search(body):
                        body = setting.sub(f"{key} = {value}", body, count=1)
                    else:
                        body = body.rstrip() + f"\n{key} = {value}\n"
                text = text[:section.start(1)] + body + text[section.end(1):]
            else:
                text = text.rstrip() + "\n\n[memory]\n" + "\n".join(memory_overrides) + "\n"
        path = self.config_dir / "torrentngd.config.toml"
        path.write_text(text)
        path.chmod(0o600)

    def _prepare_rtorrent_config(self, *, dht_enabled=None):
        base = (ROOT / "deploy/interop/rtorrent.rc").read_text().rstrip()
        if dht_enabled is not None:
            base, count = re.subn(
                r"(?m)^dht\.mode\.set\s*=\s*\S+\s*$",
                f"dht.mode.set = {'auto' if dht_enabled else 'off'}",
                base, count=1,
            )
            if count != 1:
                raise RuntimeError("rTorrent benchmark config has no explicit DHT mode")
        additions = [
            f"throttle.max_peers.normal.set = {MAX_PEERS}",
            f"throttle.max_peers.seed.set = {MAX_PEERS}",
            f"throttle.global_down.max_rate.set_kb = {RATE_BPS // 1024}",
            f"throttle.global_up.max_rate.set_kb = {RATE_BPS // 1024}",
        ]
        path = self.config_dir / "rtorrent-benchmark.rc"
        path.write_text(base + "\n" + "\n".join(additions) + "\n")
        path.chmod(0o644)
        rtorrent_config = self.config_dir / "rtorrent"
        rtorrent_config.mkdir(parents=True, exist_ok=True)
        (rtorrent_config / "session").mkdir(exist_ok=True)

    def _qbit_json(self, client, path, *, data=None, headers=None):
        request_headers = {"Host": "localhost:8080"}
        request_headers.update(headers or {})
        return self.json(client, path, data=data, headers=request_headers)

    def _transmission(self, method, arguments=None):
        session = self.sessions["transmission"]
        if session.transmission_style == "snake":
            method = method.replace("-", "_")
            arguments = {
                key.replace("-", "_"): [field.replace("-", "_") for field in value]
                if key == "fields" and isinstance(value, list) else value
                for key, value in (arguments or {}).items()
            }
        payload = json.dumps(
            {"method": method, "arguments": arguments or {}},
            separators=(",", ":"),
        ).encode()
        headers = {"Content-Type": "application/json"}
        sid = session.transmission_sid
        if sid:
            headers["X-Transmission-Session-Id"] = sid
        request = urllib.request.Request(
            self.api_url("transmission") + "/transmission/rpc",
            data=payload, headers=headers,
        )
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                return json.loads(response.read())
        except urllib.error.HTTPError as error:
            if error.code != 409:
                raise
            sid = error.headers.get("X-Transmission-Session-Id")
            if not sid:
                raise
            session.transmission_sid = sid
            headers["X-Transmission-Session-Id"] = sid
            request = urllib.request.Request(
                self.api_url("transmission") + "/transmission/rpc",
                data=payload, headers=headers,
            )
            with urllib.request.urlopen(request, timeout=15) as response:
                return json.loads(response.read())

    def _deluge(self, method, params=None):
        response = self.json(
            "deluge", "/json",
            data=json.dumps({"method": method, "params": params or [], "id": 1}).encode(),
            headers={"Content-Type": "application/json"},
        )
        if not isinstance(response, dict) or response.get("error"):
            raise RuntimeError(f"Deluge RPC {method} failed: {response}")
        return response.get("result")

    def _deluge_connect(self):
        self._deluge("auth.login", ["deluge"])
        if self._deluge("web.connected"):
            return
        hosts = self._deluge("web.get_hosts") or []
        host_id = hosts[0][0] if hosts else None
        if not host_id:
            self._deluge("web.add_host", ["127.0.0.1", 58846, "", ""])
            hosts = self._deluge("web.get_hosts") or []
            host_id = hosts[0][0] if hosts else None
        if not host_id:
            raise RuntimeError("Deluge WebUI could not connect to its daemon")
        self._deluge("web.connect", [host_id])
        if not self._deluge("web.connected"):
            raise RuntimeError("Deluge WebUI did not establish its daemon connection")

    def _scgi(self, method, *params):
        path = self.config_dir / "rtorrent/scgi.sock"
        if not path.exists():
            raise RuntimeError(f"rTorrent SCGI socket does not exist: {path}")
        body = xmlrpc.client.dumps(params, methodname=method, allow_none=True).encode()
        headers = (
            b"CONTENT_LENGTH\0" + str(len(body)).encode() + b"\0"
            + b"SCGI\0" + b"1\0"
            + b"REQUEST_METHOD\0POST\0REQUEST_URI\0/RPC2\0CONTENT_TYPE\0text/xml\0"
        )
        payload = str(len(headers)).encode() + b":" + headers + b"," + body
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(10)
            sock.connect(str(path))
            sock.sendall(payload)
            chunks = []
            while True:
                part = sock.recv(65536)
                if not part:
                    break
                chunks.append(part)
        response = b"".join(chunks).split(b"\r\n\r\n", 1)[-1]
        values, _ = xmlrpc.client.loads(response)
        return values[0] if values else None

    def _wait_ready(self, client):
        deadline = time.monotonic() + 240
        while time.monotonic() < deadline:
            try:
                if client == "torrentngd":
                    data = json.loads(self._torrentng_request("/health"))
                    if data.get("ready") is True:
                        return
                elif client == "qbittorrent":
                    with self.http(
                        client, "/api/v2/app/version",
                        headers={"Host": "localhost:8080"},
                    ) as response:
                        if response.read().decode("utf-8", "replace").strip():
                            return
                    raise RuntimeError("qBittorrent returned an empty version")
                elif client == "transmission":
                    try:
                        result = self._transmission("session-get")
                    except urllib.error.HTTPError as error:
                        if error.code not in (400, 404):
                            raise
                        self.sessions[client].transmission_style = "snake"
                        result = self._transmission("session-get")
                    if result.get("result") != "success":
                        self.sessions[client].transmission_style = "snake"
                        result = self._transmission("session-get")
                    args = result.get("arguments", {})
                    if "rpc_version_semver" in args or "peer_limit_global" in args:
                        self.sessions[client].transmission_style = "snake"
                    return
                elif client == "deluge":
                    self._deluge_connect()
                    return
                else:
                    path = self.config_dir / "rtorrent/scgi.sock"
                    if path.exists():
                        self._scgi("system.client_version")
                        return
            except (OSError, urllib.error.URLError, urllib.error.HTTPError,
                    RuntimeError, ValueError, xmlrpc.client.Error):
                pass
            time.sleep(2)
        if client == "rtorrent":
            cid = self.service_id(client)
            if cid:
                output, _ = run_command(
                    ["docker", "logs", "--tail", "80", cid],
                    env=self.env, check=False, quiet=True, timeout=10,
                )
                if output.strip():
                    print("rTorrent startup log tail:\n" + output.rstrip(), flush=True)
        raise RuntimeError(f"timed out waiting for {LABELS[client]}")

    def _torrentng_request(self, path, *, data=None, headers=None, method=None):
        hdr = {"Authorization": f"Bearer {TOKEN}"}
        hdr.update(headers or {})
        with self.http(
            "torrentngd", path, data=data, headers=hdr, method=method,
        ) as response:
            return response.read()

    def _configure(self, client):
        if client == "torrentngd":
            return
        if client == "qbittorrent":
            values = {
                "max_connec": MAX_PEERS,
                "max_connec_per_torrent": MAX_PEERS,
                "dl_limit": RATE_BPS,
                "up_limit": RATE_BPS,
            }
            data = urllib.parse.urlencode({"json": json.dumps(values)}).encode()
            self.http(
                client, "/api/v2/app/setPreferences", data=data,
                headers={"Content-Type": "application/x-www-form-urlencoded",
                         "Host": "localhost:8080"},
            ).close()
        elif client == "transmission":
            self._transmission("session-set", {
                "peer-limit-global": MAX_PEERS,
                "peer-limit-per-torrent": MAX_PEERS,
                "speed-limit-down": RATE_BPS // 1024,
                "speed-limit-down-enabled": True,
                "speed-limit-up": RATE_BPS // 1024,
                "speed-limit-up-enabled": True,
            })
        elif client == "deluge":
            self._deluge_connect()
            self._deluge("core.set_config", [{
                "max_connections_global": MAX_PEERS,
                "max_connections_per_torrent": MAX_PEERS,
                "max_download_speed": RATE_BPS / 1024,
                "max_upload_speed": RATE_BPS / 1024,
            }])
        else:
            self._scgi("throttle.max_peers.normal.set", "", MAX_PEERS)
            self._scgi("throttle.max_peers.seed.set", "", MAX_PEERS)
            self._scgi("throttle.global_down.max_rate.set_kb", "", RATE_BPS // 1024)
            self._scgi("throttle.global_up.max_rate.set_kb", "", RATE_BPS // 1024)

    def _status(self, client, info_hash, name, *, allow_name_fallback=True):
        if client == "torrentngd":
            rows = json.loads(self._torrentng_request("/api/qb/v2/torrents/info")) or []
            row = next((r for r in rows if r.get("hash", "").lower() == info_hash.lower()), None)
            if row is None and allow_name_fallback:
                row = next((r for r in rows if r.get("name") == name), None)
            return self._qbit_shape(row)
        if client == "qbittorrent":
            rows = self.json(client, "/api/v2/torrents/info",
                             headers={"Host": "localhost:8080"}) or []
            row = next((r for r in rows if r.get("hash", "").lower() == info_hash.lower()), None)
            if row is None and allow_name_fallback:
                row = next((r for r in rows if r.get("name") == name), None)
            return self._qbit_shape(row)
        if client == "transmission":
            response = self._transmission("torrent-get", {
                "fields": ["hashString", "name", "percentDone", "rateDownload",
                           "rateUpload", "peersConnected", "totalSize"],
            })
            rows = response.get("arguments", {}).get("torrents", [])
            style = self.sessions[client].transmission_style
            hash_key = "hash_string" if style == "snake" else "hashString"
            row = next((r for r in rows if r.get(hash_key, "").lower() == info_hash.lower()), None)
            if row is None and allow_name_fallback:
                row = next((r for r in rows if r.get("name") == name), None)
            progress_key = "percent_done" if style == "snake" else "percentDone"
            down_key = "rate_download" if style == "snake" else "rateDownload"
            up_key = "rate_upload" if style == "snake" else "rateUpload"
            peers_key = "peers_connected" if style == "snake" else "peersConnected"
            size_key = "total_size" if style == "snake" else "totalSize"
            return {
                "progress": float(row.get(progress_key, 0)) if row else 0.0,
                "down": int(row.get(down_key, 0)) if row else 0,
                "up": int(row.get(up_key, 0)) if row else 0,
                "peers": int(row.get(peers_key, 0)) if row else 0,
                "total": int(row.get(size_key, 0)) if row else 0,
            }
        if client == "deluge":
            response = self._deluge("web.update_ui", [[
                "name", "progress", "download_payload_rate", "upload_payload_rate",
                "num_peers", "total_size",
            ], {}]) or {}
            torrents = response.get("torrents") or {}
            if isinstance(torrents, dict):
                row = next((value for key, value in torrents.items()
                            if key.lower() == info_hash.lower()
                            or value.get("hash", "").lower() == info_hash.lower()), None)
                values = torrents.values()
            else:
                values = torrents
                row = next((r for r in values
                            if r.get("hash", "").lower() == info_hash.lower()), None)
            if row is None and allow_name_fallback:
                row = next((r for r in values if r.get("name") == name), None)
            return {
                "progress": float(row.get("progress", 0)) / 100 if row else 0.0,
                "down": int(row.get("download_payload_rate", 0)) if row else 0,
                "up": int(row.get("upload_payload_rate", 0)) if row else 0,
                "peers": int(row.get("num_peers", 0)) if row else 0,
                "total": int(row.get("total_size", 0)) if row else 0,
            }
        rows = self._scgi(
            "d.multicall2", "", "main", "d.hash=", "d.name=", "d.complete=",
            "d.size_bytes=", "d.completed_bytes=", "d.down.rate=", "d.up.rate=",
            "d.peers_connected=",
        ) or []
        row = next((r for r in rows if str(r[0]).lower() == info_hash.lower()), None)
        if row is None and allow_name_fallback:
            row = next((r for r in rows if r[1] == name), None)
        if row is None:
            return self._empty_status()
        size = int(row[3])
        done = int(row[4])
        return {
            "progress": min(1.0, done / size) if size else float(row[2]),
            "down": int(row[5]), "up": int(row[6]), "peers": int(row[7]), "total": size,
        }

    @staticmethod
    def _empty_status():
        return {"progress": 0.0, "down": 0, "up": 0, "peers": 0, "total": 0}

    @classmethod
    def _qbit_shape(cls, row):
        if not row:
            return cls._empty_status()
        return {
            "progress": float(row.get("progress", 0)),
            "down": int(row.get("dlspeed", 0)),
            "up": int(row.get("upspeed", 0)),
            "peers": int(row.get("num_seeds", 0)) + int(row.get("num_leechs", 0)),
            "total": int(row.get("size", 0)),
        }

    @staticmethod
    def _multipart(fields):
        boundary = "----tngbench" + str(time.time_ns())
        parts = []
        for name, filename, value, content_type in fields:
            disposition = f'Content-Disposition: form-data; name="{name}"'
            if filename:
                disposition += f'; filename="{filename}"'
            parts.extend([
                f"--{boundary}\r\n{disposition}\r\nContent-Type: {content_type}\r\n\r\n".encode(),
                value, b"\r\n",
            ])
        parts.append(f"--{boundary}--\r\n".encode())
        return b"".join(parts), f"multipart/form-data; boundary={boundary}"

    def _add_torrent(self, client, path, save_path, skip_check=False):
        data = path.read_bytes()
        if client == "torrentngd":
            body, content_type = self._multipart([
                ("torrents", path.name, data, "application/x-bittorrent"),
                ("savepath", None, save_path.encode(), "text/plain"),
                ("paused", None, b"false", "text/plain"),
            ])
            self._torrentng_request("/api/qb/v2/torrents/add", data=body,
                                    headers={"Content-Type": content_type})
        elif client in ("qbittorrent", "seed_leecher"):
            fields = [
                ("torrents", path.name, data, "application/x-bittorrent"),
                ("savepath", None, save_path.encode(), "text/plain"),
                ("paused", None, b"false", "text/plain"),
            ]
            if skip_check:
                fields.append(("skip_checking", None, b"true", "text/plain"))
            body, content_type = self._multipart(fields)
            self.http(
                client, "/api/v2/torrents/add", data=body,
                headers={"Content-Type": content_type, "Host": "localhost:8080"},
            ).close()
        elif client == "transmission":
            self._transmission("torrent-add", {
                "metainfo": base64.b64encode(data).decode("ascii"),
                "download-dir": save_path, "paused": False,
            })
        elif client == "deluge":
            self._deluge_connect()
            self._deluge("core.add_torrent_file", [
                path.name, base64.b64encode(data).decode("ascii"),
                {"download_location": save_path},
            ])
        else:
            shutil.copyfile(path, self.workdir / "watch/rtorrent" / path.name)

    def _pause(self, client, info_hash):
        if client == "torrentngd":
            body = urllib.parse.urlencode({"hashes": info_hash}).encode()
            self._torrentng_request(
                "/api/qb/v2/torrents/stop", data=body,
                headers={"Content-Type": "application/x-www-form-urlencoded"},
            )
        elif client == "qbittorrent":
            body = urllib.parse.urlencode({"hashes": info_hash}).encode()
            self.http(client, "/api/v2/torrents/stop", data=body,
                      headers={"Content-Type": "application/x-www-form-urlencoded",
                               "Host": "localhost:8080"}).close()
        elif client == "transmission":
            self._transmission("torrent-stop", {"ids": [info_hash]})
        elif client == "deluge":
            self._deluge("core.pause_torrent", [[info_hash]])
        else:
            self._scgi("d.stop", info_hash)

    def _remember_container(self, client):
        cid = self.service_id(client)
        if not cid:
            return
        self.container_ids[client] = cid
        output, code = run_command(
            ["docker", "inspect", "--format", "{{.State.Pid}}", cid],
            env=self.env, check=False, quiet=True, timeout=10,
        )
        try:
            pid = int(output.strip()) if code == 0 else 0
        except ValueError:
            pid = 0
        if pid <= 0:
            return
        self.container_pids[client] = pid
        try:
            cgroup_lines = Path(f"/proc/{pid}/cgroup").read_text().splitlines()
        except OSError:
            cgroup_lines = []
        for line in cgroup_lines:
            parts = line.split(":", 2)
            if len(parts) != 3:
                continue
            hierarchy, controllers, relative = parts
            if hierarchy == "0" and not controllers:
                candidate = Path("/sys/fs/cgroup") / relative.lstrip("/")
                if any((candidate / name).is_file() for name in (
                    "memory.current", "cpu.stat", "io.stat",
                )):
                    self.cgroup_paths[client] = candidate
                    break
            elif "memory" in controllers.split(","):
                candidate = Path("/sys/fs/cgroup/memory") / relative.lstrip("/")
                if (candidate / "memory.usage_in_bytes").is_file():
                    self.cgroup_paths[client] = candidate
                    break
        if client == "torrentngd":
            output, code = run_command(
                ["docker", "top", cid, "-eo", "pid,args"],
                env=self.env, check=False, quiet=True, timeout=10,
            )
            if code == 0:
                for line in output.splitlines()[1:]:
                    fields = line.strip().split(None, 1)
                    if len(fields) != 2:
                        continue
                    executable_tokens = [token.strip("'\"") for token in fields[1].split()]
                    if any(Path(token).name == "torrentngd" for token in executable_tokens):
                        try:
                            self.torrentng_pids[client] = int(fields[0])
                        except ValueError:
                            pass
                        break

    @staticmethod
    def _read_cgroup_memory(path):
        result = {
            "current": 0, "peak": 0, "anon": 0, "file": 0, "slab": 0,
            "sock": 0, "kernel_stack": 0, "pagetables": 0,
        }
        if path is None:
            return result
        files = {
            "current": ("memory.current", "memory.usage_in_bytes"),
            "peak": ("memory.peak", "memory.max_usage_in_bytes"),
        }
        for key, candidates in files.items():
            for candidate in candidates:
                try:
                    result[key] = int((path / candidate).read_text().strip())
                    break
                except (OSError, ValueError):
                    continue
        try:
            stat = {}
            for line in (path / "memory.stat").read_text().splitlines():
                fields = line.split()
                if len(fields) == 2:
                    stat[fields[0]] = int(fields[1])
            result["anon"] = stat.get("anon", stat.get("total_rss", stat.get("rss", 0)))
            result["file"] = stat.get("file", stat.get("total_cache", stat.get("cache", 0)))
            result["slab"] = stat.get("slab", stat.get("total_slab", 0))
            result["sock"] = stat.get("sock", 0)
            result["kernel_stack"] = stat.get("kernel_stack", 0)
            result["pagetables"] = stat.get("pagetables", 0)
        except (OSError, ValueError):
            pass
        return result

    @staticmethod
    def _read_cgroup_cpu(path):
        if path is None:
            return None
        try:
            return parse_cgroup_cpu_usage_usec((path / "cpu.stat").read_text())
        except OSError:
            return None

    @staticmethod
    def _read_cgroup_io(path):
        if path is None:
            return None
        try:
            return parse_cgroup_io_stat((path / "io.stat").read_text())
        except OSError:
            return None

    @staticmethod
    def _read_process_memory(pid):
        byte_fields = {
            "VmRSS": "rss", "VmHWM": "hwm", "RssAnon": "rss_anon",
            "RssFile": "rss_file", "RssShmem": "rss_shmem",
            "VmData": "vm_data", "VmStk": "vm_stk", "VmPTE": "vm_pte",
        }
        result = {value: 0 for value in byte_fields.values()}
        result.update({"threads": 0, "thread_names": {}, "smaps_rollup": {}, "mappings": []})
        if not pid:
            return result
        try:
            for line in Path(f"/proc/{pid}/status").read_text().splitlines():
                fields = line.split()
                if len(fields) < 2:
                    continue
                key = fields[0].rstrip(":")
                if key in byte_fields:
                    result[byte_fields[key]] = int(fields[1]) * 1024
                elif key == "Threads":
                    result["threads"] = int(fields[1])
        except (OSError, ValueError):
            pass

        try:
            task_root = Path(f"/proc/{pid}/task")
            for task in task_root.iterdir():
                try:
                    name = (task / "comm").read_text().strip() or "unknown"
                except OSError:
                    name = "unknown"
                result["thread_names"][name] = result["thread_names"].get(name, 0) + 1
        except OSError:
            pass

        byte_metrics = {
            "Rss", "Pss", "Pss_Anon", "Pss_File", "Pss_Shmem",
            "Shared_Clean", "Shared_Dirty", "Private_Clean", "Private_Dirty",
            "Anonymous", "AnonHugePages", "Swap",
        }
        try:
            lines = Path(f"/proc/{pid}/smaps").read_text().splitlines()
            maps = []
            current = None
            for line in lines:
                fields = line.split()
                if (len(fields) >= 5 and "-" in fields[0]
                        and len(fields[1]) == 4 and fields[1][0] in "r-"
                        and fields[1][1] in "w-" and fields[1][2] in "x-"):
                    if current is not None and current["rss_bytes"]:
                        maps.append(current)
                    try:
                        start, end = fields[0].split("-", 1)
                        size = int(end, 16) - int(start, 16)
                    except ValueError:
                        size = 0
                    current = {
                        "name": " ".join(fields[5:]) if len(fields) > 5 else "[anonymous]",
                        "size_bytes": size,
                        "rss_bytes": 0,
                        "pss_bytes": 0,
                        "anonymous_bytes": 0,
                        "private_dirty_bytes": 0,
                        "private_clean_bytes": 0,
                        "shared_clean_bytes": 0,
                    }
                elif current is not None and len(fields) >= 2 and fields[0].rstrip(":") in byte_metrics:
                    try:
                        amount = int(fields[1]) * 1024
                    except ValueError:
                        continue
                    key = fields[0].rstrip(":")
                    result["smaps_rollup"][key] = result["smaps_rollup"].get(key, 0) + amount
                    target = {
                        "Rss": "rss_bytes", "Pss": "pss_bytes",
                        "Anonymous": "anonymous_bytes", "Private_Dirty": "private_dirty_bytes",
                        "Private_Clean": "private_clean_bytes", "Shared_Clean": "shared_clean_bytes",
                    }.get(key)
                    if target:
                        current[target] += amount
            if current is not None and current["rss_bytes"]:
                maps.append(current)
            result["mappings"] = sorted(maps, key=lambda item: item["rss_bytes"], reverse=True)[:20]
        except (OSError, ValueError):
            pass
        return result

    def _memory_components(self, client):
        cgroup = self._read_cgroup_memory(self.cgroup_paths.get(client))
        process = self._read_process_memory(self.torrentng_pids.get(client))
        return cgroup, process

    def _capture_engine_memory_metrics(self, client, phase):
        if client != "torrentngd":
            return
        profile = {"utc": dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds"),
                   "client": client, "phase": phase, "metrics": {}}
        try:
            text = self._torrentng_request("/metrics").decode("utf-8", "replace")
            for line in text.splitlines():
                if not line or line.startswith("#") or " " not in line:
                    continue
                name, value = line.rsplit(" ", 1)
                metric_name = name.split("{", 1)[0]
                if (metric_name == "torrentng_memory_used_bytes"
                        or metric_name.startswith("torrentng_memory_class_")
                        or metric_name.startswith("torrentng_storage_peer_read_cache_")
                        or metric_name.startswith("torrentng_storage_frame_bytes_")
                        or metric_name.startswith("torrentng_jemalloc_")
                        or metric_name in {
                            "torrentng_peer_rx_buffer_bytes",
                            "torrentng_peer_tx_buffer_bytes",
                            "torrentng_piece_assembly_bytes",
                            "torrentng_storage_file_pool_memory_bytes",
                        }
                        or metric_name.startswith("torrentng_hot_torrent_storage_cache_bytes")
                        or metric_name.startswith("torrentng_hot_torrent_peer_buffer_bytes")):
                    try:
                        profile["metrics"][name] = float(value)
                    except ValueError:
                        continue
        except (OSError, urllib.error.URLError, urllib.error.HTTPError, ValueError) as error:
            profile["error"] = f"{type(error).__name__}: {error}"
        self.engine_memory_metrics.append(profile)
        self.last_engine_memory_metrics_at = time.monotonic()

    def _peak_engine_metric(self, client, metric_name, phases):
        values = [
            value
            for profile in self.engine_memory_metrics
            if profile.get("client") == client and profile.get("phase") in phases
            for name, value in profile.get("metrics", {}).items()
            if name.split("{", 1)[0] == metric_name
        ]
        return max(values) if values else None

    def _write_memory_profile(self):
        samples = []
        for row in self.samples:
            if len(row) < 35:
                continue
            samples.append({
                "utc": row[0], "client": row[1], "phase": row[2],
                "container_working_set_bytes": int(row[9]),
                "cgroup_memory_current_bytes": int(row[11]),
                "cgroup_memory_peak_bytes": int(row[12]),
                "cgroup_memory_anon_bytes": int(row[13]),
                "cgroup_memory_file_bytes": int(row[14]),
                "cgroup_memory_slab_bytes": int(row[15]),
                "torrentng_process_rss_bytes": int(row[16]),
                "torrentng_process_hwm_bytes": int(row[17]),
                "torrentng_process_rss_anon_bytes": int(row[18]),
                "torrentng_process_rss_file_bytes": int(row[19]),
                "torrentng_process_rss_shmem_bytes": int(row[20]),
                "torrentng_process_vm_data_bytes": int(row[21]),
                "torrentng_process_vm_stk_bytes": int(row[22]),
                "torrentng_process_vm_pte_bytes": int(row[23]),
                "torrentng_process_threads": int(row[24]),
                "cgroup_memory_sock_bytes": int(row[25]),
                "cgroup_memory_kernel_stack_bytes": int(row[26]),
                "cgroup_memory_pagetables_bytes": int(row[27]),
                "cgroup_cpu_usage_usec": int(row[28]) if row[28] != "" else None,
                "cgroup_io_read_bytes": int(row[29]) if row[29] != "" else None,
                "cgroup_io_write_bytes": int(row[30]) if row[30] != "" else None,
                "cgroup_io_read_ops": int(row[31]) if row[31] != "" else None,
                "cgroup_io_write_ops": int(row[32]) if row[32] != "" else None,
                "network_rx_bytes": int(row[33]) if row[33] != "" else None,
                "network_tx_bytes": int(row[34]) if row[34] != "" else None,
                "container_process_tree_rss_bytes": (
                    int(row[35]) if len(row) > 35 and row[35] != "" else None
                ),
                "container_process_tree_pss_bytes": (
                    int(row[36]) if len(row) > 36 and row[36] != "" else None
                ),
                "container_process_tree_count": (
                    int(row[37]) if len(row) > 37 and row[37] != "" else None
                ),
                "container_process_tree_threads": (
                    int(row[38]) if len(row) > 38 and row[38] != "" else None
                ),
            })
        self.memory_profile.parent.mkdir(parents=True, exist_ok=True)
        self.memory_profile.write_text(json.dumps({
            "schema_version": 4,
            "source_commit": self.source_commit,
            "source_tree": self.source_tree,
            "source_dirty": self.source_dirty,
            "source_patch_sha256": self.source_patch_sha256,
            "source_untracked_count": self.source_untracked_count,
            "source_untracked_sha256": self.source_untracked_sha256,
            "source_fingerprint_sha256": self.source_fingerprint_sha256,
            "cargo_features": self.cargo_features,
            "allocator": self.allocator,
            "host": self.host,
            "memory_samples": samples,
            "process_memory_snapshots": self.process_memory_snapshots,
            "engine_metrics_snapshots": self.engine_memory_metrics,
            "notes": {
                "docker_stats": "Docker-reported container working set, sampled at the configured interval.",
                "cgroup_memory_current": "Container cgroup total, including anon and file cache.",
                "cgroup_memory_anon": "Container anonymous memory from memory.stat.",
                "cgroup_cpu_usage": "Cgroup v2 cpu.stat usage_usec; phase deltas are normalized by transferred payload bytes.",
                "cgroup_io": "Cgroup v2 io.stat counters summed across devices. These are block-layer bytes and operations charged to the container; page-cache hits are not block I/O.",
                "network_bytes": "RX/TX counters from non-loopback interfaces in the container network namespace; they include all namespace traffic, not just BitTorrent DHT packets.",
                "container_process_tree_rss": "Idle-mode host /proc RSS summed across the container's process tree. Shared pages can be counted in multiple processes.",
                "container_process_tree_pss": "Idle-mode host /proc PSS summed across the container's process tree. PSS divides shared pages proportionally and is distinct from cgroup memory.",
                "process_tree_memory_sampling": "Idle process-tree RSS/PSS are sampled at the configured process-sample interval, independent of the regular resource-sample interval. Unavailable readings are null.",
                "torrentng_process_rss": "Host /proc RSS for torrentngd when available; zero means unavailable.",
                "process_memory_snapshots": "TorrentNG /proc status, thread names, smaps totals, and the 20 largest resident mappings captured with cgroup samples.",
                "engine_metrics": "TorrentNG resource-governor leases, estimated peer-buffer capacities, storage-pool gauges, and optional allocator metrics sampled during transfer and at phase boundaries.",
                "jemalloc_metrics": "Jemalloc allocator gauges are present only when the jemalloc-allocator feature is selected. They describe Jemalloc's allocator state, not total process allocations or RSS.",
                "source_fingerprint": "SHA-256 derived from commit/tree, a normalized full-index Git diff, and all non-ignored untracked file paths and contents at benchmark start.",
                "process_memory": "Linux /proc RSS, anonymous RSS, file RSS, thread count, and smaps samples are process-level readings; allocator gauges do not replace them.",
                "unavailable_cgroup_counters": "An empty CSV cell and null profile value mean the host/container runtime did not expose that counter.",
            },
        }, indent=2) + "\n")

    def _stats(self, client):
        cid = self.container_ids.get(client) or self.service_id(client)
        if not cid:
            return 0.0, 0
        output, code = run_command(
            ["docker", "stats", "--no-stream", "--format", "{{.CPUPerc}}|{{.MemUsage}}", cid],
            env=self.env, check=False, quiet=True, timeout=10,
        )
        if code:
            return 0.0, 0
        parts = output.strip().split("|", 1)
        if len(parts) != 2:
            return 0.0, 0
        try:
            return float(parts[0].rstrip("%")), to_bytes(parts[1].split("/", 1)[0])
        except (ValueError, IndexError):
            return 0.0, 0

    def _sample(self, client, phase, elapsed, status, down_override=None):
        cpu, memory = self._stats(client)
        cgroup, process = self._memory_components(client)
        cgroup_path = self.cgroup_paths.get(client)
        cgroup_cpu_usage_usec = self._read_cgroup_cpu(cgroup_path)
        cgroup_io = self._read_cgroup_io(cgroup_path)
        network_rx_bytes, network_tx_bytes = self._container_network_bytes(client)
        process_tree_memory = (None, None, None, None)
        if self.mode == "idle":
            sample_times = getattr(self, "process_tree_memory_sample_elapsed", None)
            if sample_times is None:
                sample_times = {}
                self.process_tree_memory_sample_elapsed = sample_times
            sample_key = (client, phase)
            previous = sample_times.get(sample_key)
            if (previous is None
                    or elapsed - previous >= IDLE_PROCESS_SAMPLE_SECONDS
                    or elapsed >= IDLE_PROFILE_MEASURE_SECONDS):
                process_tree_memory = self._container_process_tree_memory(client)
                sample_times[sample_key] = elapsed
        row = [
            dt.datetime.now(dt.timezone.utc).isoformat(timespec="milliseconds"),
            client, phase, f"{elapsed:.3f}", f"{status.get('progress', 0):.6f}",
            int(status.get("down", 0) if down_override is None else down_override),
            int(status.get("up", 0)), int(status.get("peers", 0)),
            f"{cpu:.2f}", memory, f"{os.getloadavg()[0]:.2f}",
            cgroup["current"], cgroup["peak"], cgroup["anon"],
            cgroup["file"], cgroup["slab"], process["rss"], process["hwm"],
            process["rss_anon"], process["rss_file"], process["rss_shmem"],
            process["vm_data"], process["vm_stk"], process["vm_pte"],
            process["threads"], cgroup["sock"], cgroup["kernel_stack"],
            cgroup["pagetables"],
            "" if cgroup_cpu_usage_usec is None else cgroup_cpu_usage_usec,
            "" if cgroup_io is None else cgroup_io["read_bytes"],
            "" if cgroup_io is None else cgroup_io["write_bytes"],
            "" if cgroup_io is None else cgroup_io["read_ops"],
            "" if cgroup_io is None else cgroup_io["write_ops"],
            "" if network_rx_bytes is None else network_rx_bytes,
            "" if network_tx_bytes is None else network_tx_bytes,
            *("" if value is None else value for value in process_tree_memory),
        ]
        self.samples.append(row)
        if client == "torrentngd":
            self.process_memory_snapshots.append({
                "utc": row[0], "phase": phase, "threads": process["threads"],
                "thread_names": process["thread_names"],
                "smaps_rollup": process["smaps_rollup"],
                "largest_mappings": process["mappings"],
            })
        with self.csv_path.open("a", newline="") as handle:
            csv.writer(handle).writerow(row)
        now = time.monotonic()
        if (self.mode != "idle" and client == "torrentngd"
                and now - self.last_engine_memory_metrics_at >= ALLOCATOR_METRIC_INTERVAL):
            self._capture_engine_memory_metrics(client, phase)
        return row

    def _container_networks(self, container_id):
        output, code = run_command(
            [
                "docker", "inspect", "--format",
                "{{json .NetworkSettings.Networks}}", container_id,
            ],
            env=self.env, check=False, quiet=True, timeout=15,
        )
        if code:
            raise RuntimeError(f"could not inspect Docker networks for {container_id}")
        try:
            networks = json.loads(output.strip() or "null")
        except json.JSONDecodeError as error:
            raise RuntimeError(f"Docker returned invalid network data for {container_id}") from error
        if networks is None:
            return {}
        if not isinstance(networks, dict):
            raise RuntimeError(f"Docker returned an unexpected network map for {container_id}")
        return networks

    def _disconnect_container_networks(self, client, container_id):
        original = self._container_networks(container_id)
        if not original:
            raise RuntimeError(f"{LABELS[client]} has no Docker network to isolate")
        detached = getattr(self, "detached_networks", None)
        if detached is None:
            detached = {}
            self.detached_networks = detached
        detached[client] = (container_id, original)
        try:
            for network in original:
                run_command(
                    ["docker", "network", "disconnect", network, container_id],
                    env=self.env, timeout=30,
                )
            if self._container_networks(container_id):
                raise RuntimeError(
                    f"{LABELS[client]} still has a Docker network after disconnect"
                )
        except Exception as error:
            try:
                self._reconnect_container_networks(client)
            except Exception as restore_error:
                raise RuntimeError(
                    f"network isolation failed ({error}); restoring the original network "
                    f"also failed ({restore_error})"
                ) from error
            raise
        isolated = getattr(self, "network_isolated_clients", None)
        if isolated is None:
            isolated = set()
            self.network_isolated_clients = isolated
        isolated.add(client)

    def _reconnect_container_networks(self, client):
        detached = getattr(self, "detached_networks", {})
        record = detached.get(client)
        if record is None:
            return
        container_id, original = record
        current = self._container_networks(container_id)
        for network, details in original.items():
            if network in current:
                continue
            args = ["docker", "network", "connect"]
            aliases = (details.get("Aliases") or []) if isinstance(details, dict) else []
            for alias in aliases:
                args.extend(["--alias", alias])
            args.extend([network, container_id])
            run_command(args, env=self.env, timeout=30)
        current = self._container_networks(container_id)
        if set(current) != set(original):
            raise RuntimeError(
                f"Docker network restore for {LABELS[client]} was incomplete: "
                f"{sorted(current)} != {sorted(original)}"
            )
        missing_aliases = {
            network: sorted(
                set((details.get("Aliases") or []) if isinstance(details, dict) else [])
                - set((current[network].get("Aliases") or [])
                      if isinstance(current[network], dict) else [])
            )
            for network, details in original.items()
        }
        missing_aliases = {
            network: aliases for network, aliases in missing_aliases.items() if aliases
        }
        if missing_aliases:
            raise RuntimeError(
                f"Docker network aliases were not restored for {LABELS[client]}: "
                f"{missing_aliases}"
            )
        detached.pop(client, None)
        getattr(self, "network_isolated_clients", set()).discard(client)

    def _container_network_bytes(self, client):
        if client in getattr(self, "network_isolated_clients", set()):
            return 0, 0
        pid = getattr(self, "container_pids", {}).get(client)
        if not pid:
            return None, None
        try:
            return parse_proc_net_dev(Path(f"/proc/{pid}/net/dev").read_text())
        except OSError:
            return None, None

    def _container_process_tree_memory(self, client):
        """Read client process-tree RSS/PSS from host /proc, when permitted."""
        pid = getattr(self, "container_pids", {}).get(client)
        if not pid:
            return None, None, None, None
        sample = sample_process_tree(pid, time.monotonic_ns())
        return sample.rss_bytes, sample.pss_bytes, sample.processes, sample.threads

    def _sample_idle(self, client):
        started = time.monotonic()
        while True:
            elapsed = time.monotonic() - started
            self._sample(client, "idle", elapsed, {
                "progress": 0.0, "down": 0, "up": 0, "peers": 0,
            })
            remaining = IDLE_SAMPLE_SECONDS - (time.monotonic() - started)
            if remaining <= 0:
                break
            time.sleep(min(INTERVAL, remaining))

    def _wait_complete(self, client, info_hash, name, expected_size, timeout, phase):
        start = time.monotonic()
        next_log = 0
        completion_observed = None
        rows = []
        self.partial_download_rows[client] = rows
        while time.monotonic() - start < timeout:
            status = self._status(client, info_hash, name)
            elapsed = time.monotonic() - start
            rows.append(self._sample(client, phase, elapsed, status))
            self.partial_download_rows[client] = rows
            if status["progress"] >= 1.0:
                payload = self._data_path(client, name)
                try:
                    actual = payload.stat().st_size if payload.is_file() else 0
                except OSError:
                    actual = 0
                if actual == expected_size:
                    return elapsed, status, rows
                if status["progress"] >= 1.0:
                    if completion_observed is None:
                        completion_observed = time.monotonic()
                    if time.monotonic() - completion_observed >= 60:
                        raise RuntimeError(
                            f"{LABELS[client]} reported completion but its file is "
                            f"{actual} bytes after 60s (expected {expected_size})"
                        )
                else:
                    completion_observed = None
            else:
                completion_observed = None
            if elapsed > next_log:
                print(
                    f"[{LABELS[client]} {phase}] {status['progress']:.1%}, "
                    f"{status['down'] / 1048576:.2f} MiB/s, {status['peers']} peers",
                    flush=True,
                )
                next_log = elapsed + 20
            time.sleep(INTERVAL)
        raise TimeoutError(f"{LABELS[client]} did not complete {phase} within {timeout}s")

    def _torrent_metadata(self):
        index = urllib.request.urlopen(TORRENT_INDEX, timeout=30).read().decode("utf-8", "replace")
        names = re.findall(r'href=["\'](debian-[^"\']+-amd64-netinst\.iso\.torrent)["\']', index)
        if not names:
            raise RuntimeError(f"official Debian directory has no amd64 netinst torrent: {TORRENT_INDEX}")
        def version_key(value):
            match = re.search(r"debian-([0-9.]+)-", value)
            return tuple(int(part) for part in match.group(1).split(".")) if match else ()
        filename = max(names, key=version_key)
        url = urllib.parse.urljoin(TORRENT_INDEX, filename)
        path = self.workdir / "torrents" / filename
        urllib.request.urlretrieve(url, path)
        sums = urllib.request.urlopen(SHA256SUMS_URL, timeout=30).read().decode("ascii", "replace")
        iso_name = filename[:-8]
        checksum = None
        for line in sums.splitlines():
            match = re.match(r"^([0-9a-fA-F]{64})\s+\*?" + re.escape(iso_name) + r"\s*$", line)
            if match:
                checksum = match.group(1).lower()
                break
        if not checksum:
            raise RuntimeError(f"official SHA256SUMS has no entry for {iso_name}")
        output, _ = run_command(["aria2c", "-S", str(path)], quiet=True, timeout=30)
        name_match = re.search(r"(?m)^Name:\s*(.+)$", output)
        size_match = re.search(r"(?m)^Total Length:.*\(([0-9,]+)\)", output)
        hash_match = re.search(r"(?m)^Info Hash:\s*([0-9a-fA-F]+)", output)
        if not all((name_match, size_match, hash_match)):
            raise RuntimeError("aria2c could not read the official torrent metadata")
        return {
            "filename": iso_name, "torrent_url": url, "torrent_path": path,
            "iso_name": name_match.group(1).strip(),
            "size": int(size_match.group(1).replace(",", "")),
            "info_hash": hash_match.group(1).lower(), "sha256": checksum,
        }

    def _start_tracker(self):
        self.compose("up", "-d", "opentracker", quiet=True)
        cid = self.service_id("opentracker")
        if not cid:
            raise RuntimeError("opentracker failed to start")
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            output, code = run_command([
                "docker", "inspect", "-f",
                "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}", cid,
            ], env=self.env, check=False, quiet=True)
            if code == 0 and output.strip():
                return output.strip()
            time.sleep(1)
        raise RuntimeError("could not determine opentracker container address")

    def _version(self, client):
        try:
            if client == "torrentngd":
                return f"build {self.source_commit[:12]}"
            if client == "qbittorrent":
                with self.http(client, "/api/v2/app/version",
                               headers={"Host": "localhost:8080"}) as response:
                    return response.read().decode("utf-8", "replace").strip()
            if client == "transmission":
                args = self._transmission("session-get").get("arguments", {})
                return str(args.get("version") or args.get("rpc-version-semver") or "unavailable")
            if client == "deluge":
                return str(self._deluge("web.get_version"))
            return str(self._scgi("system.client_version"))
        except Exception as error:
            return f"unavailable ({type(error).__name__})"

    def _data_path(self, client, iso_name):
        base = self.workdir / "downloads" / client
        if client != "rtorrent":
            base = base / "public"
        path = base / iso_name
        if path.is_file():
            return path
        matches = list(base.rglob(iso_name))
        return matches[0] if matches else path

    @staticmethod
    def _sha256(path):
        digest = hashlib.sha256()
        with path.open("rb") as handle:
            while chunk := handle.read(4 * 1024 * 1024):
                digest.update(chunk)
        return digest.hexdigest()

    def _preserve_source_artifacts(self, meta):
        if self.artifact_dir is None:
            return
        if self.seed_source is None:
            self.artifact_status = "not saved: no SHA-256-verified ISO was available"
            return
        self.artifact_dir.mkdir(parents=True, exist_ok=True)
        torrent_path = Path(meta["torrent_path"])
        sources = (
            (self.seed_source, meta["iso_name"], meta["size"], meta["sha256"]),
            (torrent_path, torrent_path.name, torrent_path.stat().st_size,
             self._sha256(torrent_path)),
        )
        for source, filename, expected_size, expected_sha256 in sources:
            destination = self.artifact_dir / filename
            if destination.is_symlink():
                raise RuntimeError(f"refusing symlink artifact destination: {destination}")
            if destination.exists():
                if (not destination.is_file()
                        or destination.stat().st_size != expected_size
                        or self._sha256(destination) != expected_sha256):
                    raise RuntimeError(f"refusing to overwrite different artifact: {destination}")
                continue
            descriptor, temporary_name = tempfile.mkstemp(
                prefix=f".{filename}.", suffix=".partial", dir=self.artifact_dir,
            )
            os.close(descriptor)
            temporary = Path(temporary_name)
            try:
                shutil.copy2(source, temporary)
                if (temporary.stat().st_size != expected_size
                        or self._sha256(temporary) != expected_sha256):
                    raise RuntimeError(f"copied artifact failed verification: {filename}")
                os.replace(temporary, destination)
            finally:
                temporary.unlink(missing_ok=True)
        self.artifact_status = f"verified ISO and torrent saved under {self.artifact_dir}"

    def _start_seed_leecher(self, torrent_path, seeder):
        client = "seed_leecher"
        # Reuse the same controlled infohash with a fresh helper state for each seeder.
        client_config = self.config_dir / "qbittorrent-seed" / seeder
        self._env("INTEROP_SEED_QBIT_CONFIG", client_config)
        self._prepare_seed_qbit_config(seeder)
        self.sessions[client] = HttpClient(self.api_url(client))
        self.compose("up", "-d", client, quiet=True, timeout=900)
        container_id = self.service_id(client)
        if container_id:
            self.seed_leecher_image = run_command(
                ["docker", "inspect", "-f", "{{.Config.Image}}", container_id],
                env=self.env, check=False, quiet=True,
            )[0].strip() or "unknown"
        deadline = time.monotonic() + 240
        ready = False
        while time.monotonic() < deadline:
            try:
                with self.http(
                    client, "/api/v2/app/version",
                    headers={"Host": "localhost:8080"},
                ) as response:
                    self.seed_leecher_version = response.read().decode().strip()
                ready = bool(self.seed_leecher_version)
                if ready:
                    break
            except (OSError, urllib.error.URLError, urllib.error.HTTPError,
                    RuntimeError, ValueError):
                time.sleep(2)
        if not ready:
            raise RuntimeError("timed out waiting for the qBittorrent seed leecher")
        preferences = {
            "encryption": 2,
            "bittorrent_protocol": 1,
            "dht": False,
            "lsd": False,
            "pex": False,
            "max_connec": MAX_PEERS,
            "max_connec_per_torrent": MAX_PEERS,
            "dl_limit": SEED_LEECHER_RATE_BPS,
            "up_limit": RATE_BPS,
        }
        self._qbit_json(
            client, "/api/v2/app/setPreferences",
            data=urllib.parse.urlencode({"json": json.dumps(preferences)}).encode(),
            headers={"Content-Type": "application/x-www-form-urlencoded"},
        )
        self._add_torrent(client, torrent_path, "/downloads")

    def _seed_leecher_status(self, info_hash):
        rows = self._qbit_json(
            "seed_leecher", "/api/v2/torrents/info?hash=" + urllib.parse.quote(info_hash),
        ) or []
        row = next((r for r in rows if r.get("hash", "").lower() == info_hash.lower()), None)
        return self._qbit_shape(row)

    def _wait_seed_leecher_tracker(self, info_hash):
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            try:
                rows = self._qbit_json(
                    "seed_leecher",
                    "/api/v2/torrents/trackers?hash=" + urllib.parse.quote(info_hash),
                ) or []
            except urllib.error.HTTPError as error:
                if error.code != 404:
                    raise
                rows = []
            tracker = next((
                row for row in rows
                if row.get("url", "").startswith(("http://", "https://", "udp://"))
            ), None)
            if (tracker and int(tracker.get("status", 0)) == 2
                    and int(tracker.get("num_seeds", 0)) >= 1):
                return
            time.sleep(2)
        raise TimeoutError("qBittorrent seed leecher did not register with the private tracker")

    def _reannounce(self, client, info_hash):
        if client == "torrentngd":
            body = urllib.parse.urlencode({"hashes": info_hash}).encode()
            self._torrentng_request(
                "/api/qb/v2/torrents/reannounce", data=body,
                headers={"Content-Type": "application/x-www-form-urlencoded"},
            )
        elif client == "qbittorrent":
            body = urllib.parse.urlencode({"hashes": info_hash}).encode()
            self.http(
                client, "/api/v2/torrents/reannounce", data=body,
                headers={"Content-Type": "application/x-www-form-urlencoded",
                         "Host": "localhost:8080"},
            ).close()
        elif client == "transmission":
            self._transmission("torrent-reannounce", {"ids": [info_hash]})
        elif client == "deluge":
            self._deluge_connect()
            self._deluge("core.force_reannounce", [[info_hash]])
        else:
            self._scgi("d.tracker_announce", info_hash)

    def _seed_test(self, client, payload, torrent_path, info_hash, size, checksum):
        save_path = "/downloads/rtorrent" if client == "rtorrent" else f"/downloads/{client}/seed"
        self._add_torrent(client, torrent_path, save_path, skip_check=True)
        deadline = time.monotonic() + 300
        while time.monotonic() < deadline:
            state = self._status(client, info_hash, payload.name, allow_name_fallback=False)
            if state["progress"] >= 1.0:
                break
            time.sleep(2)
        else:
            raise TimeoutError(f"{LABELS[client]} did not recognize the verified file as a seed")

        leecher_dir = self.workdir / "leecher" / client
        leecher_dir.mkdir(parents=True, exist_ok=True)
        self._env("INTEROP_SEED_QBIT_DOWNLOAD_DIR", leecher_dir)
        setup_started = time.monotonic()
        rows = []
        self.partial_seed_rows[client] = rows
        self.partial_seed_elapsed[client] = 0.0
        self.partial_seed_setup[client] = 0.0
        next_log = 0
        try:
            self._start_seed_leecher(torrent_path, client)
            self._wait_seed_leecher_tracker(info_hash)
            self._reannounce(client, info_hash)
            self.seed_cache_eviction[client] = evict_file_cache(payload)
            started = time.monotonic()
            deadline = started + SEED_TIMEOUT
            last_peer = started
            self.partial_seed_setup[client] = started - setup_started
            while time.monotonic() < deadline:
                state = self._seed_leecher_status(info_hash)
                total = state["total"]
                completed = int(total * state["progress"])
                elapsed = time.monotonic() - started
                rows.append(self._sample(client, "controlled_seed", elapsed, state))
                self.partial_seed_rows[client] = rows
                self.partial_seed_elapsed[client] = elapsed
                if elapsed >= next_log:
                    print(
                        f"[{LABELS[client]} seed] {state['progress']:.1%}, "
                        f"leecher {state['down'] / 1048576:.2f} MiB/s, "
                        f"{state['peers']} peers",
                        flush=True,
                    )
                    next_log = elapsed + 20
                if state["peers"]:
                    last_peer = time.monotonic()
                elif time.monotonic() - last_peer >= SEED_NO_PEER_TIMEOUT:
                    raise TimeoutError(
                        f"{LABELS[client]} controlled seed had no connected leecher peers "
                        f"after {SEED_NO_PEER_TIMEOUT}s"
                    )
                if total >= size and completed >= size:
                    candidate = leecher_dir / payload.name
                    if not candidate.is_file():
                        matches = list(leecher_dir.rglob(payload.name))
                        candidate = matches[0] if matches else candidate
                    if not candidate.is_file() or candidate.stat().st_size != size:
                        raise RuntimeError("qBittorrent completed but the leecher file is absent or short")
                    actual = self._sha256(candidate)
                    if actual != checksum:
                        raise RuntimeError("controlled leecher SHA-256 did not match Debian")
                    return time.monotonic() - started, rows, actual
                time.sleep(INTERVAL)
            raise TimeoutError(f"controlled seed transfer exceeded {SEED_TIMEOUT}s")
        finally:
            self.compose("stop", "seed_leecher", check=False, quiet=True, timeout=30)

    @staticmethod
    def _summary(rows, phase):
        records = [row for row in rows if row[2] == phase]
        def counter_delta(index):
            values = [int(row[index]) for row in records if len(row) > index and row[index] != ""]
            if len(values) < 2:
                return None
            delta = values[-1] - values[0]
            return delta if delta >= 0 else None

        def available_values(index):
            return [
                int(row[index]) for row in records
                if len(row) > index and row[index] != ""
            ]

        if not records:
            return {"mean_cpu": 0.0, "peak_cpu": 0.0, "peak_memory": 0,
                    "peak_cgroup_current": 0, "peak_cgroup_anon": 0,
                    "peak_cgroup_file": 0, "peak_process_rss": 0,
                    "mean_memory": 0, "mean_cgroup_current": 0,
                    "mean_cgroup_anon": 0,
                    "peak_rate": 0, "peak_peers": 0,
                    "cgroup_cpu_seconds": None, "cgroup_io_read_bytes": None,
                    "cgroup_io_write_bytes": None, "cgroup_io_read_ops": None,
                    "cgroup_io_write_ops": None, "network_rx_bytes": None,
                    "network_tx_bytes": None, "sampled_seconds": None,
                    "peak_process_tree_rss": None, "mean_process_tree_rss": None,
                    "peak_process_tree_pss": None, "mean_process_tree_pss": None,
                    "peak_process_tree_count": None, "peak_process_tree_threads": None,
                    "process_tree_memory_sample_count": 0}
        cpu_usec = counter_delta(28)
        try:
            sampled_seconds = float(records[-1][3]) - float(records[0][3])
            if sampled_seconds <= 0:
                sampled_seconds = None
        except (ValueError, IndexError):
            sampled_seconds = None
        process_tree_rss = available_values(35)
        process_tree_pss = available_values(36)
        process_tree_counts = available_values(37)
        process_tree_threads = available_values(38)
        return {
            "mean_cpu": sum(float(row[8]) for row in records) / len(records),
            "peak_cpu": max(float(row[8]) for row in records),
            "peak_memory": max(int(row[9]) for row in records),
            "peak_cgroup_current": max(int(row[11]) for row in records),
            "peak_cgroup_anon": max(int(row[13]) for row in records),
            "peak_cgroup_file": max(int(row[14]) for row in records),
            "peak_process_rss": max(int(row[16]) for row in records),
            "mean_memory": sum(int(row[9]) for row in records) / len(records),
            "mean_cgroup_current": sum(int(row[11]) for row in records) / len(records),
            "mean_cgroup_anon": sum(int(row[13]) for row in records) / len(records),
            "peak_rate": max(int(row[5]) for row in records),
            "peak_peers": max(int(row[7]) for row in records),
            "cgroup_cpu_seconds": cpu_usec / 1_000_000 if cpu_usec is not None else None,
            "cgroup_io_read_bytes": counter_delta(29),
            "cgroup_io_write_bytes": counter_delta(30),
            "cgroup_io_read_ops": counter_delta(31),
            "cgroup_io_write_ops": counter_delta(32),
            "network_rx_bytes": counter_delta(33),
            "network_tx_bytes": counter_delta(34),
            "peak_process_tree_rss": max(process_tree_rss) if process_tree_rss else None,
            "mean_process_tree_rss": (
                sum(process_tree_rss) / len(process_tree_rss) if process_tree_rss else None
            ),
            "peak_process_tree_pss": max(process_tree_pss) if process_tree_pss else None,
            "mean_process_tree_pss": (
                sum(process_tree_pss) / len(process_tree_pss) if process_tree_pss else None
            ),
            "peak_process_tree_count": max(process_tree_counts) if process_tree_counts else None,
            "peak_process_tree_threads": max(process_tree_threads) if process_tree_threads else None,
            "process_tree_memory_sample_count": len(process_tree_rss),
            "sampled_seconds": sampled_seconds,
        }

    def _release_client_payloads(self, client, iso_name):
        download_root = self.workdir / "downloads" / client
        payloads = [download_root / iso_name] if client == "rtorrent" else [
            download_root / "public" / iso_name,
            download_root / "seed" / iso_name,
        ]
        retained_source = self.seed_source.resolve() if self.seed_source is not None else None
        for path in payloads:
            if not path.is_file() or path.is_symlink():
                continue
            if retained_source is not None and path.resolve() == retained_source:
                continue
            path.unlink()
        leecher_dir = self.workdir / "leecher" / client
        if leecher_dir.exists() and not leecher_dir.is_symlink():
            shutil.rmtree(leecher_dir)

    def _run_one(self, client, meta, tracker_ip, private_path):
        print(f"\n==> {LABELS[client]}: public download", flush=True)
        self.compose("up", "-d", client, quiet=True, timeout=900)
        self.sessions[client] = HttpClient(self.api_url(client)) if client != "rtorrent" else None
        self._wait_ready(client)
        self._configure(client)
        self._remember_container(client)
        self._sample_idle(client)
        self._capture_engine_memory_metrics(client, "idle")
        version = self._version(client)
        torrent = meta["torrent_path"]
        save_path = "/downloads/rtorrent" if client == "rtorrent" else f"/downloads/{client}/public"
        public_result = "FAIL"
        download_error = ""
        download_elapsed = 0.0
        download_progress = 0.0
        download_bytes = 0
        download_sha = ""
        public_rows = []
        try:
            self._add_torrent(client, torrent, save_path)
            elapsed, status, public_rows = self._wait_complete(
                client, meta["info_hash"], meta["iso_name"], meta["size"],
                DOWNLOAD_TIMEOUT, "public_download",
            )
            payload = self._data_path(client, meta["iso_name"])
            if not payload.is_file() or payload.stat().st_size != meta["size"]:
                raise RuntimeError(f"completed file missing or short: {payload}")
            actual = self._sha256(payload)
            if actual != meta["sha256"]:
                raise RuntimeError(f"Debian SHA-256 mismatch: {actual}")
            public_result = "PASS"
            download_elapsed = elapsed
            download_progress = status["progress"]
            download_bytes = meta["size"]
            download_sha = actual
            if self.seed_source is None:
                self.seed_source = payload
        except Exception as error:
            download_error = f"{type(error).__name__}: {error}"
            public_rows = self.partial_download_rows.get(client, public_rows)
            if public_rows:
                latest = public_rows[-1]
                download_elapsed = float(latest[3])
                download_progress = float(latest[4])
                download_bytes = int(meta["size"] * download_progress)
        finally:
            try:
                self._pause(client, meta["info_hash"])
            except Exception:
                pass
            self._capture_engine_memory_metrics(client, "after_public_download")

        seed_result = "NOT RUN"
        seed_error = ""
        seed_elapsed = 0.0
        seed_rows = []
        seed_sha = ""
        seed_setup = 0.0
        if self.seed_source is None:
            seed_error = "no SHA-256-verified ISO source is available"
        else:
            seed_dir = self.workdir / "downloads" / client / "seed"
            if client == "rtorrent":
                seed_dir = self.workdir / "downloads/rtorrent"
            seed_dir.mkdir(parents=True, exist_ok=True)
            if client == "transmission":
                partial = self.workdir / "downloads/incomplete" / f"{meta['iso_name']}.part"
                if partial.is_file() and not partial.is_symlink():
                    partial.unlink()
            seed_payload = seed_dir / meta["iso_name"]
            if seed_payload.resolve() != self.seed_source.resolve():
                try:
                    os.link(self.seed_source, seed_payload)
                except OSError:
                    shutil.copyfile(self.seed_source, seed_payload)
            local_hash = make_private_torrent(
                seed_payload, f"http://{tracker_ip}:6969/announce", private_path,
            )
            try:
                seed_elapsed, seed_rows, seed_sha = self._seed_test(
                    client, seed_payload, private_path, local_hash,
                    meta["size"], meta["sha256"],
                )
                seed_result = "PASS"
            except Exception as error:
                seed_elapsed = self.partial_seed_elapsed.get(client, 0.0)
                seed_rows = self.partial_seed_rows.get(client, [])
                seed_sha = ""
                seed_result = "FAIL"
                seed_error = f"{type(error).__name__}: {error}"
            finally:
                seed_setup = self.partial_seed_setup.get(client, 0.0)
                try:
                    self._pause(client, local_hash)
                except Exception:
                    pass
                self._capture_engine_memory_metrics(client, "after_controlled_seed")

        idle = self._summary(
            [row for row in self.samples if row[1] == client], "idle",
        )
        down = self._summary(public_rows, "public_download")
        seed = self._summary(seed_rows, "controlled_seed")
        seed_bytes = int(meta["size"] * float(seed_rows[-1][4])) if seed_rows else 0
        seed_peer_read_cache_peak_bytes = self._peak_engine_metric(
            client,
            "torrentng_storage_peer_read_cache_bytes",
            {"controlled_seed", "after_controlled_seed"},
        )
        self.results.append({
            "client": client, "version": version, "public": public_result, "seed": seed_result,
            "download_progress": download_progress,
            "download_error": download_error,
            "download_seconds": download_elapsed,
            "download_mib_s": download_bytes / download_elapsed / 1048576 if download_elapsed else 0,
            "download_peak_mib_s": down["peak_rate"] / 1048576,
            "download_mean_cpu": down["mean_cpu"], "download_peak_cpu": down["peak_cpu"],
            "download_peak_memory": down["peak_memory"],
            "download_peak_cgroup_current": down["peak_cgroup_current"],
            "download_peak_cgroup_anon": down["peak_cgroup_anon"],
            "download_peak_cgroup_file": down["peak_cgroup_file"],
            "download_peak_process_rss": down["peak_process_rss"],
            "download_cgroup_cpu_seconds": down["cgroup_cpu_seconds"],
            "download_cpu_seconds_per_gib": per_gib(
                down["cgroup_cpu_seconds"], download_bytes,
            ),
            "download_cgroup_io_read_bytes": down["cgroup_io_read_bytes"],
            "download_cgroup_io_write_bytes": down["cgroup_io_write_bytes"],
            "download_io_read_mib_per_gib": per_gib(
                down["cgroup_io_read_bytes"], download_bytes, 1024 ** 2,
            ),
            "download_io_write_mib_per_gib": per_gib(
                down["cgroup_io_write_bytes"], download_bytes, 1024 ** 2,
            ),
            "idle_mean_memory": idle["mean_memory"],
            "idle_mean_cgroup_anon": idle["mean_cgroup_anon"],
            "download_peak_peers": down["peak_peers"],
            "download_sha": download_sha,
            "seed_seconds": seed_elapsed,
            "seed_setup_seconds": seed_setup,
            "seed_bytes": seed_bytes,
            "seed_mib_s": seed_bytes / seed_elapsed / 1048576 if seed_elapsed else 0,
            "seed_peak_mib_s": seed["peak_rate"] / 1048576,
            "seed_mean_cpu": seed["mean_cpu"], "seed_peak_cpu": seed["peak_cpu"],
            "seed_peak_memory": seed["peak_memory"],
            "seed_peak_cgroup_current": seed["peak_cgroup_current"],
            "seed_peak_cgroup_anon": seed["peak_cgroup_anon"],
            "seed_peak_cgroup_file": seed["peak_cgroup_file"],
            "seed_peak_process_rss": seed["peak_process_rss"],
            "seed_peer_read_cache_peak_bytes": seed_peer_read_cache_peak_bytes,
            "seed_cgroup_cpu_seconds": seed["cgroup_cpu_seconds"],
            "seed_cpu_seconds_per_gib": per_gib(seed["cgroup_cpu_seconds"], seed_bytes),
            "seed_cgroup_io_read_bytes": seed["cgroup_io_read_bytes"],
            "seed_cgroup_io_write_bytes": seed["cgroup_io_write_bytes"],
            "seed_io_read_mib_per_gib": per_gib(
                seed["cgroup_io_read_bytes"], seed_bytes, 1024 ** 2,
            ),
            "seed_io_write_mib_per_gib": per_gib(
                seed["cgroup_io_write_bytes"], seed_bytes, 1024 ** 2,
            ),
            "seed_peak_peers": seed["peak_peers"],
            "seed_sha": seed_sha, "seed_error": seed_error,
        })
        if public_result == "PASS" and seed_result == "PASS":
            print(
                f"[{LABELS[client]}] download {download_bytes / download_elapsed / 1048576:.2f} MiB/s; "
                f"seed {seed_bytes / seed_elapsed / 1048576:.2f} MiB/s; both hashes verified",
                flush=True,
            )
        else:
            print(
                f"[{LABELS[client]}] public {public_result} at {download_progress:.2%}; "
                f"seed {seed_result} after {seed_elapsed:.1f}s "
                f"({download_error or seed_error})",
                flush=True,
            )

    def _write_report(self, meta, client_images):
        self._write_memory_profile()
        load_end = os.getloadavg()
        load_peak = max((float(row[10]) for row in self.samples), default=self.load_start[0])
        config_device, config_type = self._findmnt(self.config_dir)

        def mib(value):
            return f"{value / 1048576:.1f}" if value else "n/a"

        def metric(value, digits=2):
            return f"{value:.{digits}f}" if value is not None else "n/a"

        runtime_overrides = [
            f"Tokio={self.tokio_worker_threads}" if self.tokio_worker_threads else "",
            f"storage I/O={self.storage_io_worker_threads}"
            if self.storage_io_worker_threads else "",
            f"storage hash={self.storage_hash_worker_threads}"
            if self.storage_hash_worker_threads else "",
            f"peer-read readahead={self.storage_peer_read_readahead_bytes}B"
            if self.storage_peer_read_readahead_bytes is not None else "",
            f"peer-read cache={self.storage_peer_read_cache_entries} entries"
            if self.storage_peer_read_cache_entries is not None else "",
            f"peer-read cache budget={self.storage_peer_read_cache_bytes}B"
            if self.storage_peer_read_cache_bytes is not None else "",
        ]
        runtime_overrides = ", ".join(value for value in runtime_overrides if value) or "defaults"
        memory_cap_overrides = ", ".join(
            value for value in (
                f"piece assembly={self.piece_assembly_cap_mb} MiB"
                if self.piece_assembly_cap_mb is not None else "",
                f"peer buffer={self.peer_buffer_cap_mb} MiB"
                if self.peer_buffer_cap_mb is not None else "",
            ) if value
        ) or "config defaults"
        observed_tokio_named_threads = {}
        for snapshot in self.process_memory_snapshots:
            count = snapshot.get("thread_names", {}).get("tokio-rt-worker", 0)
            phase = snapshot.get("phase", "unknown")
            observed_tokio_named_threads[phase] = max(
                observed_tokio_named_threads.get(phase, 0), count
            )
        observed_thread_summary = ", ".join(
            f"{phase}={count}"
            for phase, count in sorted(observed_tokio_named_threads.items())
        ) or "no TorrentNG process snapshots"
        image_description = (
            "- The TorrentNG image was built from the recorded checkout. Reference "
            "images are pinned by digest in deploy/interop/compose.yml."
            if not self.skip_build else
            f"- The TorrentNG image {self.prebuilt_image} carries matching source, fingerprint, "
            "and allocator labels. Reference images are pinned by digest in deploy/interop/compose.yml."
        )
        lines = [
            f"# TorrentNG and reference client performance: {meta['iso_name']}",
            "",
            f"- Started (UTC): {self.started}",
            f"- TorrentNG source commit: {self.source_commit}",
            f"- TorrentNG source tree: {self.source_tree}",
            f"- TorrentNG worktree dirty: {'yes' if self.source_dirty else 'no'}",
            f"- TorrentNG tracked diff SHA-256: {self.source_patch_sha256}",
            f"- TorrentNG untracked files: {self.source_untracked_count}; content SHA-256: {self.source_untracked_sha256}",
            f"- TorrentNG source fingerprint SHA-256: {self.source_fingerprint_sha256}",
            f"- Host: {self.host}; {self.cpu_count} logical CPUs; "
            f"{self.mem_total / 1073741824:.1f} GiB RAM; "
            f"{self.mem_available / 1073741824:.1f} GiB available at start; "
            f"Linux {self.kernel} ({self.machine})",
            f"- Docker: {self.docker_version}; {self.compose_version}",
            f"- Scratch filesystem: {self.scratch_device} ({self.scratch_type})",
            f"- Config filesystem: {config_device} ({config_type})",
            f"- TorrentNG allocator: {self.allocator} ({self.cargo_features})",
            f"- TorrentNG MALLOC_CONF: {self.malloc_conf if self.malloc_conf is not None else 'not applicable'}",
            f"- TorrentNG mimalloc purge delay: {self.mimalloc_purge_delay if self.allocator == 'mimalloc' and self.mimalloc_purge_delay is not None else ('allocator default' if self.allocator == 'mimalloc' else 'not applicable')}",
            f"- TorrentNG mimalloc THP override: {self.mimalloc_allow_thp if self.allocator == 'mimalloc' and self.mimalloc_allow_thp is not None else ('allocator default' if self.allocator == 'mimalloc' else 'not applicable')}",
            f"- TorrentNG build mode: {'prebuilt ' + self.prebuilt_image + ' (source, fingerprint, and allocator verified)' if self.skip_build else 'built from this checkout; image labels verified'}",
            f"- Requested TorrentNG runtime/storage worker overrides: {runtime_overrides}",
            f"- Requested TorrentNG memory-cap overrides: {memory_cap_overrides}",
            "- Peak `tokio-rt-worker`-named threads by phase (async + blocking): "
            f"{observed_thread_summary}",
            f"- Host load average (1/5/15m): "
            f"{self.load_start[0]:.2f}/{self.load_start[1]:.2f}/{self.load_start[2]:.2f} "
            f"at start; {load_end[0]:.2f}/{load_end[1]:.2f}/{load_end[2]:.2f} at end; "
            f"sampled 1m peak {load_peak:.2f}",
            f"- Public source: [official Debian torrent directory]({TORRENT_INDEX}); "
            f"resolved torrent {meta['torrent_url']}",
            f"- ISO: {meta['iso_name']}; {meta['size']:,} bytes; "
            f"torrent infohash {meta['info_hash']}",
            "- Seed fixture page-cache eviction: "
            f"POSIX_FADV_DONTNEED succeeded for "
            f"{sum(self.seed_cache_eviction.values())}/{len(self.seed_cache_eviction)} "
            "seed phases (best-effort kernel hint).",
            f"- Official SHA-256: {meta['sha256']}",
            "- Clients selected for this run: " + ", ".join(LABELS[c] for c in CLIENTS),
            f"- Client download and upload limits: {RATE_BPS:,} B/s "
            f"({RATE_BPS / 1048576:.1f} MiB/s); max peers {MAX_PEERS}",
            f"- Controlled seed leecher receive limit: {SEED_LEECHER_RATE_BPS:,} B/s "
            f"({SEED_LEECHER_RATE_BPS / 1048576:.1f} MiB/s), shared across all seeders. "
            "This stays below TorrentNG's peer-upload guard of 4,096 16 KiB requests "
            "per 10 seconds (about 6.4 MiB/s), with 37.5% headroom. Seed rates are "
            "therefore measured under a controlled receiver load, not as maximum upload capacity.",
            "- Each public download ran alone, in this order: " +
            ", ".join(LABELS[c] for c in CLIENTS),
            "- Container image references: " + "; ".join(
                f"{LABELS[c]}: {client_images.get(c, 'not started')}" for c in CLIENTS
            ) + f"; seed leecher: {client_images.get('seed_leecher', self.seed_leecher_image)}",
            f"- Seed leecher: qBittorrent {self.seed_leecher_version}",
            image_description,
            "",
            "## Public download",
            "",
            "| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |",
            "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|",
        ]
        by_client = {row["client"]: row for row in self.results}
        for client in CLIENTS:
            row = by_client.get(client, {})
            lines.append(
                f"| {LABELS[client]} | {row.get('version', 'n/a')} | "
                f"{row.get('public', 'FAIL')} | {row.get('download_progress', 0):.2%} | "
                f"{row.get('download_seconds', 0):.1f} | "
                f"{row.get('download_mib_s', 0):.2f} | {row.get('download_peak_mib_s', 0):.2f} | "
                f"{row.get('download_mean_cpu', 0):.2f} | {row.get('download_peak_cpu', 0):.2f} | "
                f"{row.get('download_peak_peers', 0)} | "
                f"{row.get('download_sha') or row.get('download_error') or row.get('error', 'n/a')} |"
            )
        lines.extend([
            "",
            "## Controlled seeding",
            "",
            "Each client seeded the same SHA-256-verified ISO through a private single-file "
            "torrent on the isolated Compose OpenTracker. A separate qBittorrent "
            "container with BitTorrent encryption disabled and TCP enabled "
            "downloaded and checked Debian's published SHA-256. This measures a "
            "controlled one-seeder/one-leecher transfer, not public swarm demand. "
            "Seed tests use a separate per-client copy of the first SHA-256-verified "
            "public download or the explicitly verified seed-source file, and can run "
            "even if that client's public-swarm download stalls.",
            "",
            "| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |",
            "|---|---:|---:|---:|---:|---:|---:|---:|---|",
        ])
        for client in CLIENTS:
            row = by_client.get(client, {})
            lines.append(
                f"| {LABELS[client]} | {row.get('seed', 'FAIL')} | "
                f"{row.get('seed_setup_seconds', 0):.1f} | {row.get('seed_seconds', 0):.1f} | "
                f"{row.get('seed_mib_s', 0):.2f} | "
                f"{row.get('seed_peak_mib_s', 0):.2f} | {row.get('seed_mean_cpu', 0):.2f} | "
                f"{row.get('seed_peak_cpu', 0):.2f} | "
                f"{row.get('seed_peak_peers', 0)} | "
                f"{row.get('seed_sha') or row.get('seed_error') or row.get('error', 'not verified')} |"
            )
        lines.extend([
            "",
            "## Memory profile",
            "",
            "Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache, so these columns help separate process/runtime growth from data caching. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.",
            "",
            "| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Peer-read cache sampled peak (MiB) |",
            "|---|---:|---:|---:|---:|---:|---:|---:|",
        ])
        for client in CLIENTS:
            row = by_client.get(client, {})
            cache_bytes = row.get("seed_peer_read_cache_peak_bytes")
            cache_peak = (
                metric(cache_bytes / 1048576, 2) if cache_bytes is not None else "n/a"
            )
            lines.append(
                f"| {LABELS[client]} | {mib(row.get('idle_mean_memory', 0))} | "
                f"{mib(row.get('idle_mean_cgroup_anon', 0))} | "
                f"{mib(row.get('download_peak_memory', 0))} | "
                f"{mib(row.get('download_peak_cgroup_anon', 0))} | "
                f"{mib(row.get('seed_peak_memory', 0))} | "
                f"{mib(row.get('seed_peak_cgroup_anon', 0))} | "
                f"{cache_peak} |"
            )
        lines.extend([
            "",
            "## CPU and block I/O",
            "",
            "Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.",
            "",
            "| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |",
            "|---|---:|---:|---:|---:|---:|---:|",
        ])
        for client in CLIENTS:
            row = by_client.get(client, {})
            lines.append(
                f"| {LABELS[client]} | "
                f"{metric(row.get('download_cpu_seconds_per_gib'))} | "
                f"{metric(row.get('download_io_read_mib_per_gib'))} | "
                f"{metric(row.get('download_io_write_mib_per_gib'))} | "
                f"{metric(row.get('seed_cpu_seconds_per_gib'))} | "
                f"{metric(row.get('seed_io_read_mib_per_gib'))} | "
                f"{metric(row.get('seed_io_write_mib_per_gib'))} |"
            )
        lines.extend([
            "",
            "The tracker is restarted between clients to discard stale peer records.",
            "- After the seeder recognizes the complete fixture and just before each "
            "timed transfer, its cached file pages are advised out so the cgroup "
            "block-read counters are not determined by which client seeded first. "
            "This is a best-effort Linux page-cache hint.",
            "",
            "## Method and scope",
            "",
            "- One current official Debian amd64 netinst v1 torrent per client; clients "
            "ran sequentially on the same host and local scratch filesystem.",
            "- Public downloads used the live Debian swarm. All seed tests used the same "
            "first SHA-256-verified public download, copied into a separate seed path for each client, so "
            "seed performance is measured independently of public-swarm completion.",
            f"- Each client was configured for a {RATE_BPS / 1048576:.1f} MiB/s global "
            f"download and upload ceiling and {MAX_PEERS} global/per-torrent peer ceiling.",
            f"- Sampling interval: {INTERVAL:g} seconds. A successful download average is "
            "verified ISO bytes divided by elapsed wall time; for an incomplete download, "
            "the reported average uses client completed-byte progress and is paired with "
            "the final percentage. The seed "
            "transfer timer starts after both clients are ready, the leecher is "
            "registered, and the seeder has reannounced; leecher setup time is listed "
            "separately. Peak rates come "
            "from client payload counters. Mean/peak CPU is Docker CPU percent, "
            "where 100% represents one logical core; cgroup CPU seconds per GiB "
            "and block read/write bytes per GiB are listed separately. Each client has an idle baseline "
            f"sampled for {IDLE_SAMPLE_SECONDS:g} seconds before adding a torrent. "
            "Cgroup anonymous/file memory and TorrentNG process/engine metrics help "
            "attribute the sampled container footprint. Jemalloc counters appear only "
            "when that allocator is selected and do not represent total process allocations.",
            f"- Public download uses the current public Debian swarm with a {DOWNLOAD_TIMEOUT}s "
            "completion timeout. Controlled seeding "
            "uses a private task-local tracker and one qBittorrent leecher on the same "
            "Compose network. The tracker is reset between clients. The no-peer "
            f"startup limit is {SEED_NO_PEER_TIMEOUT}s; full transfer limit is "
            f"{SEED_TIMEOUT}s.",
            "",
            "## Confidence and limitations",
            "",
            "- Hash validation gives high confidence that successful runs transferred "
            "the same ISO.",
            "- Throughput and resource comparisons have low confidence: "
            "one sequential public-swarm pass and one seed pass per client, on a shared "
            "host with the load reported above. Public peer availability changes over time.",
            "- Results apply to this host, storage, build, image digests, peer ceiling, "
            "rate ceiling, and test window. They do not establish a general ranking, "
            "multi-torrent capacity, or likely public seed demand.",
            "",
            "## Artifacts and cleanup",
            "",
            f"- Raw time-series samples: [{self.csv_path.name}]({self.csv_path.name})",
            f"- Cgroup resource and TorrentNG allocator profile: [{self.memory_profile.name}]({self.memory_profile.name})",
            f"- Preserved source artifacts: {self.artifact_status}",
            f"- Temporary benchmark directory: {self.workdir}",
            f"- Compose cleanup status: **{self.cleanup_status}**",
            f"- Official [Debian netinst page](https://www.debian.org/CD/netinst/)",
            "",
        ])
        self.report.parent.mkdir(parents=True, exist_ok=True)
        self.report.write_text("\n".join(lines))

    def _assert_image_labels(self, image):
        labels, code = run_command(
            [
                "docker", "image", "inspect", "--format",
                '{{index .Config.Labels "org.opencontainers.image.revision"}}|'
                '{{index .Config.Labels "org.torrentng.source.fingerprint"}}|'
                '{{index .Config.Labels "org.torrentng.cargo.features"}}',
                image,
            ],
            env=self.env, check=False, quiet=True,
        )
        if code != 0:
            raise RuntimeError(f"TorrentNG image is not available: {image}")
        actual = labels.strip().split("|", 2)
        expected = [self.source_commit, self.source_fingerprint_sha256, self.cargo_features]
        if actual != expected:
            raise RuntimeError(
                f"TorrentNG image metadata does not match this benchmark checkout: "
                f"{actual!r} != {expected!r}"
            )

    def _use_prebuilt_image(self):
        self._assert_image_labels(self.prebuilt_image)
        run_command(["docker", "image", "tag", self.prebuilt_image, self.image], env=self.env)
        print(
            f"Using prebuilt image {self.prebuilt_image}; source and allocator labels verified",
            flush=True,
        )

    def _idle_profile_order(self):
        rng = random.Random(self.idle_order_seed)
        schedule = [
            (client, profile)
            for client in CLIENTS
            for profile in IDLE_PROFILES
        ]
        rng.shuffle(schedule)
        return schedule

    def _set_idle_dht(self, client, enabled):
        if client == "torrentngd":
            self._torrentng_request(
                "/api/v1/session/features",
                data=json.dumps({"dht": enabled, "pex": False}).encode(),
                headers={"Content-Type": "application/json"},
                method="PUT",
            )
            return
        if client == "qbittorrent":
            preferences = {
                "dht": enabled, "pex": False, "lsd": False,
                "upnp": False, "natpmp": False,
            }
            self._qbit_json(
                client, "/api/v2/app/setPreferences",
                data=urllib.parse.urlencode({"json": json.dumps(preferences)}).encode(),
                headers={"Content-Type": "application/x-www-form-urlencoded"},
            )
        elif client == "transmission":
            self._transmission("session-set", {
                "dht-enabled": enabled, "pex-enabled": False,
                "lpd-enabled": False, "port-forwarding-enabled": False,
            })
        elif client == "deluge":
            self._deluge("core.set_config", [{
                "dht": enabled, "utpex": False, "lsd": False,
            }])
        else:
            self._scgi("protocol.pex.set", "", "no")

    def _read_idle_dht(self, client):
        if client == "torrentngd":
            return json.loads(self._torrentng_request("/api/v1/session/features")).get("dht")
        if client == "qbittorrent":
            return self._qbit_json(client, "/api/v2/app/preferences").get("dht")
        if client == "transmission":
            args = self._transmission("session-get").get("arguments", {})
            return args.get("dht-enabled", args.get("dht_enabled"))
        if client == "deluge":
            config = self._deluge("core.get_config", [["dht"]])
            return config.get("dht") if isinstance(config, dict) else None
        value = self._scgi("dht.mode")
        return str(value).strip().lower() not in ("off", "disabled", "0", "false")

    def _read_idle_pex(self, client):
        if client == "torrentngd":
            return json.loads(self._torrentng_request("/api/v1/session/features")).get("pex")
        if client == "qbittorrent":
            return self._qbit_json(client, "/api/v2/app/preferences").get("pex")
        if client == "transmission":
            args = self._transmission("session-get").get("arguments", {})
            return args.get("pex-enabled", args.get("pex_enabled"))
        if client == "deluge":
            config = self._deluge("core.get_config", [["utpex"]])
            return config.get("utpex") if isinstance(config, dict) else None
        value = self._scgi("protocol.pex")
        return str(value).strip().lower() not in ("off", "disabled", "0", "false", "no")

    def _idle_torrent_count(self, client):
        if client == "torrentngd":
            rows = json.loads(self._torrentng_request("/api/qb/v2/torrents/info")) or []
            return len(rows)
        if client == "qbittorrent":
            return len(self._qbit_json(client, "/api/v2/torrents/info") or [])
        if client == "transmission":
            response = self._transmission("torrent-get", {"fields": ["hashString"]})
            return len(response.get("arguments", {}).get("torrents", []))
        if client == "deluge":
            response = self._deluge("web.update_ui", [["name"], {}]) or {}
            return len(response.get("torrents") or {})
        return len(self._scgi("d.multicall2", "", "main", "d.hash=") or [])

    def _idle_profile_summary(self, client, phase):
        client_samples = [row for row in self.samples if row[1] == client]
        summary = self._summary(client_samples, phase)
        duration = summary.get("sampled_seconds")
        divisor = duration / 60 if duration and duration > 0 else None

        def per_minute(value):
            return value / divisor if value is not None and divisor else None

        summary.update({
            "cgroup_cpu_seconds_per_minute": per_minute(summary["cgroup_cpu_seconds"]),
            "cgroup_cpu_percent_one_core": (
                summary["cgroup_cpu_seconds"] * 100 / duration
                if duration and summary["cgroup_cpu_seconds"] is not None else None
            ),
            "block_read_mib_per_minute": per_minute(
                summary["cgroup_io_read_bytes"] / (1024 * 1024)
                if summary["cgroup_io_read_bytes"] is not None else None
            ),
            "block_write_mib_per_minute": per_minute(
                summary["cgroup_io_write_bytes"] / (1024 * 1024)
                if summary["cgroup_io_write_bytes"] is not None else None
            ),
            "network_rx_mib_per_minute": per_minute(
                summary["network_rx_bytes"] / (1024 * 1024)
                if summary["network_rx_bytes"] is not None else None
            ),
            "network_tx_mib_per_minute": per_minute(
                summary["network_tx_bytes"] / (1024 * 1024)
                if summary["network_tx_bytes"] is not None else None
            ),
        })
        return summary

    def _run_idle_profile(self, client, profile_name):
        profile_spec = IDLE_PROFILES[profile_name]
        profile = profile_spec["label"]
        dht_enabled = profile_spec["dht_enabled"]
        network_mode = profile_spec["network_mode"]
        phase = f"idle_{profile_name}"
        print(f"\n==> {LABELS[client]}: {profile}", flush=True)
        try:
            if client == "torrentngd":
                self._prepare_torrentng_config(dht_enabled=dht_enabled)
            elif client == "rtorrent":
                self._prepare_rtorrent_config(dht_enabled=dht_enabled)
            self.compose("up", "-d", client, quiet=True, timeout=900)
            self.sessions[client] = HttpClient(self.api_url(client)) if client != "rtorrent" else None
            self._wait_ready(client)
            self._configure(client)
            self._set_idle_dht(client, dht_enabled)
            container_id = self.service_id(client)
            if not container_id:
                raise RuntimeError(f"{LABELS[client]} container was not created")
            self.container_ids[client] = container_id
            image, _ = run_command(
                ["docker", "inspect", "-f", "{{.Config.Image}}", container_id],
                env=self.env, check=False, quiet=True,
            )
            self.idle_client_images[client] = image.strip() or "unknown"
            self._remember_container(client)
            actual_dht = self._read_idle_dht(client)
            if not isinstance(actual_dht, bool) or actual_dht != dht_enabled:
                raise RuntimeError(
                    f"{LABELS[client]} DHT readback was {actual_dht!r}; expected {dht_enabled}"
                )
            actual_pex = self._read_idle_pex(client)
            if actual_pex is not False:
                raise RuntimeError(
                    f"{LABELS[client]} PEX readback was {actual_pex!r}; expected false"
                )
            initial_torrent_count = self._idle_torrent_count(client)
            if initial_torrent_count != 0:
                raise RuntimeError(
                    f"{LABELS[client]} started the idle window with "
                    f"{initial_torrent_count} torrents"
                )
            if network_mode == "isolated":
                self._disconnect_container_networks(client, container_id)
            elif not self._container_networks(container_id):
                raise RuntimeError(f"{LABELS[client]} has no attached Docker network")
            time.sleep(IDLE_PROFILE_WARMUP_SECONDS)
            started = time.monotonic()
            while True:
                elapsed = time.monotonic() - started
                self._sample(client, phase, elapsed, {
                    "progress": 0.0, "down": 0, "up": 0, "peers": 0,
                })
                remaining = IDLE_PROFILE_MEASURE_SECONDS - (time.monotonic() - started)
                if remaining <= 0:
                    break
                time.sleep(min(INTERVAL, remaining))
            self._reconnect_container_networks(client)
            if not self._container_networks(container_id):
                raise RuntimeError(f"{LABELS[client]} was not reattached after the idle window")
            summary = self._idle_profile_summary(client, phase)
            torrent_count = self._idle_torrent_count(client)
            final_dht = self._read_idle_dht(client)
            final_pex = self._read_idle_pex(client)
            traffic_observed = (
                None
                if summary["network_rx_bytes"] is None or summary["network_tx_bytes"] is None
                else summary["network_rx_bytes"] + summary["network_tx_bytes"] > 0
            )
            profile_samples = [
                row for row in self.samples if row[1] == client and row[2] == phase
            ]
            result = {
                "client": client,
                "profile": profile,
                "profile_key": profile_name,
                "network_mode": network_mode,
                "network_isolation_verified": network_mode == "isolated",
                "dht_enabled": dht_enabled,
                "dht_readback": final_dht,
                "pex_readback": final_pex,
                "torrent_count": torrent_count,
                "network_traffic_observed": traffic_observed,
                "sample_count": len(profile_samples),
                "status": "PASS" if (
                    torrent_count == 0 and final_dht is dht_enabled and final_pex is False
                    and (network_mode != "isolated"
                         or client not in getattr(self, "detached_networks", {}))
                    and summary.get("sampled_seconds") is not None
                    and len(profile_samples) >= 2
                ) else "FAIL",
                **summary,
            }
            self.idle_results.append(result)
            print(
                f"[{LABELS[client]}] {profile}: "
                f"{summary['cgroup_cpu_seconds_per_minute']} CPU-s/min, "
                f"{summary['peak_cgroup_anon'] / 1048576:.2f} MiB cgroup anon peak, "
                f"{summary['network_rx_bytes']} RX bytes; {result['status']}",
                flush=True,
            )
        finally:
            try:
                self._reconnect_container_networks(client)
            finally:
                self.compose("stop", client, check=False, quiet=True, timeout=60)

    def _write_idle_report(self, client_images):
        def metric(value, digits=3):
            return "n/a" if value is None else f"{value:.{digits}f}"

        def mib(value):
            return "n/a" if value is None else f"{value / 1048576:.2f}"

        profile_order = " → ".join(
            f"{LABELS[row['client']]} ({row.get('network_mode')}, "
            f"{'DHT on' if row.get('dht_enabled') else 'DHT off'})"
            for row in self.idle_results
        ) or "no profiles completed"

        lines = [
            "# TorrentNG and reference client idle resource comparison",
            "",
            f"- Started (UTC): {self.started}",
            f"- Source commit: {self.source_commit}",
            f"- Source tree: {self.source_tree}",
            f"- Source worktree dirty: {str(self.source_dirty).lower()}",
            f"- Source fingerprint SHA-256: {self.source_fingerprint_sha256}",
            f"- Host: {self.host}; {self.cpu_count} logical CPUs; Linux {self.kernel} ({self.machine})",
            f"- Scratch filesystem: {self.scratch_device} ({self.scratch_type})",
            f"- Host load average at start: {self.load_start[0]:.2f}/{self.load_start[1]:.2f}/{self.load_start[2]:.2f}",
            f"- Host load average at end: {os.getloadavg()[0]:.2f}/{os.getloadavg()[1]:.2f}/{os.getloadavg()[2]:.2f}",
            f"- Container images: {json.dumps(client_images, sort_keys=True)}",
            f"- Idle DHT profile order seed: {self.idle_order_seed}",
            f"- Idle profile execution order: {profile_order}",
            f"- Warmup per profile: {IDLE_PROFILE_WARMUP_SECONDS:g}s; measured window: {IDLE_PROFILE_MEASURE_SECONDS:g}s; resource sample interval: {INTERVAL:g}s; process-tree RSS/PSS interval: {IDLE_PROCESS_SAMPLE_SECONDS:g}s",
            "",
            "## Idle resource samples",
            "",
            "Each profile has zero torrents and therefore no connected torrent peers. All profiles disable PEX and read back DHT/PEX state and zero torrents outside measured windows. The isolated profile disables DHT, detaches all Docker network endpoints for the full warmup and measurement window, then reconnects and verifies the original endpoints before post-window API checks. The attached DHT-off profile disables DHT and local discovery where supported; the attached DHT-on profile measures DHT bootstrap and routing maintenance without a torrent, tracker announce, peer search, or payload transfer. Docker healthchecks are disabled during the measured windows, and there are no client API queries during them. Network traffic is recorded for attached profiles; DHT readback does not prove a peer connection and namespace bytes are not attributed specifically to DHT.",
            "",
            "| Client | Profile | Network mode | Isolation verified | DHT readback | PEX readback | RX/TX observed | Status | Samples | CPU s/min | CPU % of one core | Mean/peak process-tree RSS (MiB) | Mean/peak process-tree PSS (MiB) | Mean/peak cgroup anon (MiB) | Mean/peak cgroup current (MiB) | Mean/peak container working set (MiB) | Block read/write (MiB/min) | Network RX/TX (MiB/min) |",
            "|---|---|---|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
        ]
        for row in self.idle_results:
            phase = (
                "idle_isolated" if row.get("network_mode") == "isolated"
                else "idle_dht_on" if row.get("dht_enabled") else "idle_dht_off"
            )
            phase_rows = [
                sample for sample in self.samples
                if sample[1] == row["client"] and sample[2] == phase
            ]
            mean_current = (
                sum(int(sample[11]) for sample in phase_rows) / len(phase_rows)
                if phase_rows else None
            )
            traffic_text = (
                "n/a" if row.get("network_traffic_observed") is None
                else str(row["network_traffic_observed"]).lower()
            )
            isolation_text = (
                "n/a" if row.get("network_mode") != "isolated"
                else str(row.get("network_isolation_verified", False)).lower()
            )
            lines.append(
                f"| {LABELS[row['client']]} | {row['profile']} | {row.get('network_mode')} | {isolation_text} | "
                f"{str(row.get('dht_readback')).lower()} | {str(row.get('pex_readback')).lower()} | "
                f"{traffic_text} | {row['status']} | {row.get('sample_count', 0)} | "
                f"{metric(row.get('cgroup_cpu_seconds_per_minute'))} | {metric(row.get('cgroup_cpu_percent_one_core'))} | "
                f"{mib(row.get('mean_process_tree_rss'))}/{mib(row.get('peak_process_tree_rss'))} | "
                f"{mib(row.get('mean_process_tree_pss'))}/{mib(row.get('peak_process_tree_pss'))} | "
                f"{mib(row.get('mean_cgroup_anon'))}/{mib(row.get('peak_cgroup_anon'))} | "
                f"{mib(mean_current)}/{mib(row.get('peak_cgroup_current'))} | "
                f"{mib(row.get('mean_memory'))}/{mib(row.get('peak_memory'))} | "
                f"{metric(row.get('block_read_mib_per_minute'))}/{metric(row.get('block_write_mib_per_minute'))} | "
                f"{metric(row.get('network_rx_mib_per_minute'))}/{metric(row.get('network_tx_mib_per_minute'))} |"
            )
            if row.get("error"):
                lines.append(f"\nSetup/measurement failure for {LABELS[row['client']]} {row['profile']}: `{row['error']}`\n")
        lines.extend([
            "",
            "## Interpretation and limits",
            "",
            "This is an idle-footprint comparison, separate from transfer CPU-s/GiB, throughput, and transfer I/O. CPU and block/network counters are normalized to elapsed idle minutes. Process-tree RSS sums each host /proc process RSS and can count shared pages more than once; process-tree PSS proportionally accounts for shared pages. Both are sampled less frequently than cgroup counters and are distinct from container working set and cgroup memory. DHT-enabled state and observed namespace bytes do not prove a peer connection, and this workload has no torrent catalog, shares, searches, or user-facing UI activity. Repeat on a quiet, dedicated host before interpreting small differences; results are specific to these image digests, settings, host, and window.",
            "",
            "- Raw time-series samples: " + f"[{self.csv_path.name}]({self.csv_path.name})",
            "- Memory and resource profile: " + f"[{self.memory_profile.name}]({self.memory_profile.name})",
            f"- Compose cleanup status: **{self.cleanup_status}**",
            "",
        ])
        self.report.parent.mkdir(parents=True, exist_ok=True)
        self.report.write_text("\n".join(lines))

    def _run_idle_profiles(self):
        schedule = self._idle_profile_order()
        self.idle_client_images = {}
        for client, profile_name in schedule:
            profile_spec = IDLE_PROFILES[profile_name]
            dht_enabled = profile_spec["dht_enabled"]
            profile = profile_spec["label"]
            phase = f"idle_{profile_name}"
            try:
                self._run_idle_profile(client, profile_name)
            except Exception as error:
                network_restore_failed = client in getattr(self, "detached_networks", {})
                phase_count = sum(
                    row[1] == client and row[2] == phase for row in self.samples
                )
                partial = self._idle_profile_summary(client, phase) if phase_count else {}
                self.idle_results.append({
                    "client": client,
                    "profile": profile,
                    "profile_key": profile_name,
                    "network_mode": profile_spec["network_mode"],
                    "network_isolation_verified": (
                        False if profile_spec["network_mode"] == "isolated" else None
                    ),
                    "dht_enabled": dht_enabled,
                    "dht_readback": None,
                    "pex_readback": None,
                    "torrent_count": None,
                    "network_traffic_observed": None,
                    "sample_count": phase_count,
                    "status": "FAIL",
                    "error": f"{type(error).__name__}: {error}",
                    **partial,
                })
                print(
                    f"[{LABELS[client]}] idle profile failed: {type(error).__name__}: {error}",
                    file=sys.stderr, flush=True,
                )
                if network_restore_failed:
                    self.failure = (
                        f"could not restore Docker network for {LABELS[client]}"
                    )
                    break
        self._write_idle_report(self.idle_client_images)
        return int(not self.idle_results or any(row["status"] != "PASS" for row in self.idle_results))

    def run(self):
        if (RATE_BPS < 1 or MAX_PEERS < 1 or INTERVAL < 1
                or (self.mode == "transfer" and SEED_LEECHER_RATE_BPS < 1)):
            raise RuntimeError("rate ceiling, peer ceiling, and sample interval must be positive")
        if self.mode == "transfer" and (DOWNLOAD_TIMEOUT < 60 or SEED_TIMEOUT < 60):
            raise RuntimeError("download and seed timeouts must be at least 60 seconds")
        if IDLE_SAMPLE_SECONDS < 0:
            raise RuntimeError("idle memory sample duration cannot be negative")
        if self.mode == "idle" and (
            not math.isfinite(IDLE_PROFILE_WARMUP_SECONDS)
            or not math.isfinite(IDLE_PROFILE_MEASURE_SECONDS)
            or not math.isfinite(IDLE_PROCESS_SAMPLE_SECONDS)
            or IDLE_PROFILE_WARMUP_SECONDS < 0
            or IDLE_PROFILE_MEASURE_SECONDS < 10
            or IDLE_PROCESS_SAMPLE_SECONDS <= 0
        ):
            raise RuntimeError(
                "idle warmup must be nonnegative, measurement at least 10 seconds, "
                "and process-tree sample interval positive"
            )
        if self.mode == "transfer" and SEED_NO_PEER_TIMEOUT < 10:
            raise RuntimeError("no-peer timeout must be at least 10 seconds")
        required_commands = ["docker", "findmnt", "git"]
        if self.mode == "transfer":
            required_commands.append("aria2c")
        for required in required_commands:
            if shutil.which(required) is None:
                raise RuntimeError(f"missing required command: {required}")
        print(f"Scratch: {self.workdir}", flush=True)
        print(f"Config scratch: {self.config_dir}", flush=True)
        print(f"Compose project: {self.project}", flush=True)
        self.prepare()
        if "torrentngd" in CLIENTS:
            if self.skip_build:
                self._use_prebuilt_image()
            else:
                self.compose("build", "torrentngd", timeout=3600)
                self._assert_image_labels(self.image)
        if self.mode == "idle":
            return self._run_idle_profiles()
        tracker_ip = self._start_tracker()
        meta = self._torrent_metadata()
        if self.seed_source_override is not None:
            source = self.seed_source_override
            if not source.is_file() or source.stat().st_size != meta["size"]:
                raise RuntimeError(f"seed source is absent or has the wrong size: {source}")
            if self._sha256(source) != meta["sha256"]:
                raise RuntimeError(f"seed source failed Debian SHA-256 verification: {source}")
            self.seed_source = source
        client_images = {}
        for client in CLIENTS:
            private_path = self.workdir / "torrents" / f"private-seed-{client}.torrent"
            try:
                self._run_one(client, meta, tracker_ip, private_path)
            except Exception as error:
                print(f"[{LABELS[client]}] {type(error).__name__}: {error}",
                      file=sys.stderr, flush=True)
                self.results.append({
                    "client": client, "version": "unavailable",
                    "public": "FAIL", "seed": "NOT RUN",
                    "error": f"{type(error).__name__}: {error}",
                })
            finally:
                cid = self.service_id(client)
                if cid:
                    image, _ = run_command(
                        ["docker", "inspect", "-f", "{{.Config.Image}}", cid],
                        env=self.env, check=False, quiet=True,
                    )
                    client_images[client] = image.strip()
                _, stop_code = self.compose(
                    "stop", client, check=False, quiet=True, timeout=60,
                )
                if stop_code == 0:
                    self._release_client_payloads(client, meta["iso_name"])
                self.compose("restart", "opentracker", check=False, quiet=True, timeout=60)
        client_images["seed_leecher"] = self.seed_leecher_image
        if self.artifact_dir is not None:
            try:
                self._preserve_source_artifacts(meta)
            except Exception as error:
                self.artifact_status = f"save failed: {type(error).__name__}: {error}"
                print(f"[artifacts] {self.artifact_status}", file=sys.stderr, flush=True)
        self._write_report(meta, client_images)
        return int(not (
            len(self.results) == len(CLIENTS)
            and all(r.get("public") == "PASS" and r.get("seed") == "PASS" for r in self.results)
        ))

    def cleanup(self):
        if not self.memory_profile.exists():
            try:
                self._write_memory_profile()
            except Exception as error:
                self.cleanup_status += f"; memory profile error: {type(error).__name__}: {error}"
        try:
            self.compose("down", "--volumes", "--remove-orphans",
                         check=False, quiet=True, timeout=180)
            self.cleanup_status = "isolated Compose project, containers, and named volumes removed"
            run_command(["docker", "image", "rm", self.image], env=self.env,
                        check=False, quiet=True, timeout=60)
        except Exception as error:
            self.cleanup_status = f"cleanup error: {type(error).__name__}: {error}"
        try:
            if self.workdir.exists() and not self.workdir.is_symlink():
                shutil.rmtree(self.workdir)
        except Exception as error:
            self.cleanup_status += f"; scratch cleanup error: {type(error).__name__}: {error}"
        if self.config_dir.parent != self.workdir:
            try:
                if self.config_dir.exists() and not self.config_dir.is_symlink():
                    shutil.rmtree(self.config_dir)
            except Exception as error:
                self.cleanup_status += (
                    f"; config scratch cleanup error: {type(error).__name__}: {error}"
                )
        if not self.report.exists():
            self.report.parent.mkdir(parents=True, exist_ok=True)
            lines = [
                "# TorrentNG client performance benchmark — incomplete",
                "",
                f"- Started (UTC): {self.started}",
                f"- Source commit: {self.source_commit}",
                f"- Source tree: {self.source_tree}",
                f"- Host: {self.host}; {self.cpu_count} logical CPUs; Linux {self.kernel} ({self.machine})",
                f"- Host load average at start: {self.load_start[0]:.2f}/{self.load_start[1]:.2f}/{self.load_start[2]:.2f}",
                f"- Failure: {self.failure or 'the run stopped before results were written'}",
                f"- Cleanup: {self.cleanup_status}",
            ]
            if self.csv_path.exists():
                lines.append(f"- Partial samples: [{self.csv_path.name}]({self.csv_path.name})")
            self.report.write_text("\n".join(lines) + "\n")
        if self.report.exists():
            content = self.report.read_text()
            content = re.sub(
                r"- Compose cleanup status: \*\*.*?\*\*",
                f"- Compose cleanup status: **{self.cleanup_status}**",
                content,
            )
            self.report.write_text(content)


def main():
    if len(sys.argv) > 1 and sys.argv[1] in ("-h", "--help"):
        print("""Usage: scripts/client_performance_benchmark.py

By default, runs one current Debian amd64 netinst public download and one
controlled private seed transfer for TorrentNG, qBittorrent, Transmission,
Deluge, and rTorrent. Set TNG_CLIENT_BENCH_MODE=idle to run no-torrent
network-isolated, attached/DHT-off, and attached/DHT-on profiles. Docker
Compose builds the current TorrentNG checkout unless a revision-matched
prebuilt image is selected. Reference client images are pinned by digest in
deploy/interop/compose.yml.

Environment:
  TNG_CLIENT_BENCH_MODE=transfer|idle (default: transfer)
  TNG_CLIENT_BENCH_CLIENTS=torrentngd,qbittorrent,... (default: all five)
  TNG_CLIENT_BENCH_ALLOCATOR=system|jemalloc|mimalloc (default: system)
  TNG_CLIENT_BENCH_SKIP_BUILD=1 (use an image with matching commit, source fingerprint, and allocator labels)
  TNG_CLIENT_BENCH_PREBUILT_IMAGE=torrentng/torrentngd:interop
  TNG_CLIENT_BENCH_SEED_SOURCE=/path/to/verified/debian.iso
  TNG_CLIENT_BENCH_REPORT=/path/to/report.md
  TNG_CLIENT_BENCH_ARTIFACT_DIR=/persistent/path (verified ISO and .torrent)
  TNG_CLIENT_BENCH_SCRATCH_ROOT=/existing/writable/path (default: /var/tmp)
  TNG_CLIENT_BENCH_WORKDIR=/new/path (must not already exist)
  TNG_CLIENT_BENCH_RATE_BPS=33554432 (download and upload ceiling)
  TNG_CLIENT_BENCH_SEED_LEECHER_RATE_BPS=4194304 (shared seed receiver ceiling)
  TNG_CLIENT_BENCH_MAX_PEERS=32
  TNG_CLIENT_BENCH_TIMEOUT_SECS=1800
  TNG_CLIENT_BENCH_SEED_TIMEOUT_SECS=900
  TNG_CLIENT_BENCH_NO_PEER_TIMEOUT_SECS=90
  TNG_CLIENT_BENCH_SAMPLE_SECS=2
  TNG_CLIENT_BENCH_IDLE_SAMPLE_SECS=8
  TNG_CLIENT_BENCH_IDLE_WARMUP_SECS=30 (idle mode only)
  TNG_CLIENT_BENCH_IDLE_MEASURE_SECS=60 (idle mode only; minimum 10)
  TNG_CLIENT_BENCH_IDLE_PROCESS_SAMPLE_SECS=15 (idle process-tree RSS/PSS interval)
  TNG_CLIENT_BENCH_PEER_BIND=0.0.0.0
  TNG_CLIENT_BENCH_MIMALLOC_PURGE_DELAY=0
  TNG_CLIENT_BENCH_MIMALLOC_ALLOW_THP=0
  TNG_CLIENT_BENCH_TOKIO_WORKER_THREADS=2
  TNG_CLIENT_BENCH_STORAGE_IO_WORKER_THREADS=2
  TNG_CLIENT_BENCH_STORAGE_HASH_WORKER_THREADS=1
  TNG_CLIENT_BENCH_STORAGE_PEER_READ_READAHEAD_BYTES=131072
  TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_ENTRIES=16
  TNG_CLIENT_BENCH_PIECE_ASSEMBLY_CAP_MB=0
  TNG_CLIENT_BENCH_PEER_BUFFER_CAP_MB=16

The report and raw CSV remain in the repository; task scratch, the isolated
Compose project, its volumes, and benchmark-tagged images are removed on exit.""")
        return 0
    bench = Benchmark()
    status = 1
    try:
        status = bench.run()
    except KeyboardInterrupt:
        bench.failure = "interrupted"
        print("benchmark interrupted", file=sys.stderr)
        status = 130
    except Exception as error:
        bench.failure = f"{type(error).__name__}: {error}"
        print(f"benchmark failed: {type(error).__name__}: {error}", file=sys.stderr)
        status = 1
    finally:
        bench.cleanup()
    if bench.report.exists():
        print(f"Report: {bench.report}")
    if bench.csv_path.exists():
        print(f"Samples: {bench.csv_path}")
    return status


if __name__ == "__main__":
    raise SystemExit(main())
