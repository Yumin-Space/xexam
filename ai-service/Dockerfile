# Xexam AI service (ai-service): multi-stage build like backend/Dockerfile; the runtime image contains only one statically
# linked executable (musl), non-root. Build context: the repository root (docker build -f ai-service/Dockerfile .).
#
# The build stage runs on the build machine's own architecture (--platform=$BUILDPLATFORM) and cross-compiles with
# cargo-zigbuild (zig as C compiler and linker), so an arm64 image is built at native speed instead of under QEMU. The
# linker wrapper and the build script of the backend are reused read-only (see backend/zigcc-aarch64.sh and
# backend/build-release.sh). TLS uses rustls with the ring provider and the Mozilla roots compiled in, so the runtime
# image needs no CA bundle and the build needs no cmake / nasm / go.

ARG RUST_VERSION=1.99.1
ARG ZIG_VERSION=0.14.1
ARG ZIGBUILD_VERSION=0.20.1

FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-slim-bookworm AS build
ARG ZIG_VERSION
ARG ZIGBUILD_VERSION
ARG TARGETARCH
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 python3-pip ca-certificates \
 && pip3 install --no-cache-dir --break-system-packages "ziglang==${ZIG_VERSION}" "cargo-zigbuild==${ZIGBUILD_VERSION}" \
 && rm -rf /var/lib/apt/lists/*
# Docker architecture name → Rust target triple
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl  > /rust-target ;; \
      arm64) echo aarch64-unknown-linux-musl > /rust-target ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && rustup target add "$(cat /rust-target)"
# aarch64: plain cargo build with the linker wrapper (it strips an argument zig does not know); x86_64: cargo zigbuild
COPY backend/zigcc-aarch64.sh /usr/local/bin/zigcc-aarch64
RUN chmod +x /usr/local/bin/zigcc-aarch64
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=/usr/local/bin/zigcc-aarch64
# The bundled SQLite of rusqlite and ring need a C compiler; the cc crate finds the same zig cc wrapper through these
ENV CC_aarch64_unknown_linux_musl=/usr/local/bin/zigcc-aarch64 AR_aarch64_unknown_linux_musl=ar
COPY backend/build-release.sh /usr/local/bin/build-release
RUN chmod +x /usr/local/bin/build-release
WORKDIR /src
COPY ai-service/Cargo.toml ai-service/Cargo.lock ai-service/rust-toolchain.toml ./
# Compile only the dependencies first, so a source change does not recompile them
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs && build-release 2>/dev/null; rm -rf src
COPY ai-service/src ./src
RUN touch src/main.rs src/lib.rs && build-release && cp "target/$(cat /rust-target)/release/xexam-ai-service" /xexam-ai-service

FROM alpine:3.21
# Time zones: the per-day usage statistics use the local date. tzdata lets TZ name any zone; /etc/localtime defaults
# to UTC and docker-compose.yml mounts the host's /etc/localtime over it, like for the backend.
RUN apk add --no-cache tzdata \
 && cp /usr/share/zoneinfo/UTC /etc/localtime \
 && addgroup -g 10001 app && adduser -D -u 10001 -G app app \
 && mkdir /data /link && chown app:app /data /link && chmod 700 /data
COPY --from=build /xexam-ai-service /usr/local/bin/xexam-ai-service
COPY VERSION /VERSION
USER app
ENV XEXAM_AI_DATA=/data XEXAM_AI_HOST=0.0.0.0 XEXAM_AI_PORT=8766 XEXAM_AI_BACKEND_URL=http://backend:8765 \
    XEXAM_LINK_DIR=/link XEXAM_VERSION_FILE=/VERSION
VOLUME ["/data"]
EXPOSE 8766
# The service's own healthcheck subcommand: GET /healthz on 127.0.0.1, independent of the backend and the provider
HEALTHCHECK --interval=30s --timeout=8s --start-period=15s --retries=3 \
  CMD ["/usr/local/bin/xexam-ai-service", "healthcheck"]
STOPSIGNAL SIGTERM
CMD ["/usr/local/bin/xexam-ai-service", "serve"]
