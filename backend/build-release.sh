#!/bin/sh
# 在 backend/Dockerfile 的构建阶段里编译发布版（目标三元组写在 /rust-target 里）。
#   arm64 → 普通 cargo build，链接器由环境变量指向 zigcc-aarch64.sh（去掉 zig 不认识的参数后交给 zig cc）
#   amd64 → cargo zigbuild
set -e
target="$(cat /rust-target)"
if [ "$target" = aarch64-unknown-linux-musl ]; then
  exec cargo build --release --target "$target" "$@"
fi
exec cargo zigbuild --release --target "$target" "$@"
