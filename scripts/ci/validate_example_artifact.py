#!/usr/bin/env python3
"""Validate the strict artifact contract used by the real-tool example CI job."""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, NoReturn, Sequence


SCHEMA_VERSION = "0.4.0"
EXPECTED_KINDS = frozenset({"test", "coverage", "size", "complexity", "deps"})
LANGUAGES = ("rust", "go", "node", "python", "kotlin")
FINDING_ID = re.compile(r"ayni:finding:v1:sha256:[0-9a-f]{64}")


@dataclass(frozen=True)
class ExpectedOutcome:
    check_exit_code: int
    aggregate_status: str
    failing_kinds: frozenset[str]
    warning_kinds: frozenset[str]
    failing_offenders: int
    warning_offenders: int
    coverage_range: tuple[float, float]


# These are deliberately semantic expectations, not just schema fixtures. The
# managed toolchain and native dependency locks make these ranges stable while
# allowing harmless formatter-level precision differences.
EXPECTED_OUTCOMES = {
    "rust": ExpectedOutcome(
        1, "fail", frozenset({"coverage"}), frozenset(), 1, 0, (45.0, 50.0)
    ),
    "go": ExpectedOutcome(
        1, "fail", frozenset({"coverage"}), frozenset(), 1, 0, (43.0, 47.0)
    ),
    "node": ExpectedOutcome(
        1, "fail", frozenset({"coverage"}), frozenset(), 1, 0, (48.0, 52.0)
    ),
    "python": ExpectedOutcome(
        0, "pass", frozenset(), frozenset(), 0, 0, (99.0, 100.0)
    ),
    "kotlin": ExpectedOutcome(
        0, "pass", frozenset(), frozenset({"coverage"}), 0, 1, (75.0, 80.0)
    ),
}


class ValidationError(ValueError):
    """An actionable artifact contract violation."""


def fail(message: str) -> NoReturn:
    raise ValidationError(message)


