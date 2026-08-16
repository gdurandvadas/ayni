#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
versions_file="$repo_root/.github/docker/ayni-env.versions"

docker_arch="$(docker info --format '{{.Architecture}}')"
case "$docker_arch" in
  amd64|x86_64) platform_arch="amd64" ;;
  arm64|aarch64) platform_arch="arm64" ;;
  *)
    echo "unsupported Docker architecture: $docker_arch" >&2
    exit 2
    ;;
esac

rust_version="$(sed -n 's/^channel = "\([^"]*\)"$/\1/p' "$repo_root/rust-toolchain.toml" | head -n1)"
if [[ -z "$rust_version" ]]; then
  echo "could not resolve Rust channel from rust-toolchain.toml" >&2
  exit 2
fi

# Compile inside Linux so the binary copied into the Debian image is ELF,
# rather than the host's macOS Mach-O executable.
docker run --rm \
  --platform "linux/$platform_arch" \
  --user "$(id -u):$(id -g)" \
  --env CARGO_HOME=/tmp/cargo \
  --volume "$repo_root:/workspace" \
  --workdir /workspace \
  "rust:${rust_version}-bookworm" \
  cargo build --locked -p ayni-cli --release

context="$(mktemp -d "${TMPDIR:-/tmp}/ayni-env-context.XXXXXX")"
trap 'rm -rf "$context"' EXIT
cp "$repo_root/target/release/ayni" "$context/ayni"
cp "$repo_root/LICENSE" "$repo_root/NOTICE" "$context/"
cp "$repo_root/.github/docker/ayni-env.Dockerfile" "$context/"

# shellcheck disable=SC1090
source "$versions_file"
docker build \
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
printf '\nBuilt ayni-env:local (%s)\n' "$base_id"
printf 'Next: cargo run -p ayni-cli -- env lock --base "ayni-env:local@%s"\n' "$base_id"
