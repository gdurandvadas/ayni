import copy
import unittest
from pathlib import Path

from validate_example_artifact import EXPECTED_KINDS, ValidationError, validate_artifact


ROOT = "fixture"
LEXICAL_ROOT = ROOT


def artifact(*, policy_fail=False):
    rows = []
    for kind in sorted(EXPECTED_KINDS):
        fails = policy_fail and kind == "coverage"
        item = {"file": "src/example.py", "level": "fail"} if fails else None
        rows.append(
            {
                "kind": kind,
                "language": "python",
                "scope": {"workspace_root": LEXICAL_ROOT},
                "pass": not fails,
                "result": {"kind": kind},
                "budget": {"kind": kind},
                "offenders": {"kind": kind, "items": [item] if item else []},
            }
        )
    return {
        "schema_version": "0.3.0",
        "repository_root": LEXICAL_ROOT,
        "invocation": {"command": "check", "languages": ["python"]},
        "completion": {
            "scope": "repository",
            "state": "complete",
            "expected_targets": 1,
            "detected_targets": 1,
            "completed_targets": 1,
            "skipped_targets": 0,
            "issues": [],
        },
        "aggregate": {
            "status": "fail" if policy_fail else "pass",
            "total_rows": 5,
            "passing_rows": 4 if policy_fail else 5,
            "failing_rows": 1 if policy_fail else 0,
            "warning_offenders": 0,
            "failing_offenders": 1 if policy_fail else 0,
        },
        "rows": rows,
    }


def validate(value):
    validate_artifact(
        value, language="python", expected_roots=["."], repository_root=ROOT
    )


class ArtifactValidatorTests(unittest.TestCase):
    def assert_invalid(self, value, text):
        with self.assertRaisesRegex(ValidationError, text):
            validate(value)

    def test_valid_pass(self):
        validate(artifact())

    def test_relative_fixture_root_is_compared_lexically(self):
        value = artifact()
        self.assertFalse(Path(value["repository_root"]).is_absolute())
        validate(value)

    def test_valid_policy_fail(self):
        validate(artifact(policy_fail=True))

    def test_missing_row(self):
        value = artifact()
        value["rows"].pop()
        self.assert_invalid(value, "exactly 5")

    def test_duplicate_row(self):
        value = artifact()
        value["rows"][-1] = copy.deepcopy(value["rows"][0])
        self.assert_invalid(value, "duplicate row key")

    def test_unexpected_extra_row(self):
        value = artifact()
        value["rows"].append(copy.deepcopy(value["rows"][0]))
        self.assert_invalid(value, "exactly 5")

    def test_incomplete_completion(self):
        value = artifact()
        value["completion"]["state"] = "incomplete"
        self.assert_invalid(value, "complete state")

    def test_result_failure(self):
        value = artifact()
        value["rows"][0]["result"]["failure"] = {"message": "tool crashed"}
        self.assert_invalid(value, "command failure")

    def test_wrong_schema(self):
        value = artifact()
        value["schema_version"] = "0.2.0"
        self.assert_invalid(value, "schema_version")

    def test_wrong_language_target(self):
        value = artifact()
        value["rows"][0]["language"] = "go"
        self.assert_invalid(value, "language")

    def test_wrong_workspace_root(self):
        value = artifact()
        value["rows"][0]["scope"]["workspace_root"] = "/wrong"
        self.assert_invalid(value, "workspace_root")

    def test_wrong_configured_root(self):
        value = artifact()
        value["rows"][0]["scope"]["path"] = "other"
        self.assert_invalid(value, "unexpected target")

    def test_wrong_kind(self):
        value = artifact()
        value["rows"][0]["kind"] = "mutation"
        self.assert_invalid(value, "mutation is enabled")

    def test_non_finite_number(self):
        value = artifact()
        value["rows"][0]["result"]["percent"] = float("nan")
        self.assert_invalid(value, "non-finite")

    def test_failing_row_requires_typed_finding(self):
        value = artifact()
        value["rows"][0]["pass"] = False
        self.assert_invalid(value, "without a typed policy finding")


if __name__ == "__main__":
    unittest.main()
