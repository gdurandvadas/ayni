#!/usr/bin/env python3
"""Regression checks for same-run handoff and fail-closed required work."""
import argparse
import copy
import json
from pathlib import Path
import tempfile
import unittest

from candidate import sha256, verify
from coordinate import JOBS, complete, plan
from run_fixture import digest, run, validate_provenance

ROOT = Path(__file__).resolve().parents[2]
SOURCE = "a" * 40


class CandidateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.path = Path(self.temporary.name)
        for name in ("ayni", "executor.tar"):
            (self.path / name).write_bytes(b"candidate")
        self.metadata = {
            "schema_version": 1,
            "source": SOURCE,
            "platform": "linux/amd64",
            "run_id": "123",
            "image_id": "sha256:" + "b" * 64,
            "files": {
                name: sha256(self.path / name) for name in ("ayni", "executor.tar")
            },
        }
        self.write()

    def write(self):
        (self.path / "candidate.json").write_text(json.dumps(self.metadata))

    def verify(self):
        return verify(self.path, SOURCE, "linux/amd64", "123")

    def test_same_run_candidate(self):
        self.assertEqual(self.verify(), self.metadata)

    def test_different_source_platform_or_run(self):
        for key in ("source", "platform", "run_id", "schema_version"):
            with self.subTest(key=key):
                original = self.metadata[key]
                self.metadata[key] = "wrong"
                self.write()
                with self.assertRaises(ValueError):
                    self.verify()
                self.metadata[key] = original

    def test_corrupt_or_unavailable_artifacts(self):
        for name in ("ayni", "executor.tar"):
            with self.subTest(name=name):
                file = self.path / name
                file.write_bytes(b"corrupt")
                with self.assertRaises(ValueError):
                    self.verify()
                file.unlink()
                with self.assertRaises(ValueError):
                    self.verify()
                file.write_bytes(b"candidate")

    def test_unexpected_inventory_and_symlinks(self):
        self.metadata["files"]["../other"] = "a" * 64
        self.write()
        with self.assertRaises(ValueError):
            self.verify()
        del self.metadata["files"]["../other"]
        self.write()
        (self.path / "ayni").unlink()
        (self.path / "ayni").symlink_to(self.path / "executor.tar")
        with self.assertRaises(ValueError):
            self.verify()


