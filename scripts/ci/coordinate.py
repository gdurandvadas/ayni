#!/usr/bin/env python3
"""Declare all required PR work, then require its results and bound evidence."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re

from run_fixture import digest, validate_provenance


JOBS = [
    "coordinate",
    "candidate",
    "lock-consistency",
    "repository",
    "fixtures",
    "quality",
]


def plan(manifest: dict, source: str) -> dict:
    if manifest.get("schema_version") != 1:
        raise ValueError("unsupported fixture manifest")
    platforms = manifest["platforms"]
    fixtures = manifest["fixtures"]
    if not platforms or not fixtures:
        raise ValueError("empty platform or fixture inventory")
    if len({item["arch"] for item in platforms}) != len(platforms) or len(
        {item["id"] for item in fixtures}
    ) != len(fixtures):
        raise ValueError("duplicate platform or fixture")
    for platform in platforms:
        if platform not in (
            {"arch": "amd64", "runner": "ubuntu-latest"},
            {"arch": "arm64", "runner": "ubuntu-24.04-arm"},
        ):
            raise ValueError("unsupported native platform")
    for fixture in fixtures:
        if (
            not re.fullmatch(r"[a-z][a-z0-9-]*", fixture["id"])
            or Path(fixture["root"]).is_absolute()
            or ".." in Path(fixture["root"]).parts
        ):
            raise ValueError("invalid fixture identity/root")
        if not fixture["expected_roots"] or fixture["expected_exit_code"] not in (0, 1):
            raise ValueError("missing fixture expectations")
    evidence = [
        {"id": "repository", "arch": platform["arch"]} for platform in platforms
    ]
    evidence += [
        {"id": fixture["id"], "arch": platform["arch"]}
        for platform in platforms
        for fixture in fixtures
    ]
    return {
        "schema_version": 1,
        "source": source,
        "jobs": JOBS,
        "platforms": platforms,
        "fixtures": fixtures,
        "evidence": evidence,
    }


def complete(expected: dict, results: dict, directory: Path, source: str) -> None:
    if (
        expected.get("source") != source
        or expected.get("jobs") != JOBS
        or set(results) != set(JOBS)
    ):
        raise ValueError("required job inventory or source mismatch")
    failures = {
        name: result.get("result")
        for name, result in results.items()
        if result.get("result") != "success"
    }
    if failures:
        raise ValueError(
            f"required jobs failed, were cancelled, missing, or skipped: {failures}"
        )
    for entry in expected["evidence"]:
        folder = directory / f'managed-{entry["id"]}-{entry["arch"]}'
        receipt = json.loads((folder / "receipt.json").read_text())
        if (
            receipt.get("schema_version"),
            receipt.get("id"),
            receipt.get("source"),
            receipt.get("platform"),
            receipt.get("success"),
        ) != (1, entry["id"], source, f'linux/{entry["arch"]}', True):
            raise ValueError(f"missing or unsuccessful evidence: {entry}")
        if (
            not {
                "lock.json",
                "build.json",
                "signals.json",
                "execution.json",
                "timings.jsonl",
            }
            <= receipt.get("files", {}).keys()
        ):
            raise ValueError(f"incomplete evidence inventory: {entry}")
        for name, value in receipt["files"].items():
            if Path(name).name != name or digest(folder / name) != value:
                raise ValueError(f"evidence digest mismatch: {entry} {name}")
        validate_provenance(folder, source, f'linux/{entry["arch"]}')
        lock = json.loads((folder / "lock.json").read_text())
        build = json.loads((folder / "build.json").read_text())
        if lock.get("fingerprint") != build.get("environment_fingerprint"):
            raise ValueError("execution is not bound to the retained lock")
    for platform in expected["platforms"]:
        folder = directory / f'managed-lock-{platform["arch"]}'
        committed = (folder / "committed.json").read_bytes()
        if (
            committed != (folder / "generated.json").read_bytes()
            or committed != (folder / "regenerated.json").read_bytes()
        ):
            raise ValueError("lock consistency evidence mismatch")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("plan", "complete"))
    parser.add_argument(
        "--manifest", type=Path, default=Path("scripts/ci/fixtures.json")
    )
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    expected = plan(json.loads(args.manifest.read_text()), args.source)
    if args.operation == "plan":
        args.plan.parent.mkdir(parents=True, exist_ok=True)
        args.plan.write_text(json.dumps(expected, indent=2) + "\n")
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            output.write(
                "platforms=" + json.dumps({"include": expected["platforms"]}) + "\n"
            )
            output.write(
                "fixtures="
                + json.dumps(
                    {
                        "include": [
                            dict(fixture, **platform)
                            for platform in expected["platforms"]
                            for fixture in expected["fixtures"]
                        ]
                    }
                )
                + "\n"
            )
    else:
        if json.loads(args.plan.read_text()) != expected:
            raise ValueError("coordinator plan changed or is incomplete")
        complete(
            expected, json.loads(os.environ["NEEDS_JSON"]), args.evidence, args.source
        )
        print("Every declared job and managed evidence artifact is complete")


if __name__ == "__main__":
    main()
