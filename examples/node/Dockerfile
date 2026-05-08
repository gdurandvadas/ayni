FROM node:22-bookworm

RUN corepack enable

ENV HOME=/tmp/ayni-home
ENV GOPATH=/tmp/ayni-go
ENV UV_TOOL_DIR=/tmp/ayni-uv-tools
ENV UV_TOOL_BIN_DIR=/tmp/ayni-bin
ENV PATH="/tmp/ayni-go/bin:/tmp/ayni-bin:${PATH}"

WORKDIR /repo
