#!/usr/bin/env python3
"""Apply or remove a pinned egress profile on a task-owned network namespace."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
from pathlib import Path
from typing import Any


FORMAT = "transfer-network-control-v1"
QDISC_HANDLE = "7fff:"
BASELINE_KIND = "noqueue"
BASELINE_HANDLE = "0:"
QUEUE_LIMIT_PACKETS = 1000
MAX_RECORD_BYTES = 64 * 1024
TIMEOUT_SECONDS = 10
NETNS_RE = re.compile(r"^tngt-[a-z0-9][a-z0-9-]{0,31}$")
INTERFACE_RE = re.compile(r"^tngt[a-z0-9][a-z0-9_.-]{0,9}$")
SHA256_RE = re.compile(r"^[a-f0-9]{64}$")

# Fixed, loss-free profiles. Delay is the one-way egress delay on this
# interface; use one record per shaped endpoint when shaping both directions.
# Random loss and jitter are intentionally excluded until their seed and
# readback semantics can be qualified across supported iproute2 versions.
PROFILES: dict[str, dict[str, int]] = {
    "lan-1g-1ms-v1": {
        "delay_ms": 1,
        "rate_bps": 1_000_000_000,
        "limit_packets": QUEUE_LIMIT_PACKETS,
    },
    "metro-100m-10ms-v1": {
        "delay_ms": 10,
        "rate_bps": 100_000_000,
        "limit_packets": QUEUE_LIMIT_PACKETS,
    },
    "wan-20m-30ms-v1": {
        "delay_ms": 30,
        "rate_bps": 20_000_000,
        "limit_packets": QUEUE_LIMIT_PACKETS,
    },
}


class NetworkControlError(ValueError):
    """The requested shaping operation is not safe or not reproducible."""


def _canonical_json(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def _record_digest(record: dict[str, Any]) -> str:
    unsigned = {key: value for key, value in record.items() if key != "artifact_sha256"}
    return hashlib.sha256(_canonical_json(unsigned)).hexdigest()


def _created_at() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace(
        "+00:00", "Z"
    )


def _validate_names(namespace: str, interface: str) -> None:
    if not NETNS_RE.fullmatch(namespace):
        raise NetworkControlError("namespace must be a task-owned tngt-* network namespace")
    if not INTERFACE_RE.fullmatch(interface) or interface == "lo":
        raise NetworkControlError("interface must be a task-owned tngt* interface, never lo")
    if len(interface.encode("utf-8")) > 15:
        raise NetworkControlError("interface name exceeds the Linux 15-byte limit")


def _namespace_identity(namespace: str) -> dict[str, int]:
    path = Path("/run/netns") / namespace
    for parent in (Path("/run"), Path("/run/netns")):
        try:
            if stat.S_ISLNK(parent.lstat().st_mode):
                raise NetworkControlError("network namespace directory must not be a symlink")
        except FileNotFoundError as error:
            raise NetworkControlError("task network namespace does not exist") from error
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
        )
    except OSError as error:
        raise NetworkControlError("cannot open task network namespace without following links") from error
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            raise NetworkControlError("network namespace handle is not a regular namespace file")
        return {"device": info.st_dev, "inode": info.st_ino}
    finally:
        os.close(descriptor)


def _tools() -> tuple[str, str]:
    ip = shutil.which("ip")
    tc = shutil.which("tc")
    if ip is None or tc is None:
        raise NetworkControlError("iproute2 ip and tc commands are required")
    return ip, tc


def _run(argv: list[str]) -> str:
    try:
        result = subprocess.run(
            argv,
            check=True,
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SECONDS,
            shell=False,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise NetworkControlError(f"command failed: {Path(argv[0]).name}: {error}") from error
    if len(result.stdout) > 1024 * 1024:
        raise NetworkControlError("command output exceeded the 1 MiB safety limit")
    return result.stdout


def _in_namespace(ip: str, namespace: str, executable: str, *arguments: str) -> list[str]:
    return [ip, "netns", "exec", namespace, executable, *arguments]


def _link_info(ip: str, namespace: str, interface: str) -> dict[str, Any]:
    output = _run(_in_namespace(ip, namespace, ip, "-j", "-d", "link", "show", "dev", interface))
    try:
        links = json.loads(output)
    except json.JSONDecodeError as error:
        raise NetworkControlError("iproute2 returned invalid link JSON") from error
    if not isinstance(links, list) or len(links) != 1 or not isinstance(links[0], dict):
        raise NetworkControlError("task interface lookup was missing or ambiguous")
    link = links[0]
    if link.get("ifname") != interface or not isinstance(link.get("ifindex"), int):
        raise NetworkControlError("task interface identity did not match the request")
    if "UP" not in link.get("flags", []):
        raise NetworkControlError("task interface must be up before shaping")
    linkinfo = link.get("linkinfo")
    if not isinstance(linkinfo, dict) or linkinfo.get("info_kind") != "veth":
        raise NetworkControlError("only task-owned veth interfaces may be shaped")
    return link


def _root_qdisc(ip: str, tc: str, namespace: str, interface: str) -> dict[str, Any]:
    output = _run(
        _in_namespace(ip, namespace, tc, "-j", "qdisc", "show", "dev", interface)
    )
    try:
        qdiscs = json.loads(output)
    except json.JSONDecodeError as error:
        raise NetworkControlError("tc returned invalid qdisc JSON") from error
    if not isinstance(qdiscs, list):
        raise NetworkControlError("tc qdisc response must be a JSON array")
    roots = [
        item for item in qdiscs
        if isinstance(item, dict) and (item.get("root") is True or "parent" not in item)
    ]
    if len(roots) != 1:
        raise NetworkControlError("task interface must have exactly one root qdisc")
    root = roots[0]
    if not isinstance(root.get("kind"), str) or not isinstance(root.get("handle"), str):
        raise NetworkControlError("root qdisc is missing its kind or handle")
    return root


def _expected_netem(profile_id: str) -> dict[str, Any]:
    profile = PROFILES[profile_id]
    return {
        "kind": "netem",
        "handle": QDISC_HANDLE,
        "delay_ns": profile["delay_ms"] * 1_000_000,
        "rate_bps": profile["rate_bps"],
        "limit_packets": profile["limit_packets"],
    }


def _as_nonnegative_int(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int) and value >= 0:
        return value
    if isinstance(value, str) and value.isdecimal():
        return int(value)
    return None


def _observed_netem(root: dict[str, Any]) -> dict[str, Any]:
    options = root.get("options")
    if not isinstance(options, dict):
        raise NetworkControlError("tc netem qdisc has no readback options")
    delay = options.get("delay")
    if isinstance(delay, dict):
        delay = delay.get("delay")
    if isinstance(delay, bool):
        delay_ns = None
    elif isinstance(delay, (int, float)) and delay >= 0:
        delay_ns = int(round(delay * 1_000_000_000))
    else:
        delay_ns = _as_nonnegative_int(delay)
    rate = options.get("rate")
    if isinstance(rate, dict):
        rate = rate.get("rate64", rate.get("rate", rate.get("value")))
    rate_bps = _as_nonnegative_int(rate)
    limit = _as_nonnegative_int(options.get("limit"))
    if delay_ns is None or rate_bps is None or limit is None:
        raise NetworkControlError("tc netem readback omitted delay, rate, or queue limit")
    return {
        "kind": root["kind"],
        "handle": root["handle"],
        "delay_ns": delay_ns,
        "rate_bps": rate_bps,
        "limit_packets": limit,
    }


def _assert_expected_qdisc(root: dict[str, Any], expected: dict[str, Any]) -> None:
    observed = _observed_netem(root)
    if observed != expected:
        raise NetworkControlError(
            "tc netem readback differs from the pinned profile: "
            + json.dumps(observed, sort_keys=True)
        )


def _write_record(path: Path, record: dict[str, Any]) -> bytes:
    path = Path(path)
    parent = path.parent
    try:
        if parent.resolve(strict=True) != parent.absolute() or not parent.is_dir():
            raise NetworkControlError("report parent must be an existing non-symlink directory")
    except OSError as error:
        raise NetworkControlError("report parent must be an existing directory") from error
    record = dict(record)
    record["artifact_sha256"] = _record_digest(record)
    payload = (json.dumps(record, indent=2, sort_keys=True) + "\n").encode("utf-8")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    flags |= getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags, 0o600)
    except OSError as error:
        raise NetworkControlError(f"cannot create immutable report {path}: {error}") from error
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb", closefd=False) as target:
            target.write(payload)
            target.flush()
            os.fsync(descriptor)
        directory_fd = os.open(parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    except BaseException:
        os.close(descriptor)
        try:
            path.unlink()
        except OSError:
            pass
        raise
    else:
        os.close(descriptor)
    return payload


def _read_record(path: Path) -> tuple[dict[str, Any], bytes]:
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
        )
    except OSError as error:
        raise NetworkControlError("cannot read applied report without following links") from error
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_RECORD_BYTES:
            raise NetworkControlError("applied report must be a bounded regular file")
        data = bytearray()
        while len(data) <= MAX_RECORD_BYTES:
            chunk = os.read(descriptor, min(4096, MAX_RECORD_BYTES + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
        if len(data) > MAX_RECORD_BYTES:
            raise NetworkControlError("applied report exceeds the 64 KiB safety limit")
    finally:
        os.close(descriptor)
    try:
        record = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise NetworkControlError("applied report is not valid JSON") from error
    if not isinstance(record, dict):
        raise NetworkControlError("applied report must be a JSON object")
    return record, bytes(data)


def _validate_applied_record(record: dict[str, Any]) -> None:
    if record.get("format") != FORMAT or record.get("action") != "apply":
        raise NetworkControlError("reset requires a transfer-network-control apply report")
    if record.get("artifact_sha256") != _record_digest(record):
        raise NetworkControlError("applied report self-digest does not match")
    profile_id = record.get("profile_id")
    if profile_id not in PROFILES:
        raise NetworkControlError("applied report has an unknown profile")
    if record.get("settings") != PROFILES[profile_id]:
        raise NetworkControlError("applied report settings differ from the pinned profile")
    if record.get("observed_root_qdisc") != _expected_netem(profile_id):
        raise NetworkControlError("applied report does not contain the expected qdisc readback")
    if record.get("baseline_root_qdisc") != {"kind": BASELINE_KIND, "handle": BASELINE_HANDLE}:
        raise NetworkControlError("applied report baseline is not the supported noqueue default")
    if not SHA256_RE.fullmatch(str(record.get("toolchain_sha256", ""))):
        raise NetworkControlError("applied report is missing its iproute2 identity digest")
    versions = {"ip": record.get("ip_version"), "tc": record.get("tc_version")}
    if not all(isinstance(value, str) and value for value in versions.values()):
        raise NetworkControlError("applied report is missing iproute2 version strings")
    expected_toolchain = hashlib.sha256(_canonical_json(versions)).hexdigest()
    if record["toolchain_sha256"] != expected_toolchain:
        raise NetworkControlError("applied report iproute2 identity digest does not match")


def _toolchain_identity(ip: str, tc: str) -> tuple[str, str, str]:
    ip_version = _run([ip, "-Version"]).strip()
    tc_version = _run([tc, "-Version"]).strip()
    digest = hashlib.sha256(_canonical_json({"ip": ip_version, "tc": tc_version})).hexdigest()
    return ip_version, tc_version, digest


def _cleanup_if_owned(ip: str, tc: str, namespace: str, interface: str,
                      expected: dict[str, Any], namespace_id: dict[str, int],
                      ifindex: int) -> None:
    if _namespace_identity(namespace) != namespace_id:
        raise NetworkControlError("rollback refused because the task namespace changed")
    if _link_info(ip, namespace, interface).get("ifindex") != ifindex:
        raise NetworkControlError("rollback refused because the task interface changed")
    current = _root_qdisc(ip, tc, namespace, interface)
    if current.get("kind") != "netem" or current.get("handle") != QDISC_HANDLE:
        return
    if _observed_netem(current) != expected:
        raise NetworkControlError("rollback refused because the qdisc no longer matches this apply")
    _run(_in_namespace(
        ip, namespace, tc, "qdisc", "del", "dev", interface,
        "root", "handle", QDISC_HANDLE,
    ))
    restored = _root_qdisc(ip, tc, namespace, interface)
    if restored.get("kind") != BASELINE_KIND or restored.get("handle") != BASELINE_HANDLE:
        raise NetworkControlError("rollback did not restore the default noqueue root qdisc")


def apply_profile(namespace: str, interface: str, profile_id: str,
                  output: Path) -> dict[str, Any]:
    _validate_names(namespace, interface)
    if profile_id not in PROFILES:
        raise NetworkControlError(f"unknown pinned network profile: {profile_id}")
    ip, tc = _tools()
    namespace_id = _namespace_identity(namespace)
    link = _link_info(ip, namespace, interface)
    before = _root_qdisc(ip, tc, namespace, interface)
    if before.get("kind") != BASELINE_KIND or before.get("handle") != BASELINE_HANDLE:
        raise NetworkControlError("refusing to replace a non-default root qdisc")

    settings = dict(PROFILES[profile_id])
    expected = _expected_netem(profile_id)
    ip_version, tc_version, toolchain_sha256 = _toolchain_identity(ip, tc)
    argv = _in_namespace(
        ip,
        namespace,
        tc,
        "qdisc", "add", "dev", interface,
        "root", "handle", QDISC_HANDLE,
        "netem", "limit", str(settings["limit_packets"]),
        "delay", f"{settings['delay_ms']}ms",
        "rate", f"{settings['rate_bps']}bit",
    )
    try:
        _run(argv)
        after_link = _link_info(ip, namespace, interface)
        if after_link.get("ifindex") != link.get("ifindex"):
            raise NetworkControlError("task interface was replaced while shaping")
        after = _root_qdisc(ip, tc, namespace, interface)
        _assert_expected_qdisc(after, expected)
        if _namespace_identity(namespace) != namespace_id:
            raise NetworkControlError("task network namespace changed while shaping")
        record = {
            "format": FORMAT,
            "action": "apply",
            "created_at": _created_at(),
            "namespace_name": namespace,
            "namespace_device": namespace_id["device"],
            "namespace_inode": namespace_id["inode"],
            "interface": interface,
            "ifindex": link["ifindex"],
            "profile_id": profile_id,
            "settings": settings,
            "baseline_root_qdisc": {"kind": BASELINE_KIND, "handle": BASELINE_HANDLE},
            "observed_root_qdisc": expected,
            "ip_version": ip_version,
            "tc_version": tc_version,
            "toolchain_sha256": toolchain_sha256,
            "applied_record_sha256": None,
        }
        _write_record(output, record)
        record["artifact_sha256"] = _record_digest(record)
        return record
    except BaseException as original_error:
        try:
            _cleanup_if_owned(
                ip, tc, namespace, interface, expected, namespace_id, link["ifindex"]
            )
        except (NetworkControlError, OSError) as cleanup_error:
            raise NetworkControlError(
                f"apply failed ({original_error}) and qdisc rollback failed ({cleanup_error})"
            ) from cleanup_error
        raise


def reset_profile(applied_report: Path, output: Path) -> dict[str, Any]:
    applied, raw = _read_record(applied_report)
    _validate_applied_record(applied)
    namespace = applied.get("namespace_name")
    interface = applied.get("interface")
    if not isinstance(namespace, str) or not isinstance(interface, str):
        raise NetworkControlError("applied report is missing namespace or interface identity")
    _validate_names(namespace, interface)
    ip, tc = _tools()
    namespace_id = _namespace_identity(namespace)
    if namespace_id != {
        "device": applied.get("namespace_device"),
        "inode": applied.get("namespace_inode"),
    }:
        raise NetworkControlError("task namespace identity changed since the profile was applied")
    link = _link_info(ip, namespace, interface)
    if link.get("ifindex") != applied.get("ifindex"):
        raise NetworkControlError("task interface identity changed since the profile was applied")
    current = _root_qdisc(ip, tc, namespace, interface)
    _assert_expected_qdisc(current, _expected_netem(applied["profile_id"]))
    _run(_in_namespace(
        ip, namespace, tc, "qdisc", "del", "dev", interface,
        "root", "handle", QDISC_HANDLE,
    ))
    restored = _root_qdisc(ip, tc, namespace, interface)
    if restored.get("kind") != BASELINE_KIND or restored.get("handle") != BASELINE_HANDLE:
        raise NetworkControlError("reset did not restore the default noqueue root qdisc")
    if _namespace_identity(namespace) != namespace_id:
        raise NetworkControlError("task network namespace changed during reset")
    ip_version, tc_version, toolchain_sha256 = _toolchain_identity(ip, tc)
    record = {
        "format": FORMAT,
        "action": "reset",
        "created_at": _created_at(),
        "namespace_name": namespace,
        "namespace_device": namespace_id["device"],
        "namespace_inode": namespace_id["inode"],
        "interface": interface,
        "ifindex": link["ifindex"],
        "profile_id": applied["profile_id"],
        "settings": dict(PROFILES[applied["profile_id"]]),
        "baseline_root_qdisc": {"kind": BASELINE_KIND, "handle": BASELINE_HANDLE},
        "observed_root_qdisc": {"kind": BASELINE_KIND, "handle": BASELINE_HANDLE},
        "ip_version": ip_version,
        "tc_version": tc_version,
        "toolchain_sha256": toolchain_sha256,
        "applied_record_sha256": hashlib.sha256(raw).hexdigest(),
    }
    _write_record(output, record)
    record["artifact_sha256"] = _record_digest(record)
    return record


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    apply_parser = subparsers.add_parser("apply", help="apply one pinned profile")
    apply_parser.add_argument("--namespace", required=True)
    apply_parser.add_argument("--interface", required=True)
    apply_parser.add_argument("--profile", choices=sorted(PROFILES), required=True)
    apply_parser.add_argument("--output", type=Path, required=True)
    reset_parser = subparsers.add_parser("reset", help="remove the exact applied profile")
    reset_parser.add_argument("--applied-report", type=Path, required=True)
    reset_parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.action == "apply":
            record = apply_profile(args.namespace, args.interface, args.profile, args.output)
        else:
            record = reset_profile(args.applied_report, args.output)
    except (NetworkControlError, OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(record, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
