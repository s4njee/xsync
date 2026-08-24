FROM rockylinux:8

# Rocky Linux 8 supplies glibc 2.28, the Linux Tier 1 ABI floor.
ARG RUST_VERSION=1.88.0
ARG TARGETARCH

RUN dnf install -y curl gcc gcc-c++ make perl pkg-config cmake git \
    openssl-devel zlib-devel && dnf clean all

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
    sh -s -- -y --default-toolchain ${RUST_VERSION} --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /xsync
