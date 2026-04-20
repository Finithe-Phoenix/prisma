"""prisma-bench command-line entry point.

Deliberately minimal: the subcommands exist to fix the shape of the API
(what a user would type in Fase 2) but each one exits with a message
that explains which Fase fills it in.

Invoke as:
    prisma-bench --help
    prisma-bench run --backend prisma --corpus dhrystone
    prisma-bench report results/
"""

from __future__ import annotations

import sys
from typing import NoReturn

import click

from . import __version__


def _not_yet(fase: str) -> NoReturn:
    """Exit with a helpful message. Keeps the CLI honest until we fill it in."""
    click.echo(
        f"prisma-bench: not yet implemented (arrives in {fase}).\n"
        "See tools/benchmarks/README.md for the schedule.",
        err=True,
    )
    sys.exit(2)


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
def run_cmd(backend: str, corpus: str, output: str) -> None:
    """Execute the corpus under the given backend and collect metrics."""
    del backend, corpus, output  # accepted for API fixing; not yet wired.
    _not_yet("Fase 2 week 49+")


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
    del results_dir, fmt
    _not_yet("Fase 2 week 49+")


@main.command("list-corpora")
def list_corpora_cmd() -> None:
    """List all named benchmark corpora known to the harness."""
    click.echo("No corpora registered yet. See tools/benchmarks/README.md.")


if __name__ == "__main__":  # pragma: no cover
    main()
