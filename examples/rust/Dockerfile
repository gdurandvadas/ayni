FROM rust:1-slim-bookworm AS ayni-builder

WORKDIR /repo
COPY . .
RUN cargo build -p ayni-cli --release

FROM rust:1-slim-bookworm

RUN apt-get update \
    && apt-get install -y --no-install-recommends g++ \
    && rm -rf /var/lib/apt/lists/*

ENV HOME=/tmp/ayni-home
ENV GOPATH=/tmp/ayni-go
ENV UV_TOOL_DIR=/tmp/ayni-uv-tools
ENV UV_TOOL_BIN_DIR=/tmp/ayni-bin
ENV PATH="/tmp/ayni-go/bin:/tmp/ayni-bin:/usr/local/cargo/bin:${PATH}"

COPY --from=ayni-builder /repo/target/release/ayni /usr/local/bin/ayni

WORKDIR /repo
