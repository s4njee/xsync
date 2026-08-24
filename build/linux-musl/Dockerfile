FROM alpine:3.20

ARG RUST_VERSION=1.88.0

RUN apk add --no-cache build-base cmake curl git linux-headers openssl-dev \
    pkgconf zstd-dev

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain ${RUST_VERSION} --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /xsync
