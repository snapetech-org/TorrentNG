#!/usr/bin/env python3
"""Publish TorrentNG GitHub releases without requiring GitHub CLI on JIT runners."""

from __future__ import annotations

import argparse
import json
import mimetypes
import os
import re
import subprocess
import sys
import urllib.parse
from pathlib import Path
from typing import Any


API_ROOT = "https://api.github.com"
API_VERSION = "2022-11-28"
ALLOWED_DISPATCH_WORKFLOWS = {"release-aur.yml", "release-copr.yml"}


class GitHubApiError(RuntimeError):
    def __init__(self, status: int, message: str) -> None:
        super().__init__(message)
        self.status = status


def curl_environment() -> dict[str, str]:
    environment = os.environ.copy()
    environment.pop("GITHUB_TOKEN", None)
    return environment


def api_request(method: str, path: str, payload: dict[str, Any] | None = None) -> Any:
    repository = os.environ.get("GITHUB_REPOSITORY", "")
    token = os.environ.get("GITHUB_TOKEN", "")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise RuntimeError("GITHUB_REPOSITORY must contain a valid owner/name")
    if not token:
        raise RuntimeError("GITHUB_TOKEN is required")
    if method not in {"GET", "POST", "PATCH", "DELETE"}:
        raise RuntimeError(f"Unsupported GitHub API method: {method}")

    url = f"{API_ROOT}/repos/{repository}/{path.lstrip('/')}"
    config = "\n".join(
        (
            f'header = "Authorization: Bearer {token}"',
            'header = "Accept: application/vnd.github+json"',
            f'header = "X-GitHub-Api-Version: {API_VERSION}"',
            'header = "User-Agent: TorrentNG-release-publisher"',
            f'url = "{url}"',
            "",
        )
    )
    command = [
        "curl",
        "--config",
        "-",
        "--silent",
        "--show-error",
        "--write-out",
        "\n%{http_code}",
    ]
    if payload is not None:
        command.extend(
            (
                "--header",
                "Content-Type: application/json",
                "--data-binary",
                json.dumps(payload),
            )
        )
    command.extend(("--request", method))

    result = subprocess.run(
        command,
        input=config,
        text=True,
        encoding="utf-8",
        capture_output=True,
        check=False,
        env=curl_environment(),
    )
    if result.returncode:
        detail = (result.stderr or result.stdout).strip()[:1000]
        raise RuntimeError(f"GitHub API {method} {path} failed: {detail}")
    try:
        content, status_text = result.stdout.rsplit("\n", maxsplit=1)
        status = int(status_text)
    except (ValueError, TypeError) as error:
        raise RuntimeError(f"GitHub API {method} {path} returned an invalid response") from error
    if not 200 <= status < 300:
        raise GitHubApiError(status, f"GitHub API {method} {path} returned {status}: {content[:1000]}")
    if not content:
        return {}
    try:
        return json.loads(content)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"GitHub API {method} {path} returned invalid JSON") from error


def api_list(path: str) -> list[dict[str, Any]]:
    values: list[dict[str, Any]] = []
    for page in range(1, 101):
        separator = "&" if "?" in path else "?"
        result = api_request("GET", f"{path}{separator}per_page=100&page={page}")
        if not isinstance(result, list):
            raise RuntimeError(f"Expected a list from GitHub API {path}")
        values.extend(result)
        if len(result) < 100:
            break
    return values


def previous_tag(current_tag: str) -> str:
    releases = api_list("releases")
    return next(
        (
            str(release["tag_name"])
            for release in releases
            if not release.get("draft") and release.get("tag_name") != current_tag
        ),
        "",
    )


def find_previous(args: argparse.Namespace) -> int:
    print(f"tag={previous_tag(args.current_tag)}")
    return 0


def upload_asset(upload_url: str, path: Path, token: str) -> None:
    parsed_upload_url = urllib.parse.urlsplit(upload_url)
    if (
        parsed_upload_url.scheme != "https"
        or parsed_upload_url.hostname != "uploads.github.com"
        or parsed_upload_url.username is not None
        or parsed_upload_url.password is not None
        or parsed_upload_url.port not in (None, 443)
    ):
        raise RuntimeError("GitHub returned an unexpected asset upload URL")

    content_type = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
    config = "\n".join(
        (
            f'header = "Authorization: Bearer {token}"',
            'header = "Accept: application/vnd.github+json"',
            f'header = "X-GitHub-Api-Version: {API_VERSION}"',
            f'url = "{upload_url}"',
            "",
        )
    )
    command = [
        "curl",
        "--config",
        "-",
        "--fail",
        "--silent",
        "--show-error",
        "--header",
        f"Content-Type: {content_type}",
        "--data-binary",
        f"@{path}",
    ]
    result = subprocess.run(
        command,
        input=config,
        text=True,
        encoding="utf-8",
        capture_output=True,
        check=False,
        env=curl_environment(),
    )
    if result.returncode:
        detail = (result.stderr or result.stdout).strip()[:1000]
        raise RuntimeError(f"Upload failed for {path.name}: {detail}")


