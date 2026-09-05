# Installation

Install the Ayni CLI first. Managed environments are provisioned separately for each repository from its committed environment lock.

## Install the latest release

The installer detects your operating system and architecture, downloads the matching release archive and `SHA256SUMS`, requires `sha256sum` or `shasum` to verify the archive, validates the archive layout, and installs `ayni` into `~/.local/bin`.

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh | sh
```

Review the installer before running it if that is your normal security practice. Running the downloaded script directly also enables its optional interactive install-directory and `PATH` prompts:

```sh
curl --proto '=https' --tlsv1.2 -fsSLO \
  https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh
less install.sh
sh install.sh
```

The installer needs `curl` or `wget`, `tar`, and `install`. Add `~/.local/bin` to `PATH` if it is not already present:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

## Pin a version or install location

Set `VERSION` to a release tag to make the installation reproducible:

```sh
VERSION=ayni-v0.10.0 sh install.sh
```

Set `INSTALL_DIR` to choose another binary directory:

```sh
INSTALL_DIR="$HOME/bin" sh install.sh
```

Both variables also work with the piped installer:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh |
  VERSION=ayni-v0.10.0 INSTALL_DIR="$HOME/bin" sh
```

## Supported release targets

| Platform | Release target |
| --- | --- |
| macOS on Apple silicon | `aarch64-apple-darwin` |
| macOS on Intel | `x86_64-apple-darwin` |
| Linux GNU on ARM64 | `aarch64-unknown-linux-gnu` |
| Linux GNU on x86-64 | `x86_64-unknown-linux-gnu` |

Windows and Linux distributions without glibc do not currently have prebuilt release archives. Managed execution is currently documented and released for the macOS and Linux targets above.

## Download a release manually

Release archives and checksums are published on the [GitHub Releases page](https://github.com/gdurandvadas/ayni/releases). A complete release contains one archive for each of the four supported targets plus `SHA256SUMS`; release automation fails if that public inventory or any installer smoke test is incomplete. Archive names include the complete release tag and follow this pattern:

```text
ayni-<release-tag>-<target>.tar.gz
```

For example, release tag `ayni-v0.10.0` produces archive names beginning with
`ayni-ayni-v0.10.0-`.

For example, select a version and target, then download both the archive and checksum manifest:

```sh
VERSION=ayni-v0.10.0
TARGET=aarch64-apple-darwin
ARCHIVE="ayni-${VERSION}-${TARGET}.tar.gz"
BASE_URL="https://github.com/gdurandvadas/ayni/releases/download/${VERSION}"

curl --proto '=https' --tlsv1.2 -fsSLO "${BASE_URL}/${ARCHIVE}"
curl --proto '=https' --tlsv1.2 -fsSLO "${BASE_URL}/SHA256SUMS"
```

Verify the download before extracting it. On macOS:

```sh
EXPECTED="$(awk -v name="$ARCHIVE" '{ file=$2; sub(/^.*\//, "", file); if (file==name) { print $1; exit } }' SHA256SUMS)"
ACTUAL="$(shasum -a 256 "$ARCHIVE" | awk '{ print $1 }')"
test -n "$EXPECTED" && test "$EXPECTED" = "$ACTUAL"
```

On Linux, replace `shasum -a 256` with `sha256sum`. Then extract and install:

```sh
tar -xzf "$ARCHIVE"
mkdir -p "$HOME/.local/bin"
install -m 0755 "ayni-${VERSION}-${TARGET}/ayni" "$HOME/.local/bin/ayni"
```

## Build from source

Building requires Git and the Rust toolchain version declared by the repository:

```sh
git clone https://github.com/gdurandvadas/ayni.git
cd ayni
cargo install --locked --path cli
```

Cargo installs the binary into `~/.cargo/bin` by default.

## Verify the installation

```sh
ayni --version
ayni --help
```

Installing the CLI does **not** install every language tool used by a repository. For reproducible execution, Ayni derives and builds a managed environment after the repository has an `.ayni.toml` and `.ayni.lock`.

## Managed-environment prerequisites

To use managed execution:

- install Docker with Buildx for `ayni env lock`'s default release-base resolution;
- keep Docker running, or use compatible Podman support for commands that consume an existing lock;
- install [Mise](https://mise.jdx.dev/), which is required and version-recorded for every `ayni env lock`; and
- commit the native project metadata and dependency locks required by each language adapter.

You can avoid base-image resolution during locking by passing an explicit immutable base:

```sh
ayni env lock --base '<image-reference>@sha256:<digest>'
```

See [Managed environments](/product/environments) for the complete lifecycle and per-language readiness requirements.

## Upgrade

Run the installer again to replace the existing binary with the latest release:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/gdurandvadas/ayni/main/install.sh | sh
```

Use `VERSION=<release-tag>` to upgrade to a specific version or roll back. After changing Ayni or repository runtime inputs, run `ayni env doctor`; regenerate and commit `.ayni.lock` when it reports stale state.

## Uninstall

Remove the installed binary from the directory you selected:

```sh
rm "$HOME/.local/bin/ayni"
```

If you added a dedicated Ayni `PATH` entry to a shell startup file, remove that line as well. Repository files such as `.ayni.toml`, `.ayni.lock`, `.ayni/`, and OCI images are not removed automatically.

Next: follow the [Quickstart](/getting-started/quickstart) to preview a policy and produce managed evidence.
