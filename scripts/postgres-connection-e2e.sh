#!/usr/bin/env bash
# 无头端到端验证：Podman 起 PostgreSQL，跑生产视图里的连接串导入与保存连接用例。
#
# 用法：scripts/postgres-connection-e2e.sh
# 可选环境变量：
#   FLUXDB_E2E_PG_PORT（宿主侧监听端口，默认 55432）
#   FLUXDB_E2E_PG_IMAGE（默认 docker.io/library/postgres:16）
# 只清理本次创建的容器，不触碰其它实例。

set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

host_port="${FLUXDB_E2E_PG_PORT:-55432}"
image="${FLUXDB_E2E_PG_IMAGE:-docker.io/library/postgres:16}"
container="fluxdb-pg-e2e-$$-$(date +%s)"
user="fluxdb"
password="fluxdb-e2e-pg-pass"
database="fluxdb_e2e"

cleanup() {
  podman rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT

podman run -d --name "$container" \
  -e POSTGRES_USER="$user" \
  -e POSTGRES_PASSWORD="$password" \
  -e POSTGRES_DB="$database" \
  -p "127.0.0.1:${host_port}:5432" \
  "$image" >/dev/null

for _ in $(seq 1 60); do
  if podman exec "$container" pg_isready -U "$user" -d "$database" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
podman exec "$container" pg_isready -U "$user" -d "$database" >/dev/null

FLUXDB_PG_E2E="127.0.0.1:${host_port}:${user}:${password}:${database}" \
  FLUXDB_PROFILE=dev \
  cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop -- postgres_
