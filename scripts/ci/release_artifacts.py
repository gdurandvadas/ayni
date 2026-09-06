#!/usr/bin/env python3
"""Publication boundaries shared by initial releases and existing-tag recovery."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin",
           "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")


def inventory(tag: str) -> set[str]:
    if not re.fullmatch(r"ayni-v[0-9]+\.[0-9]+\.[0-9]+(?:[-.][0-9A-Za-z.-]+)?", tag):
        raise ValueError("invalid release tag")
    return {f"ayni-{tag}-{target}.tar.gz" for target in TARGETS} | {"SHA256SUMS"}


def gh(*args: str) -> str:
    return subprocess.check_output(["gh", *args], text=True)


def api(path: str) -> dict:
    return json.loads(gh("api", f"repos/{os.environ['GITHUB_REPOSITORY']}/{path}"))


def resolve(tag: str, read=api) -> str:
    inventory(tag)
    obj = read(f"git/ref/tags/{tag}")["object"]
    for _ in range(6):
        if obj["type"] == "commit" and re.fullmatch(r"[0-9a-f]{40}", obj["sha"]):
            return obj["sha"]
        if obj["type"] != "tag":
            break
        obj = read(f"git/tags/{obj['sha']}")["object"]
    raise ValueError("release tag does not resolve to a commit within five annotated tags")


def archives(directory: Path, tag: str) -> dict[str, Path]:
    files = list(directory.rglob("*.tar.gz"))
    result = {path.name: path for path in files}
    if len(result) != len(files) or result.keys() != inventory(tag) - {"SHA256SUMS"}:
        raise ValueError("release archive inventory is incomplete, duplicated, or unexpected")
    return result


def checksums(directory: Path, tag: str, verify: bool = False) -> None:
    content = "".join(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {name}\n"
                      for name, path in sorted(archives(directory, tag).items()))
    manifest = directory / "SHA256SUMS"
    if verify:
        if manifest.read_text() != content:
            raise ValueError("public checksum manifest differs from exact archive bytes/inventory")
    else:
        manifest.write_text(content)


def public_release(tag: str, read=api) -> dict:
    release = read(f"releases/tags/{tag}")
    if release.get("draft") is not False or release.get("tag_name") != tag:
        raise ValueError("recovery/publication requires an existing public release")
    return release


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("source", "checksums", "upload", "verify"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--expected-source")
    parser.add_argument("--directory", type=Path, default=Path("dist"))
    args = parser.parse_args()
    if args.operation == "checksums":
        checksums(args.directory, args.tag)
        return
    source = resolve(args.tag)
    if args.expected_source and source != args.expected_source:
        raise ValueError("release tag moved away from the expected source")
    if args.operation == "source":
        print(source)
        return
    release = public_release(args.tag)
    if args.operation == "upload":
        checksums(args.directory, args.tag, verify=True)
        for asset in release["assets"]:
            if asset["name"] not in inventory(args.tag):
                gh("api", "--method", "DELETE",
                   f"repos/{os.environ['GITHUB_REPOSITORY']}/releases/assets/{asset['id']}")
        gh("release", "upload", args.tag,
           *(str(path) for path in archives(args.directory, args.tag).values()),
           str(args.directory / "SHA256SUMS"), "--clobber", "--repo", os.environ["GITHUB_REPOSITORY"])
    else:
        names = [asset["name"] for asset in release["assets"]]
        if len(names) != len(set(names)) or set(names) != inventory(args.tag):
            raise ValueError("public release assets are incomplete or unexpected")
        args.directory.mkdir(parents=True, exist_ok=True)
        gh("release", "download", args.tag, "--clobber", "--repo", os.environ["GITHUB_REPOSITORY"],
           "--dir", str(args.directory))
        checksums(args.directory, args.tag, verify=True)


if __name__ == "__main__":
    main()
