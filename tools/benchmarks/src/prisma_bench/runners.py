"""Corpus build and backend execution helpers."""

from __future__ import annotations

import platform
import shutil
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path

from .corpora import Corpus, repo_root
from .schema import BackendName, RunResult


@dataclass(frozen=True)
class BuildArtifact:
    binary: Path | None
    target_arch: str | None
    command: list[str]
    skipped: bool
    reason: str | None = None


def _find(*candidates: str) -> str | None:
    for candidate in candidates:
        path = shutil.which(candidate)
        if path is not None:
            return path
    return None


def _host_arch() -> str:
    machine = platform.machine()
    if machine in {"AMD64", "x86_64"}:
        return "x86_64"
    if machine in {"arm64", "aarch64"}:
        return "aarch64"
    return machine or "unknown"


def host_label() -> str:
    processor = platform.processor() or "unknown"
    return f"{platform.system()} {_host_arch()} / {processor}"


def git_sha(root: Path | None = None) -> str:
    root = root or repo_root()
    try:
        out = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return "unknown"
    return out.stdout.strip()


def build_corpus(corpus: Corpus, output_dir: Path) -> BuildArtifact:
    compiler = _find("cc", "gcc", "clang")
    if compiler is None:
        return BuildArtifact(
            binary=None,
            target_arch=None,
            command=[],
            skipped=True,
            reason="no C compiler found on PATH",
        )

    build_dir = output_dir / "build" / corpus.name
    build_dir.mkdir(parents=True, exist_ok=True)
    suffix = ".exe" if platform.system() == "Windows" else ""
    binary = build_dir / f"{corpus.binary_stem}{suffix}"
    command = [
        compiler,
        "-O2",
        "-std=c99",
        str(corpus.source_path),
        "-o",
        str(binary),
    ]
    try:
        subprocess.run(command, capture_output=True, text=True, check=True)
    except (OSError, subprocess.CalledProcessError) as exc:
        reason = str(exc)
        if isinstance(exc, subprocess.CalledProcessError):
            reason = (exc.stderr or exc.stdout or str(exc)).strip()
        return BuildArtifact(
            binary=None,
            target_arch=None,
            command=command,
            skipped=True,
            reason=reason[:300],
        )

    return BuildArtifact(
        binary=binary,
        target_arch=_host_arch(),
        command=command,
        skipped=False,
    )


def _tail(data: bytes) -> str:
    return data[-500:].decode(errors="replace")


def _run_command(
    *,
    backend: BackendName,
    command: list[str],
    corpus: Corpus,
    artifact: BuildArtifact,
    iterations: int,
    timeout_s: float,
) -> RunResult:
    started = time.monotonic()
    try:
        proc = subprocess.run(command, capture_output=True, timeout=timeout_s)
        wall_s = time.monotonic() - started
    except subprocess.TimeoutExpired as exc:
        return RunResult(
            benchmark=corpus.name,
            backend=backend,
            host=host_label(),
            git_sha=git_sha(),
            command=command,
            source=str(corpus.source_path),
            binary=str(artifact.binary) if artifact.binary else None,
            target_arch=artifact.target_arch,
            iterations=iterations,
            skipped=False,
            exit_code=-1,
            wall_s=timeout_s,
            stderr_tail=str(exc.stderr or "")[-500:],
            metrics={},
        )

    metrics: dict[str, float] = {}
    if proc.returncode == 0 and wall_s > 0:
        metrics["iterations_per_second"] = iterations / wall_s

    return RunResult(
        benchmark=corpus.name,
        backend=backend,
        host=host_label(),
        git_sha=git_sha(),
        command=command,
        source=str(corpus.source_path),
        binary=str(artifact.binary) if artifact.binary else None,
        target_arch=artifact.target_arch,
        iterations=iterations,
        skipped=False,
        exit_code=proc.returncode,
        wall_s=wall_s,
        stdout_tail=_tail(proc.stdout),
        stderr_tail=_tail(proc.stderr),
        metrics=metrics,
    )


def _skip(
    *,
    backend: BackendName,
    corpus: Corpus,
    iterations: int,
    reason: str,
    artifact: BuildArtifact | None = None,
) -> RunResult:
    return RunResult(
        benchmark=corpus.name,
        backend=backend,
        host=host_label(),
        git_sha=git_sha(),
        command=[],
        source=str(corpus.source_path),
        binary=str(artifact.binary) if artifact and artifact.binary else None,
        target_arch=artifact.target_arch if artifact else None,
        iterations=iterations,
        skipped=True,
        skip_reason=reason,
    )


def run_backend(
    *,
    backend: BackendName,
    corpus: Corpus,
    output_dir: Path,
    iterations: int,
    timeout_s: float,
) -> RunResult:
    if backend == "prisma":
        runner = repo_root() / "core" / "build" / "prisma_run"
        if not runner.exists():
            return _skip(
                backend=backend,
                corpus=corpus,
                iterations=iterations,
                reason="core/build/prisma_run not built",
            )
        return _skip(
            backend=backend,
            corpus=corpus,
            iterations=iterations,
            reason=(
                "prisma_run currently accepts raw blobs only; "
                "dhrystone builds an ELF/host binary"
            ),
        )

    artifact = build_corpus(corpus, output_dir)
    if artifact.skipped:
        return _skip(
            backend=backend,
            corpus=corpus,
            iterations=iterations,
            reason=artifact.reason or "corpus build skipped",
            artifact=artifact,
        )
    if artifact.binary is None:
        return _skip(
            backend=backend,
            corpus=corpus,
            iterations=iterations,
            reason="corpus build did not produce a binary",
            artifact=artifact,
        )

    if backend == "native":
        command = [str(artifact.binary), str(iterations)]
        return _run_command(
            backend=backend,
            command=command,
            corpus=corpus,
            artifact=artifact,
            iterations=iterations,
            timeout_s=timeout_s,
        )

    if artifact.target_arch != "x86_64":
        return _skip(
            backend=backend,
            corpus=corpus,
            iterations=iterations,
            reason=(
                f"{backend} requires an x86_64 corpus binary; "
                f"local compiler produced {artifact.target_arch}"
            ),
            artifact=artifact,
        )

    emulator = {
        "qemu": _find("qemu-x86_64", "qemu-x86_64-static"),
        "box64": _find("box64"),
        "fex": _find("FEXLoader", "FEXInterpreter"),
    }[backend]
    if emulator is None:
        return _skip(
            backend=backend,
            corpus=corpus,
            iterations=iterations,
            reason=f"{backend} executable not found on PATH",
            artifact=artifact,
        )

    return _run_command(
        backend=backend,
        command=[emulator, str(artifact.binary), str(iterations)],
        corpus=corpus,
        artifact=artifact,
        iterations=iterations,
        timeout_s=timeout_s,
    )
