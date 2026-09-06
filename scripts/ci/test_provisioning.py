#!/usr/bin/env python3
"""Regression coverage for native platform completeness and failure evidence."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from provisioning import recipe_digest, validate_manifest


def image(arch):
    return {"digest": "sha256:" + "a" * 64,
            "platform": {"os": "linux", "architecture": arch}}


class ProvisioningTests(unittest.TestCase):
    def test_requires_both_platforms_exactly_once(self):
        validate_manifest({"manifests": [image("amd64"), image("arm64")]})
        for platforms in [[], ["amd64"], ["amd64", "amd64"], ["amd64", "arm64", "386"]]:
            with self.subTest(platforms=platforms), self.assertRaises(ValueError):
                validate_manifest({"manifests": [image(arch) for arch in platforms]})

    def test_only_attestations_may_have_unknown_platforms(self):
        entry = {"platform": {"os": "unknown", "architecture": "unknown"}}
        manifest = {"manifests": [image("amd64"), image("arm64"), entry]}
        with self.assertRaises(ValueError):
            validate_manifest(manifest)
        entry["annotations"] = {"vnd.docker.reference.type": "attestation-manifest"}
        validate_manifest(manifest)

    def test_rejects_missing_immutable_platform_digest(self):
        broken = image("arm64")
        broken["digest"] = "latest"
        with self.assertRaises(ValueError):
            validate_manifest({"manifests": [image("amd64"), broken]})

    def test_recipe_tracks_provisioning_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in [".github/docker/provisioning.versions", ".github/docker/ayni-provisioning.Dockerfile", "LICENSE", "NOTICE"]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            original = recipe_digest(root)
            (root / "unrelated.rs").write_text("changed source")
            self.assertEqual(original, recipe_digest(root))
            (root / ".github/docker/provisioning.versions").write_text("new tool version")
            self.assertNotEqual(original, recipe_digest(root))

    def test_timing_preserves_failure_code_and_omits_command_arguments(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "timings.jsonl"
            result = subprocess.run([
                sys.executable, str(Path(__file__).with_name("timed.py")),
                "--output", str(path), "--stage", "failing-check", "--",
                sys.executable, "-c", "raise SystemExit(7)", "sensitive-argument",
            ], check=False)
            self.assertEqual(result.returncode, 7)
            record = json.loads(path.read_text())
            self.assertEqual(record["exit_code"], 7)
            self.assertEqual(record["stage"], "failing-check")
            self.assertGreaterEqual(record["duration_seconds"], 0)
            self.assertNotIn("sensitive-argument", path.read_text())


if __name__ == "__main__":
    unittest.main()
