#!/usr/bin/env python3
"""Compute and verify the independently published provisioning contract."""

import argparse
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
REPOSITORY = "ghcr.io/gdurandvadas/ayni-provisioning"


def recipe_digest(root=ROOT):
    digest = hashlib.sha256()
    for name in (
        ".github/docker/provisioning.versions",
        ".github/docker/ayni-provisioning.Dockerfile",
        "LICENSE",
        "NOTICE",
    ):
        data = (root / name).read_bytes()
        digest.update(name.encode() + b"\0" + str(len(data)).encode() + b"\0" + data)
    return "sha256:" + digest.hexdigest()


def validate_manifest(manifest):
    platforms = []
    for entry in manifest.get("manifests", []):
        platform = entry.get("platform", {})
        if platform.get("os") == "unknown" and platform.get("architecture") == "unknown":
            if entry.get("annotations", {}).get("vnd.docker.reference.type") != "attestation-manifest":
                raise ValueError("unknown manifest must be a build attestation")
            continue
        if not DIGEST.fullmatch(entry.get("digest", "")):
            raise ValueError("platform has no immutable digest")
        platforms.append((platform.get("os"), platform.get("architecture")))
    if sorted(platforms) != [("linux", "amd64"), ("linux", "arm64")]:
        raise ValueError("provisioning requires exactly linux/amd64 and linux/arm64")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("recipe")
    check = sub.add_parser("manifest")
    check.add_argument("file", type=Path)
    args = parser.parse_args()
    if args.command == "recipe":
        print(recipe_digest())
    else:
        validate_manifest(json.loads(args.file.read_text()))


if __name__ == "__main__":
    main()
