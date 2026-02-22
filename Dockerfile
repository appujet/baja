# syntax=docker/dockerfile:1.4
FROM --platform=$BUILDPLATFORM lukemathwalker/cargo-chef:latest-rust-1.85-alpine AS chef
WORKDIR /app

RUN apk add --no-cache musl-dev gcc make cmake g++ pkgconfig perl opus-dev tzdata
RUN addgroup -S rustalink && adduser -S rustalink -G rustalink

FROM chef AS planner

COPY . .

RUN cargo chef prepare --recipe-json recipe.json

FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-json recipe.json

COPY . .

ENV RUSTFLAGS="-C link-arg=-lm" CFLAGS="-fno-stack-protector" CXXFLAGS="-fno-stack-protector" CMAKE_POLICY_VERSION_MINIMUM="3.5"
RUN cargo build --release --locked

FROM scratch

LABEL org.opencontainers.image.source="https://github.com/bong-devs/Rustalink"

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /usr/share/zoneinfo /usr/share/zoneinfo

WORKDIR /app
COPY --from=builder /app/target/release/rustalink /app/rustalink
USER rustalink:rustalink

EXPOSE 2333
ENV RUST_LOG=info

ENTRYPOINT ["/app/rustalink"]
