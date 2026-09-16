# MySQL TLS/SSL 支持 · 设计与实现记录

日期：2026-09-16。状态：已实现，环境门控冒烟通过。

## 背景

MySQL 连接档案（`fluxdb-core` `MysqlTlsOptions`：`enabled` / `ssl_mode` /
`verify` / `ca` / `client_cert` / `client_key` / `sni`）早已建模并在连接
对话框 TLS 页暴露，但连接建立从未消费这些字段——sqlx MySQL 默认
`Preferred`（自发加密、不校验证书），`required` / CA / verify 全不生效。

## 实现方案

单点注入：`fluxdb-connectors/src/parts/mysql/connection_url.rs` 新增
`mysql_tls_url_params(config)`，把档案 TLS 语义折叠成 sqlx URL 参数，
追加在用户自定义 `url_params` **之后**（同名键档案覆盖用户值）。

选 URL 注入而非 `MySqlConnectOptions` setter 的原因：全部 MySQL 数据路径
（`mysql_dial` 拨号、metadata/objects/data/completion/shared 读写、
apply_changes）都经 `mysql_connection_url` 构建 URL，一处改动全路径生效。

### 模式映射

| 档案语义 | sqlx `ssl-mode` | 行为 |
|---|---|---|
| 未启用 TLS / 无档案（历史连接） | （不注入，默认 PREFERRED） | 保持既有行为不变 |
| `ssl_mode=Disabled` | `DISABLED` | 明文 |
| `Preferred` + verify + CA | `VERIFY_CA` | 加密 + 证书链校验 |
| `Required` + verify + CA | `VERIFY_CA` | 同上 |
| `Required`（无 verify 或无 CA） | `REQUIRED` | 强制加密，不校验证书 |
| `Preferred`（无 verify 或无 CA） | `PREFERRED` | 服务器支持则加密 |

- `verify=true` 且配置 CA 即视为要求校验（`Preferred+VERIFY_CA` 下服务器
  不支持 TLS 会失败——配了 CA 的用户预期就是校验）。
- `verify=false` 映射 REQUIRED/PREFERRED（sqlx 接受自签证书），覆盖
  `tls_insecure` 语义。
- 客户端证书/私钥经 `ssl-cert` / `ssl-key` 注入（mTLS），路径百分号编码。

### 已知限制

- `tls.sni`（独立校验主机名）sqlx 不支持——sqlx 校验名固定取连接 host，
  SSH 隧道下 host 为 127.0.0.1，故校验档位最高 `VERIFY_CA`（不校验主机名）。
  PostgreSQL 侧由 `tls.server_name` 支持独立校验名，无此限制。

## 测试

单测（`fluxdb-connectors` `tests.rs`）：
- `mysql_connection_url_injects_tls_params_from_profile`：VERIFY_CA 升级、
  CA/cert/key 编码注入、档案覆盖用户同名 `ssl-mode`；
- `mysql_connection_url_without_tls_profile_stays_default`：未启用不注入。

真库冒烟（环境门控）：
```
FLUXDB_MYSQL_SMOKE_TLS="127.0.0.1:55306:root:root:app|<ca>|<bad-ca>|ssluser:sslpass" \
cargo test -p fluxdb-connectors mysql_live_smoke_tls_modes
```
断言：VERIFY_CA 正例建连；错误 CA 拒绝；DISABLED 连 REQUIRE SSL 账号
（`ssluser`）被服务器拒。

## 测试环境

容器与证书见 `scripts/tls-env/certs/gen-certs.sh` 与记忆
`tls-test-env-setup`：`fluxdb-tls-mysql`（mysql:8.0.34，宿主 55306，
`--ssl-ca/--ssl-cert/--ssl-key` 启动）、`fluxdb-tls-pg`（postgres:16-alpine，
宿主 55432，`ssl=on` + `hostssl` 强制）。
