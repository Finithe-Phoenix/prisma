"""prisma-bench command-line entry point.

Invoke as:
    prisma-bench --help
    prisma-bench run --backend prisma --corpus dhrystone
    prisma-bench report results/
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import NoReturn, cast

import click

from . import __version__
from .aggregate import write_summary
from .corpora import CORPORA, get_corpus
from .runners import run_backend
from .schema import BackendName


def _not_yet(fase: str) -> NoReturn:
    """Exit with a helpful message. Keeps the CLI honest until we fill it in."""
    click.echo(
        f"prisma-bench: not yet implemented (arrives in {fase}).\n"
        "See tools/benchmarks/README.md for the schedule.",
        err=True,
    )
    sys.exit(2)


def _artifact_name(backend: str, corpus: str) -> str:
    return f"{corpus}-{backend}.json"


@click.group()
@click.version_option(__version__, prog_name="prisma-bench")
def main() -> None:
    """Run, collect, and report Prisma DBT benchmarks."""


@main.command("run")
@click.option(
    "--backend",
    type=click.Choice(["prisma", "qemu", "box64", "fex", "native"]),
    required=True,
    help="Which translator / execution mode to benchmark.",
)
@click.option(
    "--corpus",
    type=str,
    required=True,
    help=("Name of the benchmark corpus to run (dhrystone, coremark, spec-subset, ...)."),
)
@click.option(
    "--output",
    type=click.Path(dir_okay=True, file_okay=False),
    default="results/",
    help="Directory where per-run artifacts and a summary JSON are written.",
)
@click.option(
    "--iterations",
    type=int,
    default=None,
    help="Override corpus iteration count.",
)
@click.option(
    "--timeout-s",
    type=float,
    default=60.0,
    show_default=True,
    help="Backend process timeout in seconds.",
)
def run_cmd(
    backend: str,
    corpus: str,
    output: str,
    iterations: int | None,
    timeout_s: float,
) -> None:
    """Execute the corpus under the given backend and collect metrics."""
    selected = get_corpus(corpus)
    if selected is None:
        raise click.ClickException(f"unknown corpus: {corpus}")
    if iterations is not None and iterations <= 0:
        raise click.ClickException("--iterations must be positive")
    if timeout_s <= 0:
        raise click.ClickException("--timeout-s must be positive")

    output_dir = Path(output)
    output_dir.mkdir(parents=True, exist_ok=True)
    result = run_backend(
        backend=cast(BackendName, backend),
        corpus=selected,
        output_dir=output_dir,
        iterations=iterations or selected.default_iterations,
        timeout_s=timeout_s,
    )
    artifact_path = output_dir / _artifact_name(backend, corpus)
    artifact_path.write_text(result.to_json_text())
    click.echo(str(artifact_path))
    if result.skipped:
        click.echo(f"skipped: {result.skip_reason}", err=True)


@main.command("report")
@click.argument(
    "results_dir",
    type=click.Path(exists=True, dir_okay=True, file_okay=False),
)
@click.option(
    "--format",
    "fmt",
    type=click.Choice(["markdown", "latex", "json"]),
    default="markdown",
)
def report_cmd(results_dir: str, fmt: str) -> None:
    """Aggregate per-run artifacts into a single report."""
    if fmt != "json":
        _not_yet("F2-BM-007 report generation")
    out = write_summary(Path(results_dir))
    click.echo(str(out))


@main.command("list-corpora")
def list_corpora_cmd() -> None:
    """List all named benchmark corpora known to the harness."""
    for corpus in CORPORA.values():
        click.echo(f"{corpus.name}\t{corpus.display_name}")


if __name__ == "__main__":  # pragma: no cover
    main()
