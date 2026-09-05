"""Validate release workflow fail-closed publication invariants.

This deliberately checks the workflow source without adding a YAML dependency. The
contract protects the handoff from Release Please to downstream publication jobs,
which GitHub otherwise treats as a successful run when every consumer is skipped.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
JOB = re.compile(r"^  ([a-z][a-z0-9-]*):\n", re.MULTILINE)


def job_block(source: str, name: str) -> str:
    matches = list(JOB.finditer(source))
    for index, match in enumerate(matches):
        if match.group(1) != name:
            continue
        end = matches[index + 1].start() if index + 1 < len(matches) else len(source)
        return source[match.start() : end]
    raise ValueError(f"missing release workflow job: {name}")


def require(errors: list[str], condition: bool, message: str) -> None:
    if not condition:
        errors.append(message)


def main() -> int:
    source = WORKFLOW.read_text()
    errors: list[str] = []

    try:
        release = job_block(source, "release")
        completion = job_block(source, "release-completion")
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1

    outputs = release.split("\n    steps:\n", 1)[0]
    expected_outputs = (
        "release_created: ${{ steps.release-metadata.outputs.release_created }}",
        "release_tag: ${{ steps.release-metadata.outputs.tag }}",
        "release_version: ${{ steps.release-metadata.outputs.version }}",
        "release_commit: ${{ steps.release-source.outputs.commit }}",
        "release_pr_created: ${{ steps.release-pr.outputs.created }}",
        "release_pr_branch: ${{ steps.release-pr.outputs.branch }}",
    )
    for output in expected_outputs:
        require(errors, output in outputs, f"release job must export normalized {output}")
    require(
        errors,
        "||" not in outputs,
        "release job outputs must not compose skipped-step and action outputs with ||",
    )
    require(
        errors,
        "id: release-metadata" in release
        and 'echo "release_created=true"' in release
        and 'echo "tag=$TAG"' in release
        and 'echo "version=$VERSION"' in release,
        "release metadata step must emit explicit downstream publication outputs",
    )
    require(
        errors,
        "if: ${{ steps.release-metadata.outputs.release_created == 'true' }}"
        in release,
        "immutable source resolution must consume normalized release metadata",
    )
    require(
        errors,
        'if [[ "$EVENT_NAME" == "push" ]]' in release
        and 'test "$sha" = "$TRIGGER_COMMIT"' in release,
        "initial release source must equal the triggering commit",
    )
    require(
        errors,
        "id: release-pr" in release
        and 'echo "created=true"' in release
        and 'echo "branch=$branch"' in release,
        "release pull request metadata must be normalized before job export",
    )

    publication_jobs = (
        "build",
        "publish",
        "environment-image",
        "environment-manifest",
        "release-assets",
        "release-smoke",
        "environment-image-smoke",
        "release-validation",
    )
    for name in publication_jobs:
        try:
            block = job_block(source, name)
        except ValueError as error:
            errors.append(str(error))
            continue
        require(
            errors,
            "needs.release.outputs.release_created == 'true'" in block,
            f"{name} must be gated by normalized release creation metadata",
        )
        require(
            errors,
            f"      - {name}\n" in completion,
            f"release-completion must depend on {name}",
        )

    publish = job_block(source, "publish")
    delete_position = publish.find("Remove obsolete release assets before recovery upload")
    upload_position = publish.find("Upload release artifacts")
    expected_asset_patterns = (
        "SHA256SUMS|",
        '"ayni-${TAG}-aarch64-apple-darwin.tar.gz"',
        '"ayni-${TAG}-x86_64-apple-darwin.tar.gz"',
        '"ayni-${TAG}-x86_64-unknown-linux-gnu.tar.gz"',
        '"ayni-${TAG}-aarch64-unknown-linux-gnu.tar.gz"',
    )
    require(
        errors,
        delete_position >= 0
        and upload_position > delete_position
        and "releases/assets/${asset_id}" in publish
        and "--method DELETE" in publish
        and all(pattern in publish for pattern in expected_asset_patterns)
        and "overwrite_files: true" in publish,
        "publish must preserve expected names, delete obsolete assets before upload, "
        "and overwrite expected assets",
    )

    release_assets = job_block(source, "release-assets")
    require(
        errors,
        "Install the latest public release without a version override" in release_assets
        and 'releases/latest" --jq' in release_assets
        and "./install.sh" in release_assets,
        "release-assets must exercise default latest-release installer resolution",
    )

    completion_requirements = (
        "if: ${{ always() }}",
        'test "$RELEASE_RESULT" = "success"',
        "resolve_tag_commit()",
        "contents/version.txt?ref=${GITHUB_SHA}",
        'candidate_tag="ayni-v${source_version}"',
        '[[ "$candidate_commit" == "$GITHUB_SHA" ]]',
        "lookup_code=$?",
        "^HTTP/[^ ]+ 404 ",
        'exit "$lookup_code"',
        'test "$RELEASE_CREATED" = "true"',
        'test "$RELEASE_COMMIT" = "$GITHUB_SHA"',
        'test "$(resolve_tag_commit "$RELEASE_TAG")" = "$RELEASE_COMMIT"',
        "Required release job '$job' concluded '$result'.",
        "test \"$(jq '.assets | length' <<< \"$release\")\" -eq 5",
    )
    for requirement in completion_requirements:
        require(
            errors,
            requirement in completion,
            f"release-completion is missing fail-closed check: {requirement}",
        )
    require(
        errors,
        "target_commitish" not in completion,
        "release-completion must verify the peeled tag commit, not target_commitish metadata",
    )

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("release workflow preserves fail-closed publication handoff")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