def require_object(value: Any, location: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{location} must be an object")
    return value


def require_array(value: Any, location: str) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{location} must be an array")
    return value


def require_count(value: Any, location: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        fail(f"{location} must be a non-negative integer")
    return value


def reject_non_finite(value: Any, location: str = "artifact") -> None:
    if isinstance(value, float) and not math.isfinite(value):
        fail(f"{location} contains a non-finite number")
    if isinstance(value, dict):
        for key, child in value.items():
            reject_non_finite(child, f"{location}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_non_finite(child, f"{location}[{index}]")


def require_sha256(value: Any, location: str) -> str:
    if (
        not isinstance(value, str)
        or not value.startswith("sha256:")
        or len(value) != 71
        or any(character not in "0123456789abcdef" for character in value[7:])
    ):
        fail(f"{location} must be a lowercase SHA-256 fingerprint")
    return value


def normalized_root(scope: dict[str, Any], row_number: int) -> str:
    path = scope.get("path")
    if path is None:
        return "."
    if not isinstance(path, str) or not path or path == "." or path.endswith("/"):
        fail(
            f"rows[{row_number}].scope.path must be a normalized non-root string or absent"
        )
    return path


def validate_artifact(
    artifact: Any,
    *,
    language: str,
    expected_roots: Sequence[str],
    repository_root: str,
    check_exit_code: int,
) -> None:
    """Validate one example artifact, raising ValidationError on the first defect."""
    document = require_object(artifact, "artifact")
    reject_non_finite(document)
    if document.get("schema_version") != SCHEMA_VERSION:
        fail(f"schema_version must be {SCHEMA_VERSION!r}")
    if document.get("execution_mode") != "managed":
        fail("execution_mode must identify managed evidence")
    require_sha256(document.get("contract_digest"), "contract_digest")
    require_sha256(
        document.get("environment_lock_fingerprint"),
        "environment_lock_fingerprint",
    )
    require_sha256(document.get("source_fingerprint"), "source_fingerprint")
    tool_versions = require_array(document.get("tool_versions"), "tool_versions")
    if not tool_versions:
        fail("tool_versions must record managed runtime provenance")
    tool_names: list[str] = []
    for index, raw_tool in enumerate(tool_versions):
        tool = require_object(raw_tool, f"tool_versions[{index}]")
        name = tool.get("tool")
        version = tool.get("version")
        if not isinstance(name, str) or not name or not isinstance(version, str) or not version:
            fail(f"tool_versions[{index}] must contain non-empty tool and version strings")
        tool_names.append(name)
    if tool_names != sorted(set(tool_names)):
        fail("tool_versions must be sorted by unique tool name")

    # Ayni preserves the normalized lexical parent of the supplied config path.
    # Do not resolve this path: CI intentionally checks fixtures through a
    # repository-relative config path, and the artifact records that same root.
    expected_repository_root = repository_root
    if document.get("repository_root") != expected_repository_root:
        fail(
            "repository_root does not identify the checked fixture: "
            f"expected {expected_repository_root!r}, got {document.get('repository_root')!r}"
        )
    invocation = require_object(document.get("invocation"), "invocation")
    if invocation.get("command") != "check" or invocation.get("languages") != [language]:
        fail(f"invocation must be repository check for language {language!r}")

    roots = list(expected_roots)
    if not roots or len(set(roots)) != len(roots):
        fail("expected roots must be a non-empty unique list")
    expected_targets = {(language, root) for root in roots}

    completion = require_object(document.get("completion"), "completion")
    expected_count = len(expected_targets)
    if completion.get("scope") != "repository" or completion.get("state") != "complete":
        fail("completion must have repository scope and complete state")
    for field in ("expected_targets", "detected_targets", "completed_targets"):
        if (
            require_count(completion.get(field), f"completion.{field}")
            != expected_count
        ):
            fail(f"completion.{field} must equal {expected_count}")
    if (
        require_count(completion.get("skipped_targets"), "completion.skipped_targets")
        != 0
    ):
        fail("completion.skipped_targets must be zero")
    if require_array(completion.get("issues"), "completion.issues"):
        fail("completion.issues must be empty")

    rows = require_array(document.get("rows"), "rows")
    expected_row_count = expected_count * len(EXPECTED_KINDS)
    if len(rows) != expected_row_count:
        fail(f"rows must contain exactly {expected_row_count} entries, got {len(rows)}")

    row_keys: set[tuple[Any, ...]] = set()
    kinds_by_target: dict[tuple[str, str], set[str]] = {}
    passing_rows = 0
    warning_offenders = 0
    failing_offenders = 0
    failing_kinds: set[str] = set()
    warning_kinds: set[str] = set()
    coverage_percent: float | None = None
    for index, raw_row in enumerate(rows):
        row = require_object(raw_row, f"rows[{index}]")
        kind = row.get("kind")
        row_language = row.get("language")
        if row_language != language:
            fail(f"rows[{index}].language must be {language!r}")
        if kind not in EXPECTED_KINDS:
            fail(f"rows[{index}].kind is unexpected or mutation is enabled: {kind!r}")
        scope = require_object(row.get("scope"), f"rows[{index}].scope")
        if scope.get("workspace_root") != expected_repository_root:
            fail(f"rows[{index}].scope.workspace_root does not match repository_root")
        root = normalized_root(scope, index)
        target = (row_language, root)
        if target not in expected_targets:
            fail(f"rows[{index}] represents unexpected target {row_language}/{root}")
        if scope.get("package") is not None or scope.get("file") is not None:
            fail(f"rows[{index}] must be an unselected repository-analysis row")

        key = (kind, row_language, root, scope.get("package"), scope.get("file"))
        if key in row_keys:
            fail(f"duplicate row key for {row_language}/{root}/{kind}")
        row_keys.add(key)
        kinds_by_target.setdefault(target, set()).add(kind)

        result = require_object(row.get("result"), f"rows[{index}].result")
        budget = require_object(row.get("budget"), f"rows[{index}].budget")
        offenders = require_object(row.get("offenders"), f"rows[{index}].offenders")
        if (
            result.get("kind") != kind
            or budget.get("kind") != kind
            or offenders.get("kind") != kind
        ):
            fail(
                f"rows[{index}] typed result, budget, and offenders must match kind {kind!r}"
            )
        if "failure" in result:
            fail(f"rows[{index}].result.failure reports a collector command failure")
        if kind == "coverage":
            measured = result.get("percent")
            if isinstance(measured, bool) or not isinstance(measured, (int, float)):
                fail(f"rows[{index}].result.percent must contain measured coverage")
            coverage_percent = float(measured)
        items = require_array(offenders.get("items"), f"rows[{index}].offenders.items")
        row_pass = row.get("pass")
        if not isinstance(row_pass, bool):
            fail(f"rows[{index}].pass must be boolean")
        if row_pass:
            passing_rows += 1
        elif not items:
            fail(f"rows[{index}] fails without a typed policy finding")
        else:
            failing_kinds.add(kind)
        row_failing_offenders = 0
        for item_index, raw_item in enumerate(items):
            item = require_object(
                raw_item, f"rows[{index}].offenders.items[{item_index}]"
            )
            finding_id = item.get("id")
            if not isinstance(finding_id, str) or not FINDING_ID.fullmatch(finding_id):
                fail(
                    f"rows[{index}].offenders.items[{item_index}].id must be a "
                    "schema-v4 finding ID"
                )
            verification = require_object(
                item.get("verification"),
                f"rows[{index}].offenders.items[{item_index}].verification",
            )
            command = verification.get("command")
            expected_command = f"ayni verify {kind} "
            if not isinstance(command, str) or not command.startswith(expected_command):
                fail(
                    f"rows[{index}].offenders.items[{item_index}].verification.command "
                    "must be an exact Ayni verification command"
                )
            level = "fail" if kind == "test" else item.get("level")
            if level == "warn":
                warning_offenders += 1
                warning_kinds.add(kind)
            elif level == "fail":
                failing_offenders += 1
                row_failing_offenders += 1
            else:
                fail(f"rows[{index}].offenders.items[{item_index}].level is invalid")
        if row_pass == (row_failing_offenders > 0):
            fail(f"rows[{index}].pass is inconsistent with its typed fail findings")

    if set(kinds_by_target) != expected_targets:
        fail("rows do not represent exactly the configured language/root targets")
    for target, kinds in kinds_by_target.items():
        if kinds != EXPECTED_KINDS:
            fail(
                f"target {target[0]}/{target[1]} does not contain the exact five enabled kinds"
            )

    failure_summaries = document.get("failure_summaries", [])
    if require_array(failure_summaries, "failure_summaries"):
        fail("failure_summaries must be absent or empty")

    aggregate = require_object(document.get("aggregate"), "aggregate")
    failing_rows = len(rows) - passing_rows
    expected_status = "pass" if failing_rows == 0 else "fail"
    expected_aggregate = {
        "status": expected_status,
        "total_rows": len(rows),
        "passing_rows": passing_rows,
        "failing_rows": failing_rows,
        "warning_offenders": warning_offenders,
        "failing_offenders": failing_offenders,
    }
    for field, expected in expected_aggregate.items():
        if field != "status":
            require_count(aggregate.get(field), f"aggregate.{field}")
        if aggregate.get(field) != expected:
            fail(
                f"aggregate.{field} must be {expected!r}, got {aggregate.get(field)!r}"
            )

    outcome = EXPECTED_OUTCOMES[language]
    if check_exit_code != outcome.check_exit_code:
        fail(
            f"check exit code must be {outcome.check_exit_code} for {language}, "
            f"got {check_exit_code}"
        )
    if aggregate.get("status") != outcome.aggregate_status:
        fail(
            f"aggregate.status must be {outcome.aggregate_status!r} for {language}"
        )
    if failing_kinds != outcome.failing_kinds:
        fail(
            f"failing signal kinds must be {sorted(outcome.failing_kinds)!r} for "
            f"{language}, got {sorted(failing_kinds)!r}"
        )
    if warning_kinds != outcome.warning_kinds:
        fail(
            f"warning signal kinds must be {sorted(outcome.warning_kinds)!r} for "
            f"{language}, got {sorted(warning_kinds)!r}"
        )
    if failing_offenders != outcome.failing_offenders:
        fail(
            f"failing offender count must be {outcome.failing_offenders} for {language}"
        )
    if warning_offenders != outcome.warning_offenders:
        fail(
            f"warning offender count must be {outcome.warning_offenders} for {language}"
        )
    if coverage_percent is None or not (
        outcome.coverage_range[0] <= coverage_percent <= outcome.coverage_range[1]
    ):
        fail(
            f"coverage percent must be within {outcome.coverage_range!r} for {language}, "
            f"got {coverage_percent!r}"
        )


def parser() -> argparse.ArgumentParser:
    cli = argparse.ArgumentParser(
        description="Validate a schema-0.4.0 Ayni artifact from a real-tool mono example.",
    )
    cli.add_argument(
        "--artifact", required=True, type=Path, help="Path to .ayni/last/signals.json"
    )
    cli.add_argument(
        "--language", required=True, choices=LANGUAGES, help="Expected adapter language"
    )
    cli.add_argument(
        "--expected-root",
        required=True,
        action="append",
        dest="expected_roots",
        metavar="ROOT",
        help="Configured target root; repeat once per expected target",
    )
    cli.add_argument(
        "--repository-root",
        required=True,
        help="Normalized lexical fixture root supplied to check",
    )
    cli.add_argument(
        "--check-exit-code",
        required=True,
        type=int,
        help="Exit code returned by the managed check",
    )
    return cli


def main(argv: Sequence[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        with args.artifact.open(encoding="utf-8") as artifact_file:
            artifact = json.load(
                artifact_file,
                parse_constant=lambda value: fail(
                    f"invalid JSON numeric constant {value!r}"
                ),
            )
        validate_artifact(
            artifact,
            language=args.language,
            expected_roots=args.expected_roots,
            repository_root=args.repository_root,
            check_exit_code=args.check_exit_code,
        )
    except (OSError, json.JSONDecodeError, ValidationError) as error:
        print(f"artifact validation failed: {error}", file=sys.stderr)
        return 1
    print(
        f"validated {args.language} artifact: {len(args.expected_roots)} targets, "
        f"{len(artifact['rows'])} complete rows"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
