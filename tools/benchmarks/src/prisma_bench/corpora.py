"""Benchmark corpus registry."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Corpus:
    name: str
    display_name: str
    source_path: Path
    binary_stem: str
    default_iterations: int
    description: str


def repo_root() -> Path:
    return Path(__file__).resolve().parents[4]


def corpus_root() -> Path:
    return repo_root() / "tools" / "benchmarks" / "corpora"


CORPORA: dict[str, Corpus] = {
    "dhrystone": Corpus(
        name="dhrystone",
        display_name="Prisma Dhrystone-style integer benchmark",
        source_path=corpus_root() / "dhrystone" / "dhry_prisma.c",
        binary_stem="dhry_prisma",
        default_iterations=50000,
        description=(
            "Self-authored branch-heavy integer workload for first benchmark "
            "harness validation."
        ),
    ),
}


def get_corpus(name: str) -> Corpus | None:
    return CORPORA.get(name)
