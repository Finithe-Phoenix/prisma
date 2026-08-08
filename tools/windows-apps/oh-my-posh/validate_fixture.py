#!/usr/bin/env python3
"""Validate the pinned Oh My Posh Windows x86-64 compatibility fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parent
LOCK_PATH = ROOT / "fixture.lock.json"
HEX_SHA256 = re.compile(r"[0-9a-f]{64}")
AMD64_MACHINE = 0x8664


class FixtureError(ValueError):
    """Raised when fixture metadata or the acquired artifact is invalid."""


def load_manifest(path: Path = LOCK_PATH) -> dict[str, Any]:
    with path.open(encoding="utf-8") as manifest_file:
        manifest: dict[str, Any] = json.load(manifest_file)
    return manifest


def validate_manifest(manifest: dict[str, Any], root: Path = ROOT) -> None:
    required = {
        "schema",
        "id",
        "upstream",
        "version",
        "tag",
        "architecture",
        "pe_machine",
        "filename",
        "artifact_path",
        "official_release_url",
        "source_url",
        "checksums_url",
        "size",
        "sha256",
        "license",
    }
    missing = sorted(required - manifest.keys())
    if missing:
        raise FixtureError(f"manifest is missing fields: {', '.join(missing)}")

    if manifest["schema"] != "prisma-windows-fixture/v1":
        raise FixtureError("unsupported fixture schema")
    if manifest["id"] != "oh-my-posh-windows-amd64":
        raise FixtureError("unexpected fixture id")
    if manifest["upstream"] != "JanDeDobbeleer/oh-my-posh":
        raise FixtureError("unexpected upstream repository")
    if manifest["tag"] != f"v{manifest['version']}":
        raise FixtureError("tag must be derived from the exact pinned version")
    if manifest["architecture"] != "x86-64" or manifest["pe_machine"] != "0x8664":
        raise FixtureError("fixture must target Windows x86-64 PE")

    tag = manifest["tag"]
    filename = manifest["filename"]
    expected_release = (
        f"https://github.com/JanDeDobbeleer/oh-my-posh/releases/tag/{tag}"
    )
    expected_source = (
        "https://github.com/JanDeDobbeleer/oh-my-posh/releases/download/"
        f"{tag}/{filename}"
    )
    expected_checksums = (
        "https://github.com/JanDeDobbeleer/oh-my-posh/releases/download/"
        f"{tag}/checksums.txt"
    )
    if manifest["official_release_url"] != expected_release:
        raise FixtureError("release URL is not pinned to the declared tag")
    if manifest["source_url"] != expected_source:
        raise FixtureError("source URL is not pinned to the declared tag and asset")
    if manifest["checksums_url"] != expected_checksums:
        raise FixtureError("checksums URL is not pinned to the declared tag")

    for field in ("official_release_url", "source_url", "checksums_url"):
        parsed = urlparse(manifest[field])
        if parsed.scheme != "https" or parsed.hostname != "github.com":
            raise FixtureError(f"{field} must use HTTPS on github.com")
        if "latest" in parsed.path.casefold().split("/"):
            raise FixtureError(f"{field} must never resolve through latest")

    if not isinstance(manifest["size"], int) or manifest["size"] <= 0:
        raise FixtureError("artifact size must be a positive integer")
    if not HEX_SHA256.fullmatch(manifest["sha256"]):
        raise FixtureError("SHA-256 must be 64 lowercase hexadecimal characters")

    license_info = manifest["license"]
    if license_info.get("spdx") != "MIT":
        raise FixtureError("Oh My Posh attribution must declare MIT")
    attribution_path = root / license_info.get("attribution_file", "")
    if not attribution_path.is_file():
        raise FixtureError("MIT attribution file is missing")
    attribution = attribution_path.read_text(encoding="utf-8")
    if license_info.get("copyright") not in attribution:
        raise FixtureError("copyright attribution does not match the vendored license")
    if "Permission is hereby granted, free of charge" not in attribution:
        raise FixtureError("vendored attribution is not the expected MIT license")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as artifact:
        for chunk in iter(lambda: artifact.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pe_machine(path: Path) -> int:
    with path.open("rb") as artifact:
        dos_header = artifact.read(64)
        if len(dos_header) != 64 or dos_header[:2] != b"MZ":
            raise FixtureError("artifact does not have a valid DOS header")
        pe_offset = struct.unpack_from("<I", dos_header, 0x3C)[0]
        artifact.seek(pe_offset)
        pe_header = artifact.read(6)
    if len(pe_header) != 6 or pe_header[:4] != b"PE\0\0":
        raise FixtureError("artifact does not have a valid PE signature")
    return struct.unpack_from("<H", pe_header, 4)[0]


def validate_artifact(path: Path, manifest: dict[str, Any]) -> None:
    if not path.is_file():
        raise FixtureError(f"artifact is missing: {path}")
    if path.stat().st_size != manifest["size"]:
        raise FixtureError("artifact size does not match fixture.lock.json")
    if sha256(path) != manifest["sha256"]:
        raise FixtureError("artifact SHA-256 does not match fixture.lock.json")
    machine = pe_machine(path)
    if machine != AMD64_MACHINE:
        raise FixtureError(f"expected PE machine 0x8664, found 0x{machine:04x}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--artifact",
        type=Path,
        help="artifact to validate (defaults to artifact_path from the lock)",
    )
    parser.add_argument(
        "--require-artifact",
        action="store_true",
        help="fail instead of validating metadata only when the artifact is absent",
    )
    args = parser.parse_args()

    manifest = load_manifest()
    validate_manifest(manifest)
    artifact = args.artifact or ROOT / manifest["artifact_path"]
    if artifact.exists() or args.require_artifact or args.artifact is not None:
        validate_artifact(artifact, manifest)
        print(
            f"PASS Oh My Posh {manifest['version']} Windows x86-64: "
            f"{manifest['sha256']}"
        )
    else:
        print(f"PASS metadata-only Oh My Posh {manifest['version']} (artifact absent)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
