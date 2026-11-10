#!/bin/sh
# aarch64 的链接器包装脚本（只在 backend/Dockerfile 的构建阶段用）。
#
# rustc 给 aarch64 目标的链接命令里默认带 -Wl,--fix-cortex-a53-843419（链接器侧对 Cortex-A53 CPU 勘误 #843419 的规避）。
# zig 的链接器不认识这个参数，链接直接失败：error: unsupported linker arg: --fix-cortex-a53-843419
# （实测 ziglang 0.14.1 / 0.15.2 / 0.16.0 / 0.17.1 都一样；只设 RUSTFLAGS 去关它也没有用）。
# 所以这里把含这个字样的参数去掉，再交给 zig cc。
#
# 取舍：aarch64 的产物里不带这个规避。这是一个针对 Cortex-A53 核心的链接器规避（据我所知，勘误 #843419 只涉及 Cortex-A53；
# 我没能取回 Arm 勘误文档原文来核实这一点）。已知的目标机器 Oracle Cloud Ampere A1（Ampere Altra Q80-30，据 Wikipedia 用 Arm Neoverse N1 核心）
# 不是 Cortex-A53。如果以后要在 Cortex-A53 核心的板子上跑（部分老款开发板），请改用别的交叉编译方案。
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
