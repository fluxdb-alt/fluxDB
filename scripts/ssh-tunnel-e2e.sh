#!/usr/bin/env bash
# 无头端到端验证：Podman 起数据库 + sshd，默认只发布 sshd 端口。
#
# 数据库与 sshd 同 pod（共享网络命名空间），数据库不发布宿主端口，因此宿主机只能
# 经跳板机访问它，用例成功即证明连接真的走了 SSH 转发。MySQL 另发布一个宿主端口，
# 供 TLS / 代理 / 字符集用例直连；代理用例再核对本地代理进程日志确认流量经过代理。
#
# 用法：scripts/ssh-tunnel-e2e.sh [mongo|mssql|oracle|dm|mysql|all]
# 可选环境变量：
#   FLUXDB_E2E_SSH_PORT（默认 2222，宿主侧监听端口）
#   FLUXDB_E2E_MONGO_IMAGE（默认 docker.io/library/mongo:7）
#   FLUXDB_E2E_MSSQL_IMAGE（默认 mcr.microsoft.com/mssql/server:2022-latest）
#   FLUXDB_E2E_ORACLE_IMAGE（默认 container-registry.oracle.com/database/free:latest）
#   FLUXDB_E2E_SSH_IMAGE（默认 docker.io/linuxserver/openssh-server:latest）
#   FLUXDB_E2E_DM_TAR（达梦官方镜像 tar；仅 dm/all 需要）
# 只清理本次创建的 pod，不触碰其它容器。

set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

what="${1:-all}"
host_ssh_port="${FLUXDB_E2E_SSH_PORT:-2222}"
suffix="$$-$(date +%s)"
pod="fluxdb-ssh-e2e-${suffix}"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/fluxdb-ssh-e2e.XXXXXX")"
key_path="$work_dir/id_ed25519"
known_hosts="$work_dir/known_hosts"
ssh_user="fluxdb"
ssh_password="fluxdb-e2e-ssh-pass"
proxy_user="fluxdb"
proxy_password="fluxdb-e2e-proxy-pass"

mongo_image="${FLUXDB_E2E_MONGO_IMAGE:-docker.io/library/mongo:7}"
mongo_user="fluxdb"
mongo_password="fluxdb-e2e-mongo-pass"
mssql_image="${FLUXDB_E2E_MSSQL_IMAGE:-mcr.microsoft.com/mssql/server:2022-latest}"
mssql_password="FluxdbE2e!Passw0rd"
oracle_image="${FLUXDB_E2E_ORACLE_IMAGE:-container-registry.oracle.com/database/free:latest}"
oracle_password="FluxdbE2ePassw0rd"
dm_image="localhost/dm8_single:dm8_20241022_rev244896_x86_rh6_64"
dm_password="SYSDBA001"

proxy_pids=()

cleanup() {
  for pid in "${proxy_pids[@]:-}"; do
    kill "$pid" >/dev/null 2>&1 || true
  done
  podman pod rm -f "$pod" >/dev/null 2>&1 || true
  rm -rf "$work_dir"
}
trap cleanup EXIT

wait_for() {
  local label="$1"
  shift
  for _ in $(seq 1 120); do
    if "$@" >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
  done
  echo "超时：$label" >&2
  return 1
}

wait_tcp() {
  wait_for "$1" podman exec "$pod-ssh" bash -c "echo > /dev/tcp/127.0.0.1/$2"
}

# 取一个空闲的宿主回环端口（代理辅助进程与「不可达代理」用例使用）。
free_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

# 启动本地代理辅助进程；日志用于证明流量确实经过代理。
start_proxy() {
  local kind="$1" port="$2" log="$3"
  python3 scripts/mysql-proxy-e2e.py --kind "$kind" --port "$port" \
    --user "$proxy_user" --password "$proxy_password" --log "$log" &
  proxy_pids+=("$!")
  wait_for "$kind 代理就绪" python3 -c "import socket; socket.create_connection(('127.0.0.1', $port), 2).close()"
}

run_test() {
  local crate="$1"
  local test_target="$2"
  local filter="$3"
  shift 3
  local args=(--ignored --nocapture --test-threads=1)
  if [[ -n "$filter" ]]; then
    args+=(--exact "$filter")
  fi
  env SSH_KNOWN_HOSTS="$known_hosts" "$@" \
    cargo test --locked -p "$crate" --test "$test_target" -- "${args[@]}"
}

