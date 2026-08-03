FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

# Install system dependencies for native build scripts in the dependency tree (pr.yml's
# runners preinstall or install these, so only the Docker build needs them spelled out):
# protobuf-compiler for sp1-prover-types (prost codegen for the network prover's gRPC protos),
# cmake for aws-lc-sys (rustls' crypto provider), m4 for gmp-mpfr-sys (bigint-rug GMP build).
RUN apt-get update && apt-get -y upgrade && apt-get install -y \
    cmake \
    libclang-dev \
    m4 \
    pkg-config \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Builds a cargo-chef plan
FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

###############################################################################
#                                                                             #
#                                Base Builder                                 #
#                                                                             #
###############################################################################
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json

# Install dependencies
RUN apt-get update && apt-get -y upgrade && apt-get install -y jq

# Install Rust Stable
RUN rustup toolchain install stable

# Builds dependencies
RUN cargo chef cook --profile release --recipe-path recipe.json

# Install SP1. Keep this version in sync with the sp1up version in .github/workflows/pr.yml:
# the toolchain compiles the guest ELF, and a skewed ELF means a skewed verification key
# relative to the one registered with ethproofs.
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up -v v6.3.1 && \
    ~/.sp1/bin/cargo-prove prove --version

###############################################################################
#                                                                             #
#                             Continuous Builder                              #
#                                                                             #
###############################################################################
FROM builder AS continuous-builder

# Build continuous application
COPY . .
RUN cargo build --profile release --locked --bin continuous

# ARG is not resolved in COPY so we have to hack around it by copying the
# binary to a temporary location
RUN cp /app/target/release/continuous /app/continuous


###############################################################################
#                                                                             #
#                             Ethproofs Builder                               #
#                                                                             #
###############################################################################
FROM builder AS ethproofs-builder

# Build ethproofs application. The state-fetch backend is a runtime choice (--state-backend);
# the binary defaults to the single-call `debug_executionWitness` path.
#
# `--features arena` selects the arena MPT backend for host witness emission AND (via
# build.rs forwarding CARGO_FEATURE_ARENA) for the guest ELF baked into this binary. This
# changes the guest's verification key: the VK uploaded to the ethproofs cluster must come
# from an arena build of the same SP1 toolchain version.
COPY . .
RUN cargo build --profile release --locked -p ethproofs --bin ethproofs --features arena

# ARG is not resolved in COPY so we have to hack around it by copying the
# binary to a temporary location
RUN cp /app/target/release/ethproofs /app/ethproofs

###############################################################################
#                                                                             #
#                               Base Runtime                                  #
#                                                                             #
###############################################################################
FROM ubuntu AS runtime
WORKDIR /app

# Install Rust and required dependencies
RUN apt-get update && apt-get install -y \
    curl \
    jq \
    clang \
    build-essential \
    git \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/root/.cargo/bin:$PATH

# Install SP1 (kept in sync with the builder stage above)
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up -v v6.3.1 && \
    ~/.sp1/bin/cargo-prove prove --version

###############################################################################
#                                                                             #
#                            Continuous Runtime                               #
#                                                                             #
###############################################################################
FROM runtime AS rsp-continuous

COPY --from=continuous-builder /app/continuous /usr/local/bin

ENTRYPOINT ["/usr/local/bin/continuous"]

###############################################################################
#                                                                             #
#                            Ethproofs Runtime                                #
#                                                                             #
###############################################################################
FROM runtime AS rsp-ethproofs

COPY --from=ethproofs-builder /app/ethproofs /usr/local/bin

ENTRYPOINT ["/usr/local/bin/ethproofs"]
