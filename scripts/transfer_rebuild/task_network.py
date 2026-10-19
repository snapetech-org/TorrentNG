#!/usr/bin/env python3
"""Create and remove one isolated client/seeder Linux network topology."""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import os
import re
import secrets
import shutil
import stat
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from strict_json import StrictJsonError, loads as strict_json_loads

FORMAT = "transfer-task-network-v1"
SUFFIX_RE = re.compile(r"^[a-f0-9]{8,12}$")
BRIDGE_BASE = "198.18.0.0/15"
INTERFACE_NAME = "tngveth0"
COMMAND_TIMEOUT_SECONDS = 15


class TaskNetworkError(RuntimeError):
    """A task-owned network could not be created or removed safely."""


@dataclass(frozen=True)
class TaskNetwork:
    suffix: str
    target_namespace: str
    seed_namespace: str
    bridge: str
    bridge_ifindex: int
    subnet: str
    gateway: str
    target_address: str
    seed_address: str
    target_interface: str
    target_ifindex: int
    seed_interface: str
    seed_ifindex: int
    target_host_interface: str
    target_host_ifindex: int
    seed_host_interface: str
    seed_host_ifindex: int
    target_namespace_identity: dict[str, int]
    seed_namespace_identity: dict[str, int]

    def as_record(self) -> dict[str, Any]:
        record: dict[str, Any] = {
            "schema_version": FORMAT,
            "suffix": self.suffix,
            "target_namespace": self.target_namespace,
            "seed_namespace": self.seed_namespace,
            "bridge": self.bridge,
            "bridge_ifindex": self.bridge_ifindex,
            "subnet": self.subnet,
            "gateway": self.gateway,
            "target": {
                "address": self.target_address,
                "interface": self.target_interface,
                "ifindex": self.target_ifindex,
                "host_interface": self.target_host_interface,
                "host_ifindex": self.target_host_ifindex,
                "namespace_identity": self.target_namespace_identity,
            },
            "seed": {
                "address": self.seed_address,
                "interface": self.seed_interface,
                "ifindex": self.seed_ifindex,
                "host_interface": self.seed_host_interface,
                "host_ifindex": self.seed_host_ifindex,
                "namespace_identity": self.seed_namespace_identity,
            },
        }
        record["artifact_sha256"] = _record_sha256(record)
        return record


def client_api_context(record: dict[str, Any], role: str) -> dict[str, Any]:
    """Build the ephemeral API endpoint guard for one recorded namespace."""
    if (
        not isinstance(record, dict)
        or record.get("schema_version") != FORMAT
        or record.get("artifact_sha256") != _record_sha256(record)
        or not isinstance(role, str)
        or role not in {"target", "seed"}
    ):
        raise TaskNetworkError("cannot build an API context from an invalid task topology")
    endpoint = record.get(role)
    namespace = record.get(f"{role}_namespace")
    if (
        not isinstance(endpoint, dict)
        or not isinstance(namespace, str)
        or endpoint.get("interface") != INTERFACE_NAME
        or not _positive_index(endpoint.get("ifindex"))
        or not _namespace_identity_record(endpoint.get("namespace_identity"))
    ):
        raise TaskNetworkError(f"task topology has no valid {role} API endpoint")
    return {
        "address": endpoint.get("address"),
        "namespace": namespace,
        "namespace_identity": dict(endpoint["namespace_identity"]),
        "subnet": record.get("subnet"),
        "interface": endpoint["interface"],
        "ifindex": endpoint["ifindex"],
        "topology_sha256": record["artifact_sha256"],
    }


def _canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def _record_sha256(value: dict[str, Any]) -> str:
    unsigned = {key: item for key, item in value.items() if key != "artifact_sha256"}
    return hashlib.sha256(_canonical_json(unsigned)).hexdigest()


def _ip() -> str:
    executable = shutil.which("ip")
    if executable is None:
        raise TaskNetworkError("iproute2 is required to create task network namespaces")
    return executable


