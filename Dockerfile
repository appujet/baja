FROM debian:bookworm-slim

LABEL org.opencontainers.image.source="https://github.com/bong-devs/Rustalink"

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && addgroup --system rustalink && adduser --system --ingroup rustalink rustalink

WORKDIR /app

# The binary path is passed from the GitHub Action context
ARG TARGETPLATFORM
COPY bin/${TARGETPLATFORM}/rustalink /app/rustalink
RUN chmod +x /app/rustalink

USER rustalink:rustalink

EXPOSE 2333
ENV RUST_LOG=info

ENTRYPOINT ["/app/rustalink"]

