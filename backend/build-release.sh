#!/bin/sh
# Compile the release build in the build stage of backend/Dockerfile (the target triple is written to /rust-target).
#   arm64 → plain cargo build, with the linker pointed at zigcc-aarch64.sh by an environment variable (it strips the arguments zig does not recognize, then hands over to zig cc)
#   amd64 → cargo zigbuild
set -e
target="$(cat /rust-target)"
if [ "$target" = aarch64-unknown-linux-musl ]; then
  exec cargo build --release --target "$target" "$@"
fi
exec cargo zigbuild --release --target "$target" "$@"
