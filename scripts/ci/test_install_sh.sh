#!/bin/sh

set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT INT TERM
library="$tmp_dir/install-library.sh"
sed '$d' "$ROOT/install.sh" > "$library"

make_path() {
  test_bin="$1"
  mkdir -p "$test_bin"
  for command in sed tail grep; do
    command_path=$(command -v "$command")
    ln -s "$command_path" "$test_bin/$command"
  done
}

# GitHub's redirect location contains the actual ayni-v tag, not a bare v tag.
wget_bin="$tmp_dir/wget-bin"
make_path "$wget_bin"
cat > "$wget_bin/wget" <<'EOF'
#!/bin/sh
echo '  Location: https://github.com/gdurandvadas/ayni/releases/tag/ayni-v1.2.3' >&2
exit 8
EOF
chmod +x "$wget_bin/wget"
(
  PATH="$wget_bin"
  export PATH
  . "$library"
  test "$(resolve_latest_version)" = "ayni-v1.2.3"
)

# A release must never be installed without a working SHA-256 implementation.
checksum_bin="$tmp_dir/checksum-bin"
make_path "$checksum_bin"
cat > "$checksum_bin/awk" <<'EOF'
#!/bin/sh
printf '%s\n' deadbeef
EOF
chmod +x "$checksum_bin/awk"
if (
  PATH="$checksum_bin"
  export PATH
  . "$library"
  checksum_verify archive SHA256SUMS archive
) >"$tmp_dir/checksum.out" 2>&1; then
  echo "checksum verification unexpectedly succeeded without SHA-256 support" >&2
  exit 1
fi
grep -Fq 'sha256sum or shasum is required' "$tmp_dir/checksum.out"

# Only the known packaged root and files may be extracted.
layout_bin="$tmp_dir/layout-bin"
make_path "$layout_bin"
cat > "$layout_bin/tar" <<'EOF'
#!/bin/sh
printf '%s\n' 'ayni-ayni-v1.2.3-x86_64-unknown-linux-gnu/'
printf '%s\n' 'ayni-ayni-v1.2.3-x86_64-unknown-linux-gnu/ayni'
printf '%s\n' 'ayni-ayni-v1.2.3-x86_64-unknown-linux-gnu/LICENSE'
printf '%s\n' 'ayni-ayni-v1.2.3-x86_64-unknown-linux-gnu/NOTICE'
EOF
chmod +x "$layout_bin/tar"
(
  PATH="$layout_bin"
  export PATH
  . "$library"
  validate_archive_layout archive ayni-ayni-v1.2.3-x86_64-unknown-linux-gnu
)
cat > "$layout_bin/tar" <<'EOF'
#!/bin/sh
printf '%s\n' '../escape'
EOF
chmod +x "$layout_bin/tar"
if (
  PATH="$layout_bin"
  export PATH
  . "$library"
  validate_archive_layout archive ayni-ayni-v1.2.3-x86_64-unknown-linux-gnu
) >"$tmp_dir/layout.out" 2>&1; then
  echo "unsafe archive layout unexpectedly succeeded" >&2
  exit 1
fi
grep -Fq 'unexpected path' "$tmp_dir/layout.out"

echo 'install.sh tests passed'
