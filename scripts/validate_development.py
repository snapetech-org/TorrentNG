#!/usr/bin/env python3
"""Run TorrentNG's native checks with source-bound plans and live progress."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import queue
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Iterable

if __package__:
    from scripts.validation_engine.trusted_sandbox import require_trusted_local_test_sandbox
else:
    from validation_engine.trusted_sandbox import require_trusted_local_test_sandbox


ROOT = Path(__file__).resolve().parents[1]
REPOSITORY = "torrentng"
ENGINE_VERSION = 1
PROGRESS_INTERVAL_SECONDS = 60
MAX_OUTPUT_BYTES = 8 * 1024 * 1024
MAX_LOG_BYTES = 16 * 1024 * 1024
LOG_TRUNCATION_MARKER = b"\n[validation log truncated after 16 MiB]\n"
IGNORED_PARTS = {".git", "target", "node_modules", "dist", "coverage"}
INHERITED_ENVIRONMENT_KEYS = {
    "ASDF_DATA_DIR", "ASDF_DIR", "COMSPEC", "LANG", "LC_ALL", "LC_CTYPE",
    "MISE_DATA_DIR", "MISE_HOME", "PATH", "PATHEXT", "PYENV_ROOT",
    "RUSTUP_HOME", "SYSTEMROOT", "TZ", "WINDIR",
}
APPROVED_TASK_PASS_ENVIRONMENT: dict[str, frozenset[str]] = {
    "workspace-format": frozenset({"RUSTUP_TOOLCHAIN"}),
    "sidecar-format": frozenset({"RUSTUP_TOOLCHAIN"}),
    "workspace-tests": frozenset({"RUSTUP_TOOLCHAIN"}),
    "workspace-clippy": frozenset({"RUSTUP_TOOLCHAIN"}),
    "sidecar-tests": frozenset({"RUSTUP_TOOLCHAIN"}),
    "sidecar-clippy": frozenset({"RUSTUP_TOOLCHAIN"}),
}


TASKS: tuple[dict[str, Any], ...] = (
    {
        "id": "workspace-format",
        "name": "Format TorrentNG workspace",
        "kind": "check",
        "command": ["cargo", "fmt", "--all", "--", "--check"],
        "pass_env": ["RUSTUP_TOOLCHAIN"],
        "lanes": [],
        "timeout": 300,
    },
    {
        "id": "sidecar-format",
        "name": "Format compatible-client service",
        "kind": "check",
        "command": ["cargo", "fmt", "--manifest-path", "sidecar/Cargo.toml", "--", "--check"],
        "pass_env": ["RUSTUP_TOOLCHAIN"],
        "lanes": [],
        "timeout": 300,
    },
    {
        "id": "api-contract",
        "name": "Validate TorrentNG API contract",
        "kind": "check",
        "command": ["python3", "scripts/validate_openapi.py"],
        "lanes": [],
        "timeout": 300,
    },
    {
        "id": "python-contract-tests",
        "name": "Python policy, load-runner, workflow, and engine tests",
        "kind": "test",
        "command": [
            "python3", "-m", "unittest",
            "scripts.test_protected_target",
            "scripts.test_backend_burndown_api_load",
            "scripts.test_workflow_security",
            "scripts.test_release_notes",
            "scripts.test_validation_engine",
        ],
        "case_parser": "python-unittest",
        "minimum_cases": 1,
        "lanes": ["python-tests", "validation-engine"],
        "timeout": 900,
    },
    {
        "id": "workspace-tests",
        "name": "Test TorrentNG client workspace",
        "kind": "test",
        "command": [
            "cargo", "test", "--workspace", "--all-targets", "--locked",
            "--", "--test-threads=1",
        ],
        "pass_env": ["RUSTUP_TOOLCHAIN"],
        "case_parser": "cargo-test",
        "minimum_cases": 1,
        "lanes": ["rust-workspace-tests"],
        "timeout": 3600,
    },
    {
        "id": "workspace-clippy",
        "name": "Lint TorrentNG client workspace",
        "kind": "check",
        "command": ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
        "pass_env": ["RUSTUP_TOOLCHAIN"],
        "lanes": [],
        "timeout": 3600,
    },
    {
        "id": "sidecar-tests",
        "name": "Test compatible-client service",
        "kind": "test",
        "command": [
            "cargo", "test", "--locked", "--manifest-path", "sidecar/Cargo.toml",
            "--", "--test-threads=1",
        ],
        "pass_env": ["RUSTUP_TOOLCHAIN"],
        "case_parser": "cargo-test",
        "minimum_cases": 1,
        "lanes": ["rust-workspace-tests"],
        "timeout": 2400,
    },
    {
        "id": "sidecar-clippy",
        "name": "Lint compatible-client service",
        "kind": "check",
        "command": [
            "cargo", "clippy", "--all-targets", "--locked", "--manifest-path",
            "sidecar/Cargo.toml", "--", "-D", "warnings",
        ],
        "pass_env": ["RUSTUP_TOOLCHAIN"],
        "lanes": [],
        "timeout": 2400,
    },
    {
        "id": "desktop-tests",
        "name": "Test TorrentNG Desktop",
        "kind": "test",
        "command": [
            "cargo", "test", "--locked", "--manifest-path",
            "crates/torrentng-desktop/Cargo.toml", "--", "--test-threads=1",
        ],
        "case_parser": "cargo-test",
        "minimum_cases": 1,
        "lanes": ["rust-workspace-tests"],
        "timeout": 2400,
    },
    {
        "id": "desktop-clippy",
        "name": "Lint TorrentNG Desktop",
        "kind": "check",
        "command": [
            "cargo", "clippy", "--all-targets", "--locked", "--manifest-path",
            "crates/torrentng-desktop/Cargo.toml", "--", "-D", "warnings",
        ],
        "lanes": [],
        "timeout": 2400,
    },
    {
        "id": "webui-tests",
        "name": "Test TorrentNG WebUI",
        "kind": "test",
        "command": ["npm", "test"],
        "cwd": "webui",
        "case_parser": "vitest",
        "minimum_cases": 1,
        "lanes": ["webui-tests"],
        "timeout": 1800,
    },
    {
        "id": "webui-build",
        "name": "Build TorrentNG WebUI",
        "kind": "build",
        "command": ["npm", "run", "build"],
        "cwd": "webui",
        "lanes": [],
        "timeout": 1800,
    },
    {
        "id": "webui-lint",
        "name": "Lint TorrentNG WebUI",
        "kind": "check",
        "command": ["npm", "run", "lint"],
        "cwd": "webui",
        "lanes": [],
        "timeout": 1800,
    },
    {
        "id": "certification-policy-selftest",
        "name": "Certification policy self-test",
        "kind": "test",
        "command": ["bash", "scripts/certification_policy_selftest.sh"],
        "lanes": ["certification-selftests"],
        "timeout": 900,
    },
    {
        "id": "certification-bundle-selftest",
        "name": "Certification bundle self-test",
        "kind": "test",
        "command": ["bash", "scripts/certification_bundle_selftest.sh"],
        "lanes": ["certification-selftests"],
        "timeout": 900,
    },
    {
        "id": "storage-certification-selftest",
        "name": "Storage certification self-test",
        "kind": "test",
        "command": ["bash", "scripts/storage_certification_selftest.sh"],
        "lanes": ["certification-selftests"],
        "timeout": 900,
        "default_selected": False,
        "exclusion_reason": "This self-test verifies the clean-source release gate and is run explicitly by CI on a clean checkout.",
    },
)


class ValidationError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def git(root: Path, *args: str) -> bytes:
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode != 0:
        raise ValidationError(result.stderr.decode("utf-8", "replace").strip() or "git command failed")
    return result.stdout


def candidate_snapshot(root: Path) -> dict[str, str]:
    """Fingerprint HEAD plus staged, unstaged, and untracked working-tree changes."""
    commit = git(root, "rev-parse", "HEAD").decode("ascii").strip()
    digest = hashlib.sha256()
    digest.update(b"HEAD\0" + commit.encode("ascii") + b"\0")
    diff = git(root, "diff", "--binary", "HEAD", "--")
    digest.update(b"DIFF\0" + diff)
    untracked = git(root, "ls-files", "--others", "--exclude-standard", "-z")
    for raw_path in sorted(item for item in untracked.split(b"\0") if item):
        relative = os.fsdecode(raw_path)
        path = root / relative
        digest.update(b"UNTRACKED\0" + raw_path + b"\0")
        if path.is_symlink():
            digest.update(b"SYMLINK\0" + os.fsencode(os.readlink(path)))
        elif path.is_file():
            digest.update(path.read_bytes())
        else:
            digest.update(b"MISSING")
        digest.update(b"\0")
    return {"commit": commit, "source_sha256": digest.hexdigest()}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(128 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _visible_files(directory: Path) -> Iterable[Path]:
    if not directory.is_dir():
        return
    for current, directories, filenames in os.walk(directory, followlinks=False):
        current_path = Path(current)
        directories[:] = sorted(
            name for name in directories
            if name not in IGNORED_PARTS and not (current_path / name).is_symlink()
        )
        for filename in sorted(filenames):
            path = current_path / filename
            if path.is_file() and not path.is_symlink():
                yield path


def discover_inventory(root: Path = ROOT) -> dict[str, list[dict[str, str]]]:
    """Record native test sources without imposing a second test framework."""
    lanes: dict[str, list[dict[str, str]]] = {
        "validation-engine": [],
        "python-tests": [],
        "rust-workspace-tests": [],
        "webui-tests": [],
        "certification-selftests": [],
    }
    for base in (root / "scripts", root / "crates", root / "sidecar", root / "webui"):
        for path in _visible_files(base):
            relative = path.relative_to(root).as_posix()
            parts = Path(relative).parts
            contents: bytes | None = None
            if relative in {"scripts/validate_development.py", "scripts/test_validation_engine.py"}:
                lanes["validation-engine"].append({"file": relative, "sha256": sha256_file(path)})
            if relative.startswith("scripts/test_") and relative.endswith(".py") and relative != "scripts/test_validation_engine.py":
                lanes["python-tests"].append({"file": relative, "sha256": sha256_file(path)})
            if relative.startswith("scripts/") and path.name.endswith("selftest.sh"):
                lanes["certification-selftests"].append({"file": relative, "sha256": sha256_file(path)})
            if path.suffix == ".rs" and ("crates" in parts or "sidecar" in parts):
                if "tests" in parts:
                    is_test = True
                else:
                    contents = path.read_bytes()
                    is_test = re.search(rb"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", contents) is not None
                if is_test:
                    lanes["rust-workspace-tests"].append({"file": relative, "sha256": sha256_file(path)})
            if relative.startswith(("webui/src/", "webui/e2e/")) and path.suffix in {".ts", ".tsx", ".js", ".jsx"}:
                if re.search(r"(?:\.test|\.spec)\.[^.]+$", path.name):
                    lanes["webui-tests"].append({"file": relative, "sha256": sha256_file(path)})
    for entries in lanes.values():
        entries.sort(key=lambda item: item["file"])
    return lanes


def task_contract(task: dict[str, Any], selected: bool) -> dict[str, Any]:
    return {
        "id": task["id"],
        "name": task["name"],
        "kind": task["kind"],
        "command": list(task["command"]),
        "cwd": task.get("cwd", "."),
        "timeout": task.get("timeout", 3600),
        "case_parser": task.get("case_parser"),
        "minimum_cases": task.get("minimum_cases"),
        "lanes": list(task.get("lanes", [])),
        "depends_on": list(task.get("depends_on", [])),
        "exclusion_reason": task.get("exclusion_reason"),
        "selected": selected,
    }


def _selection(tasks: tuple[dict[str, Any], ...], tests_only: bool, task_id: str | None) -> set[str]:
    by_id = {task["id"]: task for task in tasks}
    if task_id is not None and task_id not in by_id:
        raise ValidationError(f"Unknown validation task: {task_id}")
    selected = {task_id} if task_id else {
        task["id"] for task in tasks
        if task.get("default_selected", True) and (not tests_only or task["kind"] == "test")
    }
    visiting: set[str] = set()
    visited: set[str] = set()

    def include(task_name: str) -> None:
        if task_name in visiting:
            raise ValidationError(f"Validation task dependency cycle at {task_name}")
        if task_name in visited:
            return
        task = by_id.get(task_name)
        if task is None:
            raise ValidationError(f"Unknown validation task dependency: {task_name}")
        visiting.add(task_name)
        for dependency in task.get("depends_on", []):
            selected.add(dependency)
            include(dependency)
        visiting.remove(task_name)
        visited.add(task_name)

    for task_name in tuple(selected):
        include(task_name)
    return selected


def prerequisites(root: Path, tasks: tuple[dict[str, Any], ...], selected: set[str]) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for task in tasks:
        if task["id"] not in selected:
            continue
        command = task["command"][0]
        result.append({"task": task["id"], "command": command, "available": shutil.which(command) is not None})
        if task.get("cwd") == "webui":
            node_modules = root / "webui" / "node_modules"
            result.append({
                "task": task["id"],
                "command": "webui/node_modules",
                "available": node_modules.is_dir(),
            })
    return result


def create_plan(
    root: Path = ROOT,
    tasks: tuple[dict[str, Any], ...] = TASKS,
    *,
    tests_only: bool = False,
    task_id: str | None = None,
) -> dict[str, Any]:
    root = root.resolve()
    for task in tasks:
        if not isinstance(task.get("id"), str) or re.fullmatch(r"[a-z0-9][a-z0-9-]*", task["id"]) is None:
            raise ValidationError("Validation task ID must be a safe lowercase identifier.")
        pass_env = task.get("pass_env", [])
        if not isinstance(pass_env, list) or any(
            not isinstance(name, str) or name not in APPROVED_TASK_PASS_ENVIRONMENT.get(task["id"], frozenset())
            for name in pass_env
        ):
            raise ValidationError(f"Validation task requests an unapproved inherited environment variable: {task['id']}")
    selected = _selection(tasks, tests_only, task_id)
    inventory = discover_inventory(root)
    used_lanes = sorted({
        lane for task in tasks if task["id"] in selected for lane in task.get("lanes", [])
    })
    empty_lanes = [lane for lane in used_lanes if not inventory.get(lane)]
    candidate = candidate_snapshot(root)
    plan: dict[str, Any] = {
        "schema_version": ENGINE_VERSION,
        "repository": REPOSITORY,
        "created_at": utc_now(),
        "selection": "tests-only" if tests_only else (task_id or "default"),
        "candidate": candidate,
        "inventory_counts": {lane: len(entries) for lane, entries in inventory.items()},
        "inventory_sha256": {
            lane: hashlib.sha256(canonical(entries)).hexdigest() for lane, entries in inventory.items()
        },
        "inventory": inventory,
        "prerequisites": prerequisites(root, tasks, selected),
        "inventory_errors": empty_lanes,
        "tasks": [task_contract(task, task["id"] in selected) for task in tasks],
    }
    plan["plan_sha256"] = hashlib.sha256(canonical(plan)).hexdigest()
    return plan


def format_elapsed(seconds: float) -> str:
    total = max(0, int(seconds))
    return f"{total // 60}m{total % 60:02d}s"


def format_progress(
    repository: str,
    tasks: list[dict[str, Any]],
    receipts: dict[str, dict[str, Any]],
    current: dict[str, Any] | None = None,
    elapsed_seconds: float = 0,
) -> str:
    statuses = [receipts.get(task["id"], {}).get("status") for task in tasks]
    passed = statuses.count("passed")
    failed = sum(status in {"failed", "incomplete"} for status in statuses)
    blocked = sum(status in {"blocked", "not-run"} for status in statuses)
    completed = passed + failed
    pending = max(0, len(tasks) - completed - blocked)
    active = f"; running {current['name']} ({format_elapsed(elapsed_seconds)})" if current else ""
    return (
        f"[validation] {repository}: {completed}/{len(tasks)} selected tasks complete "
        f"({passed} passed, {failed} failed, {blocked} blocked, {pending} pending){active}"
    )


def _count_cases(parser: str | None, output: bytes) -> int | None:
    if parser is None:
        return None
    text = output.decode("utf-8", "replace")
    if parser == "cargo-test":
        return sum(int(value) for value in re.findall(r"test result: ok\.\s+(\d+) passed\b", text))
    if parser == "python-unittest":
        matches = re.findall(r"Ran\s+(\d+) tests?\b", text)
        return int(matches[-1]) if matches else 0
    if parser == "vitest":
        matches = re.findall(r"Tests\s+(\d+)\s+passed\b", text)
        return sum(int(value) for value in matches) if matches else 0
    raise ValidationError(f"Unknown native test summary parser: {parser}")


def _safe_environment(
    task: dict[str, Any],
    run_root: Path,
    inherited: dict[str, str] | None = None,
) -> dict[str, str]:
    source = os.environ if inherited is None else inherited
    task_id = task.get("id")
    if not isinstance(task_id, str) or re.fullmatch(r"[a-z0-9][a-z0-9-]*", task_id) is None:
        raise ValidationError("Validation task ID must be a safe lowercase identifier.")
    env: dict[str, str] = {}
    inherited_values: dict[str, str] = {}
    for key, value in source.items():
        normalized = key.upper()
        if normalized not in INHERITED_ENVIRONMENT_KEYS or not isinstance(value, str):
            continue
        if normalized not in inherited_values or key == normalized:
            inherited_values[normalized] = value
    env.update(inherited_values)
    if not env.get("RUSTUP_HOME"):
        inherited_home = source.get("HOME") or source.get("USERPROFILE")
        if inherited_home:
            env["RUSTUP_HOME"] = str(Path(inherited_home) / ".rustup")
    approved_pass_env = APPROVED_TASK_PASS_ENVIRONMENT.get(task.get("id"), frozenset())
    pass_env = task.get("pass_env", [])
    if not isinstance(pass_env, list) or any(
        not isinstance(name, str) or name not in approved_pass_env for name in pass_env
    ):
        raise ValidationError(f"Validation task requests an unapproved inherited environment variable: {task.get('id')}")
    for name in pass_env:
        match = next((value for key, value in source.items() if key.upper() == name.upper()), None)
        if isinstance(match, str):
            env[name] = match
    env.update({key: value for key, value in task.get("env", {}).items() if isinstance(value, str)})

    environment_root = run_root / f"{task_id}-environment"
    directories = {
        "HOME": environment_root / "home",
        "USERPROFILE": environment_root / "home",
        "TMPDIR": environment_root / "tmp",
        "TMP": environment_root / "tmp",
        "TEMP": environment_root / "tmp",
        "XDG_CONFIG_HOME": environment_root / "xdg-config",
        "XDG_CACHE_HOME": environment_root / "xdg-cache",
        "APPDATA": environment_root / "AppData" / "Roaming",
        "LOCALAPPDATA": environment_root / "AppData" / "Local",
        "CARGO_HOME": run_root / "cargo-home",
        "PIP_CACHE_DIR": environment_root / "pip-cache",
    }
    for directory in set(directories.values()):
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    env.update({key: str(value) for key, value in directories.items()})
    env["NO_COLOR"] = "1"
    env["CARGO_TERM_COLOR"] = "never"
    env["CI"] = "true"
    env["PYTHONNOUSERSITE"] = "1"
    if task.get("kind") == "test":
        env["NODE_ENV"] = "test"
    return env


def _stop_process(process: subprocess.Popen[bytes]) -> None:
    try:
        if os.name == "nt":
            process.terminate()
        else:
            os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    except OSError:
        process.terminate()
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        try:
            if os.name == "nt":
                process.kill()
            else:
                os.killpg(process.pid, signal.SIGKILL)
        except OSError:
            process.kill()


def run_command(
    task: dict[str, Any],
    root: Path,
    log_path: Path,
    *,
    elapsed_progress: Callable[[float], None],
    heartbeat_seconds: float,
) -> dict[str, Any]:
    started = time.monotonic()
    started_at = utc_now()
    cwd = (root / task.get("cwd", ".")).resolve()
    if cwd != root and root not in cwd.parents:
        raise ValidationError(f"Task working directory escapes the repository: {task['id']}")
    output_queue: queue.Queue[bytes | None] = queue.Queue(maxsize=16)
    log_digest = hashlib.sha256()
    output_bytes = 0
    output_tail = bytearray()
    log_bytes_written = 0
    log_truncated = False
    log_error: str | None = None
    timed_out = False
    launch_error: str | None = None
    reader_error: str | None = None
    orphaned_processes_stopped = False
    exit_group_checked = False
    cleanup_sent_at: float | None = None

    log_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with log_path.open("xb") as log:
        os.chmod(log_path, 0o600)
        log_content_limit = MAX_LOG_BYTES - len(LOG_TRUNCATION_MARKER)

        def persist(chunk: bytes) -> None:
            nonlocal log_bytes_written, log_truncated, log_error
            if log_error:
                return
            length = min(len(chunk), max(0, log_content_limit - log_bytes_written))
            view = memoryview(chunk)[:length]
            try:
                while view:
                    written = log.write(view)
                    if not written:
                        raise OSError("short write while recording validation output")
                    log_digest.update(view[:written])
                    log_bytes_written += written
                    view = view[written:]
            except OSError as error:
                log_error = str(error)
                log_truncated = True
            if length < len(chunk):
                log_truncated = True

        try:
            process = subprocess.Popen(
                task["command"],
                cwd=cwd,
                env=_safe_environment(task, log_path.parent),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                bufsize=8192,
                start_new_session=os.name != "nt",
            )
        except OSError as error:
            process = None
            launch_error = str(error)

        if process is not None:
            assert process.stdout is not None

            def pump_output() -> None:
                nonlocal reader_error
                try:
                    while True:
                        chunk = process.stdout.read1(64 * 1024)
                        if not chunk:
                            break
                        output_queue.put(chunk)
                except OSError as error:
                    reader_error = str(error)
                finally:
                    output_queue.put(None)

            reader = threading.Thread(target=pump_output, daemon=True)
            reader.start()
            next_heartbeat = started + heartbeat_seconds
            deadline = started + task.get("timeout", 3600)
            ended = False
            while not ended:
                now = time.monotonic()
                if now >= deadline and process.poll() is None and not timed_out:
                    timed_out = True
                    _stop_process(process)
                wait_for = max(0.01, min(0.5, next_heartbeat - now))
                try:
                    chunk = output_queue.get(timeout=wait_for)
                except queue.Empty:
                    chunk = b""
                if chunk is None:
                    ended = True
                    if process.poll() is None:
                        _stop_process(process)
                elif chunk:
                    output_bytes += len(chunk)
                    output_tail.extend(chunk)
                    if len(output_tail) > MAX_OUTPUT_BYTES:
                        del output_tail[: len(output_tail) - MAX_OUTPUT_BYTES]
                    persist(chunk)
                    try:
                        log.flush()
                    except OSError as error:
                        log_error = log_error or str(error)
                        log_truncated = True
                    sys.stderr.write(chunk.decode("utf-8", "replace"))
                    sys.stderr.flush()
                now = time.monotonic()
                if process.poll() is not None and os.name != "nt":
                    if not exit_group_checked:
                        exit_group_checked = True
                        try:
                            os.killpg(process.pid, 0)
                        except ProcessLookupError:
                            pass
                        else:
                            orphaned_processes_stopped = True
                            try:
                                os.killpg(process.pid, signal.SIGTERM)
                            except OSError:
                                pass
                            cleanup_sent_at = now
                    elif cleanup_sent_at is not None and now - cleanup_sent_at >= 2 and reader.is_alive():
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except OSError:
                            pass
                        cleanup_sent_at = now
                if now >= next_heartbeat and process.poll() is None:
                    elapsed_progress(now - started)
                    next_heartbeat = now + heartbeat_seconds
            process.wait()
            reader.join(timeout=2)
            if reader.is_alive() and os.name != "nt":
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                reader.join(timeout=2)
                if reader.is_alive():
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            reader.join()
            process.stdout.close()
            exit_code = process.returncode
        else:
            message = (launch_error or "command failed to start").encode()
            output_tail.extend(message[-MAX_OUTPUT_BYTES:])
            output_bytes = len(message)
            persist(message)
            try:
                log.flush()
            except OSError as error:
                log_error = log_error or str(error)
                log_truncated = True
            exit_code = None

        try:
            if log_truncated and not log_error:
                log.write(LOG_TRUNCATION_MARKER)
                log_digest.update(LOG_TRUNCATION_MARKER)
            log.flush()
            os.fsync(log.fileno())
        except OSError as error:
            log_error = log_error or str(error)

    active_cases = _count_cases(task.get("case_parser"), bytes(output_tail))
    status = "failed" if timed_out or launch_error or log_error or reader_error or orphaned_processes_stopped or exit_code != 0 else "passed"
    if status == "passed" and task["kind"] == "test" and task.get("case_parser"):
        if active_cases is None or active_cases < task.get("minimum_cases", 1):
            status = "incomplete"
    return {
        "task_id": task["id"],
        "status": status,
        "started_at": started_at,
        "duration_ms": int((time.monotonic() - started) * 1000),
        "exit_code": exit_code,
        "timed_out": timed_out,
        "error": launch_error or log_error or reader_error or ("task left subprocesses running" if orphaned_processes_stopped else None),
        "output_bytes": output_bytes,
        "log_sha256": log_digest.hexdigest(),
        "log_file": log_path.name,
        "active_cases": active_cases,
    }


def execute_plan(
    root: Path,
    plan: dict[str, Any],
    tasks: tuple[dict[str, Any], ...] = TASKS,
    *,
    progress: Callable[[str], None] = lambda message: sys.stderr.write(message + "\n"),
    heartbeat_seconds: float = PROGRESS_INTERVAL_SECONDS,
    run_root: Path | None = None,
) -> dict[str, Any]:
    plan_body = {key: value for key, value in plan.items() if key != "plan_sha256"}
    if hashlib.sha256(canonical(plan_body)).hexdigest() != plan.get("plan_sha256"):
        raise ValidationError("Validation plan digest does not match its contents")
    selected = [task for task in plan["tasks"] if task["selected"]]
    selected_ids = {task["id"] for task in selected}
    task_by_id = {task["id"]: task for task in tasks}
    receipts: dict[str, dict[str, Any]] = {
        task["id"]: {
            "task_id": task["id"],
            "status": "excluded",
            "reason": task.get("exclusion_reason") or plan["selection"],
        }
        for task in plan["tasks"] if not task["selected"]
    }
    evidence = run_root or Path(tempfile.mkdtemp(prefix=f"{REPOSITORY}-validation-"))
    evidence.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(evidence, 0o700)  # nosemgrep: python.lang.security.audit.insecure-file-permissions.insecure-file-permissions -- Explicitly makes the validation evidence directory private to its owner.
    started_at = utc_now()
    missing = [item for item in plan["prerequisites"] if not item["available"]]
    missing.extend({"command": f"test inventory: {lane} is empty", "available": False}
                   for lane in plan["inventory_errors"])
    if missing:
        for task in selected:
            receipts[task["id"]] = {
                "task_id": task["id"],
                "status": "not-run",
                "reason": "required command or test inventory is unavailable",
            }
        result = _build_receipt(plan, started_at, receipts, missing, evidence)
        progress(format_progress(REPOSITORY, selected, receipts))
        return result

    source_changed = False
    last_summary: str | None = None

    def report(current: dict[str, Any] | None = None, elapsed: float = 0) -> None:
        nonlocal last_summary
        summary = format_progress(REPOSITORY, selected, receipts, current, elapsed)
        if summary != last_summary:
            last_summary = summary
            progress(summary)

    for planned_task in selected:
        task_id = planned_task["id"]
        dependency_failure = next((
            receipts.get(dependency, {}).get("status")
            for dependency in planned_task.get("depends_on", [])
            if receipts.get(dependency, {}).get("status") != "passed"
        ), None)
        if dependency_failure:
            receipts[task_id] = {"task_id": task_id, "status": "blocked", "reason": "dependency did not pass"}
            report()
            continue
        if candidate_snapshot(root) != plan["candidate"]:
            source_changed = True
            receipts[task_id] = {"task_id": task_id, "status": "blocked", "reason": "candidate source changed after planning"}
            report()
            break
        task = task_by_id[task_id]
        task_started = time.monotonic()
        report(task, 0)
        def heartbeat(elapsed: float) -> None:
            report(task, elapsed)

        receipt = run_command(
            task,
            root,
            evidence / f"{task_id}.log",
            elapsed_progress=heartbeat,
            heartbeat_seconds=heartbeat_seconds,
        )
        receipts[task_id] = receipt
        report()
        if candidate_snapshot(root) != plan["candidate"]:
            source_changed = True
            break

    for planned_task in plan["tasks"]:
        if planned_task["id"] not in receipts:
            receipts[planned_task["id"]] = {
                "task_id": planned_task["id"],
                "status": "blocked" if source_changed else "excluded",
                "reason": "candidate source changed during execution" if source_changed else "not selected",
            }
    result = _build_receipt(plan, started_at, receipts, missing, evidence, source_changed)
    report()
    return result


def _build_receipt(
    plan: dict[str, Any],
    started_at: str,
    receipts: dict[str, dict[str, Any]],
    missing: list[dict[str, Any]],
    evidence: Path,
    source_changed: bool = False,
) -> dict[str, Any]:
    task_receipts = [receipts[task["id"]] for task in plan["tasks"]]
    selected = [task for task in plan["tasks"] if task["selected"]]
    totals = {
        "planned": len(selected),
        "executed": sum("started_at" in receipt for receipt in task_receipts),
        "passed": sum(receipt["status"] == "passed" for receipt in task_receipts),
        "failed": sum(receipt["status"] in {"failed", "incomplete"} for receipt in task_receipts),
        "blocked": sum(receipt["status"] in {"blocked", "not-run"} for receipt in task_receipts),
        "excluded": sum(receipt["status"] == "excluded" for receipt in task_receipts),
        "active_cases": sum(receipt.get("active_cases") or 0 for receipt in task_receipts),
    }
    status = "passed"
    if missing or source_changed or any(receipts[task["id"]]["status"] != "passed" for task in selected):
        status = "failed"
    result = {
        "schema_version": ENGINE_VERSION,
        "repository": REPOSITORY,
        "selection": plan["selection"],
        "run_id": hashlib.sha256(f"{started_at}:{os.getpid()}".encode()).hexdigest()[:24],
        "started_at": started_at,
        "completed_at": utc_now(),
        "plan_sha256": plan["plan_sha256"],
        "candidate": plan["candidate"],
        "inventory_counts": plan["inventory_counts"],
        "inventory_sha256": plan["inventory_sha256"],
        "status": status,
        "prerequisites": plan["prerequisites"],
        "missing_prerequisites": missing,
        "tasks": task_receipts,
        "totals": totals,
        "evidence_directory": str(evidence),
    }
    receipt_path = evidence / "receipt.json"
    receipt_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.chmod(receipt_path, 0o600)
    result["exit_code"] = 0 if status == "passed" else 1
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", action="store_true", help="inspect tasks and test inventory without executing")
    parser.add_argument("--json", action="store_true", help="print the plan as JSON (requires --plan)")
    parser.add_argument("--tests-only", action="store_true", help="select registered test tasks only")
    parser.add_argument("--task", help="run one registered task and its dependencies")
    args = parser.parse_args(argv)
    if args.json and not args.plan:
        parser.error("--json is available only with --plan")
    try:
        plan = create_plan(ROOT, tests_only=args.tests_only, task_id=args.task)
        if args.plan:
            if args.json:
                print(json.dumps(plan, indent=2))
            else:
                selected_count = sum(task["selected"] for task in plan["tasks"])
                test_count = sum(plan["inventory_counts"].get(lane, 0) for lane in {
                    lane for task in plan["tasks"] if task["selected"] for lane in task["lanes"]
                })
                available = sum(item["available"] for item in plan["prerequisites"])
                print(f"{REPOSITORY}: {test_count} inventoried test sources; {selected_count} planned tasks; "
                      f"{available}/{len(plan['prerequisites'])} prerequisites available.")
            return 0
        if any(task["selected"] and task["kind"] == "test" for task in plan["tasks"]):
            require_trusted_local_test_sandbox()
        result = execute_plan(ROOT, plan)
        summary = {key: result[key] for key in ("repository", "status", "plan_sha256", "totals", "evidence_directory", "exit_code")}
        print(json.dumps(summary, indent=2))
        return int(result["exit_code"])
    except (ValidationError, OSError, subprocess.SubprocessError) as error:
        print(f"validation engine: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
