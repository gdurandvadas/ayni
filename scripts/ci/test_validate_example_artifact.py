import copy
import unittest
from pathlib import Path

from validate_example_artifact import (
    EXPECTED_KINDS,
    EXPECTED_OUTCOMES,
    ValidationError,
    validate_artifact,
)


REPOSITORY_ROOT = "/workspace"


def finding(level):
    return {
        "id": "ayni:finding:v1:sha256:" + "4" * 64,
        "verification": {"command": "ayni verify coverage --language python"},
        "file": "src/example.py",
        "level": level,
    }


def artifact(*, language="python"):
    outcome = EXPECTED_OUTCOMES[language]
    rows = []
    for kind in sorted(EXPECTED_KINDS):
        fails = kind in outcome.failing_kinds
        warns = kind in outcome.warning_kinds
        item = (
            finding("fail")
            if fails
            else finding("warn")
            if warns
            else None
        )
        result = {"kind": kind}
        if kind == "coverage":
            result["percent"] = sum(outcome.coverage_range) / 2
        rows.append(
            {
                "kind": kind,
                "language": language,
                "scope": {"workspace_root": REPOSITORY_ROOT},
                "pass": not fails,
                "result": result,
                "budget": {"kind": kind},
                "offenders": {"kind": kind, "items": [item] if item else []},
            }
        )
    return {
        "schema_version": "0.4.0",
        "execution_mode": "managed",
        "contract_digest": "sha256:" + "1" * 64,
        "environment_lock_fingerprint": "sha256:" + "2" * 64,
        "source_fingerprint": "sha256:" + "3" * 64,
        "tool_versions": [
            {"tool": f"{language}:.:runtime:{language}", "version": "1.0.0"}
        ],
        "repository_root": REPOSITORY_ROOT,
        "invocation": {"command": "check", "languages": [language]},
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
            "status": outcome.aggregate_status,
            "total_rows": 5,
            "passing_rows": 5 - len(outcome.failing_kinds),
            "failing_rows": len(outcome.failing_kinds),
            "warning_offenders": outcome.warning_offenders,
            "failing_offenders": outcome.failing_offenders,
        },
        "rows": rows,
    }


def validate(value, *, language="python", check_exit_code=None):
    if check_exit_code is None:
        check_exit_code = EXPECTED_OUTCOMES[language].check_exit_code
    validate_artifact(
        value,
        language=language,
        expected_roots=["."],
        repository_root=REPOSITORY_ROOT,
        check_exit_code=check_exit_code,
    )


class ArtifactValidatorTests(unittest.TestCase):
    def assert_invalid(self, value, text, **kwargs):
        with self.assertRaisesRegex(ValidationError, text):
            validate(value, **kwargs)

    def test_expected_fixture_outcomes(self):
        for language in EXPECTED_OUTCOMES:
            with self.subTest(language=language):
                validate(artifact(language=language), language=language)

    def test_managed_repository_root_is_canonical(self):
        value = artifact()
        self.assertTrue(Path(value["repository_root"]).is_absolute())
        validate(value)

    def test_wrong_check_exit_code(self):
        self.assert_invalid(artifact(), "check exit code", check_exit_code=1)

    def test_unexpected_python_policy_failure(self):
        value = artifact()
        coverage = next(row for row in value["rows"] if row["kind"] == "coverage")
        coverage["pass"] = False
        coverage["offenders"]["items"] = [finding("fail")]
        value["aggregate"].update(
            status="fail", passing_rows=4, failing_rows=1, failing_offenders=1
        )
        self.assert_invalid(value, "aggregate.status", language="python")

    def test_coverage_drift_is_rejected(self):
        value = artifact(language="go")
        coverage = next(row for row in value["rows"] if row["kind"] == "coverage")
        coverage["result"]["percent"] = 90.0
        self.assert_invalid(value, "coverage percent", language="go")

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

    def test_host_evidence_is_rejected(self):
        value = artifact()
        value["execution_mode"] = "host"
        self.assert_invalid(value, "managed evidence")

    def test_missing_lock_fingerprint_is_rejected(self):
        value = artifact()
        del value["environment_lock_fingerprint"]
        self.assert_invalid(value, "environment_lock_fingerprint")

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

    def test_finding_requires_schema_v4_identity(self):
        value = artifact(language="kotlin")
        del value["rows"][1]["offenders"]["items"][0]["id"]
        self.assert_invalid(value, "schema-v4 finding ID", language="kotlin")


if __name__ == "__main__":
    unittest.main()
