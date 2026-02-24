# Builder Stage
FROM rust:1.93-slim-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    cmake \
    git \
    clang \
    libclang-dev \
    build-essential \
    perl \
    && rm -rf /var/lib/apt/lists/*

# Enable static opus
ENV LIBOPUS_STATIC=1 \
    OPUS_STATIC=1 \
    AUDIOPUS_STATIC=1 \
    CMAKE_POLICY_VERSION_MINIMUM=3.5

# Cache dependencies first
COPY Cargo.toml Cargo.lock ./

RUN mkdir src && echo "fn main() {}" > src/main.rs

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked

RUN rm -rf src

# Copy real source
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked

RUN strip target/release/rustalink


# Runtime Stage

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system rustalink \
    && adduser --system --ingroup rustalink rustalink

WORKDIR /app

COPY --from=builder /app/target/release/rustalink /app/rustalink

USER rustalink

EXPOSE 2333
ENV RUST_LOG=info

ENTRYPOINT ["/app/rustalink"]