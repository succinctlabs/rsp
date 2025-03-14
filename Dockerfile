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

# Builds dependencies
RUN cargo chef cook --profile release --recipe-path recipe.json

# Install SP1
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up && \
    ~/.sp1/bin/cargo-prove prove --version

###############################################################################
#                                                                             #
#                             Continuous Builder                              #
#                                                                             #
###############################################################################
FROM builder as continuous-builder

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
FROM builder as eth-proofs-builder

# Build eth-proofs application
COPY . .
RUN cargo build --profile release --locked --bin eth-proofs

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

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/root/.cargo/bin:$PATH

# Install SP1
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up && \
    ~/.sp1/bin/cargo-prove prove --version

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