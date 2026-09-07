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
    publication = (WORKFLOW.parent / "release-publication.yml").read_text()
    errors: list[str] = []

    try:
        release = job_block(source, "release")
        completion = job_block(publication, "release-completion")
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1

    for removed_job in ("validate-release-dispatch", "quality", "managed-quality"):
        require(
            errors,
            f"  {removed_job}:\n" not in source,
            f"release workflow must not rerun the {removed_job} job on every main push",
        )
    require(
        errors,
        'WORKFLOW_REF: ${{ github.ref }}' in release
        and 'test "$WORKFLOW_REF" = "refs/heads/main"' in release,
        "manual recovery must validate the main workflow revision inside release metadata resolution",
    )

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
            block = job_block(publication, name)
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

    publish = job_block(publication, "publish")
    require(errors,
            "id: publication-token" in publish
            and "permission-contents: write" in publish
            and "GH_TOKEN: ${{ steps.publication-token.outputs.token }}" in publish
            and 'release_artifacts.py upload --tag "$TAG" --expected-source "$EXPECTED_COMMIT"' in publish,
            "publication must use a fresh app token and the source-bound overwrite helper")
    require(errors,
            'release-publication-${{ needs.release.outputs.release_tag }}' in source
            and 'cancel-in-progress: false' in job_block(source, 'publication')
            and 'release-pr-maintenance' in job_block(source, 'sync-release-lock')
            and 'if: ${{ always() }}' in job_block(source, 'release-completion')
            and 'publication $PUBLICATION_RESULT' in job_block(source, 'release-completion'),
            "publication must serialize each release identity and retain an independent parent completion gate")
    require(errors,
            'cargo build' not in job_block(publication, 'environment-image')
            and 'Download same-run release executable' in job_block(publication, 'environment-image')
            and 'source .github/docker/ayni-env.versions' in job_block(publication, 'build'),
            "Linux archives and executor images must share one compatible compilation")
    require(errors,
            'ubuntu-24.04-arm' in job_block(publication, 'fixture-plan')
            and 'Run declared fixture against public artifacts' in job_block(publication, 'release-validation'),
            "public evidence must exercise the tagged fixture inventory on both Linux architectures")

    release_assets = job_block(publication, "release-assets")
    require(
        errors,
        "Install the latest public release without a version override" in release_assets
        and 'releases/latest" --jq' in release_assets
        and 'release_artifacts.py verify --tag "$TAG" --expected-source "$EXPECTED_COMMIT"' in release_assets
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
