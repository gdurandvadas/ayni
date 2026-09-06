#!/usr/bin/env bash
set -euo pipefail
# Native Linux only: the very same executable orchestrates and executes checks.
source .github/docker/ayni-env.versions
output="${1:?candidate output directory required}"
platform="${2:?platform required}"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
case "$platform/$(uname -m)" in
  linux/amd64/x86_64|linux/arm64/aarch64) ;;
  *) echo "candidate builds require a matching native Linux runner" >&2; exit 1 ;;
esac
# The cache is local to this unprivileged job; never consumed by publication.
mkdir -p .ayni/ci-cargo .ayni/ci-target
docker run --rm --platform "$platform" --user "$(id -u):$(id -g)" \
  --env CARGO_HOME=/workspace/.ayni/ci-cargo \
  --env CARGO_TARGET_DIR=/workspace/.ayni/ci-target \
  --volume "$PWD:/workspace" --workdir /workspace "$RUST_BUILDER_IMAGE" \
  cargo build --locked --release -p ayni-cli
cp .ayni/ci-target/release/ayni "$output/ayni"
cp LICENSE NOTICE "$output/"
version="$("$output/ayni" --version)"
version="${version#ayni }"
source_revision="$(git rev-parse HEAD)"
docker build --provenance=false --platform "$platform" \
  --build-arg "DEBIAN_IMAGE=$DEBIAN_IMAGE" --build-arg "AYNI_VERSION=$version" \
  --build-arg "SOURCE_REVISION=$source_revision" \
  --file .github/docker/ayni-candidate.Dockerfile --tag ayni-candidate:checkout "$output"
docker save --output "$output/executor.tar" ayni-candidate:checkout
python3 scripts/ci/candidate.py record --directory "$output" --source "$source_revision" \
  --platform "$platform" --run-id "${GITHUB_RUN_ID:?same-run identity required}"
