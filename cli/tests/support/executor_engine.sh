# Shared fake OCI executor protocol; test delegates own preparation and failures.
engine_dir=$(dirname "$0")
state="$engine_dir/executor-image.json"
case "$1" in
  buildx) printf '{"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}\n'; exit 0;;
  pull) for argument in "$@"; do case "$argument" in registry.example/missing*) echo "executor unavailable" >&2; exit 1;; esac; done; exit 0;;
esac
last=''
entrypoint=''
previous=''
for argument in "$@"; do
  [ "$previous" != --entrypoint ] || entrypoint=$argument
  previous=$argument
  last=$argument
done
case "$1:$2:$last" in
  image:inspect:*@sha256:*)
    printf '[{"Id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Os":"linux","Architecture":"%s","Config":{"Labels":{"org.opencontainers.image.revision":"ffffffffffffffffffffffffffffffffffffffff","dev.ayni.executor.lock-schema":"0.7.0","dev.ayni.executor.recipe":"%s","dev.ayni.provisioning.schema":"1","dev.ayni.environment.variant":"debian","dev.ayni.environment.mise-version":"2025.2.4"}}}]\n' "$ARCH" "${AYNI_TEST_EXECUTOR_RECIPE:-1}"
    exit 0;;
esac
if [ "$1" = run ]; then
  case "$entrypoint" in
    sha256sum) echo 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee  /usr/local/bin/ayni'; exit 0;;
    /usr/local/bin/ayni) if [ "$last" = --version ]; then echo "ayni $VERSION"; exit 0; fi;;
  esac
fi
case "$1:$2:$last" in
  image:inspect:ayni-env:lock-*) if [ "$3" != --format ]; then cat "$state"; exit $?; fi;;
esac
if [ "$1" = build ]; then
  previous=''
  for argument in "$@"; do
    case "$previous" in --file) file=$argument;; --tag) tag=$argument;; esac
    previous=$argument
  done
  "$engine_dir/docker-delegate" "$@" || exit $?
  if command -v sha256sum >/dev/null 2>&1; then digest=$(sha256sum "$file"); else digest=$(shasum -a 256 "$file"); fi
  digest=${digest%% *}
  {
    printf '[{"Id":"sha256:%s","RepoTags":["%s"],"Config":{"Labels":{' "$digest" "$tag"
    separator=''
    for label in owner schema lock-fingerprint base-digest ayni-version mise-version platform preparation-digest executor recipe; do
      value=$(sed -n "s/.*dev.ayni.environment.$label=\"\([^\"]*\)\".*/\1/p" "$file")
      printf '%s"dev.ayni.environment.%s":"%s"' "$separator" "$label" "$value"
      separator=','
    done
    printf '}}}]\n'
  } > "$state"
  exit 0
fi
"$engine_dir/docker-delegate" "$@"
result=$?
if [ "$result" = 0 ] && [ "$1" = run ] && [ "${AYNI_TEST_OMIT_ARTIFACT:-0}" != 1 ]; then
  for argument in "$@"; do
    if [ "$argument" = check ]; then
      mkdir -p "$engine_dir/../.ayni/last"
      printf '{"schema_version":"0.4.0","fixture":"engine-protocol"}\n' > "$engine_dir/../.ayni/last/signals.json"
    fi
  done
fi
exit "$result"
