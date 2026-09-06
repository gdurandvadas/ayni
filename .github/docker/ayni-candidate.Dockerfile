# The candidate carries only the executor; provisioning belongs to the lock.
ARG DEBIAN_IMAGE
FROM ${DEBIAN_IMAGE}
ARG AYNI_VERSION
ARG SOURCE_REVISION
LABEL org.opencontainers.image.source="https://github.com/gdurandvadas/ayni" \
      org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.version="${AYNI_VERSION}" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      dev.ayni.executor.lock-schema="0.7.0" \
      dev.ayni.executor.recipe="1"
COPY --chmod=0755 ayni /usr/local/bin/ayni
COPY LICENSE NOTICE /usr/share/doc/ayni/
ENTRYPOINT ["ayni"]
