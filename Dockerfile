# syntax=docker/dockerfile:1.4
FROM gcr.io/distroless/static-debian12:latest

ARG TARGETARCH

# We expect the binaries to be placed in the /bin dir relative to Docker context
COPY bin/rustalink-x86_64-unknown-linux-musl /app/rustalink-amd64
COPY bin/rustalink-aarch64-unknown-linux-musl /app/rustalink-arm64

WORKDIR /app

# Switch to root to perform renaming/cleanup
USER root
RUN ["/busybox/sh", "-c", "if [ \"$TARGETARCH\" = \"amd64\" ]; then cp /app/rustalink-amd64 /app/rustalink; elif [ \"$TARGETARCH\" = \"arm64\" ]; then cp /app/rustalink-arm64 /app/rustalink; fi && rm /app/rustalink-amd64 /app/rustalink-arm64"]
RUN ["/busybox/sh", "-c", "chmod +x /app/rustalink"]

EXPOSE 2333
ENV RUST_LOG=info

# Distroless runs as nonroot
USER nonroot:nonroot

ENTRYPOINT ["/app/rustalink"]
