#!/usr/bin/env bash
# 一键运行跨 SQL 引擎 conformance。
#
# 用法：scripts/sql-conformance.sh [sqlite|postgres|mysql|mariadb|tidb|mssql|dm|kingbase|oracle|all]
#
# 可选环境变量：
#   FLUXDB_CONFORMANCE_SSH=user@host   在远程主机启动数据库，经 SSH 隧道连接
#   FLUXDB_CONFORMANCE_RUNTIME=podman  指定容器运行时；默认自动检测 podman/docker
#   FLUXDB_CONFORMANCE_PG_PORT=55432
#   FLUXDB_CONFORMANCE_MYSQL_PORT=53306
#   FLUXDB_CONFORMANCE_MARIADB_PORT=53307
#   FLUXDB_CONFORMANCE_MSSQL_PORT=51433
#   FLUXDB_CONFORMANCE_DM_PORT=55236
#   FLUXDB_CONFORMANCE_ORACLE_PORT=51521
#   FLUXDB_CONFORMANCE_KB_PORT=54321
#   FLUXDB_CONFORMANCE_TIDB_PORT=4000
#   FLUXDB_CONFORMANCE_PG_IMAGE=docker.io/library/postgres:16-alpine
#   FLUXDB_CONFORMANCE_MYSQL_IMAGE=docker.io/library/mysql:8.0
#   FLUXDB_CONFORMANCE_MARIADB_IMAGE=docker.io/library/mariadb:11.4
#   FLUXDB_CONFORMANCE_MSSQL_IMAGE=mcr.microsoft.com/mssql/server:2022-latest
#   FLUXDB_CONFORMANCE_DM_IMAGE=docker.io/xuxuclassmate/dameng:latest
#   FLUXDB_CONFORMANCE_ORACLE_IMAGE=container-registry.oracle.com/database/free:23.6.0.0-lite
#   FLUXDB_CONFORMANCE_KB_IMAGE=   Kingbase 镜像；未设置时 all 跳过 Kingbase
#   FLUXDB_CONFORMANCE_TIDB_IMAGE=docker.io/pingcap/tidb:v8.5.0
#
# 只创建和清理本次使用的容器；不触碰已有实例。

set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

what="${1:-all}"
ssh_target="${FLUXDB_CONFORMANCE_SSH:-}"
runtime="${FLUXDB_CONFORMANCE_RUNTIME:-}"
profile="${FLUXDB_PROFILE:-dev}"
suffix="$$-$(date +%s)"
container_prefix="fluxdb-sql-conformance-${suffix}"
pg_port="${FLUXDB_CONFORMANCE_PG_PORT:-55432}"
mysql_port="${FLUXDB_CONFORMANCE_MYSQL_PORT:-53306}"
mariadb_port="${FLUXDB_CONFORMANCE_MARIADB_PORT:-53307}"
mssql_port="${FLUXDB_CONFORMANCE_MSSQL_PORT:-51433}"
dm_port="${FLUXDB_CONFORMANCE_DM_PORT:-55236}"
oracle_port="${FLUXDB_CONFORMANCE_ORACLE_PORT:-51521}"
kb_port="${FLUXDB_CONFORMANCE_KB_PORT:-54321}"
tidb_port="${FLUXDB_CONFORMANCE_TIDB_PORT:-4000}"
pg_image="${FLUXDB_CONFORMANCE_PG_IMAGE:-docker.io/library/postgres:16-alpine}"
mysql_image="${FLUXDB_CONFORMANCE_MYSQL_IMAGE:-docker.io/library/mysql:8.0}"
mariadb_image="${FLUXDB_CONFORMANCE_MARIADB_IMAGE:-docker.io/library/mariadb:11.4}"
mssql_image="${FLUXDB_CONFORMANCE_MSSQL_IMAGE:-mcr.microsoft.com/mssql/server:2022-latest}"
dm_image="${FLUXDB_CONFORMANCE_DM_IMAGE:-docker.io/xuxuclassmate/dameng:latest}"
oracle_image="${FLUXDB_CONFORMANCE_ORACLE_IMAGE:-container-registry.oracle.com/database/free:23.6.0.0-lite}"
kb_image="${FLUXDB_CONFORMANCE_KB_IMAGE:-}"
tidb_image="${FLUXDB_CONFORMANCE_TIDB_IMAGE:-docker.io/pingcap/tidb:v8.5.0}"
pg_password="fluxdb-conformance-pg"
mysql_password="fluxdb-conformance-mysql"
mariadb_password="fluxdb-conformance-mariadb"
mssql_password="FluxdbConformance!Passw0rd"
dm_password="SYSDBA001"
oracle_password="FluxdbConformance1"
kb_password="kingbase"
tunnel_pid=""
containers=()

