FROM python:3.12-bookworm

RUN python -m venv /opt/ayni-venv \
 && /opt/ayni-venv/bin/pip install --no-cache-dir --upgrade pip uv \
 && chmod -R a+rwX /opt/ayni-venv

ENV HOME=/tmp/ayni-home
ENV VIRTUAL_ENV=/opt/ayni-venv
ENV GOPATH=/tmp/ayni-go
ENV UV_TOOL_DIR=/tmp/ayni-uv-tools
ENV UV_TOOL_BIN_DIR=/tmp/ayni-bin
ENV PATH="/opt/ayni-venv/bin:/tmp/ayni-go/bin:/tmp/ayni-bin:${PATH}"

WORKDIR /repo
