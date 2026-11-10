# Rust 后端：多阶段构建，运行镜像里只有一个静态链接的可执行文件（musl），非 root。
#
# 构建阶段固定在「构建机自己的架构」上运行（--platform=$BUILDPLATFORM），用 cargo-zigbuild（zig 当 C 编译器和链接器）
# 交叉编译到目标架构。这样给 arm64 构建时，Rust 编译器跑的是原生 x86 速度，而不是 QEMU 模拟（模拟下要慢一个数量级）。
# 只有最后的运行阶段是目标架构（alpine，里面只有 tzdata 和一个已经编好的文件），QEMU 对它几乎没有负担。
#
# 在 arm64 机器上直接构建 arm64、在 x86 机器上构建 amd64 时，目标和构建机相同，同样走这条路，不受影响。

ARG RUST_VERSION=1.99.1
ARG ZIG_VERSION=0.14.1
ARG ZIGBUILD_VERSION=0.20.1

FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-slim-bookworm AS build
ARG ZIG_VERSION
ARG ZIGBUILD_VERSION
ARG TARGETARCH
# zig 通过 pip 的 ziglang 包安装（带完整的 zig 工具链）；cargo-zigbuild 同样用 pip 的预编译包，免去再编译一遍它自己
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 python3-pip ca-certificates \
 && pip3 install --no-cache-dir --break-system-packages "ziglang==${ZIG_VERSION}" "cargo-zigbuild==${ZIGBUILD_VERSION}" \
 && rm -rf /var/lib/apt/lists/*
# Docker 的架构名 → Rust 的目标三元组
RUN case "$TARGETARCH" in \
      amd64) echo x86_64-unknown-linux-musl  > /rust-target ;; \
      arm64) echo aarch64-unknown-linux-musl > /rust-target ;; \
      *) echo "不支持的架构：$TARGETARCH" >&2; exit 1 ;; \
    esac \
 && rustup target add "$(cat /rust-target)"
# aarch64：rustc 默认给链接器加 -Wl,--fix-cortex-a53-843419，zig 的链接器不认识，链接会失败（详见 backend/zigcc-aarch64.sh）。
# 所以 aarch64 用普通 `cargo build` + 我们自己的链接器包装脚本（去掉这个参数后交给 zig cc）；x86_64 仍用 `cargo zigbuild`。
COPY backend/zigcc-aarch64.sh /usr/local/bin/zigcc-aarch64
RUN chmod +x /usr/local/bin/zigcc-aarch64
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=/usr/local/bin/zigcc-aarch64
# rusqlite 的 bundled SQLite 要用 C 编译器；cc crate 按这个环境变量找，指向同一个 zig cc 包装
ENV CC_aarch64_unknown_linux_musl=/usr/local/bin/zigcc-aarch64 AR_aarch64_unknown_linux_musl=ar
# 统一的构建命令：arm64 → cargo build（链接器见上），amd64 → cargo zigbuild（见 backend/build-release.sh）
COPY backend/build-release.sh /usr/local/bin/build-release
RUN chmod +x /usr/local/bin/build-release
WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock backend/rust-toolchain.toml ./
# 先只编译依赖，源码变了不必重新编译它们
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs  && build-release 2>/dev/null; rm -rf src
COPY backend/src ./src
RUN touch src/main.rs src/lib.rs  && build-release  && cp "target/$(cat /rust-target)/release/xexam-backend" /xexam-backend

FROM alpine:3.21
# 时区库：数据库里的时间戳是本地时间（TZ=Asia/Tokyo），没有它会悄悄变成 UTC
RUN apk add --no-cache tzdata \
 && addgroup -g 10001 app && adduser -D -u 10001 -G app app \
 && mkdir /data && chown app:app /data
COPY --from=build /xexam-backend /usr/local/bin/xexam-backend
COPY VERSION /VERSION
USER app
ENV XEXAM_DATA=/data XEXAM_HOST=0.0.0.0 XEXAM_PORT=8765 XEXAM_VERSION_FILE=/VERSION
VOLUME ["/data"]
EXPOSE 8765
# 后端自己的 healthcheck 子命令（用 127.0.0.1 访问自己，不受 Host 白名单影响）
HEALTHCHECK --interval=30s --timeout=8s --start-period=15s --retries=3 \
  CMD ["/usr/local/bin/xexam-backend", "healthcheck"]
STOPSIGNAL SIGTERM
CMD ["/usr/local/bin/xexam-backend", "serve"]
