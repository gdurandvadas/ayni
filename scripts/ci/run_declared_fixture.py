#!/usr/bin/env python3
"""Resolve a fixture manifest entry for the shared candidate/release runner."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--id", required=True)
    parser.add_argument("--cli", required=True)
    parser.add_argument("--executor", required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(Path("scripts/ci/fixtures.json").read_text())
    entry, = [item for item in manifest["fixtures"] if item["id"] == args.id]
    command = [sys.executable, "scripts/ci/run_fixture.py", "--cli", args.cli,
               "--executor-image", args.executor, "--id", args.id, "--source", args.source,
               "--platform", args.platform, "--artifact-directory", str(args.artifacts),
               "--expected-exit-code", str(entry["expected_exit_code"])]
    if entry.get("composition"):
        root = Path(os.environ["RUNNER_TEMP"]) / "composition" / entry["composition"]
        command += ["--composition", entry["composition"]]
    else:
        root = Path(entry["root"])
        command += ["--language", entry["language"]]
        for target in entry["expected_roots"]:
            command += ["--expected-root", target]
    command += ["--fixture-root", str(root)]
    for name in ("CACHE_FROM", "CACHE_TO"):
        if os.environ.get(name):
            command += ["--" + name.lower().replace("_", "-"), os.environ[name]]
    subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
