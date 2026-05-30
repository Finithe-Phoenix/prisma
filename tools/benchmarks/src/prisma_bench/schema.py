"""Stable JSON schema for benchmark artifacts."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, Field, model_validator

SCHEMA_VERSION: Literal["prisma-bench-result/v1"] = "prisma-bench-result/v1"
BackendName = Literal["prisma", "qemu", "box64", "fex", "native"]


class RunResult(BaseModel):
    schema_version: Literal["prisma-bench-result/v1"] = SCHEMA_VERSION
    benchmark: str
    backend: BackendName
    host: str
    git_sha: str
    timestamp: datetime = Field(default_factory=lambda: datetime.now(UTC))
    command: list[str] = Field(default_factory=list)
    source: str
    binary: str | None = None
    target_arch: str | None = None
    iterations: int
    skipped: bool = False
    skip_reason: str | None = None
    exit_code: int | None = None
    wall_s: float | None = None
    stdout_tail: str = ""
    stderr_tail: str = ""
    metrics: dict[str, float] = Field(default_factory=dict)

    @model_validator(mode="after")
    def validate_skip_reason(self) -> RunResult:
        if self.skipped and not self.skip_reason:
            raise ValueError("skipped results require skip_reason")
        if not self.skipped and self.exit_code is None:
            raise ValueError("non-skipped results require exit_code")
        return self

    def to_json_text(self) -> str:
        return self.model_dump_json(indent=2) + "\n"


class BackendSummary(BaseModel):
    benchmark: str
    backend: BackendName
    runs: int
    successful_runs: int
    skipped_runs: int
    best_wall_s: float | None = None
    median_wall_s: float | None = None
    best_iterations_per_second: float | None = None


class Summary(BaseModel):
    schema_version: Literal["prisma-bench-summary/v1"] = "prisma-bench-summary/v1"
    generated_at: datetime = Field(default_factory=lambda: datetime.now(UTC))
    records: int
    summaries: list[BackendSummary]

    def to_json_text(self) -> str:
        return self.model_dump_json(indent=2) + "\n"