def release_assets(release_id: int) -> list[dict[str, Any]]:
    return api_list(f"releases/{release_id}/assets")


def publish(args: argparse.Namespace) -> int:
    tag = args.tag
    body = os.environ.get(args.body_env, "")
    if not body.strip():
        raise RuntimeError(f"{args.body_env} must contain the curated release notes")

    asset_dir = Path(args.asset_dir)
    assets = sorted(path for path in asset_dir.iterdir() if path.is_file())
    if not assets:
        raise RuntimeError(f"No release assets found in {asset_dir}")

    encoded_tag = urllib.parse.quote(tag, safe="")
    try:
        release = api_request("GET", f"releases/tags/{encoded_tag}")
    except GitHubApiError as error:
        if error.status != 404:
            raise
        release = api_request(
            "POST",
            "releases",
            {
                "tag_name": tag,
                "name": args.title,
                "body": body,
                "draft": True,
                "prerelease": False,
            },
        )
    else:
        if release.get("tag_name") != tag:
            raise RuntimeError(
                "GitHub returned a release for an unexpected tag: "
                f"{release.get('tag_name')}"
            )
        release = api_request(
            "PATCH",
            f"releases/{release['id']}",
            {
                "name": args.title,
                "body": body,
                "prerelease": False,
            },
        )

    release_id = int(release["id"])
    upload_base = str(release["upload_url"]).split("{", maxsplit=1)[0]
    token = os.environ["GITHUB_TOKEN"]
    existing_assets = {str(asset["name"]): asset for asset in release_assets(release_id)}
    desired_names = {path.name for path in assets}

    for name, asset in existing_assets.items():
        if name not in desired_names:
            api_request("DELETE", f"releases/assets/{asset['id']}")

    for path in assets:
        if "\n" in path.name or "\r" in path.name:
            raise RuntimeError(f"Invalid line break in asset name: {path.name!r}")
        old_asset = existing_assets.get(path.name)
        if old_asset is not None:
            api_request("DELETE", f"releases/assets/{old_asset['id']}")
        url = f"{upload_base}?{urllib.parse.urlencode({'name': path.name})}"
        upload_asset(url, path, token)

    uploaded = {str(asset["name"]): asset for asset in release_assets(release_id)}
    if set(uploaded) != desired_names:
        raise RuntimeError("GitHub release assets do not match the uploaded asset set")
    for path in assets:
        asset = uploaded[path.name]
        if asset.get("state") != "uploaded" or int(asset["size"]) != path.stat().st_size:
            raise RuntimeError(f"GitHub asset verification failed for {path.name}")

    if release.get("draft"):
        release = api_request(
            "PATCH",
            f"releases/{release_id}",
            {"draft": False, "make_latest": "true"},
        )

    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as summary:
            summary.write(
                f"Published [{args.title}]({release['html_url']}) "
                f"with {len(assets)} verified assets.\n"
            )
    print(f"Published {args.title}: {release['html_url']} ({len(assets)} verified assets)")
    return 0


def dispatch(args: argparse.Namespace) -> int:
    if args.workflow not in ALLOWED_DISPATCH_WORKFLOWS:
        raise RuntimeError(f"Workflow dispatch is not allowed for {args.workflow}")
    workflow = urllib.parse.quote(args.workflow, safe="")
    api_request(
        "POST",
        f"actions/workflows/{workflow}/dispatches",
        {"ref": args.ref, "inputs": {"tag": args.tag}},
    )
    print(f"Dispatched {args.workflow} for {args.tag} from {args.ref}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    previous_parser = subparsers.add_parser("previous-tag")
    previous_parser.add_argument("--current-tag", required=True)
    previous_parser.set_defaults(run=find_previous)

    publish_parser = subparsers.add_parser("publish")
    publish_parser.add_argument("--tag", required=True)
    publish_parser.add_argument("--title", required=True)
    publish_parser.add_argument("--body-env", default="RELEASE_BODY")
    publish_parser.add_argument("--asset-dir", default="dist")
    publish_parser.set_defaults(run=publish)

    dispatch_parser = subparsers.add_parser("dispatch")
    dispatch_parser.add_argument("--workflow", required=True)
    dispatch_parser.add_argument("--ref", required=True)
    dispatch_parser.add_argument("--tag", required=True)
    dispatch_parser.set_defaults(run=dispatch)

    args = parser.parse_args()
    try:
        return int(args.run(args))
    except (GitHubApiError, RuntimeError, OSError, KeyError) as error:
        print(f"Release publication failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
