#!/usr/bin/env python3
"""Fail when an external GitHub Action is not SHA-pinned and documented."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
USES = re.compile(r"^\s*uses:\s*([^\s#]+)(?:\s+#\s*(\S.*))?$")
PINNED = re.compile(r".+@[0-9a-f]{40}$")
VERSION = re.compile(r"v?\d+(?:\.\d+){1,2}")


def main() -> int:
    errors: list[str] = []
    workflows = sorted((ROOT / ".github").rglob("*.yml"))
    workflows.extend(sorted((ROOT / ".github").rglob("*.yaml")))
    for workflow in workflows:
        for line_number, line in enumerate(workflow.read_text().splitlines(), 1):
            match = USES.match(line)
            if not match:
                continue
            reference, version = match.groups()
            if reference.startswith("./"):
                continue
            location = f"{workflow.relative_to(ROOT)}:{line_number}"
            if not PINNED.fullmatch(reference):
                errors.append(f"{location}: external action is not pinned: {reference}")
            if not version or not VERSION.fullmatch(version.strip()):
                errors.append(
                    f"{location}: pinned action needs an exact version comment "
                    "such as v4.4.0"
                )

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("all external GitHub Actions are SHA-pinned with version comments")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
