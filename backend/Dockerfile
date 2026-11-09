# Rust 后端：多阶段构建，运行镜像里只有一个静态链接的可执行文件（musl），非 root。
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev gcc
WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock backend/rust-toolchain.toml ./
# 先只编译依赖，源码变了不必重新编译它们
RUN mkdir src && echo "fn main() {}" > src/main.rs && echo "" > src/lib.rs \
 && cargo build --release 2>/dev/null; rm -rf src
COPY backend/src ./src
RUN touch src/main.rs src/lib.rs && cargo build --release \
 && strip target/release/xexam-backend

FROM alpine:3.21
# 时区库：数据库里的时间戳是本地时间（TZ=Asia/Tokyo），没有它会悄悄变成 UTC
RUN apk add --no-cache tzdata \
 && addgroup -g 10001 app && adduser -D -u 10001 -G app app \
 && mkdir /data && chown app:app /data
COPY --from=build /src/target/release/xexam-backend /usr/local/bin/xexam-backend
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
