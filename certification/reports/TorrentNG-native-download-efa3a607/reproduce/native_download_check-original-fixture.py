#!/usr/bin/env python3
"""Exercise the native download actor with a deterministic BEP 3 seed.

Correctness only: no CPU/RAM comparison. Records requests, HAVE messages,
and a hash of the completed file. A stall is bounded by socket timeout.
"""
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
TOKEN = "native-download-local-check"


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
            raise EOFError("native peer closed")
        data.extend(part)
    return bytes(data)


def frame(peer):
    size = struct.unpack(">I", exact(peer, 4))[0]
    if size > 2 * PIECE:
        raise RuntimeError("oversized frame")
    return exact(peer, size)


def check(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    download = output / "download"
    download.mkdir()
    piece = hashlib.shake_256(b"native-download-piece-v1").digest(PIECE)
    info = {b"length": PIECE * args.pieces, b"name": b"payload.bin", b"piece length": PIECE,
            b"pieces": hashlib.sha1(piece).digest() * args.pieces, b"private": 1}
    info_hash = hashlib.sha1(bencode(info)).digest()
    api_port = port()
    config = output / "config.toml"
    config.write_text(f'''[daemon]
api_bind = "127.0.0.1:{api_port}"
session_dir = {json.dumps(str(output / "session"))}
log_level = "debug"
shutdown_timeout_secs = 2
[network]
listen_port = {port()}
max_peers = 32
download_rate_limit = 33554432
[storage]
download_dir = {json.dumps(str(download))}
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
filter = "warn,rt_engine::torrent_task=debug"
''')
    env = os.environ.copy()
    env.update(TORRENTNGD_CONFIG=str(config), TOKIO_WORKER_THREADS="2",
               TNG_STORAGE_BACKEND="pread", RUST_LOG="warn,rt_engine::torrent_task=debug")

    def api(path, body=None):
        request = urllib.request.Request(f"http://127.0.0.1:{api_port}" + path,
            data=json.dumps(body).encode() if body is not None else None,
            headers={"Authorization": "Bearer " + TOKEN, "Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=5) as response:
            payload = response.read()
            return json.loads(payload) if payload else None

    result = {"status": "FAIL", "measurement": "correctness only", "pieces": args.pieces,
              "piece_bytes": PIECE, "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest()}
    sent = 0
    requested_pieces = set()
    have = set()
    with (output / "daemon.log").open("w") as log, (output / "requests.jsonl").open("w") as requests:
        daemon = subprocess.Popen([str(args.binary.resolve())], env=env, stdout=log, stderr=log)
        try:
            for _ in range(200):
                if daemon.poll() is not None:
                    raise RuntimeError("daemon exited at startup")
                try:
                    api("/health")
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.05)
            else:
                raise TimeoutError("readiness")
            api("/api/v1/torrents", {"torrent_b64": base64.b64encode(bencode({b"info": info})).decode(),
                                     "save_path": str(download), "start": True})
            # Allow the initial absent-file recheck to finish before connecting.
            time.sleep(0.5)
            with socket.socket() as listener:
                listener.bind(("127.0.0.1", 0))
                listener.listen(1)
                listener.settimeout(15)
                api("/api/v1/torrents/" + info_hash.hex() + "/peers",
                    {"peers": ["127.0.0.1:" + str(listener.getsockname()[1])]})
                peer, _ = listener.accept()
            with peer:
                peer.settimeout(10)
                peer.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                peer.sendall(b"\x13BitTorrent protocol" + b"\0" * 8 + info_hash + b"-AU0001-123456789012")
                if exact(peer, 68)[28:48] != info_hash:
                    raise RuntimeError("wrong handshake")
                bits = bytearray([255] * ((args.pieces + 7) // 8))
                if args.pieces % 8:
                    bits[-1] &= 255 << (8 - args.pieces % 8)
                peer.sendall(struct.pack(">IB", 1 + len(bits), 5) + bits + struct.pack(">IB", 1, 1))
                started = time.monotonic()
                next_send = started
                while len(have) < args.pieces:
                    if time.monotonic() - started > args.timeout:
                        raise TimeoutError("whole transfer deadline")
                    msg = frame(peer)
                    if not msg:
                        continue
                    if msg[0] == 4:
                        have.add(struct.unpack(">I", msg[1:])[0])
                    if msg[0] != 6:
                        continue
                    index, begin, length = struct.unpack(">III", msg[1:])
                    if index >= args.pieces or length == 0 or length > BLOCK or begin + length > PIECE:
                        raise RuntimeError("invalid request")
                    requests.write(json.dumps([index, begin, length]) + "\n")
                    requested_pieces.add(index)
                    if args.rate:
                        next_send += length / args.rate
                        time.sleep(max(0, next_send - time.monotonic()))
                    peer.sendall(struct.pack(">IBII", 9 + length, 7, index, begin) + piece[begin:begin + length])
                    sent += length
                expected = hashlib.sha256()
                for _ in range(args.pieces):
                    expected.update(piece)
                actual = hashlib.file_digest((download / "payload.bin").open("rb"), "sha256").hexdigest()
                if actual != expected.hexdigest():
                    raise RuntimeError("completed file hash mismatch")
                result.update(status="PASS", payload_sha256=actual)
        except Exception as error:
            result["error"] = f"{type(error).__name__}: {error}"
        finally:
            result.update(sent_bytes=sent, distinct_requested_pieces=len(requested_pieces), verified_have_count=len(have))
            try:
                result["torrent_status"] = api("/api/qb/v2/torrents/info?hashes=" + info_hash.hex())
            except Exception as error:
                result["status_error"] = str(error)
            if daemon.poll() is None:
                daemon.terminate()
                try:
                    daemon.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()
            (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return result["status"] == "PASS"


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pieces", type=int, default=64)
    parser.add_argument("--rate", type=int, default=4 * 1024 * 1024)
    parser.add_argument("--timeout", type=int, default=45)
    raise SystemExit(0 if check(parser.parse_args()) else 1)
