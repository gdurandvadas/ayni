#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
versions_file="$repo_root/.github/docker/ayni-env.versions"
# shellcheck disable=SC1090
source "$versions_file"

docker_arch="$(docker info --format '{{.Architecture}}')"
case "$docker_arch" in
  amd64|x86_64) platform_arch="amd64" ;;
  arm64|aarch64) platform_arch="arm64" ;;
  *)
    echo "unsupported Docker architecture: $docker_arch" >&2
    exit 2
    ;;
esac

# Compile inside Linux so the binary copied into the Debian image is ELF,
# rather than the host's macOS Mach-O executable.
docker run --rm \
  --platform "linux/$platform_arch" \
  --user "$(id -u):$(id -g)" \
  --env CARGO_HOME=/tmp/cargo \
  --volume "$repo_root:/workspace" \
  --workdir /workspace \
  "$RUST_BUILDER_IMAGE" \
  cargo build --locked -p ayni-cli --release

context="$(mktemp -d "${TMPDIR:-/tmp}/ayni-env-context.XXXXXX")"
trap 'rm -rf "$context"' EXIT
cp "$repo_root/target/release/ayni" "$context/ayni"
cp "$repo_root/LICENSE" "$repo_root/NOTICE" "$context/"
cp "$repo_root/.github/docker/ayni-env.Dockerfile" "$context/"

docker build \
  --provenance=false \
  --platform "linux/$platform_arch" \
  --build-arg "DEBIAN_IMAGE=$DEBIAN_IMAGE" \
  --build-arg "AYNI_VERSION=local" \
  --build-arg "SOURCE_REVISION=local" \
  --build-arg "MISE_VERSION=$MISE_VERSION" \
  --build-arg "MISE_SHA256_AMD64=$MISE_SHA256_AMD64" \
  --build-arg "MISE_SHA256_ARM64=$MISE_SHA256_ARM64" \
  --file "$context/ayni-env.Dockerfile" \
  --tag ayni-env:local \
  "$context"

base_id="$(docker image inspect ayni-env:local --format '{{.Id}}')"
base_reference="$(docker image inspect ayni-env:local --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}')"
printf '\nBuilt ayni-env:local (%s)\n' "$base_id"
if [[ -n "$base_reference" ]]; then
  printf 'Next: cargo run -p ayni-cli -- env lock --base "%s"\n' "$base_reference"
else
  printf '%s\n' \
    'This engine did not expose a repository manifest digest for the local tag.' \
    'Push the image to a local registry, then pass its exact RepoDigest to env lock.'
fi