mysql_image="${FLUXDB_E2E_MYSQL_IMAGE:-docker.io/library/mysql:8.0}"
mysql_password="fluxdb-e2e-mysql-pass"
# 仅 MySQL 用例需要：发布一个宿主端口给 TLS 用例直连；SSH 用例仍用 pod 内主机名，
# 该主机名宿主机不可解析，因此隧道用例无法退化成直连。
mysql_host_port="${FLUXDB_E2E_MYSQL_PORT:-13306}"
mysql_certs="$work_dir/mysql-certs"

echo "创建 pod $pod"
podman pod create --name "$pod" \
  -p "127.0.0.1:${host_ssh_port}:2222" \
  -p "127.0.0.1:${mysql_host_port}:3306" >/dev/null

echo "生成测试密钥"
ssh-keygen -q -t ed25519 -N "" -f "$key_path"

echo "启动 sshd"
podman run -d --pod "$pod" --name "$pod-ssh" \
  -e PUID=1000 -e PGID=1000 -e TZ=UTC \
  -e USER_NAME="$ssh_user" \
  -e PUBLIC_KEY="$(cat "$key_path.pub")" \
  -e PASSWORD_ACCESS=true \
  -e USER_PASSWORD="$ssh_password" \
  "${FLUXDB_E2E_SSH_IMAGE:-docker.io/linuxserver/openssh-server:latest}" >/dev/null

wait_for "sshd 就绪" ssh -i "$key_path" -p "$host_ssh_port" \
  -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -o ConnectTimeout=5 "$ssh_user@127.0.0.1" true

# 镜像默认关闭 TCP 转发；不同版本配置位置不同（/config/sshd 或 /etc/ssh），都改。
podman exec "$pod-ssh" sh -c 'for f in /config/sshd/sshd_config /etc/ssh/sshd_config; do
  [ -f "$f" ] && sed -i "s/^AllowTcpForwarding no/AllowTcpForwarding yes/" "$f"
done'
podman restart "$pod-ssh" >/dev/null
wait_for "sshd 恢复（已开启 TCP 转发）" ssh -i "$key_path" -p "$host_ssh_port" \
  -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -o ConnectTimeout=5 "$ssh_user@127.0.0.1" true

# libssh2 按「[host]:port」匹配 known_hosts；不限定 -t，避免只钉 ED25519 而误报密钥变更。
ssh-keyscan -p "$host_ssh_port" 127.0.0.1 2>/dev/null >"$known_hosts"
test -s "$known_hosts" || { echo "known_hosts 为空，无法校验收端主机密钥" >&2; exit 1; }

if [[ "$what" == "mongo" || "$what" == "all" ]]; then
  echo "== MongoDB =="
  podman run -d --pod "$pod" --name "$pod-db" \
    -e MONGO_INITDB_ROOT_USERNAME="$mongo_user" \
    -e MONGO_INITDB_ROOT_PASSWORD="$mongo_password" \
    "$mongo_image" --bind_ip_all >/dev/null
  wait_for "MongoDB 就绪" podman exec "$pod-db" mongosh --quiet \
    --username "$mongo_user" --password "$mongo_password" --authenticationDatabase admin \
    --eval 'db.runCommand({ ping: 1 }).ok'
  run_test fluxdb-connectors ssh_tunnel_live "" \
    FLUXDB_SSH_E2E_SSH="127.0.0.1:${host_ssh_port}" \
    FLUXDB_SSH_E2E_SSH_USER="$ssh_user" \
    FLUXDB_SSH_E2E_SSH_PASSWORD="$ssh_password" \
    FLUXDB_SSH_E2E_SSH_KEY="$key_path" \
    FLUXDB_SSH_E2E_MONGO="${pod}-db:27017" \
    FLUXDB_SSH_E2E_MONGO_USER="$mongo_user" \
    FLUXDB_SSH_E2E_MONGO_PASSWORD="$mongo_password"
  echo "运行生产视图无头 E2E（MongoDB SSH 页签）"
  env SSH_KNOWN_HOSTS="$known_hosts" \
    FLUXDB_SSH_E2E_SSH="127.0.0.1:${host_ssh_port}" \
    FLUXDB_SSH_E2E_SSH_USER="$ssh_user" \
    FLUXDB_SSH_E2E_SSH_PASSWORD="$ssh_password" \
    FLUXDB_SSH_E2E_SSH_KEY="$key_path" \
    FLUXDB_SSH_E2E_MONGO="${pod}-db:27017" \
    FLUXDB_SSH_E2E_MONGO_USER="$mongo_user" \
    FLUXDB_SSH_E2E_MONGO_PASSWORD="$mongo_password" \
    cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop -- \
      i18n_e2e_tests::mongo_connection_form_connects_through_ssh_tunnel \
      --nocapture --test-threads=1
  podman rm -f "$pod-db" >/dev/null
fi

