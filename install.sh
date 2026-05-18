#!/bin/sh

set -eu

REPO="${REPO:-gdurandvadas/ayni}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${VERSION:-}"
BIN_NAME="ayni"
DEFAULT_DIR="$HOME/.local/bin"

say() {
  printf '%s\n' "$*"
}

fail() {
  say "error: $*" >&2
  exit 1
}

warn() {
  say "warning: $*" >&2
}

have_cmd() {
  command -v "$1" >/dev/null 2>&1
}

is_tty() {
  [ -t 0 ] && [ -t 1 ]
}

confirm() {
  prompt="$1"
  default="${2:-Y}"

  if ! is_tty; then
    case "$default" in
      Y|y) return 0 ;;
      *) return 1 ;;
    esac
  fi

  printf '%s ' "$prompt"
  read -r answer || true
  answer=$(printf '%s' "$answer" | tr '[:upper:]' '[:lower:]')

  if [ -z "$answer" ]; then
    answer=$(printf '%s' "$default" | tr '[:upper:]' '[:lower:]')
  fi

  case "$answer" in
    y|yes) return 0 ;;
    n|no) return 1 ;;
    *)
      say "Please answer y or n."
      confirm "$prompt" "$default"
      ;;
  esac
}

download() {
  url="$1"
  output="$2"

  if have_cmd curl; then
    curl --proto '=https' --tlsv1.2 -fsSL "$url" -o "$output"
    return
  fi

  if have_cmd wget; then
    wget -qO "$output" "$url"
    return
  fi

  fail "curl or wget is required"
}

resolve_latest_version() {
  latest_url="https://github.com/$REPO/releases/latest"

  if have_cmd curl; then
    resolved="$(curl --proto '=https' --tlsv1.2 -fsSL -o /dev/null -w '%{url_effective}' "$latest_url")"
  elif have_cmd wget; then
    resolved="$(wget -qO- --max-redirect=0 "$latest_url" 2>&1 | sed -n 's/.*Location: .*\/tag\/\(v[0-9][^[:space:]]*\).*/\1/p' | tail -n 1)"
  else
    fail "curl or wget is required"
  fi

  version="${resolved##*/}"
  [ -n "$version" ] || fail "could not resolve the latest release version"
  printf '%s\n' "$version"
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Darwin) os_part="apple-darwin" ;;
    Linux) os_part="unknown-linux-gnu" ;;
    *) fail "unsupported operating system: $os" ;;
  esac

  case "$arch" in
    arm64|aarch64) arch_part="aarch64" ;;
    x86_64|amd64) arch_part="x86_64" ;;
    *) fail "unsupported architecture: $arch" ;;
  esac

  printf '%s-%s\n' "$arch_part" "$os_part"
}

ensure_install_dir() {
  if [ "$INSTALL_DIR" = "$DEFAULT_DIR" ] && is_tty; then
    if ! confirm "Install to $DEFAULT_DIR? [Y/n]" "Y"; then
      printf 'Install directory: '
      read -r chosen_dir
      [ -n "$chosen_dir" ] || fail "install directory cannot be empty"
      INSTALL_DIR="$chosen_dir"
    fi
  fi

  INSTALL_DIR="$(normalize_dir "$INSTALL_DIR")"
  mkdir -p "$INSTALL_DIR"
}

checksum_verify() {
  archive_path="$1"
  checksums_path="$2"
  archive_name="$3"

  expected="$(
    awk -v archive_name="$archive_name" '
      {
        file = $2
        sub(/^.*\//, "", file)
        if (file == archive_name) {
          print $1
          exit
        }
      }
    ' "$checksums_path"
  )"
  [ -n "$expected" ] || fail "checksum entry for ${archive_name} not found"

  if have_cmd sha256sum; then
    actual="$(sha256sum "$archive_path" | awk '{print $1}')"
  elif have_cmd shasum; then
    actual="$(shasum -a 256 "$archive_path" | awk '{print $1}')"
  else
    warn "sha256sum/shasum not found; skipping checksum verification"
    return
  fi

  [ "$expected" = "$actual" ] || fail "checksum verification failed for ${archive_name}"
}

append_path() {
  rc_file="$1"
  install_dir="$2"
  line="export PATH=\"$install_dir:\$PATH\""

  if [ ! -f "$rc_file" ]; then
    : > "$rc_file"
  fi

  if grep -Fqs "$line" "$rc_file"; then
    say "PATH entry already present in $rc_file"
    return
  fi

  {
    printf '\n'
    printf '%s\n' "$line"
  } >> "$rc_file"

  say "Added PATH entry to $rc_file"
}

maybe_update_path() {
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) return 0 ;;
  esac

  say
  warn "$INSTALL_DIR is not currently on PATH"

  shell_name="${SHELL##*/}"
  rc_file=""
  case "$shell_name" in
    zsh) rc_file="$HOME/.zshrc" ;;
    bash) rc_file="$HOME/.bashrc" ;;
  esac

  if is_tty && [ -n "$rc_file" ]; then
    if confirm "Append PATH update to $rc_file? [y/N]" "N"; then
      append_path "$rc_file" "$INSTALL_DIR"
      say "Open a new shell or run: . \"$rc_file\""
      return 0
    fi
  fi

  say "Add this line to your shell config:"
  say "export PATH=\"$INSTALL_DIR:\$PATH\""
}

normalize_dir() {
  case "$1" in
    "~") printf '%s\n' "$HOME" ;;
    "~/"*) printf '%s/%s\n' "$HOME" "${1#~/}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

main() {
  if [ -z "$VERSION" ]; then
    VERSION="$(resolve_latest_version)"
  fi

  target="$(detect_target)"
  archive="ayni-${VERSION}-${target}.tar.gz"
  base_url="https://github.com/$REPO/releases/download/$VERSION"

  ensure_install_dir

  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' EXIT INT TERM

  archive_path="$tmp_dir/$archive"
  checksums_path="$tmp_dir/SHA256SUMS"

  say "Downloading $archive"
  download "$base_url/$archive" "$archive_path"
  download "$base_url/SHA256SUMS" "$checksums_path"
  checksum_verify "$archive_path" "$checksums_path" "$archive"

  tar -xzf "$archive_path" -C "$tmp_dir"
  binary_path="$(find "$tmp_dir" -type f -name "$BIN_NAME" | head -n 1)"
  [ -n "$binary_path" ] || fail "could not find $BIN_NAME in the release archive"
  install -m 0755 "$binary_path" "$INSTALL_DIR/$BIN_NAME"

  say
  say "Installed $BIN_NAME to $INSTALL_DIR/$BIN_NAME"
  maybe_update_path
  say
  say "Verify with: $BIN_NAME --version"
}

main "$@"
