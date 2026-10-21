#!/usr/bin/env python3
"""Native-daemon correctness check. No CPU, RAM, or superiority measurements."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import time
import urllib.error
import urllib.request

BLOCK = 16 * 1024
PIECE = 1024 * 1024
TOKEN = "local-native-wire-check"


def bencode(value):
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    return b"d" + b"".join(bencode(k) + bencode(v) for k, v in sorted(value.items())) + b"e"


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def exact(peer, size):
    data = bytearray()
    while len(data) < size:
        part = peer.recv(size - len(data))
        if not part:
            raise EOFError("peer closed before the complete payload arrived")
        data.extend(part)
    return bytes(data)


def frame(peer):
    size = struct.unpack(">I", exact(peer, 4))[0]
    if size > 2 * PIECE:
        raise RuntimeError("oversized peer frame")
    return exact(peer, size)


def check(binary, output, window):
    output.mkdir(parents=True, exist_ok=False)
    payload = output / "payload.bin"
    piece_hashes = []
    digest = hashlib.sha256()
    with payload.open("wb") as stream:
        for index in range(8):
            data = hashlib.shake_256(b"native-architecture-wire-v1" + struct.pack(">I", index)).digest(PIECE)
            stream.write(data)
            piece_hashes.append(hashlib.sha1(data, usedforsecurity=False).digest())  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BitTorrent v1 piece hashes require SHA-1.
            digest.update(data)
    info = {b"length": payload.stat().st_size, b"name": payload.name.encode(),
            b"piece length": PIECE, b"pieces": b"".join(piece_hashes), b"private": 1}
    info_hash = hashlib.sha1(bencode(info), usedforsecurity=False).digest()  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BitTorrent v1 info hashes require SHA-1.
    api_port, peer_port = port(), port()
    config = output / "config.toml"
    config.write_text(f'''[daemon]
api_bind = "127.0.0.1:{api_port}"
session_dir = {json.dumps(str(output / "session"))}
log_level = "warn"
shutdown_timeout_secs = 2
[network]
listen_port = {peer_port}
max_peers = 32
upload_rate_limit = 33554432
download_rate_limit = 33554432
[storage]
download_dir = {json.dumps(str(output))}
[dht]
enabled = false
bootstrap_nodes = []
[tracker]
allow_loopback_egress = true
[auth]
api_tokens = ["{TOKEN}"]
[logging]
format = "json"
profile = "basic"
filter = "warn"
''')
    env = os.environ.copy()
    env.update(TORRENTNGD_CONFIG=str(config), TOKIO_WORKER_THREADS="2",
               TNG_STORAGE_BACKEND="pread", RUST_LOG="warn")

    def api(path, body=None):
        request = urllib.request.Request(
            f"http://127.0.0.1:{api_port}" + path,
            data=json.dumps(body).encode() if body is not None else None,
            headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=5) as response:  # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected -- The URL is fixed to the test daemon on loopback.
            payload = response.read()
            return json.loads(payload) if payload else None

    with (output / "daemon.log").open("w") as log:
        daemon = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
        try:
            for _ in range(200):
                if daemon.poll() is not None:
                    raise RuntimeError("native daemon exited at startup")
                try:
                    api("/health")
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.1)
            else:
                raise TimeoutError("native daemon readiness")
            torrent = base64.b64encode(bencode({b"info": info})).decode()
            added = api("/api/v1/torrents", {"torrent_b64": torrent, "save_path": str(output), "start": True})
            if added["info_hash"] != info_hash.hex():
                raise RuntimeError("incorrect torrent identity")
            for _ in range(600):
                rows = api("/api/qb/v2/torrents/info?hashes=" + info_hash.hex())
                if rows and rows[0]["progress"] >= 1:
                    break
                time.sleep(0.1)
            else:
                raise TimeoutError("native seed verification")
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                listener.settimeout(20)
                api("/api/v1/torrents/" + info_hash.hex() + "/peers",
                    {"peers": ["127.0.0.1:" + str(listener.getsockname()[1])]})
                peer, _ = listener.accept()
            with peer:
                peer.settimeout(30)
                peer.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                peer.sendall(b"\x13BitTorrent protocol" + b"\0" * 8 + info_hash + b"-AU0001-123456789012")
                if exact(peer, 68)[28:48] != info_hash:
                    raise RuntimeError("incorrect handshake")
                peer.sendall(struct.pack(">IB", 1, 2))
                while True:
                    message = frame(peer)
                    if message and message[0] == 1:
                        break
                size = payload.stat().st_size
                total = size // BLOCK
                sent = 0
                seen = set()
                received = bytearray(size)

                def requests(count):
                    nonlocal sent
                    data = bytearray()
                    for _ in range(min(count, total - sent)):
                        offset = sent * BLOCK
                        data.extend(struct.pack(">IBIII", 13, 6, offset // PIECE, offset % PIECE, BLOCK))
                        sent += 1
                    if data:
                        peer.sendall(data)

                requests(window)
                while len(seen) < total:
                    message = frame(peer)
                    if not message:
                        continue
                    if message[0] == 0:
                        raise RuntimeError("seed choked during transfer")
                    if message[0] != 7:
                        continue
                    piece, begin = struct.unpack(">II", message[1:9])
                    data = message[9:]
                    offset = piece * PIECE + begin
                    index = offset // BLOCK
                    if len(data) != BLOCK or offset % BLOCK or index >= sent or index in seen:
                        raise RuntimeError("invalid, unsolicited, or duplicate block")
                    received[offset:offset + BLOCK] = data
                    seen.add(index)
                    requests(1)
                actual = hashlib.sha256(received).hexdigest()
                if actual != digest.hexdigest():
                    raise RuntimeError("received payload failed SHA-256 verification")
                result = {"status": "PASS", "request_window": window, "refill_blocks": 1,
                          "bytes": size, "blocks": len(seen), "payload_sha256": actual,
                          "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                          "measurement": "correctness only"}
                (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
                return result
        finally:
            if daemon.poll() is None:
                daemon.terminate()
                try:
                    daemon.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    results = [check(args.binary.resolve(), args.output.resolve() / f"window-{window}", window)
               for window in (1, 32)]
    print(json.dumps(results, indent=2))