if [[ "$what" == "mssql" || "$what" == "all" ]]; then
  echo "== SQL Server =="
  podman run -d --pod "$pod" --name "$pod-mssql" \
    -e ACCEPT_EULA=Y -e MSSQL_SA_PASSWORD="$mssql_password" -e MSSQL_PID=Developer \
    "$mssql_image" >/dev/null
  wait_for "SQL Server 就绪" podman exec "$pod-mssql" \
    /opt/mssql-tools18/bin/sqlcmd -C -S localhost -U sa -P "$mssql_password" -Q "SELECT 1"
  run_test fluxdb-connectors sqlserver_tunnel_live "" \
    FLUXDB_MSSQL_SMOKE="${pod}-mssql:1433:sa:${mssql_password}:master" \
    FLUXDB_MSSQL_SSH="127.0.0.1:${host_ssh_port}:${ssh_user}:${ssh_password}"
  podman rm -f "$pod-mssql" >/dev/null
fi

if [[ "$what" == "oracle" || "$what" == "all" ]]; then
  echo "== Oracle =="
  podman run -d --pod "$pod" --name "$pod-oracle" \
    -e ORACLE_PASSWORD="$oracle_password" \
    -e APP_USER=fluxdb -e APP_USER_PASSWORD=fluxdb \
    "$oracle_image" >/dev/null
  wait_for "Oracle 就绪" podman exec "$pod-oracle" healthcheck.sh
  run_test fluxdb-connectors oracle_live oracle_live_objects_data_and_ddl \
    FLUXDB_ORACLE_SMOKE="${pod}-oracle:1521:FREEPDB1:fluxdb:fluxdb" \
    FLUXDB_ORACLE_SSH="127.0.0.1:${host_ssh_port}:${ssh_user}:${ssh_password}"
  podman rm -f "$pod-oracle" >/dev/null
fi

if [[ "$what" == "dm" || "$what" == "all" ]]; then
  echo "== 达梦 DM8 =="
  dm_tar="${FLUXDB_E2E_DM_TAR:-/tmp/dm8.tar}"
  test -f "$dm_tar" || { echo "缺少达梦镜像 tar：设置 FLUXDB_E2E_DM_TAR" >&2; exit 1; }
  if ! podman image exists "$dm_image"; then
    podman load -i "$dm_tar" >/dev/null
  fi
  podman run -d --pod "$pod" --name "$pod-dm" "$dm_image" >/dev/null
  wait_tcp "达梦端口" 5236
  sleep 15
  run_test fluxdb-connectors dameng_live \
    dameng_live_connection_objects_query_and_transaction \
    FLUXDB_DM_SMOKE="${pod}-dm:5236:SYSDBA:${dm_password}" \
    FLUXDB_DM_SSH="127.0.0.1:${host_ssh_port}:${ssh_user}:${ssh_password}"
fi

