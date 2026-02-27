# Runtime Stage Only — binaries are pre-built by CI
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system rustalink \
    && adduser --system --ingroup rustalink rustalink

WORKDIR /app

# Binary is injected at build time by CI (bin/linux/amd64 or bin/linux/arm64)
ARG TARGETARCH
COPY bin/linux/${TARGETARCH}/rustalink /app/rustalink

USER rustalink

EXPOSE 2333
ENV RUST_LOG=info

ENTRYPOINT ["/app/rustalink"]