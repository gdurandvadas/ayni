"""Regression evidence for composition, proportional selection and release recovery."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import composition
from coordinate import plan
import release_artifacts as release
from test_validate_example_artifact import artifact


def composed_artifact(case):
    targets = composition.expected_targets(case)
    documents = []
    for root, language in targets.items():
        document = artifact(language=language)
        for field in ("rows", "applied_thresholds", "offender_summaries"):
            for row in document[field]:
                row["scope"]["path"] = root
        documents.append(document)
    result = copy.deepcopy(documents[0])
    result["invocation"]["languages"] = sorted(set(targets.values()))
    for field in ("expected_targets", "completed_targets", "detected_targets"):
        result["completion"][field] = len(targets)
    for field in ("rows", "applied_thresholds", "offender_summaries"):
        result[field] = [row for document in documents for row in document[field]]
    for field in result["aggregate"]:
        if field != "status":
            result["aggregate"][field] = sum(document["aggregate"][field] for document in documents)
    result["aggregate"]["status"] = "fail"
    return result


class CompositionTests(unittest.TestCase):
    def test_all_targets_and_aggregate_failures_are_required(self):
        for case in composition.CASES:
            expected = composition.expected_targets(case)
            document = composed_artifact(case)
            composition.validate(document, expected, 1)
            for mutate in (lambda d: d["rows"].pop(),
                           lambda d: d["completion"].update(skipped_targets=1),
                           lambda d: d["aggregate"].update(failing_rows=0),
                           lambda d: d["rows"][0]["result"].update(failure="missing tool")):
                changed = copy.deepcopy(document)
                mutate(changed)
                with self.assertRaises(ValueError):
                    composition.validate(changed, expected, 1)

    def test_conservative_fixture_selection(self):
        manifest = json.loads((Path(__file__).parents[2] / "scripts/ci/fixtures.json").read_text())
        full = {item["id"] for item in manifest["fixtures"]}
        for changes in (None, [], ["Cargo.lock"], ["core/src/lib.rs"], ["unknown"], [".github/workflows/release.yml"]):
            self.assertEqual({item["id"] for item in plan(manifest, "source", changes)["fixtures"]}, full)
        selected = plan(manifest, "source", ["adapters/go/src/lib.rs"])
        self.assertEqual({item["id"] for item in selected["fixtures"]}, {"go", "all-five", "kotlin-go"})
        self.assertTrue(selected["selection"]["omitted"])


class ReleaseTests(unittest.TestCase):
    def test_tagged_source_and_annotated_tag_recovery(self):
        sha = "a" * 40
        self.assertEqual(release.resolve("ayni-v1.2.3", lambda _: {"object": {"type": "commit", "sha": sha}}), sha)
        objects = iter([{"type": "tag", "sha": "b" * 40}, {"type": "commit", "sha": sha}])
        self.assertEqual(release.resolve("ayni-v1.2.3", lambda _: {"object": next(objects)}), sha)
        for obj in ({"type": "blob", "sha": sha}, {"type": "tag", "sha": sha}, {"type": "commit", "sha": "bad"}):
            with self.assertRaises(ValueError):
                release.resolve("ayni-v1.2.3", lambda _: {"object": obj})

    def test_partial_and_corrupt_publication_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tag = "ayni-v1.2.3"
            for name in release.inventory(tag) - {"SHA256SUMS"}:
                (root / name).write_bytes(b"binary archive")
            release.checksums(root, tag)
            release.checksums(root, tag, verify=True)
            file = next(root.glob("*.tar.gz"))
            file.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                release.checksums(root, tag, verify=True)
            file.unlink()
            with self.assertRaises(ValueError):
                release.checksums(root, tag)

    def test_upload_revalidates_source_and_overwrites_existing_assets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            tag, sha = "ayni-v1.2.3", "a" * 40
            for name in release.inventory(tag) - {"SHA256SUMS"}:
                (root / name).write_bytes(b"archive")
            release.checksums(root, tag)
            args = ["release_artifacts.py", "upload", "--tag", tag, "--expected-source", sha, "--directory", directory]
            with patch("sys.argv", args), patch.object(release, "resolve", return_value=sha), \
                    patch.object(release, "public_release", return_value={"assets": [{"name": "obsolete", "id": 42}]}), \
                    patch.object(release, "gh") as gh, patch.dict("os.environ", GITHUB_REPOSITORY="owner/repo"):
                release.main()
                self.assertEqual(gh.call_args_list[0].args[:3], ("api", "--method", "DELETE"))
                self.assertIn("--clobber", gh.call_args_list[1].args)
            with patch("sys.argv", args), patch.object(release, "resolve", return_value="b" * 40), patch.object(release, "gh") as gh:
                with self.assertRaises(ValueError):
                    release.main()
                gh.assert_not_called()


if __name__ == "__main__":
    unittest.main()