if [[ "$what" == "mysql" || "$what" == "all" ]]; then
  echo "== MySQL =="
  mkdir -p "$mysql_certs"
  # 自签 CA 签发独立服务端证书：rustls 同时拒绝 v1 证书和「CA 证书当服务端证书」
  # （CaUsedAsEndEntity），因此不能像以前那样只用一张自签证书。
  openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -keyout "$mysql_certs/ca-key.pem" -out "$mysql_certs/ca-cert.pem" \
    -subj "/CN=fluxdb-e2e-ca" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" >/dev/null 2>&1
  openssl req -newkey rsa:2048 -nodes \
    -keyout "$mysql_certs/server-key.pem" -out "$mysql_certs/server.csr" \
    -subj "/CN=fluxdb-e2e-mysql" >/dev/null 2>&1
  openssl x509 -req -days 1 -in "$mysql_certs/server.csr" \
    -CA "$mysql_certs/ca-cert.pem" -CAkey "$mysql_certs/ca-key.pem" \
    -CAcreateserial -out "$mysql_certs/server-cert.pem" \
    -extfile <(printf '%s\n' \
      "basicConstraints=critical,CA:FALSE" \
      "keyUsage=critical,digitalSignature,keyEncipherment" \
      "extendedKeyUsage=serverAuth" \
      "subjectAltName=DNS:fluxdb-e2e-mysql,DNS:localhost,IP:127.0.0.1") \
    >/dev/null 2>&1
  chmod 644 "$mysql_certs"/*.pem
  podman run -d --pod "$pod" --name "$pod-mysql" \
    -e MYSQL_ROOT_PASSWORD="$mysql_password" -e MYSQL_ROOT_HOST=% \
    -e MYSQL_DATABASE=fluxdb_e2e \
    -v "$mysql_certs:/certs:ro,Z" \
    "$mysql_image" \
    --ssl-ca=/certs/server-cert.pem \
    --ssl-cert=/certs/server-cert.pem \
    --ssl-key=/certs/server-key.pem >/dev/null
  wait_for "MySQL 就绪" podman exec "$pod-mysql" \
    mysqladmin ping -h 127.0.0.1 -uroot -p"$mysql_password" --silent
  run_test fluxdb-connectors mysql_ssh_live "" \
    FLUXDB_MYSQL_SSH="127.0.0.1:${host_ssh_port}:${ssh_user}:${ssh_password}" \
    FLUXDB_MYSQL_TARGET="${pod}-mysql:3306:root:${mysql_password}"
  run_test fluxdb-connectors mysql_tls_live "" \
    FLUXDB_MYSQL_TLS="127.0.0.1:${mysql_host_port}:root:${mysql_password}" \
    FLUXDB_MYSQL_TLS_CA="$mysql_certs/ca-cert.pem"

  echo "== MySQL 代理 / 字符集 / TCP 保活 =="
  proxy_log="$work_dir/mysql-proxy.log"
  : > "$proxy_log"
  proxy_socks_port="$(free_port)"
  proxy_http_port="$(free_port)"
  proxy_dead_port="$(free_port)"
  start_proxy socks5 "$proxy_socks_port" "$proxy_log"
  start_proxy http "$proxy_http_port" "$proxy_log"
  proxy_target="127.0.0.1:${mysql_host_port}:root:${mysql_password}"
  run_test fluxdb-connectors mysql_transport_live "" \
    FLUXDB_MYSQL_TARGET="$proxy_target" \
    FLUXDB_MYSQL_PROXY_SOCKS="127.0.0.1:${proxy_socks_port}:${proxy_user}:${proxy_password}" \
    FLUXDB_MYSQL_PROXY_HTTP="127.0.0.1:${proxy_http_port}:${proxy_user}:${proxy_password}" \
    FLUXDB_MYSQL_PROXY_DEAD="127.0.0.1:${proxy_dead_port}"

  # 日志核对：成功转发的目标必须出现代理记录，否则用例只是直连成功。
  relayed="$(grep -c "relay 127.0.0.1:${mysql_host_port}" "$proxy_log" || true)"
  test "${relayed:-0}" -ge 2 || {
    echo "代理日志未记录转发（$relayed 次），流量未经过代理" >&2
    cat "$proxy_log" >&2
    exit 1
  }
  if grep -q "relay 127.0.0.1:${proxy_dead_port}" "$proxy_log"; then
    echo "不可达代理用例出现了转发记录，检查接口是否绕过代理" >&2
    exit 1
  fi

  echo "运行生产视图无头 E2E（MySQL SSH 页签）"
  env SSH_KNOWN_HOSTS="$known_hosts" \
    FLUXDB_SSH_E2E_SSH="127.0.0.1:${host_ssh_port}" \
    FLUXDB_SSH_E2E_SSH_USER="$ssh_user" \
    FLUXDB_SSH_E2E_SSH_PASSWORD="$ssh_password" \
    FLUXDB_SSH_E2E_SSH_KEY="$key_path" \
    FLUXDB_SSH_E2E_MYSQL="${pod}-mysql:3306:root:${mysql_password}" \
    cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop -- \
      i18n_e2e_tests::mysql_connection_form_connects_through_ssh_tunnel \
      --nocapture --test-threads=1

  echo "运行生产视图无头 E2E（MySQL 代理页签）"
  before_relayed="$(grep -c "relay 127.0.0.1:${mysql_host_port}" "$proxy_log" || true)"
  env SSH_KNOWN_HOSTS="$known_hosts" \
    FLUXDB_SSH_E2E_MYSQL="$proxy_target" \
    FLUXDB_SSH_E2E_MYSQL_PROXY="127.0.0.1:${proxy_socks_port}" \
    FLUXDB_SSH_E2E_MYSQL_PROXY_USER="$proxy_user" \
    FLUXDB_SSH_E2E_MYSQL_PROXY_PASSWORD="$proxy_password" \
    cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop -- \
      i18n_e2e_tests::mysql_connection_form_uses_proxy \
      --nocapture --test-threads=1
  after_relayed="$(grep -c "relay 127.0.0.1:${mysql_host_port}" "$proxy_log" || true)"
  test "${after_relayed:-0}" -gt "${before_relayed:-0}" || {
    echo "UI 代理用例未产生代理转发记录，保存路径可能绕过代理" >&2
    exit 1
  }
  podman rm -f "$pod-mysql" >/dev/null
fi

echo "完成：$what"
