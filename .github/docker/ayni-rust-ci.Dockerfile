ARG RUST_VERSION
FROM rust:${RUST_VERSION}-bookworm

ARG RUST_VERSION
ARG CARGO_LLVM_COV_VERSION=0.8.5
ARG RUST_CODE_ANALYSIS_CLI_VERSION

RUN rustup component add llvm-tools-preview \
    && cargo install cargo-llvm-cov --version "${CARGO_LLVM_COV_VERSION}" --locked \
    && if [ -n "${RUST_CODE_ANALYSIS_CLI_VERSION}" ]; then cargo install rust-code-analysis-cli --version "${RUST_CODE_ANALYSIS_CLI_VERSION}" --locked; else cargo install rust-code-analysis-cli --locked; fi
