#!/bin/sh
# Linker wrapper script for aarch64 (used only in the build stage of backend/Dockerfile).
#
# By default rustc includes -Wl,--fix-cortex-a53-843419 in the link command for aarch64 targets (a linker-side workaround for Cortex-A53 CPU erratum #843419).
# zig's linker does not recognize this flag and linking fails outright: error: unsupported linker arg: --fix-cortex-a53-843419
# (Observed to be the same with ziglang 0.14.1 / 0.15.2 / 0.16.0 / 0.17.1; setting only RUSTFLAGS to turn it off does not help either.)
# So any argument containing this string is stripped here before the rest is passed to zig cc.
#
# Trade-off: the aarch64 output does not carry this workaround. It is a linker workaround for Cortex-A53 cores (as far as I know, erratum #843419 affects only Cortex-A53;
# I could not retrieve the original Arm erratum document to verify this). The known target machine, Oracle Cloud Ampere A1 (Ampere Altra Q80-30, which per Wikipedia uses Arm Neoverse N1 cores),
# is not a Cortex-A53. If you later need to run on a board with Cortex-A53 cores (some older dev boards), use a different cross-compilation approach.
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
