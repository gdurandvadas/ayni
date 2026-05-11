FROM rust:1-slim-bookworm AS ayni-builder

WORKDIR /repo
COPY . .
RUN cargo build -p ayni-cli --release

FROM gradle:9.5.0-jdk25

ENV HOME=/tmp/ayni-home
ENV GOPATH=/tmp/ayni-go
ENV UV_TOOL_DIR=/tmp/ayni-uv-tools
ENV UV_TOOL_BIN_DIR=/tmp/ayni-bin
ENV PATH="/tmp/ayni-go/bin:/tmp/ayni-bin:${PATH}"

COPY --from=ayni-builder /repo/target/release/ayni /usr/local/bin/ayni

WORKDIR /repo