class CompletionTests(unittest.TestCase):
    def setUp(self):
        self.manifest = json.loads((ROOT / "scripts/ci/fixtures.json").read_text())
        self.plan = plan(self.manifest, SOURCE)
        self.results = {job: {"result": "success"} for job in JOBS}

    def test_explicit_rust_independent_of_repository(self):
        self.assertIn({"id": "repository", "arch": "amd64"}, self.plan["evidence"])
        self.assertIn({"id": "rust", "arch": "amd64"}, self.plan["evidence"])
        self.assertEqual(len(self.plan["evidence"]), 9)

    def test_cancelled_failed_and_skipped_jobs(self):
        for job in JOBS:
            for result in ("failure", "cancelled", "skipped", ""):
                with self.subTest(job=job, result=result):
                    results = copy.deepcopy(self.results)
                    results[job]["result"] = result
                    with self.assertRaisesRegex(ValueError, "required jobs"):
                        complete(self.plan, results, Path("unused"), SOURCE)

    def test_missing_job_even_when_others_succeeded(self):
        del self.results["fixtures"]
        with self.assertRaisesRegex(ValueError, "inventory"):
            complete(self.plan, self.results, Path("unused"), SOURCE)

    def test_missing_evidence_even_when_jobs_succeeded(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(FileNotFoundError):
                complete(self.plan, self.results, Path(directory), SOURCE)

    def test_complete_evidence_then_reject_missing_target_and_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for entry in self.plan["evidence"]:
                folder = root / f'managed-{entry["id"]}-{entry["arch"]}'
                folder.mkdir()
                build = {
                    "executor": {"source_revision": SOURCE, "platform": "linux/amd64"},
                    "environment_fingerprint": "fingerprint",
                }
                signals = {}
                if entry['id'] in ('all-five', 'rust-node', 'kotlin-go'):
                    from test_delivery import composed_artifact
                    from composition import expected_targets
                    signals = composed_artifact(entry['id'])
                    (folder / 'composition.json').write_text(json.dumps({'targets': expected_targets(entry['id']), 'quality_launches': 1}))
                (folder / "signals.json").write_text(json.dumps(signals))
                (folder / "build.json").write_text(json.dumps(build))
                (folder / "lock.json").write_text('{"fingerprint": "fingerprint"}')
                (folder / "timings.jsonl").write_text("{}\n")
                (folder / "execution.json").write_text(
                    json.dumps(
                        {
                            "schema_version": "1",
                            "artifact": "signals.json",
                            "artifact_digest": digest(folder / "signals.json"),
                            "build": build,
                        }
                    )
                )
                receipt = {
                    "schema_version": 1,
                    "id": entry["id"],
                    "source": SOURCE,
                    "platform": "linux/amd64",
                    "success": True,
                    "files": {file.name: digest(file) for file in folder.iterdir()},
                }
                (folder / "receipt.json").write_text(json.dumps(receipt))
            lock = root / "managed-lock-amd64"
            lock.mkdir()
            for name in ("committed.json", "generated.json", "regenerated.json"):
                (lock / name).write_text("{}")
            complete(self.plan, self.results, root, SOURCE)
            (lock / "regenerated.json").write_text('{"changed": true}')
            with self.assertRaisesRegex(ValueError, "lock consistency"):
                complete(self.plan, self.results, root, SOURCE)
            (lock / "regenerated.json").write_text("{}")
            signals = root / "managed-node-amd64/signals.json"
            signals.write_text('{"tampered": true}')
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                complete(self.plan, self.results, root, SOURCE)
            signals.unlink()
            with self.assertRaises(FileNotFoundError):
                complete(self.plan, self.results, root, SOURCE)

    def test_empty_or_duplicate_fixture_inventory(self):
        for fixtures in ([], [self.manifest["fixtures"][0]] * 2):
            self.manifest["fixtures"] = fixtures
            with self.assertRaises(ValueError):
                plan(self.manifest, SOURCE)

    def test_provenance_binds_signals_and_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "signals.json").write_text("{}")
            (root / "build.json").write_text('{"executor": "original"}')
            (root / "execution.json").write_text(
                json.dumps(
                    {
                        "schema_version": "1",
                        "artifact": "signals.json",
                        "artifact_digest": digest(root / "signals.json"),
                        "build": {"executor": "original"},
                    }
                )
            )
            validate_provenance(root)
            (root / "signals.json").write_text('{"changed": true}')
            with self.assertRaises(ValueError):
                validate_provenance(root)

    def test_failed_setup_retains_diagnostics_and_unsuccessful_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            cli = root / "ayni"
            cli.write_text("#!/bin/sh\necho unavailable executor >&2\nexit 42\n")
            cli.chmod(0o755)
            artifacts = root / "artifacts"
            args = argparse.Namespace(
                cli=cli,
                fixture_root=fixture,
                artifact_directory=artifacts,
                id="node",
                source=SOURCE,
                platform="linux/amd64",
                committed_lock=False,
                executor_image="unused",
                expected_exit_code=1,
                language="node",
                expected_root=["."],
            )
            with self.assertRaisesRegex(ValueError, "exited 42"):
                run(args)
            receipt = json.loads((artifacts / "receipt.json").read_text())
            self.assertFalse(receipt["success"])
            self.assertIn("unavailable executor", (artifacts / "lock.log").read_text())
            self.assertEqual(
                json.loads((artifacts / "timings.jsonl").read_text())["exit_code"], 42
            )
            self.assertTrue(fixture.exists())

    def test_fork_workflow_has_no_privileged_candidate_consumers(self):
        workflow = (ROOT / ".github/workflows/ayni-status.yml").read_text()
        self.assertIn("  pull_request:", workflow)
        self.assertNotIn("pull_request_target", workflow)
        self.assertNotIn("workflow_run", workflow)
        self.assertNotIn(": write", workflow)
        self.assertNotIn("secrets:", workflow)
        action = (ROOT / ".github/actions/use-candidate/action.yml").read_text()
        self.assertNotIn("run-id:", action)
        self.assertNotIn("github-token:", action)
        for filename in ("ayni-fixtures.yml", "ayni-repository.yml"):
            consumer = (ROOT / ".github/workflows" / filename).read_text()
            self.assertNotIn("cargo build", consumer)
            self.assertNotIn("build-local-environment", consumer)
        self.assertNotIn(
            "candidate-", (ROOT / ".github/workflows/release.yml").read_text()
        )


if __name__ == "__main__":
    unittest.main()
