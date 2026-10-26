import json
from pathlib import Path
import subprocess
import sys
import unittest

from transfer_churn_validation import is_complete_transfer, has_public_transfer


class TransferChurnValidationTests(unittest.TestCase):
    def test_complete_transfer_requires_api_counters_and_matching_payload(self) -> None:
        row = {"progress": 1.0, "downloaded": 4}
        self.assertTrue(
            is_complete_transfer(
                row,
                expected_bytes=4,
                disk_bytes=4,
                expected_sha256="a" * 64,
                actual_sha256="a" * 64,
            )
        )

    def test_preallocated_file_size_does_not_count_as_download(self) -> None:
        row = {"progress": 0.0, "downloaded": 0}
        self.assertFalse(
            is_complete_transfer(
                row,
                expected_bytes=4,
                disk_bytes=4,
                expected_sha256="a" * 64,
                actual_sha256="a" * 64,
            )
        )

    def test_api_completion_with_wrong_payload_hash_does_not_pass(self) -> None:
        row = {"progress": 1.0, "downloaded": 4}
        self.assertFalse(
            is_complete_transfer(
                row,
                expected_bytes=4,
                disk_bytes=4,
                expected_sha256="a" * 64,
                actual_sha256="0" * 64,
            )
        )

    def test_partial_download_does_not_pass(self) -> None:
        self.assertFalse(
            is_complete_transfer(
                {"progress": 0.75, "downloaded": 3},
                expected_bytes=4,
                disk_bytes=4,
                expected_sha256="a" * 64,
                actual_sha256="a" * 64,
            )
        )

    def test_short_or_malformed_file_fails(self) -> None:
        self.assertFalse(
            is_complete_transfer(
                {"progress": 1.0, "downloaded": 4},
                expected_bytes=4,
                disk_bytes=3,
                expected_sha256="a" * 64,
                actual_sha256="a" * 64,
            )
        )
        self.assertFalse(
            is_complete_transfer(
                {"progress": float("nan"), "downloaded": 4},
                expected_bytes=4,
                disk_bytes=4,
                expected_sha256="a" * 64,
                actual_sha256="a" * 64,
            )
        )

    def test_public_cycle_requires_actual_downloaded_bytes(self) -> None:
        self.assertFalse(has_public_transfer({"progress": 0.0, "downloaded": 0}, 16))
        self.assertFalse(has_public_transfer({"progress": 0.1, "downloaded": 15}, 16))
        self.assertTrue(has_public_transfer({"progress": 0.01, "downloaded": 16}, 16))

    def test_completion_cli_checks_the_payload_hash(self) -> None:
        command = [
            sys.executable,
            str(Path(__file__).with_name("transfer_churn_validation.py")),
            "complete",
            "--expected-bytes",
            "4",
            "--disk-bytes",
            "4",
            "--expected-sha256",
            "a" * 64,
            "--actual-sha256",
            "0" * 64,
        ]
        result = subprocess.run(command, input=json.dumps({"progress": 1, "downloaded": 4}), text=True)
        self.assertEqual(result.returncode, 1)

    def test_public_cli_rejects_an_added_torrent_without_payload(self) -> None:
        command = [
            sys.executable,
            str(Path(__file__).with_name("transfer_churn_validation.py")),
            "public",
            "--minimum-downloaded-bytes",
            "16",
        ]
        result = subprocess.run(command, input=json.dumps({"progress": 0, "downloaded": 0}), text=True)
        self.assertEqual(result.returncode, 1)

    def test_category_cleanup_rejects_rows_without_valid_hashes(self) -> None:
        command = [
            sys.executable,
            str(Path(__file__).with_name("transfer_churn_validation.py")),
            "category-hashes",
        ]
        invalid_rows = (
            [{"name": "missing hash"}],
            [{"hash": ""}],
            [{"hash": "g" * 40}],
            [None],
            {"hash": "a" * 40},
        )
        for rows in invalid_rows:
            with self.subTest(rows=rows):
                result = subprocess.run(command, input=json.dumps(rows), text=True, capture_output=True)
                self.assertEqual(result.returncode, 1)

        valid_hash = "a" * 40
        result = subprocess.run(command, input=json.dumps([{"hash": valid_hash}]), text=True, capture_output=True)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), valid_hash)

        result = subprocess.run(command, input="[]", text=True, capture_output=True)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "EMPTY")


if __name__ == "__main__":
    unittest.main()
