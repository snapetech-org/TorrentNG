#!/usr/bin/env python3
"""Run narrowly scoped reproductions without editing the supplied repository."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REVISION = "16da409daa4b0a6bf91beace73e1af552718f862"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--allow-different-revision", action="store_true")
    args = parser.parse_args()
    repo = args.repo.resolve(strict=True)
    revision = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
    dirty = subprocess.check_output(["git", "-C", str(repo), "status", "--porcelain=v1", "--untracked-files=no"], text=True)
    if (revision != REVISION or dirty) and not args.allow_different_revision:
        parser.error("Expected the clean reviewed revision; use --allow-different-revision for explicitly labeled exploratory runs.")
    print(f"Repository revision: {revision}; tracked changes: {bool(dirty)}", flush=True)
    print("Scope: current storage/codec crates and production upload scheduler acquisition pattern; no daemon benchmark.", flush=True)
    here = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="torrentng-followup-") as temp:
        project = Path(temp)
        (project / "src").mkdir()
        manifest = (here / "Cargo.toml.in").read_text()
        for name in ("storage", "path", "metrics", "peer-wire"):
            manifest = manifest.replace("@RT_" + name.upper().replace("-", "_") + "@", json.dumps(str(repo / "crates" / ("rt-" + name))))
        source = (here / "main.rs.in").read_text()
        scheduler_path = str(repo / "crates/rt-engine/src/transfer/upload_scheduler.rs")
        rust_path = '"' + scheduler_path.replace("\\", "\\\\").replace('"', '\\"') + '"'
        source = source.replace("@SCHEDULER_SOURCE@", rust_path)
        (project / "Cargo.toml").write_text(manifest)
        (project / "src/main.rs").write_text(source)
        shutil.copyfile(here / "Cargo.lock", project / "Cargo.lock")
        environment = os.environ.copy()
        if args.target_dir:
            environment["CARGO_TARGET_DIR"] = str(args.target_dir.resolve())
        command = [args.cargo, "run", "--release", "--locked", "--manifest-path", str(project / "Cargo.toml"), "-j", "4"]
        if args.offline:
            command.append("--offline")
        result = subprocess.run(command, env=environment, check=False)
        return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
