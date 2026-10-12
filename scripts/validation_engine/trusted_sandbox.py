"""Require a trusted outer sandbox before running local validation tests."""

from __future__ import annotations

import hmac
import os
from pathlib import Path
from typing import Mapping

MARKER_PATH = Path('/run/validation-engine-sandbox')


def require_trusted_local_test_sandbox(
    environment: Mapping[str, str] | None = None,
    marker_path: Path = MARKER_PATH,
) -> None:
    env = os.environ if environment is None else environment
    if env.get('GITHUB_ACTIONS') == 'true':
        return
    token = env.get('VALIDATION_OUTER_SANDBOX_TOKEN', '')
    if len(token) != 64 or any(character not in '0123456789abcdef' for character in token):
        raise RuntimeError('Local test execution requires the trusted bubblewrap launcher.')
    try:
        if marker_path.is_symlink() or not marker_path.is_file():
            raise OSError('sandbox marker is not a regular file')
        marker = marker_path.read_text(encoding='utf-8')
    except OSError as error:
        raise RuntimeError('Local test execution requires the trusted bubblewrap launcher.') from error
    if not hmac.compare_digest(marker, token):
        raise RuntimeError('Local test execution requires the trusted bubblewrap launcher.')
