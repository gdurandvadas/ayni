# syntax=docker/dockerfile:1.7
ARG DEBIAN_IMAGE
FROM ${DEBIAN_IMAGE}

ARG TARGETARCH
ARG AYNI_VERSION
ARG SOURCE_REVISION
ARG MISE_VERSION
ARG MISE_SHA256_AMD64
ARG MISE_SHA256_ARM64

LABEL org.opencontainers.image.title="Ayni code environment" \
      org.opencontainers.image.description="Universal Debian base for Ayni-managed development and analysis environments" \
      org.opencontainers.image.source="https://github.com/gdurandvadas/ayni" \
      org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.version="${AYNI_VERSION}" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      dev.ayni.environment.variant="debian" \
      dev.ayni.environment.mise-version="${MISE_VERSION}"

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git gzip openssh-client pkg-config tar unzip xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) mise_arch=x64; mise_sha="${MISE_SHA256_AMD64}" ;; \
      arm64) mise_arch=arm64; mise_sha="${MISE_SHA256_ARM64}" ;; \
      *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    curl --fail --location --silent --show-error \
      "https://github.com/jdx/mise/releases/download/v${MISE_VERSION}/mise-v${MISE_VERSION}-linux-${mise_arch}" \
      --output /tmp/mise; \
    printf '%s  %s\n' "${mise_sha}" /tmp/mise | sha256sum --check --strict; \
    install -m 0755 /tmp/mise /usr/local/bin/mise; \
    rm /tmp/mise

COPY ayni /usr/local/bin/ayni
COPY LICENSE NOTICE /usr/share/doc/ayni/

RUN chmod 0755 /usr/local/bin/ayni \
    && groupadd --gid 10001 ayni \
    && useradd --uid 10001 --gid 10001 --create-home --shell /bin/sh ayni \
    && mkdir -p /workspace /opt/ayni/mise /home/ayni/.cache \
    && chown -R 10001:10001 /workspace /opt/ayni /home/ayni

ENV HOME=/home/ayni \
    XDG_CACHE_HOME=/home/ayni/.cache \
    MISE_DATA_DIR=/opt/ayni/mise \
    MISE_CACHE_DIR=/home/ayni/.cache/mise \
    MISE_AUTO_INSTALL=0 \
    RUSTUP_HOME=/home/ayni/.rustup \
    CARGO_HOME=/home/ayni/.cache/cargo \
    npm_config_cache=/home/ayni/.cache/npm \
    PATH=/opt/ayni/mise/shims:/usr/local/bin:/usr/bin:/bin

USER 10001:10001
WORKDIR /workspace
ENTRYPOINT ["ayni"]
