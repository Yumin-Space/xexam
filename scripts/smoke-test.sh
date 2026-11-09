#!/usr/bin/env bash
# 冒烟测试：对一个本地构建好的镜像跑一遍。CI 里每种 CPU 架构各跑一次；本机装了 Docker 也能直接运行：
#     bash scripts/smoke-test.sh xexam:test amd64
# 参数：1 = 镜像名:标签   2 = 期望的架构（amd64 或 arm64）
set -euo pipefail

IMAGE="${1:?用法: smoke-test.sh <镜像> <amd64|arm64>}"
ARCH="${2:?用法: smoke-test.sh <镜像> <amd64|arm64>}"
PORT="${SMOKE_PORT:-18765}"
NAME="xexam-test-$ARCH"
VOL="xexam-test-data-$ARCH"
BASE="http://127.0.0.1:$PORT"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; docker volume rm "$VOL" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

fail() { echo "FAIL: $*" >&2; docker logs "$NAME" 2>&1 | tail -30 >&2 || true; exit 1; }

# ---- 镜像本身 ----
got="$(docker image inspect "$IMAGE" --format '{{.Architecture}}')"
echo "镜像架构：$got（期望 $ARCH）"
[ "$got" = "$ARCH" ] || fail "镜像架构不对"

# 和 docker-compose.yml 一样用命名卷（不是宿主机目录）保存数据
docker volume create "$VOL" >/dev/null
run_app() { docker run -d --platform "linux/$ARCH" --name "$NAME" -p "127.0.0.1:$PORT:8765" -v "$VOL:/data" "$@" "$IMAGE" >/dev/null; }
wait_up() {
  for _ in $(seq 1 90); do
    if curl -fsS "$BASE/api/info" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  return 1
}
api() { curl -fsS "$@"; }          # 失败时返回非 0（配合 set -e）

run_app
wait_up || fail "服务没有在 90 秒内启动"

echo "容器里运行的用户：$(docker exec "$NAME" id)"
echo "容器里报告的 CPU：$(docker exec "$NAME" uname -m)"
[ "$(docker exec "$NAME" id -u)" != 0 ] || fail "容器不应该以 root 运行"

# ---- 基本可用 ----
page="$(api "$BASE/")" || fail "取不到首页"
echo "$page" | grep -q "Xexam" || fail "首页内容不对"
info="$(api "$BASE/api/info")"; echo "api/info：$info"
echo "$info" | grep -q '"bankCount": 0' || fail "全新的卷里题数应为 0"
want="$(tr -d '[:space:]' < "$HERE/VERSION")"
echo "$info" | grep -q "\"version\": \"$want\"" || fail "镜像里的版本号不是 $want"
for f in apple-touch-icon.png icon.svg; do api -o /dev/null "$BASE/$f" || fail "取不到 $f"; done

# ---- Host / Origin 检查 ----
code="$(curl -s -o /dev/null -w '%{http_code}' -H 'Host: evil.example.com' "$BASE/api/bank")"
[ "$code" = 403 ] || fail "陌生 Host 应返回 403，实际 $code"
code="$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'Content-Type: application/json' -H 'Origin: https://evil.example.com' -d '{}' "$BASE/api/import/cancel")"
[ "$code" = 403 ] || fail "跨站 Origin 的 POST 应返回 403，实际 $code"
code="$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'Content-Type: application/json' -H "Origin: http://127.0.0.1:$PORT" -d '{}' "$BASE/api/import/cancel")"
[ "$code" = 200 ] || fail "同源的 POST 应返回 200，实际 $code"

# ---- 数据在卷里 ----
docker exec "$NAME" sh -c 'test -f /data/xexam.db' || fail "没有在卷里生成 xexam.db"

# ---- 重启 + 健康检查 ----
docker restart "$NAME" >/dev/null
wait_up || fail "重启后服务没有起来"
s=""
for _ in $(seq 1 60); do
  s="$(docker inspect -f '{{.State.Health.Status}}' "$NAME")"
  [ "$s" = healthy ] && break
  sleep 2
done
echo "健康状态：$s"
[ "$s" = healthy ] || fail "健康检查没有变成 healthy"

# ---- 持久化：往卷里写一个标记，删除容器，用同一个卷重建，标记必须还在 ----
docker exec "$NAME" sh -c 'echo persisted > /data/marker.txt'
docker rm -f "$NAME" >/dev/null
run_app
wait_up || fail "用同一个卷重建后服务没有起来"
[ "$(docker exec "$NAME" sh -c 'cat /data/marker.txt')" = persisted ] || fail "删除容器重建后，卷里的数据丢了"
docker exec "$NAME" sh -c 'test -f /data/xexam.db' || fail "删除容器重建后，数据库丢了"
echo "删除容器后用同一个卷重建，数据仍在"

# ---- docker stop 要在几秒内正常结束（不是等 10 秒被强杀）----
t0=$(date +%s)
docker stop "$NAME" >/dev/null
dt=$(( $(date +%s) - t0 ))
echo "docker stop 用时 ${dt} 秒"
[ "$dt" -lt 9 ] || fail "docker stop 用了 ${dt} 秒，说明程序没有响应 SIGTERM，被强制结束了"

echo "冒烟测试通过（$ARCH）"
