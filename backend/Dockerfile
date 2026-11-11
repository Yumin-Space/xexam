# Rust backend: multi-stage build; the runtime image contains only one statically linked executable (musl), non-root.
#
# The build stage always runs on the build machine's own architecture (--platform=$BUILDPLATFORM), using cargo-zigbuild (zig as the C compiler and linker)
# to cross-compile to the target architecture. This way, when building for arm64, the Rust compiler runs at native x86 speed instead of under QEMU emulation (an order of magnitude slower).
# Only the final runtime stage uses the target architecture (alpine, containing just tzdata and one already-built binary), which puts almost no load on QEMU.
#
# When building arm64 directly on an arm64 machine, or amd64 on an x86 machine, the target equals the build machine and takes the same path, unaffected.

ARG RUST_VERSION=1.99.1
ARG ZIG_VERSION=0.14.1
ARG ZIGBUILD_VERSION=0.20.1

FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-slim-bookworm AS build
ARG ZIG_VERSION
ARG ZIGBUILD_VERSION
ARG TARGETARCH
# zig is installed via the pip ziglang package (which ships the complete zig toolchain); cargo-zigbuild also uses a prebuilt pip package, which avoids compiling it again
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
# aarch64: rustc by default adds -Wl,--fix-cortex-a53-843419 to the linker, which zig's linker does not recognize, so linking fails (see backend/zigcc-aarch64.sh).
# So aarch64 uses plain `cargo build` + our own linker wrapper script (which strips this argument and then hands over to zig cc); x86_64 still uses `cargo zigbuild`.
COPY backend/zigcc-aarch64.sh /usr/local/bin/zigcc-aarch64
RUN chmod +x /usr/local/bin/zigcc-aarch64
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=/usr/local/bin/zigcc-aarch64
# The bundled SQLite of rusqlite needs a C compiler; the cc crate looks it up via this environment variable, pointing to the same zig cc wrapper
ENV CC_aarch64_unknown_linux_musl=/usr/local/bin/zigcc-aarch64 AR_aarch64_unknown_linux_musl=ar
# Unified build command: arm64 → cargo build (linker as above), amd64 → cargo zigbuild (see backend/build-release.sh)
COPY backend/build-release.sh /usr/local/bin/build-release
RUN chmod +x /usr/local/bin/build-release
WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock backend/rust-toolchain.toml ./
# Compile only the dependencies first, so a source change does not recompile them
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs  && build-release 2>/dev/null; rm -rf src
COPY backend/src ./src
RUN touch src/main.rs src/lib.rs  && build-release  && cp "target/$(cat /rust-target)/release/xexam-backend" /xexam-backend

FROM alpine:3.21
# Time zones: timestamps in the database are local time. tzdata lets TZ name any zone; /etc/localtime defaults to UTC
# and docker-compose.yml mounts the host's /etc/localtime over it, so the container follows the host's time zone.
RUN apk add --no-cache tzdata \
 && cp /usr/share/zoneinfo/UTC /etc/localtime \
 && addgroup -g 10001 app && adduser -D -u 10001 -G app app \
 && mkdir /data /link && chown app:app /data /link
COPY --from=build /xexam-backend /usr/local/bin/xexam-backend
COPY VERSION /VERSION
USER app
# /link (the volume xexam_link in docker-compose.yml): serve creates internal.token there, the secret it shares with the AI
# service (ai-service mounts the same volume read-only). It never holds a database.
ENV XEXAM_DATA=/data XEXAM_HOST=0.0.0.0 XEXAM_PORT=8765 XEXAM_VERSION_FILE=/VERSION XEXAM_LINK_DIR=/link
VOLUME ["/data"]
EXPOSE 8765
# The backend's own healthcheck subcommand (it reaches itself via 127.0.0.1, unaffected by the Host allowlist)
HEALTHCHECK --interval=30s --timeout=8s --start-period=15s --retries=3 \
  CMD ["/usr/local/bin/xexam-backend", "healthcheck"]
STOPSIGNAL SIGTERM
CMD ["/usr/local/bin/xexam-backend", "serve"]
