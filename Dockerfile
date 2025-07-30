FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

# Install system dependencies
RUN apt-get update && apt-get -y upgrade && apt-get install -y \
    libclang-dev \
    pkg-config \
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

# Add the SSH private key to the container
RUN mkdir -p /root/.ssh && chmod 700 /root/.ssh \
    && ssh-keyscan github.com >> /root/.ssh/known_hosts \
    && git config --global url."git@github.com:".insteadOf "https://github.com/"

# Builds dependencies
RUN --mount=type=ssh CARGO_NET_GIT_FETCH_WITH_CLI=true cargo chef cook --profile release --recipe-path recipe.json

# Install SP1
RUN --mount=type=ssh git clone git@github.com:succinctlabs/sp1-wip.git
RUN --mount=type=ssh cd sp1-wip \
    && git checkout 1ac3cb1cf5edcdb5245ff62be05bfb0f022387b3 \
    && cargo run -p sp1-cli -- prove install-toolchain --token ghp_XXX \
    && cd crates/cli \
    && CARGO_NET_GIT_FETCH_WITH_CLI=true cargo install --force --locked --path .

###############################################################################
#                                                                             #
#                             Continuous Builder                              #
#                                                                             #
###############################################################################
FROM builder as continuous-builder

# Build continuous application
COPY . .
RUN --mount=type=ssh CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build --profile release --locked --bin continuous

# ARG is not resolved in COPY so we have to hack around it by copying the
# binary to a temporary location
RUN cp /app/target/release/continuous /app/continuous


###############################################################################
#                                                                             #
#                             Ethproofs Builder                               #
#                                                                             #
###############################################################################
FROM builder as eth-proofs-builder

# Build eth-proofs application
COPY . .
RUN --mount=type=ssh CARGO_NET_GIT_FETCH_WITH_CLI=true cargo build --profile release --locked --bin eth-proofs

# ARG is not resolved in COPY so we have to hack around it by copying the
# binary to a temporary location
RUN cp /app/target/release/eth-proofs /app/eth-proofs

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

# Define a TMPDIR on the same "filesystem" as /root for rustup
ENV RUSTUP_TMPDIR=/root/rustup_tmp
# For general temp operations, also good to ensure it's not on a different device
ENV TMPDIR=${RUSTUP_TMPDIR}

RUN mkdir -p /root/rustup_tmp

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/root/.cargo/bin:$PATH

# Add the SSH private key to the container
RUN mkdir -p /root/.ssh && chmod 700 /root/.ssh \
    && ssh-keyscan github.com >> /root/.ssh/known_hosts \
    && git config --global url."git@github.com:".insteadOf "https://github.com/"

# Install SP1
RUN --mount=type=ssh git clone git@github.com:succinctlabs/sp1-wip.git
RUN --mount=type=ssh cd sp1-wip \
    && git checkout 1ac3cb1cf5edcdb5245ff62be05bfb0f022387b3 \
    && cargo run -p sp1-cli -- prove install-toolchain --token ghp_XXX \
    && cd crates/cli \
    && CARGO_NET_GIT_FETCH_WITH_CLI=true cargo install --force --locked --path .

###############################################################################
#                                                                             #
#                            Continuous Runtime                               #
#                                                                             #
###############################################################################
FROM runtime as rsp-continuous

COPY --from=continuous-builder /app/continuous /usr/local/bin

ENTRYPOINT ["/usr/local/bin/continuous"]

###############################################################################
#                                                                             #
#                            Ethproofs Runtime                                #
#                                                                             #
###############################################################################
FROM runtime as rsp-eth-proofs

COPY --from=eth-proofs-builder /app/eth-proofs /usr/local/bin

ENTRYPOINT ["/usr/local/bin/eth-proofs"]