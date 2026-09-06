#!/usr/bin/env python3
"""Run a declared managed fixture with an explicit host CLI and executor identity."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import tempfile

from validate_example_artifact import validate_artifact
import composition


def digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def validate_provenance(
    directory: Path, source: str | None = None, platform: str | None = None
) -> None:
    execution = json.loads((directory / "execution.json").read_text())
    build = json.loads((directory / "build.json").read_text())
    if (
        execution.get("schema_version") != "1"
        or execution.get("artifact") != "signals.json"
    ):
        raise ValueError("unsupported execution provenance")
    if (
        source is not None
        and build.get("executor", {}).get("source_revision") != source
    ):
        raise ValueError("execution source differs from the expected candidate/tag")
    if platform is not None and build.get("executor", {}).get("platform") != platform:
        raise ValueError("execution platform differs from the expected platform")
    if (
        execution.get("artifact_digest") != digest(directory / "signals.json")
        or execution.get("build") != build
    ):
        raise ValueError("execution provenance does not match signals and build record")


def run(args: argparse.Namespace) -> None:
    fixture = args.fixture_root.resolve()
    artifacts = args.artifact_directory.resolve()
    artifacts.mkdir(parents=True, exist_ok=True)
    cli = str(args.cli.resolve())
    targets = None
    if getattr(args, "composition", None):
        targets = composition.expected_targets(args.composition)
    environment = os.environ.copy()
    wrapper = tempfile.TemporaryDirectory(prefix="ayni-engine-observer-")
    launches = artifacts / "container-launches.jsonl"
    docker = shutil.which("docker")
    if targets:
        observer = Path(wrapper.name) / "docker"
        observer.write_text("#!/usr/bin/env python3\nimport json, os, sys\n"
                            "args = sys.argv[1:]\n"
                            "if args and args[0] == 'run':\n"
                            "    kind = 'quality' if any('/opt/ayni/checkout' in arg for arg in args) else 'setup-or-access'\n"
                            f"    with open({str(launches)!r}, 'a') as out: out.write(json.dumps({{'kind': kind}}) + '\\n')\n"
                            f"os.execv({docker!r}, [{docker!r}] + args)\n")
        observer.chmod(0o755)
        environment["PATH"] = wrapper.name + os.pathsep + environment["PATH"]
    receipt = {
        "schema_version": 1,
        "id": args.id,
        "source": args.source,
        "platform": args.platform,
        "success": False,
    }
    timings = artifacts / "timings.jsonl"

    def stage(name: str, *command: str, expected: int = 0) -> int:
        started = time.monotonic()
        code = 127
        try:
            with (artifacts / f"{name}.log").open("w") as log:
                with subprocess.Popen(
                    command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                    env=environment,
                ) as process:
                    assert process.stdout is not None
                    for line in process.stdout:
                        print(line, end="", flush=True)
                        log.write(line)
                    code = process.wait()
        finally:
            with timings.open("a") as output:
                output.write(
                    json.dumps(
                        {
                            "stage": name,
                            "duration_seconds": round(time.monotonic() - started, 3),
                            "exit_code": code,
                        }
                    )
                    + "\n"
                )
        if code != expected:
            raise ValueError(
                f"{name} exited {code}, expected {expected}; see {artifacts}"
            )
        return code

    try:
        if targets:
            if not docker:
                raise ValueError("composition validation requires Docker launch accounting")
            if not args.committed_lock:
                composition.materialize(Path.cwd(), fixture, args.composition)
        # Fixtures are disposable; the repository contract consumes its committed lock.
        if not args.committed_lock:
            shutil.rmtree(fixture / ".ayni", ignore_errors=True)
            (fixture / ".ayni.lock").unlink(missing_ok=True)
            stage(
                "lock",
                cli,
                "env",
                "lock",
                "--repo-root",
                str(fixture),
                "--config",
                ".ayni.toml",
            )
        stage(
            "build",
            cli,
            "env",
            "build",
            "--repo-root",
            str(fixture),
            "--executor-image",
            args.executor_image,
            *(value for option in ("cache_from", "cache_to")
              for cache in getattr(args, option, [])
              for value in ("--" + option.replace("_", "-"), cache)),
        )
        stage("prepare", cli, "env", "run", "--repo-root", str(fixture), "--", "true")
        stage("doctor", cli, "env", "doctor", "--repo-root", str(fixture))
        if targets:
            commands = ['test "$PWD" = /workspace', 'test "$(jq --version)" = jq-1.7.1',
                        'case "$PATH" in *node_modules/.bin*|*.venv/bin*) exit 1;; esac',
                        'touch /workspace/development-write']
            if "rust" in targets:
                commands.append('rustc /workspace/interactions/rust-node.rs -o /tmp/rust-node && /tmp/rust-node')
            stage("repository-access", cli, "env", "run", "--repo-root", str(fixture),
                  "--", "sh", "-ec", "\n".join(commands))
            if not (fixture / "development-write").exists():
                raise ValueError("repository access did not preserve writable development source")
        code = stage(
            "check",
            cli,
            "check",
            "--config",
            str(fixture / ".ayni.toml"),
            expected=args.expected_exit_code,
        )
        started = time.monotonic()
        signals = json.loads((fixture / ".ayni/last/signals.json").read_text())
        if targets:
            composition.validate(signals, targets, code)
            observed = [json.loads(line) for line in launches.read_text().splitlines()]
            quality = sum(item["kind"] == "quality" for item in observed)
            if quality != 1:
                raise ValueError(f"expected one quality workload container, observed {quality}")
            (artifacts / "composition.json").write_text(json.dumps({
                "targets": targets, "quality_launches": quality,
                "setup_and_access_launches": len(observed) - quality,
            }, indent=2) + "\n")
        if args.language:
            validate_artifact(
                signals,
                language=args.language,
                expected_roots=args.expected_root,
                repository_root="/workspace",
                check_exit_code=code,
            )
        # Check success itself enforces the unscoped repository policy contract.
        # Sidecars are independently bound and revalidated by the completion job.
        for source, destination in (
            (".ayni/last/signals.json", "signals.json"),
            (".ayni/last/execution.json", "execution.json"),
            (".ayni/environment/build.json", "build.json"),
        ):
            shutil.copyfile(fixture / source, artifacts / destination)
        validate_provenance(artifacts, args.source, args.platform)
        with timings.open("a") as output:
            output.write(
                json.dumps(
                    {
                        "stage": "validate",
                        "duration_seconds": round(time.monotonic() - started, 3),
                        "exit_code": 0,
                    }
                )
                + "\n"
            )
        receipt["success"] = True
    finally:
        # Preserve diagnostics even when setup or checking fails, before any cleanup.
        for source, destination in (
            (".ayni.lock", "lock.json"),
            (".ayni/last/signals.json", "signals.json"),
            (".ayni/last/execution.json", "execution.json"),
            (".ayni/environment/build.json", "build.json"),
        ):
            path = fixture / source
            if path.is_file():
                shutil.copyfile(path, artifacts / destination)
        receipt["files"] = {
            path.name: digest(path)
            for path in artifacts.iterdir()
            if path.is_file() and path.name != "receipt.json"
        }
        (artifacts / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        wrapper.cleanup()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--executor-image", required=True)
    parser.add_argument("--fixture-root", type=Path, required=True)
    parser.add_argument("--artifact-directory", type=Path, required=True)
    parser.add_argument("--id", required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--language")
    parser.add_argument("--composition", choices=composition.CASES)
    parser.add_argument("--cache-from", action="append", default=[])
    parser.add_argument("--cache-to", action="append", default=[])
    parser.add_argument("--expected-root", action="append", default=[])
    parser.add_argument("--expected-exit-code", type=int, required=True)
    parser.add_argument("--committed-lock", action="store_true")
    args = parser.parse_args()
    try:
        run(args)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"managed fixture failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
