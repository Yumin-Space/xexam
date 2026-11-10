#!/bin/sh
# aarch64 的链接器包装脚本（只在 backend/Dockerfile 的构建阶段用）。
#
# rustc 给 aarch64 目标的链接命令里默认带 -Wl,--fix-cortex-a53-843419（链接器侧对 Cortex-A53 CPU 勘误 #843419 的规避）。
# zig 的链接器不认识这个参数，链接直接失败：error: unsupported linker arg: --fix-cortex-a53-843419
# （实测 ziglang 0.14.1 / 0.15.2 / 0.16.0 / 0.17.1 都一样；只设 RUSTFLAGS 去关它也没有用）。
# 所以这里把含这个字样的参数去掉，再交给 zig cc。
#
# 取舍：aarch64 的产物里不带这个规避。该勘误只影响某些早期步进的 Cortex-A53 核心。
n=$#
i=0
while [ "$i" -lt "$n" ]; do
  a=$1
  shift
  i=$((i + 1))
  case "$a" in
    *fix-cortex-a53-843419*) ;;
    *) set -- "$@" "$a" ;;
  esac
done
exec cargo-zigbuild zig cc -- -g -fno-sanitize=all -target aarch64-linux-musl "$@"
