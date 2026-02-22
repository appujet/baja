FROM rust:1.88-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk update && \
    apk add --no-cache musl-dev gcc make cmake g++ pkgconf perl tzdata && \
    addgroup -S rustalink && adduser -S rustalink -G rustalink

# Set build environment variables
# AUDIOPUS_STATIC=1 forces the crate to build its own static libopus
ENV AUDIOPUS_STATIC="1" \
    CFLAGS="-fno-stack-protector" \
    CXXFLAGS="-fno-stack-protector" \
    LDFLAGS="-fno-stack-protector" \
    CMAKE_POLICY_VERSION_MINIMUM="3.5" \
    OPENSSL_STATIC="1"

COPY . .

# Build the application
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
