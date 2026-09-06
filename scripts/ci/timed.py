#!/usr/bin/env python3
"""Run a CI stage without a shell and retain timing even when it fails."""

import argparse
import json
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--stage", required=True)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("a command is required after --")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    started = time.time()
    monotonic = time.monotonic()
    code = 127
    try:
        code = subprocess.run(command, check=False).returncode
        code = 128 - code if code < 0 else code
    finally:
        with args.output.open("a") as output:
            output.write(json.dumps({
                "stage": args.stage,
                "started_at": started,
                "duration_seconds": round(time.monotonic() - monotonic, 3),
                "exit_code": code,
            }, sort_keys=True) + "\n")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