def _run(argv: list[str], *, allow_failure: bool = False) -> str:
    try:
        result = subprocess.run(
            argv,
            check=False,
            capture_output=True,
            text=True,
            timeout=COMMAND_TIMEOUT_SECONDS,
            shell=False,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise TaskNetworkError(f"network command failed ({Path(argv[0]).name}): {error}") from error
    if result.returncode != 0 and not allow_failure:
        detail = result.stderr.strip()[-400:]
        raise TaskNetworkError(f"network command failed ({Path(argv[0]).name}): {detail}")
    return result.stdout


def _json_command(argv: list[str]) -> Any:
    try:
        return strict_json_loads(_run(argv), label=f"{Path(argv[0]).name} output")
    except StrictJsonError as error:
        raise TaskNetworkError(f"{Path(argv[0]).name} returned invalid JSON") from error


def _occupied_subnets(ip: str) -> list[ipaddress.IPv4Network]:
    occupied: list[ipaddress.IPv4Network] = []
    routes = _json_command([ip, "-j", "route", "show", "table", "main"])
    if not isinstance(routes, list):
        raise TaskNetworkError("iproute2 route table must be a JSON array")
    for route in routes:
        if not isinstance(route, dict):
            continue
        destination = route.get("dst")
        if not isinstance(destination, str) or destination == "default":
            continue
        try:
            network = ipaddress.ip_network(destination, strict=False)
        except ValueError:
            continue
        if isinstance(network, ipaddress.IPv4Network):
            occupied.append(network)
    addresses = _json_command([ip, "-j", "address", "show"])
    if not isinstance(addresses, list):
        raise TaskNetworkError("iproute2 address list must be a JSON array")
    for interface in addresses:
        info = interface.get("addr_info") if isinstance(interface, dict) else None
        if not isinstance(info, list):
            continue
        for item in info:
            if not isinstance(item, dict) or item.get("family") != "inet":
                continue
            local = item.get("local")
            prefixlen = item.get("prefixlen")
            if not isinstance(local, str) or isinstance(prefixlen, bool) or not isinstance(prefixlen, int):
                continue
            try:
                occupied.append(ipaddress.ip_network(f"{local}/{prefixlen}", strict=False))
            except ValueError:
                continue
    return occupied


def _choose_subnet(ip: str) -> ipaddress.IPv4Network:
    reserved = ipaddress.ip_network(BRIDGE_BASE)
    occupied = _occupied_subnets(ip)
    choices = [
        ipaddress.ip_network((int(reserved.network_address) + index * 256, 24))
        for index in range(reserved.num_addresses // 256)
    ]
    start = secrets.randbelow(len(choices))
    for offset in range(len(choices)):
        candidate = choices[(start + offset) % len(choices)]
        if not any(candidate.overlaps(network) for network in occupied):
            return candidate
    raise TaskNetworkError("no unused /24 remains in the reserved benchmark subnet pool")


def _namespace_identity(name: str) -> dict[str, int]:
    if re.fullmatch(r"tng[ts]-[a-f0-9]{8,12}", name) is None:
        raise TaskNetworkError("namespace name is outside the task-owned naming scheme")
    try:
        parent_fd = os.open(
            "/run/netns",
            os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
        )
    except OSError as error:
        raise TaskNetworkError("/run/netns must be an existing non-symlink directory") from error
    try:
        descriptor = os.open(
            name,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
            dir_fd=parent_fd,
        )
    except OSError as error:
        os.close(parent_fd)
        raise TaskNetworkError(f"cannot open task namespace {name!r} without following links") from error
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise TaskNetworkError("task network namespace handle must be a regular file")
        return {"device": metadata.st_dev, "inode": metadata.st_ino}
    finally:
        os.close(descriptor)
        os.close(parent_fd)


def _positive_index(value: Any) -> bool:
    return not isinstance(value, bool) and isinstance(value, int) and value > 0


def _namespace_identity_record(value: Any) -> bool:
    return (
        isinstance(value, dict)
        and set(value) == {"device", "inode"}
        and not isinstance(value.get("device"), bool)
        and isinstance(value.get("device"), int)
        and value["device"] >= 0
        and _positive_index(value.get("inode"))
    )


def _link_index(ip: str, namespace: str | None, interface: str) -> int:
    prefix = [ip]
    if namespace is not None:
        prefix.extend(["-n", namespace])
    links = _json_command(prefix + ["-j", "-d", "link", "show", "dev", interface])
    if not isinstance(links, list) or len(links) != 1 or not isinstance(links[0], dict):
        raise TaskNetworkError(f"cannot read task interface identity for {interface}")
    if links[0].get("ifname") != interface:
        raise TaskNetworkError("task interface name changed during setup")
    if namespace is not None:
        linkinfo = links[0].get("linkinfo")
        if not isinstance(linkinfo, dict) or linkinfo.get("info_kind") != "veth":
            raise TaskNetworkError("task namespace interface is not the created veth")
    if "UP" not in links[0].get("flags", []):
        raise TaskNetworkError("task interface did not remain up after setup")
    index = links[0].get("ifindex")
    if isinstance(index, bool) or not isinstance(index, int) or index <= 0:
        raise TaskNetworkError("iproute2 returned an invalid task interface index")
    return index


def _host_veth_index(ip: str, interface: str, bridge: str) -> int:
    links = _json_command([ip, "-j", "-d", "link", "show", "dev", interface])
    if not isinstance(links, list) or len(links) != 1 or not isinstance(links[0], dict):
        raise TaskNetworkError(f"cannot read host-side task interface identity for {interface}")
    link = links[0]
    if link.get("ifname") != interface or link.get("master") != bridge:
        raise TaskNetworkError(f"host-side task interface {interface} is no longer attached to its bridge")
    linkinfo = link.get("linkinfo")
    if not isinstance(linkinfo, dict) or linkinfo.get("info_kind") != "veth":
        raise TaskNetworkError(f"host-side task interface {interface} is not a veth")
    if "UP" not in link.get("flags", []):
        raise TaskNetworkError(f"host-side task interface {interface} is down")
    index = link.get("ifindex")
    if isinstance(index, bool) or not isinstance(index, int) or index <= 0:
        raise TaskNetworkError("iproute2 returned an invalid host-side interface index")
    return index


def _connect_veth(ip: str, namespace: str, bridge: str, address: str,
                  host_name: str, peer_name: str) -> tuple[int, int]:
    _run([ip, "link", "add", host_name, "type", "veth", "peer", "name", peer_name])
    try:
        _run([ip, "link", "set", host_name, "master", bridge])
        _run([ip, "link", "set", host_name, "up"])
        _run([ip, "link", "set", peer_name, "netns", namespace])
        _run([ip, "-n", namespace, "link", "set", peer_name, "name", INTERFACE_NAME])
        _run([ip, "-n", namespace, "address", "add", f"{address}/24", "dev", INTERFACE_NAME])
        _run([ip, "-n", namespace, "link", "set", INTERFACE_NAME, "up"])
        _run([ip, "-n", namespace, "link", "set", "lo", "up"])
        return (
            _host_veth_index(ip, host_name, bridge),
            _link_index(ip, namespace, INTERFACE_NAME),
        )
    except BaseException:
        # If the peer has not yet moved, deleting either veth end removes both.
        _run([ip, "link", "del", host_name], allow_failure=True)
        raise


def _assert_isolated_namespace(ip: str, namespace: str) -> None:
    links = _json_command([ip, "-n", namespace, "-j", "link", "show"])
    if (
        not isinstance(links, list)
        or len(links) != 1
        or not isinstance(links[0], dict)
        or links[0].get("ifname") != "lo"
    ):
        raise TaskNetworkError("attached container namespace must contain only loopback before task setup")
    routes = _json_command([ip, "-n", namespace, "-j", "route", "show", "table", "main"])
    if not isinstance(routes, list) or routes:
        raise TaskNetworkError("attached container namespace must have no main-table routes before task setup")


def create_topology(
    suffix: str | None = None,
    *,
    target_pid: int | None = None,
    seed_pid: int | None = None,
) -> TaskNetwork:
    """Create isolated target and seed namespaces connected only by a task bridge.

    Either endpoint may attach to an existing, isolated container namespace.
    Containers start with ``--network none`` before their veth is added. This
    lets paired trials connect native and container clients through the same
    bridge while keeping both namespaces free of default routes.
    """
    if os.geteuid() != 0:
        raise TaskNetworkError("creating task network namespaces requires root")
    suffix = secrets.token_hex(5) if suffix is None else suffix
    if not isinstance(suffix, str) or SUFFIX_RE.fullmatch(suffix) is None:
        raise TaskNetworkError("task network suffix must be 8–12 lowercase hex characters")
    if target_pid is not None and (
        isinstance(target_pid, bool) or not isinstance(target_pid, int) or target_pid <= 0
    ):
        raise TaskNetworkError("container target PID must be a positive integer")
    if seed_pid is not None and (
        isinstance(seed_pid, bool) or not isinstance(seed_pid, int) or seed_pid <= 0
    ):
        raise TaskNetworkError("container seed PID must be a positive integer")
    if target_pid is not None and seed_pid is not None and target_pid == seed_pid:
        raise TaskNetworkError("target and seed containers must use separate processes")
    ip = _ip()
    subnet = _choose_subnet(ip)
    target_namespace = f"tngt-{suffix}"
    seed_namespace = f"tngs-{suffix}"
    bridge = f"tngb{suffix}"
    target_host = f"tngh{suffix}"
    seed_host = f"tngs{suffix}"
    if len({bridge, target_host, seed_host}) != 3:
        raise TaskNetworkError("generated host interface names are not unique")
    if any(
        (Path("/run/netns") / name).exists() or (Path("/run/netns") / name).is_symlink()
        for name in (target_namespace, seed_namespace)
    ):
        raise TaskNetworkError("generated task namespace name is already in use")
    gateway = str(subnet.network_address + 1)
    seed_address = str(subnet.network_address + 2)
    target_address = str(subnet.network_address + 3)

    bridge_created = False
    target_created = False
    seed_created = False
    target_veth_created = False
    seed_veth_created = False
    try:
        _run([ip, "link", "add", bridge, "type", "bridge"])
        bridge_created = True
        _run([ip, "address", "add", f"{gateway}/24", "dev", bridge])
        _run([ip, "link", "set", bridge, "up"])
        if target_pid is None:
            _run([ip, "netns", "add", target_namespace])
            target_created = True
        else:
            _run([ip, "netns", "attach", target_namespace, str(target_pid)])
            target_created = True
            _assert_isolated_namespace(ip, target_namespace)
        if seed_pid is None:
            _run([ip, "netns", "add", seed_namespace])
        else:
            _run([ip, "netns", "attach", seed_namespace, str(seed_pid)])
            _assert_isolated_namespace(ip, seed_namespace)
        seed_created = True
        target_host_ifindex, target_ifindex = _connect_veth(
            ip, target_namespace, bridge, target_address, target_host, f"tgpt{suffix}"
        )
        target_veth_created = True
        seed_host_ifindex, seed_ifindex = _connect_veth(
            ip, seed_namespace, bridge, seed_address, seed_host, f"tgps{suffix}"
        )
        seed_veth_created = True
        bridge_ifindex = _link_index(ip, None, bridge)
        target_identity = _namespace_identity(target_namespace)
        seed_identity = _namespace_identity(seed_namespace)
        return TaskNetwork(
            suffix=suffix,
            target_namespace=target_namespace,
            seed_namespace=seed_namespace,
            bridge=bridge,
            bridge_ifindex=bridge_ifindex,
            subnet=str(subnet),
            gateway=gateway,
            target_address=target_address,
            seed_address=seed_address,
            target_interface=INTERFACE_NAME,
            target_ifindex=target_ifindex,
            seed_interface=INTERFACE_NAME,
            seed_ifindex=seed_ifindex,
            target_host_interface=target_host,
            target_host_ifindex=target_host_ifindex,
            seed_host_interface=seed_host,
            seed_host_ifindex=seed_host_ifindex,
            target_namespace_identity=target_identity,
            seed_namespace_identity=seed_identity,
        )
    except BaseException as error:
        for namespace, created, veth_created in (
            (target_namespace, target_created, target_veth_created),
            (seed_namespace, seed_created, seed_veth_created),
        ):
            if created and veth_created:
                _run([ip, "-n", namespace, "link", "del", INTERFACE_NAME], allow_failure=True)
        for namespace, created in ((target_namespace, target_created), (seed_namespace, seed_created)):
            if created:
                _run([ip, "netns", "del", namespace], allow_failure=True)
        if bridge_created:
            _run([ip, "link", "del", bridge], allow_failure=True)
        if isinstance(error, TaskNetworkError):
            raise
        raise TaskNetworkError(f"task network setup failed: {error}") from error


def _write_new_record(path: Path, record: dict[str, Any]) -> None:
    path = Path(path)
    parent = path.parent
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise TaskNetworkError("topology artifact path must be absolute and normalized")
    try:
        if parent.resolve(strict=True) != parent.absolute() or not parent.is_dir():
            raise TaskNetworkError("topology artifact parent must be an existing non-symlink directory")
    except OSError as error:
        raise TaskNetworkError("topology artifact parent must be an existing directory") from error
    encoded = (json.dumps(record, sort_keys=True, indent=2) + "\n").encode("utf-8")
    directory_fd = _open_directory_nofollow(parent)
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW
    try:
        descriptor = os.open(path.name, flags, 0o600, dir_fd=directory_fd)
    except BaseException:
        os.close(directory_fd)
        raise
    try:
        os.fchmod(descriptor, 0o600)
        with os.fdopen(descriptor, "wb", closefd=False) as output:
            output.write(encoded)
            output.flush()
            os.fsync(descriptor)
        os.close(descriptor)
        descriptor = -1
        os.fsync(directory_fd)
    except BaseException:
        if descriptor >= 0:
            os.close(descriptor)
        try:
            os.unlink(path.name, dir_fd=directory_fd)
        except OSError:
            pass
        raise
    finally:
        os.close(directory_fd)


def _open_directory_nofollow(path: Path) -> int:
    """Open an absolute path one component at a time without following symlinks."""
    if not path.is_absolute() or ".." in path.parts:
        raise TaskNetworkError("directory path must be absolute and normalized")
    descriptor = os.open("/", os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY)
    try:
        for component in path.parts[1:]:
            if component in ("", "."):
                continue
            next_descriptor = os.open(
                component,
                os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
                dir_fd=descriptor,
            )
            os.close(descriptor)
            descriptor = next_descriptor
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _load_record(path: Path) -> dict[str, Any]:
    path = Path(path)
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise TaskNetworkError("topology artifact path must be absolute and normalized")
    directory_fd = _open_directory_nofollow(path.parent)
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK
    try:
        descriptor = os.open(path.name, flags, dir_fd=directory_fd)
    except OSError as error:
        os.close(directory_fd)
        raise TaskNetworkError(f"cannot open task topology record safely: {error}") from error
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 64 * 1024:
            raise TaskNetworkError("task topology record must be a bounded regular non-symlink file")
        chunks: list[bytes] = []
        remaining = 64 * 1024 + 1
        while remaining:
            chunk = os.read(descriptor, min(remaining, 16 * 1024))
            if not chunk:
                break
            chunks.append(chunk)
            remaining -= len(chunk)
        encoded = b"".join(chunks)
        if len(encoded) > 64 * 1024:
            raise TaskNetworkError("task topology record exceeds the size limit")
        value = strict_json_loads(encoded, label="task topology record")
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, StrictJsonError) as error:
        raise TaskNetworkError(f"cannot read task topology record: {error}") from error
    finally:
        os.close(descriptor)
        os.close(directory_fd)
    if not isinstance(value, dict) or value.get("schema_version") != FORMAT:
        raise TaskNetworkError("task topology record has an unsupported format")
    if value.get("artifact_sha256") != _record_sha256(value):
        raise TaskNetworkError("task topology record digest does not match")
    return value


def cleanup_topology(path: Path) -> dict[str, Any]:
    """Remove only the namespaces, links, and bridge recorded by this task."""
    if os.geteuid() != 0:
        raise TaskNetworkError("removing task network namespaces requires root")
    record = _load_record(path)
    target = record.get("target")
    seed = record.get("seed")
    if not isinstance(target, dict) or not isinstance(seed, dict):
        raise TaskNetworkError("task topology record is missing endpoint identities")
    return cleanup_topology_record(record)


def cleanup_topology_record(record: dict[str, Any]) -> dict[str, Any]:
    target = record.get("target")
    seed = record.get("seed")
    target_name = record.get("target_namespace")
    seed_name = record.get("seed_namespace")
    bridge = record.get("bridge")
    if (
        record.get("schema_version") != FORMAT
        or record.get("artifact_sha256") != _record_sha256(record)
        or not isinstance(target, dict)
        or not isinstance(seed, dict)
        or not isinstance(record.get("suffix"), str)
    ):
        raise TaskNetworkError("task topology identity is malformed or has an invalid digest")
    suffix = record["suffix"]
    if SUFFIX_RE.fullmatch(suffix) is None:
        raise TaskNetworkError("task topology suffix is invalid")
    expected_target = f"tngt-{suffix}"
    expected_seed = f"tngs-{suffix}"
    expected_bridge = f"tngb{suffix}"
    if (target_name, seed_name, bridge) != (expected_target, expected_seed, expected_bridge):
        raise TaskNetworkError("task topology names do not match its task-owned suffix")
    subnet_value = record.get("subnet")
    if not isinstance(subnet_value, str):
        raise TaskNetworkError("task topology subnet is malformed")
    try:
        subnet = ipaddress.ip_network(subnet_value, strict=True)
    except ValueError as error:
        raise TaskNetworkError("task topology subnet is malformed") from error
    if (
        not isinstance(subnet, ipaddress.IPv4Network)
        or subnet.prefixlen != 24
        or not subnet.subnet_of(ipaddress.ip_network(BRIDGE_BASE))
    ):
        raise TaskNetworkError("task topology subnet is outside its reserved /24 pool")
    expected_addresses = {
        "gateway": str(subnet.network_address + 1),
        "seed": str(subnet.network_address + 2),
        "target": str(subnet.network_address + 3),
    }
    if record.get("gateway") != expected_addresses["gateway"]:
        raise TaskNetworkError("task topology gateway does not match its subnet")
    for name, endpoint, host_prefix in (
        ("target", target, "tngh"),
        ("seed", seed, "tngs"),
    ):
        if (
            endpoint.get("address") != expected_addresses[name]
            or endpoint.get("interface") != INTERFACE_NAME
            or endpoint.get("host_interface") != f"{host_prefix}{suffix}"
            or not _positive_index(endpoint.get("ifindex"))
            or not _positive_index(endpoint.get("host_ifindex"))
            or not _namespace_identity_record(endpoint.get("namespace_identity"))
        ):
            raise TaskNetworkError(f"{name} endpoint identity is malformed")
    if not _positive_index(record.get("bridge_ifindex")):
        raise TaskNetworkError("task bridge identity is malformed")
    if os.geteuid() != 0:
        raise TaskNetworkError("removing task network namespaces requires root")
    ip = _ip()
    if _namespace_identity(target_name) != target.get("namespace_identity"):
        raise TaskNetworkError("target namespace identity changed; refusing cleanup")
    if _namespace_identity(seed_name) != seed.get("namespace_identity"):
        raise TaskNetworkError("seed namespace identity changed; refusing cleanup")
    if _link_index(ip, None, bridge) != record.get("bridge_ifindex"):
        raise TaskNetworkError("task bridge identity changed; refusing cleanup")
    for endpoint in (target, seed):
        if _host_veth_index(ip, endpoint["host_interface"], bridge) != endpoint["host_ifindex"]:
            raise TaskNetworkError("host-side task interface identity changed; refusing cleanup")
    for namespace, endpoint in ((target_name, target), (seed_name, seed)):
        if _link_index(ip, namespace, endpoint["interface"]) != endpoint.get("ifindex"):
            raise TaskNetworkError(f"task interface identity changed in {namespace}; refusing cleanup")
    seed_pids = _run([ip, "netns", "pids", seed_name]).split()
    if seed_pids:
        raise TaskNetworkError(f"seed namespace {seed_name} still contains processes")
    # The target may be a running container namespace. Remove only the recorded
    # task veth, then detach its /run/netns alias; do not require the container
    # itself to stop just to release the benchmark network.
    _run([ip, "-n", seed_name, "link", "del", seed["interface"]])
    _run([ip, "-n", target_name, "link", "del", target["interface"]])
    _run([ip, "netns", "del", target_name])
    _run([ip, "netns", "del", seed_name])
    _run([ip, "link", "del", bridge])
    return {
        "schema_version": FORMAT,
        "action": "cleanup",
        "topology_sha256": record["artifact_sha256"],
        "cleaned": True,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    create = subparsers.add_parser("create")
    create.add_argument("--suffix")
    create.add_argument("--target-pid", type=int)
    create.add_argument("--seed-pid", type=int)
    create.add_argument("--output", type=Path, required=True)
    cleanup = subparsers.add_parser("cleanup")
    cleanup.add_argument("--topology", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.action == "create":
            topology = create_topology(
                args.suffix,
                target_pid=args.target_pid,
                seed_pid=args.seed_pid,
            )
            record = topology.as_record()
            try:
                _write_new_record(args.output, record)
            except BaseException:
                cleanup_topology_record(record)
                raise
        else:
            record = cleanup_topology(args.topology)
    except (TaskNetworkError, OSError, ValueError) as error:
        print(f"task network: {error}", file=sys.stderr)
        return 2
    print(json.dumps(record, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
