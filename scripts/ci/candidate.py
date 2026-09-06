#!/usr/bin/env python3
"""Create and verify a same-run Linux candidate before importing any image.

The Docker archive is an OCI-compatible image transport, not a published artifact.
No credentials, registry publication, or cross-workflow downloads are used.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def command(*args: str) -> str:
    return subprocess.check_output(args, text=True).strip()


def verify(directory: Path, source: str, platform: str, run_id: str) -> dict:
    metadata = json.loads((directory / "candidate.json").read_text())
    if (
        metadata.get("schema_version"),
        metadata.get("source"),
        metadata.get("platform"),
        metadata.get("run_id"),
    ) != (1, source, platform, run_id):
        raise ValueError("candidate schema/source/platform/run identity mismatch")
    if not re.fullmatch(r"[0-9a-f]{40}", source):
        raise ValueError("candidate source must be an exact commit")
    if platform not in ("linux/amd64", "linux/arm64"):
        raise ValueError("unsupported candidate platform")
    if set(metadata.get("files", {})) != {"ayni", "executor.tar"}:
        raise ValueError("candidate file inventory mismatch")
    for name, digest in metadata["files"].items():
        path = directory / name
        if (
            path.is_symlink()
            or not path.is_file()
            or not re.fullmatch(r"[0-9a-f]{64}", digest)
            or sha256(path) != digest
        ):
            raise ValueError(f"candidate checksum mismatch: {name}")
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", metadata.get("image_id", "")):
        raise ValueError("candidate image identity is missing")
    return metadata


def inspect_executor(identity: str, metadata: dict) -> None:
    data = json.loads(command("docker", "image", "inspect", identity))[0]
    labels = data["Config"].get("Labels", {})
    required = {
        "org.opencontainers.image.revision": metadata["source"],
        "org.opencontainers.image.version": metadata["version"],
        "dev.ayni.executor.lock-schema": metadata["lock_schema"],
        "dev.ayni.executor.recipe": metadata["recipe"],
    }
    if (
        data["Id"] != metadata["image_id"]
        or f'{data["Os"]}/{data["Architecture"]}' != metadata["platform"]
        or any(labels.get(key) != value for key, value in required.items())
    ):
        raise ValueError("executor image metadata mismatch")
    digest = command(
        "docker",
        "run",
        "--rm",
        "--entrypoint",
        "sha256sum",
        identity,
        "/usr/local/bin/ayni",
    ).split()[0]
    version = command("docker", "run", "--rm", identity, "--version")
    if digest != metadata["files"]["ayni"] or version != f'ayni {metadata["version"]}':
        raise ValueError("host candidate and container executor differ")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("record", "verify", "import"))
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--image", default="ayni-candidate:checkout")
    args = parser.parse_args()
    directory = args.directory.resolve()
    if args.operation == "record":
        image = json.loads(command("docker", "image", "inspect", args.image))[0]
        labels = image["Config"]["Labels"]
        metadata = {
            "schema_version": 1,
            "source": args.source,
            "platform": args.platform,
            "run_id": args.run_id,
            "image_id": image["Id"],
            "version": labels["org.opencontainers.image.version"],
            "lock_schema": labels["dev.ayni.executor.lock-schema"],
            "recipe": labels["dev.ayni.executor.recipe"],
            "files": {
                name: sha256(directory / name) for name in ("ayni", "executor.tar")
            },
        }
        inspect_executor(args.image, metadata)
        (directory / "candidate.json").write_text(json.dumps(metadata, indent=2) + "\n")
    metadata = verify(directory, args.source, args.platform, args.run_id)
    if args.operation == "import":
        # Check the entire archive before Docker reads it. Artifact source/run
        # selection is additionally constrained by download-artifact in the caller.
        subprocess.run(
            ["docker", "load", "--input", str(directory / "executor.tar")], check=True
        )
        inspect_executor(metadata["image_id"], metadata)
        destination = "localhost:5000/ayni-candidate:checkout"
        subprocess.run(["docker", "tag", metadata["image_id"], destination], check=True)
        subprocess.run(["docker", "push", destination], check=True)
        digests = json.loads(command("docker", "image", "inspect", destination))[0][
            "RepoDigests"
        ]
        matches = [
            item
            for item in digests
            if re.fullmatch(r"localhost:5000/ayni-candidate@sha256:[0-9a-f]{64}", item)
        ]
        if len(matches) != 1:
            raise ValueError(
                "local registry did not return one immutable executor identity"
            )
        (directory / "ayni").chmod(0o755)
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a") as output:
                output.write(f"cli={directory / 'ayni'}\nexecutor={matches[0]}\n")
        print(matches[0])
    print(f"verified candidate {args.source} {args.platform}")


if __name__ == "__main__":
    main()