fail() {
  echo "错误：$*" >&2
  exit 1
}

host_sh() {
  local command="$1"
  if [[ -n "$ssh_target" ]]; then
    ssh -o BatchMode=yes "$ssh_target" "$command"
  else
    bash -c "$command"
  fi
}

detect_runtime() {
  if [[ -n "$runtime" ]]; then
    return
  fi
  if [[ -n "$ssh_target" ]]; then
    runtime="$(ssh -o BatchMode=yes "$ssh_target" 'command -v podman || command -v docker' 2>/dev/null || true)"
  else
    runtime="$(command -v podman || command -v docker || true)"
  fi
  [[ -n "$runtime" ]] || fail "未找到 podman 或 docker"
}

cleanup() {
  if [[ -n "$tunnel_pid" ]]; then
    kill "$tunnel_pid" >/dev/null 2>&1 || true
    wait "$tunnel_pid" 2>/dev/null || true
  fi
  local container
  for container in "${containers[@]:-}"; do
    host_sh "$runtime rm -f '$container' >/dev/null 2>&1 || true"
  done
}
trap cleanup EXIT INT TERM

wait_for() {
  local label="$1"
  local command="$2"
  local attempt
  for attempt in $(seq 1 120); do
    if host_sh "$command" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  fail "等待 $label 超时"
}

start_tunnel() {
  local port="$1"
  [[ -n "$ssh_target" ]] || return 0
  ssh -o BatchMode=yes -o ExitOnForwardFailure=yes \
    -N -L "127.0.0.1:${port}:127.0.0.1:${port}" "$ssh_target" &
  tunnel_pid=$!
  local attempt
  for attempt in $(seq 1 50); do
    if python3 -c "import socket; socket.create_connection(('127.0.0.1', $port), 0.5).close()" 2>/dev/null; then
      return 0
    fi
    sleep 0.2
  done
  fail "SSH 隧道 127.0.0.1:${port} 未就绪"
}

stop_tunnel() {
  if [[ -n "$tunnel_pid" ]]; then
    kill "$tunnel_pid" >/dev/null 2>&1 || true
    wait "$tunnel_pid" 2>/dev/null || true
    tunnel_pid=""
  fi
}

run_sqlite() {
  echo "== SQLite =="
  FLUXDB_PROFILE="$profile" \
    cargo test --locked -p fluxdb-connectors --test sql_conformance \
      sqlite_sql_conformance -- --nocapture
}

run_engine_test() {
  local test_name="$1"
  shift
  env FLUXDB_PROFILE="$profile" "$@" \
    cargo test --locked -p fluxdb-connectors --test sql_conformance \
      "$test_name" -- --ignored --nocapture --test-threads=1
}

run_postgres() {
  local container="${container_prefix}-pg"
  echo "== PostgreSQL =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -e POSTGRES_PASSWORD='$pg_password' \
    -p 127.0.0.1:${pg_port}:5432 '$pg_image' >/dev/null"
  wait_for "PostgreSQL" "$runtime exec '$container' pg_isready -U postgres"
  start_tunnel "$pg_port"
  run_engine_test "adapters::postgres_sql_conformance" \
    FLUXDB_PG_SMOKE="127.0.0.1:${pg_port}:postgres:${pg_password}"
  stop_tunnel
}

run_mysql() {
  local container="${container_prefix}-mysql"
  echo "== MySQL =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -e MYSQL_ROOT_PASSWORD='$mysql_password' \
    -e MYSQL_ROOT_HOST=% \
    -p 127.0.0.1:${mysql_port}:3306 '$mysql_image' >/dev/null"
  wait_for "MySQL" "$runtime exec '$container' mysqladmin ping -h 127.0.0.1 -uroot -p'$mysql_password' --silent"
  start_tunnel "$mysql_port"
  run_engine_test "adapters::mysql_sql_conformance" \
    FLUXDB_MYSQL_TARGET="127.0.0.1:${mysql_port}:root:${mysql_password}"
  stop_tunnel
}

run_mariadb() {
  local container="${container_prefix}-mariadb"
  echo "== MariaDB =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -e MARIADB_ROOT_PASSWORD='$mariadb_password' \
    -e MARIADB_ROOT_HOST=% \
    -p 127.0.0.1:${mariadb_port}:3306 '$mariadb_image' >/dev/null"
  wait_for "MariaDB" "$runtime exec '$container' mariadb-admin ping -h 127.0.0.1 -uroot -p'$mariadb_password' --silent"
  start_tunnel "$mariadb_port"
  run_engine_test "adapters::mariadb_sql_conformance" \
    FLUXDB_MARIADB_SMOKE="127.0.0.1:${mariadb_port}:root:${mariadb_password}"
  stop_tunnel
}

