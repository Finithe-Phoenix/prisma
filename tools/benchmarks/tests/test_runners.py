from __future__ import annotations

from pathlib import Path

from pytest import MonkeyPatch

from prisma_bench.corpora import get_corpus
from prisma_bench.runners import build_corpus, run_backend


def test_build_corpus_soft_skips_without_compiler(
    monkeypatch: MonkeyPatch,
    tmp_path: Path,
) -> None:
    import prisma_bench.runners as runners

    monkeypatch.setattr(runners, "_find", lambda *args: None)
    corpus = get_corpus("dhrystone")
    assert corpus is not None
    artifact = build_corpus(corpus, tmp_path)
    assert artifact.skipped is True
    assert artifact.reason == "no C compiler found on PATH"


def test_qemu_soft_skips_when_compiler_missing(
    monkeypatch: MonkeyPatch,
    tmp_path: Path,
) -> None:
    import prisma_bench.runners as runners

    monkeypatch.setattr(runners, "_find", lambda *args: None)
    corpus = get_corpus("dhrystone")
    assert corpus is not None
    result = run_backend(
        backend="qemu",
        corpus=corpus,
        output_dir=tmp_path,
        iterations=10,
        timeout_s=1.0,
    )
    assert result.skipped is True
    assert result.skip_reason == "no C compiler found on PATH"
