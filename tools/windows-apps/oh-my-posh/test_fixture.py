from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import validate_fixture


class OhMyPoshFixtureTest(unittest.TestCase):
    def test_checked_in_manifest_and_attribution(self) -> None:
        manifest = validate_fixture.load_manifest()
        validate_fixture.validate_manifest(manifest)

    def test_latest_release_path_is_rejected(self) -> None:
        manifest = validate_fixture.load_manifest()
        manifest["source_url"] = (
            "https://github.com/JanDeDobbeleer/oh-my-posh/releases/"
            "latest/download/posh-windows-amd64.exe"
        )
        with self.assertRaisesRegex(validate_fixture.FixtureError, "not pinned"):
            validate_fixture.validate_manifest(manifest)

    def test_corrupt_artifact_is_rejected(self) -> None:
        manifest = validate_fixture.load_manifest()
        with tempfile.TemporaryDirectory() as temp_directory:
            artifact = Path(temp_directory) / "oh-my-posh.exe"
            artifact.write_bytes(b"not a PE fixture")
            manifest["size"] = artifact.stat().st_size
            manifest["sha256"] = validate_fixture.sha256(artifact)
            with self.assertRaisesRegex(validate_fixture.FixtureError, "DOS header"):
                validate_fixture.validate_artifact(artifact, manifest)

    def test_lock_is_canonical_json(self) -> None:
        lock_path = validate_fixture.LOCK_PATH
        manifest = validate_fixture.load_manifest(lock_path)
        canonical = json.dumps(manifest, indent=2, ensure_ascii=False) + "\n"
        self.assertEqual(lock_path.read_text(encoding="utf-8"), canonical)


if __name__ == "__main__":
    unittest.main()