run_mssql() {
  local container="${container_prefix}-mssql"
  echo "== SQL Server =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -e ACCEPT_EULA=Y \
    -e MSSQL_SA_PASSWORD='$mssql_password' \
    -e MSSQL_PID=Developer \
    -p 127.0.0.1:${mssql_port}:1433 '$mssql_image' >/dev/null"
  wait_for "SQL Server" "$runtime exec '$container' \
    /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P '$mssql_password' -Q 'SELECT 1'"
  start_tunnel "$mssql_port"
  run_engine_test "adapters::sqlserver_sql_conformance" \
    FLUXDB_MSSQL_SMOKE="127.0.0.1:${mssql_port}:sa:${mssql_password}:master"
  stop_tunnel
}

run_dm() {
  local container="${container_prefix}-dm"
  echo "== DM8 =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -e SYSDBA_PWD='$dm_password' \
    -p 127.0.0.1:${dm_port}:5236 '$dm_image' >/dev/null"
  wait_for "DM8" "$runtime exec '$container' bash -c 'echo > /dev/tcp/127.0.0.1/5236'"
  start_tunnel "$dm_port"
  run_engine_test "adapters::dameng_sql_conformance" \
    FLUXDB_DM_SMOKE="127.0.0.1:${dm_port}:SYSDBA:${dm_password}"
  stop_tunnel
}

run_oracle() {
  local container="${container_prefix}-oracle"
  echo "== Oracle =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -e ORACLE_PASSWORD='$oracle_password' \
    -e APP_USER=fluxdb \
    -e APP_USER_PASSWORD=fluxdb \
    -p 127.0.0.1:${oracle_port}:1521 '$oracle_image' >/dev/null"
  wait_for "Oracle" "$runtime exec '$container' healthcheck.sh"
  start_tunnel "$oracle_port"
  run_engine_test "adapters::oracle_sql_conformance" \
    FLUXDB_ORACLE_SMOKE="127.0.0.1:${oracle_port}:FREEPDB1:fluxdb:fluxdb"
  stop_tunnel
}

run_kingbase() {
  local container="${container_prefix}-kingbase"
  [[ -n "$kb_image" ]] || fail "需要设置 FLUXDB_CONFORMANCE_KB_IMAGE"
  echo "== Kingbase =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -e DB_USER=system \
    -e DB_PASSWORD='$kb_password' \
    -e DB_MODE=pg \
    -e ENABLE_CI=no \
    -e ENCODING=UTF-8 \
    -p 127.0.0.1:${kb_port}:54321 '$kb_image' >/dev/null"
  wait_for "Kingbase" "$runtime exec '$container' bash -c 'echo > /dev/tcp/127.0.0.1/54321'"
  start_tunnel "$kb_port"
  run_engine_test "adapters::kingbase_sql_conformance" \
    FLUXDB_KB_SMOKE="127.0.0.1:${kb_port}:system:${kb_password}:test"
  stop_tunnel
}

run_tidb() {
  local container="${container_prefix}-tidb"
  echo "== TiDB =="
  containers+=("$container")
  host_sh "$runtime run --rm -d --name '$container' \
    -p 127.0.0.1:${tidb_port}:4000 '$tidb_image' \
    --store=unistore --path=/tmp/tidb >/dev/null"
  wait_for "TiDB" "$runtime exec '$container' bash -c 'echo > /dev/tcp/127.0.0.1/4000'"
  start_tunnel "$tidb_port"
  run_engine_test "adapters::tidb_sql_conformance" \
    FLUXDB_TIDB_TARGET="127.0.0.1:${tidb_port}:root:"
  stop_tunnel
}

case "$what" in
  sqlite)
    run_sqlite
    ;;
  postgres)
    detect_runtime
    run_postgres
    ;;
  mysql)
    detect_runtime
    run_mysql
    ;;
  mariadb)
    detect_runtime
    run_mariadb
    ;;
  tidb)
    detect_runtime
    run_tidb
    ;;
  mssql)
    detect_runtime
    run_mssql
    ;;
  dm)
    detect_runtime
    run_dm
    ;;
  oracle)
    detect_runtime
    run_oracle
    ;;
  kingbase)
    detect_runtime
    run_kingbase
    ;;
  all)
    detect_runtime
    run_sqlite
    run_postgres
    run_mysql
    run_mariadb
    run_tidb
    run_mssql
    run_dm
    run_oracle
    if [[ -n "$kb_image" ]]; then
      run_kingbase
    else
      echo "SKIP Kingbase: FLUXDB_CONFORMANCE_KB_IMAGE 未设置"
    fi
    ;;
  *)
    fail "用法：$0 [sqlite|postgres|mysql|mariadb|tidb|mssql|dm|kingbase|oracle|all]"
    ;;
esac
