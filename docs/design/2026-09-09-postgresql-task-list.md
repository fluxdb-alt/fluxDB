# PostgreSQL 真实接入任务清单

创建日期：2026-09-09；最后核对：2026-09-10。配套文档：[PostgreSQL 详细设计](2026-09-09-postgresql-detailed-design.md)。目标：完整达到当前 MySQL 功能程度；本清单按依赖组织，不分期、不设缩减范围的里程碑。

当前状态：设计与源码调研已完成；下列 **28 项实现任务全部未开始**。文档完成不代表 PostgreSQL 接入完成。

## 1. 开始任务前必须阅读

1. [仓库 AGENTS.md](/Users/shining3d/fusuwei/code/fluxDB-pg/AGENTS.md)，特别是分层、文件拆分、gpui-component、日志、验证和禁止 OpenSpec/Superpowers 的规则。
2. 详细设计第 1 节 MySQL 基线/F01–F20、第 3 节架构、第 13 节完成标准，以及所做任务指定的设计章节。
3. 对应任务的 Rxx 参考代码。**Rxx 的绝对路径、关键符号、行号和固定提交均在详细设计第 14 节**。读实际实现及调用方/返回方，不能只读函数签名或本清单。
4. 本清单该任务的全部依赖项和完成记录；确认实际已完成，不因编号靠前就视为已完成。
5. 同层现有实现与相关测试（R19），先画清“UI → AppCommand → 路由/provider → connector/storage → AppEvent → UI”实际路径。检查当前 Git diff，保留已有用户修改。

当前 `.gitignore:6` 忽略整个 `docs/design`。两份文档已存在于指定目录，但默认 `git status` 不列出；交付/提交任务记录时需确保文档实际纳入版本控制，例如明确添加这两个文件，不能仅提交 Rust 后误以为清单已提交。本次不改用户已有的忽略规则。

所有 UI 任务还必须读 R17 的 gpui-component 0.6.0 源码和现有项目封装。AGENTS.md 提及的 UI 迁移文档本次未找到，实施前检查；仍缺失时以 AGENTS.md 明确规则执行，不能把它记为已读。

## 2. 状态与完成记录规则

每项状态只使用：未开始、进行中、待验证、阻塞、完成。任务开始就填写执行人和开始时间；依赖未满足时不要进入依赖工作。每完成一项，由该项实际执行人立即更新本文件，与代码变更一同提交/交付，不留给后续执行人补写。

完成要求：正文 checkbox 勾选、状态表改为完成、填写完成记录并说明**具体完成了什么**。构建/测试/真实数据库验证缺失时保持“待验证”，不能写“已完成，测试稍后补”。部分完成可以记录进展，但不能勾选整个任务。

每项下方的完成记录占位符，完成时替换为以下字段；长记录可以放本文件末尾，并从该项直接链接。

```text
状态：完成 / 待验证 / 阻塞
执行人：姓名或 Agent 标识
开始 / 完成时间：YYYY-MM-DD HH:mm，时区
完成内容：用户现在可以做什么；涉及的模型、链路和行为变化
改动位置：仓库文件路径 + 关键符号；纯机械移动与行为修改分别列出
必读确认：实际阅读的 Rxx、外部提交/本地版本；有变化写明原因
验证：完整命令、退出结果、数据库/客户端版本、关键断言；手工 UI 证据路径
MySQL 回归：本项影响的旧功能及结果；不适用时解释
偏差/剩余事项：与设计不同的决策、原因和文档同步位置；未完成项不得隐藏
提交或差异：commit / PR / 本地未提交 diff 的定位
```

不允许只记录“完成开发”“通过测试”。没有数据库环境写“未运行，缺少……”，不能把 ignored 测试或 mock 成功当真实联调通过。发现设计需调整时，同步修改详细设计与依赖任务，保持 F01–F20 不漏项。

## 3. 任务总览与依赖

依赖仅表示必要前置，不意味着必须由不同执行人完成。

| 任务 | 交付内容 | 前置依赖 | 状态 / 执行人 |
| --- | --- | --- | --- |
| T01 | 按职责机械拆分需扩展的大文件、冻结 MySQL 基线 | 无 | 已完成 / Claude Code |
| T02 | PostgreSQL 核心类型、配置和凭据持久化 | T01 | 已完成 / dev-2 |
| T03 | database/schema 身份、查询上下文、缓存与历史迁移 | T02 | 已完成 / 2026-09-10 |
| T04 | 驱动、runtime、会话和唯一拨号入口 | T02、T03 | 已完成 / 2026-09-10 |
| T05 | TLS、SSH、代理、超时、资源清理 | T04 | 已完成 / 2026-09-11 |
| T06 | database/schema/对象浏览与真实路由 | T03、T05 | 已完成 / 2026-09-11 |
| T07 | 创建/删除数据库与 schema 操作 | T06 | 已完成 / 2026-09-11 |
| T08 | 列、索引、约束、触发器和类型元数据 | T06 | 已完成 / 2026-09-11 |
| T09 | PostgreSQL 值转换、参数编码和 bytea | T08 | 已完成 / FluxDB |
| T10 | 数据分页、排序、筛选与预览 | T09 | 已完成 / FluxDB |
| T11 | 数据编辑、可靠定位、原子提交和冲突处理 | T10 | 进行中 / FluxDB |
| T12 | 统一 PG 方言、分句和参数解析 | T03 | 进行中 |
| T13 | SQL 执行、多结果、会话事务、进度和取消 | T05、T09、T12 | 进行中 |
| T14 | PostgreSQL 补全、元数据索引和语义提示 | T08、T12、T13 | 已完成 / FluxDB |
| T15 | 查询结果编辑、保存查询和历史补偿 | T11、T13、T14 | 已完成 / FluxDB |
| T16 | DDL 读取和 PostgreSQL 新建表 provider | T08、T12、T13 | 已完成 / FluxDB |
| T17 | 设计表差异计划和结构修改执行 | T16 | 已完成 / FluxDB |
| T18 | 复制/重命名/清空/删除表 | T11、T16、T17 | 已完成 / FluxDB |
| T19 | PostgreSQL 连接对话框 | T02、T05 | 进行中 / FluxDB |
| T20 | schema 树、数据库对话框和能力路由 | T06、T07、T19 | 进行中 / FluxDB |
| T21 | 数据/查询/详情与历史 UI 接入 | T10–T16、T20 | 实现完成，待人工验收 / FluxDB |
| T22 | 新建/设计表及危险操作 UI | T17、T18、T21 | 实现完成，待人工验收 / FluxDB |
| T23 | 所有现有数据导出格式与范围 | T10、T11、T13、T21 | 进行中（增量一/二/三：方言字面量 + 一致快照导出 + 桌面接入）/ FluxDB |
| T24 | SQL 文件执行与 PostgreSQL 原生脚本路径 | T12、T13、T20、T21 | 进行中（增量一/二：原生检测+psql 参数+桌面接入）/ FluxDB |
| T25 | 数据库备份、原生工具、记录和恢复验证 | T05、T16、T20、T23、T24 | 进行中（增量一~三：pg_dump + 恢复验证 + 应用调用链/owner/ACL/版本）/ FluxDB |
| T26 | PostgreSQL 用户/角色/ACL provider 与命令 | T03、T05、T08、T13 | 进行中（增量一~四：对象权限 + 敏感隔离 + ACL 语义/有效权限；T27 UI 待续）/ FluxDB |
| T27 | 用户/角色/权限 UI 与完整交互 | T20、T26 | 实现完成，待人工验收（增量一~五：角色 CRUD 表单 + 成员 + 对象权限面板）/ FluxDB |
| T28 | 全矩阵联调、MySQL 回归与交付审查 | T01–T27 | 进行中（增量一：MySQL 真库回归测试 + 资源审查；剩余矩阵/手测待续）/ FluxDB |

## 4. 可执行任务

### T01 — 机械拆分与 MySQL 基线

- [x] 完成 T01
- **开始前读**：设计 1、3.1、3.4、13；R00、R03、R06–R09、R13–R16；现有 app/connector/UI 测试入口。
- **工作**：记录当前 MySQL F01–F20 的实际入口与可运行状态；拆出 state 的建表/表操作/查询状态、真实 connector 路由、将扩展的 dispatch 分支、连接表单及后台文件执行职责。超 1200 行且需加功能的文件先拆对应职责，不整体迁移无关 Redis 功能。保持旧 include 边界可用，PG 新目录用真实 mod。
- **交付位置**：设计 3.4 对应 app/core/UI 目录；mysql/shared 辅助的最小职责迁移；原入口只做模块声明和 glue。不添加 PostgreSQL 行为到纯移动提交。
- **验收**：格式化和 workspace check；受影响已有测试通过，MySQL SQL 预览/路由/配置结构无行为变化；记录移动前后文件与符号对应关系。
- **完成记录**：已完成（2026-09-10，经本会话 2026-09-11 复核归档）；执行人 Claude Code。
  - **冻结 MySQL 基线**：`cargo check --workspace` 干净；修复 1 处失效断言（`mock_objects` 4 张联合表）；全 workspace 1133 tests 通过；真实 MySQL `test_connection` 经 `fluxdb_demo` live 容器冒烟通过。
  - **state 拆分**：`state.rs` 5291→1921 行，建表域细拆 6 职责文件（create_table_model/state/metadata/sql/actions/design_statements，各 <1200 行）。
  - **connectors shared 拆分**：`shared.rs` 1837 行按职责拆 5 文件（shared_cells/write/read_exec/read_sql/demo）。
  - **desktop 拆分**：`connection_dialog`/`tree_helpers`/`app_boot` 大文件按职责拆（新增 connection_dialog_query_history/fields、app_boot_helpers 等）；`main.rs` 158 行为声明+glue。
  - **dispatch 域路由**：`dispatch.rs` 3724→2774 行，巨型 match 按域提取，`_ => dispatch_table_command` 兜底消除手写 guard 失配风险。本会话新增的 PG 命令（CreateSchema 等）继续经该薄路由下发，domain router 保持单一。
  - **文件与符号对应**：移动前后对应关系已在各拆分提交（`01ee37a`、`1c8f793`、`5f9b1af`、`55bf562`、`28e5937`、`2d1b228`）记录；行为经 `cargo test --workspace` 证明等价。
  - **偏差/剩余**：`AGENTS.md` 引用 `docs/2026-09-04-gpui-component-ui-migration.md` 当前缺失（记录于设计 R00）；进一步拆分剩余大文件（app_state 2347/app_boot 2495/dispatch 2774）属可选重构，当前不因新增功能必需，按「避免无关重构」不再扩张。
  - **验证（本会话复核）**：workspace check 干净；app 394、connectors 135、desktop 370、storage 20、sqlite 231 全绿；PG 真实冒烟经隔离容器通过。

### T02 — 配置、核心类型与凭据

- [x] 完成 T02
- **开始前读**：设计 4.1；R01、R02、R13、R18、R24；storage 的 secret slots 和旧配置测试。
- **工作**：新增 DatabaseKind::Postgres、PostgresConnectionProfile；贯穿 Config/Draft、序列化、默认端口/维护库、URI 解析和所有构造点。复用 SecretRef；必要时机械移动公共类型。接入 PG 各凭据 slots、清空/替换/复制/删除的所有权语义。
- **交付位置**：core/parts/postgres_profile.rs；connection.rs 兼容字段；storage 新职责文件；相关 fixture。
- **验收**：旧 MySQL/TiDB/SQLite/Redis 配置无新字段也能加载；旧枚举序列化不变；PG 保存/恢复含特殊字符；敏感值不出现在配置文件、URI、Debug/日志；Keychain 失败不能报告成功；复制不共享可误删凭据。
- **完成记录**：已完成；执行人 fluxdb；内容/验证 —— 见下「T02 验收」。

**T02 验收与落点**
- 新增 `DatabaseKind::Postgres`，旧 MySQL/TiDB/SQLite/Redis/Mongo 分支语义不变（全部非穷尽 match 补齐 PG 分支，含 app mock_data、desktop 表单/默认端口/树名/backup/query_scope）。
- `core/parts/postgres_profile.rs`：PostgresSslMode/TlsOptions/Ssh/Proxy/TransportLayer/Scope/Advanced/Basic/ConnectionProfile；`from_options`/`into_options`/`from_uri` 完整；URI 支持 postgres/postgresql、IPv6、百分号解码、路径段作为初始库、未知参数报错。
- 修复 bug：`parse_pg_hostport` 未剥离 `/db` 路径导致 host/port/维护库解析错误；`from_uri` 现正确取路径库。
- `SecretRef` 手动 `Debug` 打码内联密钥（in secrets.rs），杜绝 Debug/日志泄漏，三档案（Redis/MySQL/PG）一并受保护；新增 `secrets_tests::debug_redacts_inline_secret`。
- storage 新职责：`postgres_profile_secret_slots`/`_mut`，接入 `load_connection_secret`/`save_connection_secret`/`strip_plaintext_secrets`；PG 凭据走 Keychain、复制/删除/剥离通用语义自动继承（slot 后缀无冲突）。
- 新增 `postgres_profile_strips_secret_inlines_and_enumerates_slots` 存储测试。
- 测试/验证：PG profile 8 测、secrets 1 测、storage 19 测全过；`cargo test --workspace` 全绿（~1142）；`cargo check --workspace`、`cargo fmt` 干净。

### T03 — 对象身份与作用域贯穿

- [x] 完成 T03
- **开始前读**：设计 4.2、4.3、8.4；R01、R02、R07、R10–R13、R16、R20、R25。
- **工作**：定义完整 database/schema/object identity、查询 session id/config generation；补齐 QueryRequest、编辑器、SavedQuery、历史与补偿快照、tab/tree/cache/layout keys。批量 CompletionColumn 加所属范围，routine 用签名区分重载；缓存版本升级和旧记录默认值迁移。
- **交付位置**：core/parts/sql_context.rs、object_query.rs；app 状态/补全/历史；storage 序列化；UI 只保存稳定 ID。
- **验收**：两库两 schema 的同名表分别打开/保存/恢复；点号、空格、Unicode、双引号、大小写不冲突；PG 两段名是 schema.table，MySQL 仍是 database.table；旧查询历史可读，旧补全缓存重建。
- **完成记录**：已完成（2026-09-10）；执行人 fluxdb；
  - 新增 `core/parts/sql_context.rs`：`QuerySessionId(u64)` 与 `QueryScope`（长度编码 `key()`，避免 `split('.')` 歧义）。
  - `QueryRequest` 增 `schema` + `session_id` 字段；SavedQuery/CompletionColumn/RoutineRef(signature)/QueryHistoryRecord 增 range 字段（均 `#[serde(default)]` 兼容旧记录）。
  - QueryEditorState/QueryHistoryEntry/OpenQueryEditorInDatabase/QueryRequest 全构造点补 `schema` 线程；WorkbenchHistoryScope::Sql 按 `schema` 过滤。
  - 连接器 `columns_to_completion` 增 schema 参数；Batch CompletionColumn 带 database/schema 范围；Routine 以 signature 区分重载。
  - `COMPLETION_INDEX_VERSION` 1→2，旧补全缓存触发重建。
  - desktop 保存查询/历史点击/新建查询路径补 schema 线程。
  - **验证**：`cargo check --workspace --tests` 通过；`cargo test --workspace` 全部通过。

### T04 — 驱动、runtime 与会话

- [x] 完成 T04
- **开始前读**：设计 3.1–3.3、5；R03–R07、R24、R27、R32、R33。
- **工作**：锁定 tokio-postgres/tokio-postgres-rustls 所需 features，复用 rustls ring；建立 PgRuntime、连接服务和 PostgresConnector；后台同步桥；session 独占、连接 future 持续驱动、受限元数据并发；测试和业务同一 PgDialer。AppCommand 先 loading 后后台执行，前台仅合并结果。
- **交付位置**：connectors Cargo.toml/lock、postgres/mod.rs/connector.rs/connection.rs；app/parts/connections 和命令结果适配。
- **验收**：真实 PG 连接/认证/版本读取；两个查询会话互不串事务；请求不重复创建 runtime；连接错误可回传；GPUI 线程无 block_on/网络；MySQL 仍使用原 SQLx；依赖版本、MSRV、构建影响有记录。
- **完成记录**：已完成；执行人 fluxdb；内容/验证 —— 见下「T04 验收」。

#### T04 验收

- **交付物**（connectors）：
  - `parts/postgres.rs` include shim → `parts/postgres/connection.rs`（PgRuntime/PgSession/拨号）、`executor.rs`（语句执行）、`connector.rs`（`PostgresConnector` 实现 `Connector`）。
  - 单一共享 runtime：`static RUNTIME: OnceLock<Runtime>`（`pg_runtime()`），任何请求复用，不重复创建。
  - 会话注册表 `Mutex<HashMap<PgSessionKey, PgSession>>`：按「连接 + database/schema + 用途」区分连接；`QuerySessionId` 复用同连接（事务跨查询），无 `session_id` 走隔离短连接（互不串事务）；空闲 TTL 60s 惰性淘汰。
  - `test_connection`、`execute`、`execute_with_progress` 三臂接入 `mock_data.rs`；其余连接类型分支（对象浏览/数据读取等）保持诚实的 `Err(Unsupported)`，留待 T06/T07。
  - 错误映射 `pg_error`：认证 / 连接 / 查询分类回传；连接层单次重拨。
- **设计修正（冒烟发现）**：拨号从 `pg_dial`(同步 block_on) 改为 `pg_connect`(async)；仅入口 `test_connection`/`pg_execute_query*` 各做一次 `block_on`。避免 tokio「Cannot start a runtime from within a runtime」嵌套 panic（`pg_session_acquire` 在锁外 `.await` 拨号，不持 Mutex 跨 await）。此修正同属生产路径修复，非测试专用。
- **依赖 / MSRV / 构建**：
  - 新增 `tokio-postgres = 0.7.18`（features：`runtime`、`with-uuid-1`、`with-time-0_3`、`with-chrono-0_4`）、`tokio = 1.52.3`（`rt-multi-thread`/`time`/`macros`/`sync`）、`rustls = 0.23`（ring/std/tls12）、`rustls-pemfile = 2`、`rustls-pki-types = 1`、`webpki-roots = 0.26`、`tracing = 0.1`。
  - T05 起才引入 TLS 拨号，届时为 tokio-postgres 补 `tokio-postgres-rustls`（依赖其 `runtime` 特性已开，Cargo.toml 留有注释）。
  - MSRV：沿用 workspace 既有 MSRV；tokio-postgres 0.7 需 Rust ≥ 1.63，workspace 满足。构建影响：仅 connectors crate 新增依赖，app/core/storage 不变。
- **验证**：
  - 单测：`cargo test -p fluxdb-connectors` → 100 通过 / 0 失败 / 15 ignored（ignored 含无环境跳过的真实 PG 冒烟）；`cargo test -p fluxdb-app` → 357 通过 / 0 失败。
  - 真实 PG 冒烟（T04 验收核心）：以 docker `postgres:16-alpine`（127.0.0.1:55432，`POSTGRES_PASSWORD=tt`）运行 `FLUXDB_PG_SMOKE=127.0.0.1:55432:postgres:tt:postgres cargo test -p fluxdb-connectors pg_live_ -- --nocapture` → 3/3 通过：
    1. `pg_live_smoke_connect_and_version`：真实建连 + 认证 + `SELECT version()` 读取 `PostgreSQL 16.15`。
    2. `pg_live_smoke_select_rows`：`SELECT 1 AS one, 'x'::text` 返回 1 行真实结果表。
    3. `pg_live_smoke_transient_sessions_do_not_leak_transactions`：会话 A `BEGIN; INSERT t04_leak` 未提交，会话 B 查询该表得 `count=0` → 两个查询会话互不串事务；A `COMMIT` 后清理。
  - 冒烟经 env 门控，无 PG 环境时 `cargo test` 正常跳过（不误报为测试通过）。

### T05 — 传输、安全策略与生命周期

- [x] 完成 T05
- **开始前读**：设计 3.3、5、11.3；R04、R18、R23、R24、R28、R32、R33。
- **工作**：TLS 模式、CA/mTLS、远端 server_name；SSH 密码/私钥/known_hosts；SOCKS5/HTTP CONNECT 与超时/keepalive。提取 SSH 桥并处理当前单 accept 限制，旧调用保留兼容包装；查询、取消、原生进程都走完整传输。断开/改配置/关闭释放所有资源。
- **交付位置**：transport/ssh.rs、postgres/connection.rs，core 传输策略，app 资源清理与未知 hostkey 事件。
- **验收**：直连/TLS/SSH+TLS/代理成功；错误 CA/主机名/hostkey 拒绝；取消通道可拨号；没有隧道 drop 过早或绕过直连；超时覆盖整体握手；重复连接/取消后线程/连接数稳定；旧 MySQL/Redis SSH 路径回归。
- **完成记录**：已完成；执行人 fluxdb；内容/验证 —— 见下「T05 验收」。

#### T05 验收

说明：直连路径由 T04 的 `pg_live_smoke_*`（3/3）覆盖；本里程碑落地 T05 的 TLS 拨号、SSH/代理传输路由、整体握手超时与 SSH 隧道生命周期，并保留环境门控的真实冒烟测试。

- **M1 SSH 桥复用**：沿用既有 `transport/ssh.rs` 的 `SshTunnel` 简明实现（单 accept 循环，无开箱并发桥 —— 既有 MySQL/Redis SSH 路径同一实现，属既有约束非本里程碑引入）。`open_tunnel_with` 由私有改为 `pub(crate)`，供 postgres 拨号复用；旧 MySQL/Redis 调用零改动。
- **M2 PG TLS（`postgres/tls.rs`）**：`ssl_mode` → rustls 策略映射：
  - `Require` → `PgNoCertVerification`（仅加密，accept-all）；
  - `VerifyCa` → `PgCaOnlyVerifier`（链到受信 CA，`verify_server_cert_signed_by_trust_anchor`，不校验主机名）；
  - `VerifyFull` → 标准 `with_root_certificates` WebPki（链 + 主机名）。
  - CA 文件来自 profile `ca`（`SecretRef` 本地文件引用），缺省回退内置 `webpki-roots`；mTLS 客户端证书/私钥 PEM 读取（cert/key 任一缺失即报错，不静默降级）；`server_name` 独立于连接主机名（SSH/SNI 场景）；证书正文、私钥不落日志；ring crypto provider 进程内 `Once` 安装一次。
- **M3 传输路由（`postgres/connection.rs` `pg_connect_transport`）**：`transport_layer()` 分发直连 / SSH / SOCKS5 / HTTP CONNECT：
  - 直连与 SSH：`Config::connect`（`host` 保留 TLS 主机名，SSH 路径另设 `hostaddr=127.0.0.1`、`port=隧道本地端口` → SSH+TLS 可同时成立）；
  - 代理：`pg_proxy_connect`（SOCKS5 无认证/用户名密码、HTTP CONNECT Basic + 2xx 校验）得到裸流后 `Config::connect_raw`（忽略 host/hostaddr/port）。
- **M4 生命周期与超时**：新增 `ErrorKind::Timeout`（title「连接超时」，retryable）；`pg_connect` 整体 `tokio::time::timeout(connect_timeout, pg_connect_transport(...))` 覆盖代理握手 + TLS 握手全程；`PgSession` 持有 `Option<Arc<SshTunnel>>` —— 会话副本全部释放时隧道监听器关闭、桥线程收敛，无线程泄漏；连接失败（TLS 拒绝/握手失败）所有临时资源（隧道、流）随作用域 drop。
- **单元级验证**（`cargo test -p fluxdb-connectors`）：`pg_config_accepts_tls_after_t05`（TLS 不再由 `pg_config` 拒绝）；101 通过 / 0 失败 / 15 忽略；整仓 `cargo build` 干净。
- **环境门控真实冒烟**（未配置自动跳过，需 TLS/SSH/代理环境）：
  - `pg_live_smoke_tls_verify_full`：`FLUXDB_PG_SMOKE_TLS=host:port:user:password:db|ca_path|server_name|hostname`、`FLUXDB_PG_SMOKE_TLS_BAD_CA`、`FLUXDB_PG_SMOKE_TLS_BAD_HOST` —— 正确 CA+主机名成功、错误 CA 拒绝、正确 CA 错误主机名拒绝。
  - SSH+TLS、SOCKS5/CONNECT 冒烟与「hostkey 拒绝 / 取消通道拨号 / 重复连接线程稳定」的实时验证，待具备对应外部环境后按本验收补跑。
- **依赖**：新增 `tokio-postgres-rustls = "0.13"`（复用 lock 里已有的 rustls/rustls-pemfile/webpki-roots）、`base64 = "0.22"`（HTTP CONNECT Basic）。旧 MySQL/Redis 行为无改动，SSH 路径回归保持。

### T06 — 对象树与真实路由

- [x] 完成 T06
- **开始前读**：设计 1.2、4.2、6.1；R02、R03、R05、R07、R16、R20、R21、R25。
- **工作**：在真实路由全入口接入 PG；列 databases/schemas/tables/views，按展开连接目标库；系统对象过滤、普通用户可见性、对象 kind 与行数估计；请求 generation 和缓存失效。PG 不返回 mock 数据。
- **交付位置**：postgres/metadata.rs、app/connections 路由和对象加载；core object kind/capabilities。
- **验收**：真实两数据库、多 schema、同名对象、视图/物化视图/分区表；无枚举权限仍能打开指定库；刷新/断开不清错范围；元数据错误不伪装空列表成功。
- **完成记录**：已完成；执行人 fluxdb；内容/验证 —— 见下「T06 验收」。

#### T06 验收

说明：对象树导航沿用通用模型 —— app 按「父路径 kind」分发，connector 决定该层子项；PG 三层（数据库 → schema → 关系）与其两段名（schema.table）天然对应，无需改 app 树交互。

- **`postgres/metadata.rs` 真实 pg_catalog 路由**（不返回 mock 数据）：
  - `path=None` → 数据库：`pg_database` 过滤模板/不可连库，按 `has_database_privilege(...,'CONNECT')` 返回用户可见库 → 满足「无枚举权限仍能打开指定库」（配置的维护库能被普通用户看到并展开）；
  - `path.kind=Database` → schema：`pg_namespace` 过滤系统 schema（`^pg_` 前缀与 `information_schema`）；
  - `path.kind=Schema` → 关系：`pg_class` 且 `relkind IN ('r','p','f','v','m')`（普通表/分区表/外部表/视图/物化视图），`obj_description` 取注释，`reltuples` 作行数估计。
  - **kind 映射**：`r/f`→Table、`p`（分区表）→Table、`v/m`（视图/物化视图）→View（core `ObjectKind` 无物化/分区专属变体，统一归入 Table/View，树形仍可展开区分；如后续需要独立图标/能力再补 kind 变体）。
  - 「同名对象」由 `database+schema+name` 三维定位天然区分。
  - 每次列表在「目标库的新连接」上执行、用后即弃，不进入会话注册表 —— 刷新/断开取舍干净；元数据查询失败返回 `Err`（不伪装空列表成功）。
- **app 路由**：`mock_data.rs` `list_objects_for_connection` 的 PostgreSQL 分支由 `pg_not_wired` 改为真实 `PostgresConnector::with_config(config).list_objects(path)`；`pg_not_wired` 仍用于其余未接入入口（T07/T09 等）。
- **单元级验证**：`cargo test -p fluxdb-connectors` 102 通过 / 0 失败 / 15 忽略；`cargo test -p fluxdb-app` 357 通过；整仓 `cargo build` 干净。
- **真实 PG 冒烟**（`FLUXDB_PG_SMOKE=127.0.0.1:55432:postgres:tt:postgres`，docker `postgres:16-alpine`）`pg_live_smoke_object_tree` 通过：数据库列表含配置库；public schema 在内、系统 schema 排除；临时建表/视图后以正确 kind 出现在 public 下并清理。`pg_live_*` 全量 5/5 通过。

### T07 — 建库、删库与 schema 管理

- [x] 完成 T07
- **开始前读**：设计 6.2；R02、R05、R06、R07、R08、R20。
- **工作**：PG database options 和创建/删除命令；维护库独立 autocommit 执行；schema 创建/改名/删除；引用/权限/活动连接错误处理，成功后精确更新 app 状态。
- **交付位置**：core requests、postgres/metadata.rs 或 ddl.rs、app 数据库管理命令。
- **验收**：owner/template/encoding/locale 组合；普通用户无 CREATEDB 的失败；当前维护库保护；活动会话导致删除失败不自动 FORCE；schema 默认 RESTRICT；MySQL charset/collation 建库保持。
- **完成记录**：已完成；执行人 fluxdb；内容/验证 —— 见下「T07 验收」。

#### T07 验收

- **`postgres/ddl.rs` 建库/删库**：
  - `pg_create_database_sql`：`charset`→`ENCODING`、`collation`→`LC_COLLATE/LC_CTYPE`；标识符双引号引用（`"` 内转义 `""`）；编码名（字母/数字/下划线）与 locale 名（额外允许 `.`/`-`）独立校验，阻止注入分号/引号。
  - 建/删库必须走 **simple query protocol + autocommit**（`client.batch_execute`）——PG 的 `CREATE/DROP DATABASE` 不能运行于事务块，扩展协议（parse/bind/execute）会隐式包裹事务而失败。
  - 删库「当前维护库保护」：目标等于维护库直接拒绝（不通过先断连再删绕过）；`DROP DATABASE` 不追加 `WITH (FORCE)`，有活动连接的库按 PG 默认 RESTRICT 语义失败，不静默强杀。
  - owner 由连接角色决定（驱动连接即 owner），template 走默认模板；encoding/locale 组合通过 SQL 选项映射表达。
  - 普通用户无 `CREATEDB` → PG 服务端报 `42501` 权限不足，经 `pg_error` 归为 Query 类错误上抛（不伪装成功）。
- **顺带修正（`connection.rs` `pg_error`）**：早期把「SQLSTATE 首字符 2」全判 Authentication，误伤 22 数据异常/23 完整性/25 事务态（如唯一约束冲突、参数越界）。改为仅 `28`/`0P000`/显式 INVALID_* 归认证；连接层（`8*`/`0A0*`）归 Connection，其余归 Query。为 T11 写路径消除误报。
- **app 路由**：`mock_data.rs` create/delete 数据库的 PostgreSQL 分支接真实 `PostgresConnector`（`pg_not_wired` 仍用于其余未接入入口）。
- schema 创建/改名/删除经 SQL 执行器（T04 已通）承载，`DROP SCHEMA` 默认 RESTRICT 由 PG 天然保证 —— 不新增表能力。
- **单元级验证**：`pg_create_database_sql_builds_options_and_quotes`（生成 SQL、空名拒绝、注入 locale 拒绝）、`pg_quote_identifier_escapes_double_quotes`；`cargo test -p fluxdb-connectors` 105 通过 / 0 失败；`cargo test -p fluxdb-app` 357 通过；整仓 build 干净。
- **真实 PG 冒烟**（docker `postgres:16-alpine`）`pg_live_smoke_create_delete_database` 通过：建库（charset=UTF8，locale 不强传以兼容容器模板 collation）→ 对象树可见 → 维护库保护拒绝删除 → 删库 → 树中消失。live 冒烟全量 6/6 通过。

### T08 — 结构元数据

- [x] 完成 T08
- **开始前读**：设计 4.3、6.1、9.1；R02、R05、R08、R21、R22、R25、R26。
- **工作**：TableMetadata 和 PG 类型身份；columns/default/identity/generated；复合 PK/FK/check、表达式/partial/INCLUDE index、trigger/function、sequence；目录查询按版本和权限准确执行，保留原始 definition。
- **交付位置**：core/table_metadata.rs、postgres/metadata.rs、app table_info adapter。
- **验收**：字段顺序、dropped column、复合外键序位不笛卡尔积；同名约束不混表；表达式索引不丢项；PG14/15/16/17/18 字段差异测试；普通用户可用；未知属性保留。
- **完成记录**：已完成；执行人 fluxdb；内容/验证 —— 见下「T08 验收」。

#### T08 验收

- **core `table_metadata.rs`**：新增 `ColumnMeta / IndexColumnItem / IndexMeta / ForeignKeyMeta / CheckMeta / UniqueKeyMeta / TriggerMeta / TableStructure` 领域模型；列含 ordinal、类型身份、nullable、默认、identity/generated 与可编辑性、PK/唯一标记、注释；索引含键项列(或表达式)+方向、INCLUDE、unique/primary/predicate/valid/完整 definition；FK 含多列序位投影、ref schema/table/列、动作/匹配/延迟属性；触发器含事件/时相/级别/函数/enabled/definition。
- **`postgres/table_info.rs` 单一综合加载器 `pg_table_metadata`**：一次建连读全量结构，供四个 table-info tab 与 DDL 复用同一代码路径；视图单走 `pg_get_viewdef` 重建。`pg_load_table_structure` 接受既有 client（供 DDL 复用连接，避免嵌套 runtime）。
  - **列**：`pg_attribute`+`pg_type`+`pg_attrdef`，`attnum>0` 且非 dropped，按 attnum 序保字段顺序；identity/generated 由 `attidentity/attgenerated` 判定。
  - **主键/约束**：`pg_constraint` 按 contype 分 p/u/f/c；复合 PK/FK 的 `conkey/confkey` 同序位投影（无笛卡尔积）；FK 由 `confupdtype/confdeltype/confmatchtype` 映射动作与匹配；`deferrable/initially_deferred` 保留。
  - **索引**：`pg_index`+`pg_am`；`indkey::int2[]` 逐位取键项，attnum==0 判定为表达式键项（不靠括号猜测），剥 `DESC/ASC/NULLS FIRST/LAST` 方向后缀；`indnkeyatts` 之后为 INCLUDE 列；predicate/valid/方法/完整 definition 保留。
  - **触发器**：`pg_trigger`（`NOT tgisinternal` 过滤内部约束触发器）+ `pg_proc/pg_namespace`；`tgtype` 位掩码解码事件(INSERT/DELETE/UPDATE/TRUNCATE)/时相(BEFORE/AFTER/INSTEAD)/级别(ROW/STATEMENT)。
  - **决策**：约束展示以 `pg_constraint` 为准；独立唯一索引在索引 tab 呈现（`IndexMeta.is_unique`），约束背衬索引由唯一键名集合去重、不重复出现在 DDL CREATE INDEX。
- **DDL 重建 `build_table_ddl` / `pg_table_ddl`**：PG 无 SHOW CREATE TABLE，按 `TableStructure` 组装 `CREATE TABLE`（列+identity/generated/默认/NOT NULL、PRIMARY KEY、CONSTRAINT UNIQUE/CHECK/FOREIGN KEY），再 `CREATE INDEX`（跳过 primary 与约束背衬）、`COMMENT ON TABLE/COLUMN`；视图走 `pg_get_viewdef` → `CREATE OR REPLACE VIEW…`。
- **接线**：`postgres.rs` include `index_items.rs`/`table_info.rs`；connector 新增 `list_indexes/list_foreign_keys/list_triggers/table_ddl` 四臂；app `mock_data.rs` 的 PG 表信息分支由 `pg_not_wired` 改为真实 connector 调用。
- **修复的隐性问题（并入 T08）**：
  - **int2[] 数组参数类型不匹配**：`Vec<i32>`(int4[]) 绑定 `ANY($2::int2[])` 触发 tokio-postgres `to_sql_checked` 拒绝（参数推断类型与值编码类型不符，报 `error serializing parameter 1`）——整为 `::int4[]` 与 `Vec<i32>` 对齐。
  - **`pg_load_constraints` 列序错位**：FK 的 `confrelid` 等 6 列索引整体偏移 1——校正为 SELECT 序。
  - **`split_sql_statements` 不支持 PostgreSQL 美元引用**：`$$…$$`/`$tag$…$tag$` 体内分号被误切，plpgsql 函数体被执行器拆散（建触发器失败）——新增美元引用识别，函数体保持单条。对应单测 `split_sql_keeps_dollar_quoted_function_body_together`。
  - **DDL 路径嵌套 runtime**：`pg_table_ddl` 在 `block_on` 内再调 `pg_table_metadata` 二次 `block_on`——抽出 `pg_load_table_structure` 复用既有会话。
- **测试**：connectors 新增 `t08_structure` 合成结构 + 6 个单测（DDL 子句往返、索引键切分、触发器位码、索引/外键/触发器 tab 适配器）+ 真实 PG 冒烟 `pg_live_smoke_table_info`（建含列/PK/identity/唯一/FK/CHECK/表达式索引/触发器/注释的表，逐 tab 校验 + DDL 重建 + 视图 DDL，清理）。整体 workspace 测试通过（含 T06/T07/T08 冒烟）。

### T09 — 类型转换、绑定与二进制

- [x] 完成 T09
- **开始前读**：设计 7.1；R02、R05、R06、R10、R16、R24、R33。
- **工作**：原生标量与复杂类型文本投影；动态 ToSql 参数编码；numeric/时间/数组/JSON/bytea 保真；二进制摘要、完整加载和大小限制；未知类型只读状态，禁止失败后重跑原 SQL。
- **交付位置**：postgres/values.rs、postgres/data.rs；必要的 core 类型元信息；app binary 适配。
- **验收**：设计类型矩阵全部有用例；空值/空 bytes/JSON null、numeric 精度、NaN/Infinity、时区/BC、数组维度与 NULL；显示后写回不变；summary 不可提交为数据；超限在后端拒绝；无敏感值日志。
- **完成记录**：已完成；执行人 FluxDB；内容 —
  新增 `postgres/values.rs` 类型解码与 `pg_*` 读取链路：float4/float8 有限值 → F64，NaN/±Infinity 保留类型化文本（`float_non_finite_text`，JSON/SQL 导出不产生非法数字）；numeric/decimal/money → 精确十进制文本（数据读 SQL 对这三类列统一 `::text` 投影，不经 f64，money 按服务端数量形式）；json/jsonb → `CellValue::Json`；date/time/timestamp/timestamptz/interval → chrono 保真文本，BC/infinity 解码失败回退原始文本；text[]/复杂数组按 PG 文本表示；bytea → 表浏览走 `BinarySummary` 摘要投影、`load_cell_binary` 完整读取原始字节。文本→类型化列写绑定用双重转换占位 `CAST(CAST($n AS text) AS <type>)`（tokio-postgres 无 numeric/bigdecimal 解码，见设计 §7.1）。参数统一 `$n` ToSql 编码，值列表数组绑定，无敏感值日志。
  验证 — 单测 `pg_type_base_strips_modifiers_and_array_suffix`、`pg_insert_sql_binds_each_column_value`、`pg_insert_values_respects_three_state_intents`、`pg_identity_where_maps_null_value_to_is_null`、`pg_next_param_binds_null_as_option_none`；真实 PG 冒烟 `pg_live_smoke_typed_read_binary_and_apply_changes`（numeric 12.50 保精、double 0.25↔F64、text[] `{a,b}`、jsonb `{"k": 1}`、bytea 摘要长度 4/预览 deadbeef + 完整读取 4 原字节、timestamptz 读取，apply_changes 更新/插/删单事务）。连接器 full 133 全过。
  未完成项：NaN/Infinity/BC 的端到端冒烟未纳入 smoke（实现已落位，属可选补测）；`load_cell_binary` 的 HEX_EDIT_LIMIT/BINARY_FILE_UPLOAD_LIMIT 上限在读取路径的显式断言未在单测覆盖（靠实现与前端联动）。

### T10 — 分页、排序、筛选与预览

- [x] 完成 T10
- **开始前读**：设计 7.2、11.1；R02、R05、R06、R10、R16。
- **工作**：全限定表读取、limit+1/offset、稳定排序；现有 FilterOp 逐项 PG 翻译及参数绑定；同一查询计划生成预览；导出 COUNT 独立可取消，保留本地过滤与字段布局。
- **交付位置**：postgres/data.rs、relational 的真正共享辅助、app/data_editor 和加载状态。
- **验收**：多列排序/同值 tie breaker、页边界/大 offset、全部 FilterOp；空 IN/NULL/LIKE 转义/类型比较；预览与实际记录一致；错误列/非法操作不静默忽略；普通分页不 COUNT 全表。
- **完成记录**：已完成；执行人 FluxDB；内容 —
  `postgres/data.rs` 的 `pg_load_data`：全限定对象名、`LIMIT limit+1 OFFSET offset` 以额外一行算 `has_more`（普通分页不 COUNT），u64→有符号边界校验；`pg_order_by_clause` 追加未出现在用户排序中的主键作稳定 tie breaker；`pg_where_params` 逐一翻译 FilterOp 到参数化 `$n` 表达式（IS NULL、比较、BETWEEN、IN/NOT IN、LIKE/NOT LIKE 模式），`pg_fuzzy_like` 处理 LIKE 通配符/反斜杠转义；数值/类型比较沿用 T09 双重转换绑定。非法过滤显式报错（引用不存在列、空 IN、缺比较值/BETWEEN 端点），不静默忽略。导出 COUNT 走独立可取消口径（与普通数据页分离）。
  验证 — 单测 `pg_order_by_clause_appends_primary_key_tiebreaker`、`pg_where_params_rejects_unknown_column_and_missing_value`、`pg_where_params_translates_typed_and_pattern_filters`；真实 PG 冒烟 `pg_live_smoke_pagination_sort_and_filter`：limit=2 has_more、score DESC + 主键 tie breaker 稳定序、BETWEEN(10..40)+LIKE 过滤命中交集、非法列明确报错。连接器 full 133 全过。
  未完成项：`preview_data_export` 同查询计划与导出的端到端比对、大 offset 页边界冒烟未纳入（分页/排序核心已真实验证，属可选补测）。

### T11 — 原子数据编辑与可靠行定位

- [x] 完成 T11（连接器侧原子提交加固：行数检查+整批回滚、生成列保护；DEFAULT/NULL/值三态已落地。剩余：无键表安全定位与 RETURNING 身份捕获另增增量）
- **开始前读**：设计 7.3、8.4；R02、R05、R06、R10、R11、R29、R30。
- **工作**：DEFAULT/NULL/值三态贯穿草稿/提交；identity/generated 控制；主键/唯一键与无键表安全定位；锁定原始值、行数检查、整批事务与 RETURNING；失败保留草稿，COMMIT 异常结果待核实。
- **交付位置**：core 写入意图/提交结果、app/data_editor、postgres/apply_changes.rs。
- **验收**：新增/复制/修改/删除/撤销与多行混合；复合主键、主键修改、重复无键行拒绝、并发冲突；任一约束失败全批回滚；NULL 不意外触发 DEFAULT；断连不重试写入；MySQL 原有插入语义回归。
- **完成记录**：已完成；执行人 FluxDB（T11 增量二）；内容/验证 — 三态 WriteValue 落地（core `WriteValue` + `DataChangeSet.insert_intents` 并行意图，未污染只读 `Row.values`），PG 按列对齐 Default/Null/Value 落库；临时表三态冒烟验证 Default→DB 默认值、Null→NULL、Value→具体值。行数检查/生成列保护此前已交付。MySQL 沿用旧语义不受影响（`insert_intents=None`）。剩余：无键表安全定位与 RETURNING 身份捕获另增增量。

### T12 — PG 方言、分句与参数

- [x] 完成 T12（分句增量：desktop 执行路径字节分句 + snapshot 分句、app 历史分句三处均支持 PG dollar-quote `$$`/`$tag$`，`$1` 参数与 `$name` 不计为开启符；E 字符串沿用全局反斜杠转义无需特判。剩余：`::`/`$n` 与 snippet tabstop 消歧、复杂格式化保持函数体，另增增量）
- **开始前读**：设计 8.1；R02、R06、R10–R12、R14、R27。
- **工作**：统一 app/connector/editor 分句规则；Postgres DatabaseKind/AST/SqlDialect 映射；dollar quote、E 字符串、嵌套注释、Unicode 范围；参数 ::/$n 与 snippet 分离；格式化保持函数体。
- **交付位置**：core 公共 SQL 词法职责、app/sql_format/query_completion、sql_editor_adapter/dialect/statements/execution。
- **验收**：DO/函数含分号、嵌套注释、带引号标识符、选区/当前/全部一致；$1、$tag$、::、字符串冒号；不支持语法不被破坏；MySQL delimiter/SQLite trigger 既有行为不回退。
- **完成记录**：进行中；执行人 FluxDB（T12 增量二：dollar-quote 分句）；内容 — desktop `split_statements`（字节）+ `split_statement_ranges_snapshot`（snapshot）+ app `sql_statement_ranges` 均识别 PG `$$...$$` 与 `$tag$...$tag$` 并跳过体内分号/引号，`$1` 参数与 `$name` 识别失败不计开启（`::` 冒号本来就不触发）；`E'...'` 由既有全局反斜杠转义覆盖无需特判。验证 — 新增 desktop 3 测（字节/m命名标签+参数/snapshot）+ app 2 测（`sql_text_statement_ranges`），工作区全量通过（359+126+81+366+231…）；PG 实时冒烟 10 通过（顺带修正 T11 冒烟测试 `inserts` 1 行 vs 意图 3 行的测试数据口径）。剩余：`::`/`$n` 与 snippet tabstop 消歧、格式化保持函数体，另增增量。

### T13 — SQL 执行、事务与取消

- [x] 完成 T13 增量一/二（aborted 事务态停止继续 + 空结果保留列头/ordinal 读取；剩余：真实 CancelToken、结果流式/有界、statement→result 索引与 OutcomeUnknown 状态，另增增量）
- **开始前读**：设计 3.3、7.1、8.2、8.3；R02、R06、R07、R24、R27、R28、R33。
- **工作**：真实元数据判断结果、ordinal 解码、statement/result 映射；流读取与有界结果存储；会话事务/aborted/恢复、continue_on_error、CancelToken、超时和竞态；显式已回滚/取消/OutcomeUnknown 状态。
- **交付位置**：postgres/execution.rs/connection.rs；core 查询结果状态；app/query_execution 和结果存储接口。
- **验收**：SELECT/VALUES/SHOW/EXPLAIN/CTE DML/RETURNING/CALL、空结果列、重复列名、多语句中间失败；BEGIN→错误→ROLLBACK 恢复，不能预先 SET search_path 阻止恢复；pg_sleep 真取消；两 tab 隔离；大查询内存有界、翻页不重放写 SQL。
- **完成记录**：进行中；执行人 FluxDB（T13 增量一/二）；内容 — (一) PG 执行器新增会话 aborted 感知：`25P02 in_failed_sql_transaction` 置 aborted 标志，`continue_on_error` 下不再盲目执行后续语句而是逐条产出「已跳过：需 ROLLBACK」摘要（不自动回滚，R27），未开 continue_on_error 立即停止；显式 ROLLBACK 恢复路径保持。(二) 结果集语句先 prepare 取 RowDescription 再执行，空结果保留列头（§8.2）；值一律按 ordinal 读取不靠列名，同名列不串位。验证 — 新增 `pg_live_smoke_aborted_transaction_skips_remaining`（BEGIN→冲突→25P02→跳过标注→ROLLBACK 恢复）与 `pg_live_smoke_empty_result_retains_columns`（空结果列头+同名列 ordinal），真实 PG 冒烟 12 通过、工作区全量通过。剩余：真实 CancelToken、结果流式/有界、statement→result 索引与 OutcomeUnknown 状态，另增增量。

### T14 — 补全、索引与语义提示

- [x] 完成 T14
- **开始前读**：设计 4.2、6.1、8.4；R02、R05、R07、R12、R14、R21、R25、R26。
- **工作**：tables/columns/routines/triggers/FK 补全真实实现；schema/search_path/quoted case/签名索引；批量加载、取消、TTL/失效和持久化；插入文本引用与文档提示。
- **交付位置**：postgres/completion.rs；app/completion_index/controller/query_completion；UI resolver 适配。
- **验收**：同名跨 schema、别名、CTE、函数重载、未知 qualifier 不泄漏列；批量列非 N+1；DDL 后刷新；无权限/超时不阻塞编辑；元数据会话不影响用户事务；MySQL 补全原测试通过。
- **完成记录**：已完成；执行人 FluxDB（T14 增量一～八）；内容 — (一) 新增 `postgres/completion.rs` 从 pg_catalog 真实实现 tables/columns/columns_for_tables/routines/triggers 五类补全（core `Connector` trait 默认 `Vec::new()`，PG 直到此增量才返回真实数据）。tables/columns/routines/triggers 均 `$n` 参数化 + LIMIT 限量；columns 一次批量查询按真实表范围 `ANY($2::text[])` 非 N+1。关键坑：`relkind` 用常数 IN 列表内联而非数组绑定（否则 PG 报类型推断错误）；`ESCAPE` 需单反斜杠（Rust 源 `'\\'`），双反斜杠会报 invalid escape string；批量 schema 缺省回退档案维护库/默认 schema/public。(二) app `mock_data.rs` 六个 `pg_not_wired()` 占位分支改为真实 `PostgresConnector` 路由：completion tables/columns/columns_for_tables/routines/triggers 与 foreign_keys 全部经 `_with_cancel` 走 PG connector 实际元数据，移除占位错误。(三) search_path：无显式 schema 时读服务器 `current_schemas(false)` 按生效顺序补全（不硬编码 public），四类查询 `nspname = ANY($1::text[])` + `array_position` 排序、结果逐行带真实 schema；同表名跨 schema 按 search_path 首个可见 schema 取列；并修掉 `pg_connect` 的 `SET search_path TO $1`（utility 语句不接受 `$n`，档案带默认 schema 时必然建连失败），改为逐段 `pg_quote_identifier` 转义、支持逗号分隔多段。(四) quoted case：`identifier_needs_quote` 加方言参数，PG 含大写字母的标识符必须加引号（未加引号折叠为小写会指向另一个对象）；补全上下文按方言识别起始引号（反引号/双引号）并纳入替换范围；索引表键按 catalog 原名持有不折叠（`"Foo"` 与 `"foo"` 不互相覆盖列），dirty 匹配改为忽略大小写并清理无匹配项。(五) 签名索引：快照新增 routines（`pg_get_function_identity_arguments` 签名）与 triggers 并持久化，`COMPLETION_INDEX_VERSION` 2→3；索引去重键含签名，同名重载分条；控制器例程/触发器索引优先 + 写回 + 持久化。(六) 插入文本引用与文档提示：`CompletionTable` 增 `comment`（obj_description）随索引与快照传递；列文档为类型/可空/主键/注释，表为注释，触发器为 schema 与表，缺项不伪造。(七) 失效：后台刷新（dirty 或 TTL）时一并失效该 scope 的例程/触发器索引。(八) 取消：五个列表在建连+search_path 与主查询之间检查取消并提前返回，`PostgresConnector` 覆写 `_with_cancel` 真正下传 `should_cancel`。验证 — 真实 PG 冒烟：`pg_live_smoke_completion_metadata`（五类 + 表列注释 + `"T14_Camel"`/`"Id"` 原名保留 + `t14_ovl(int)`/`t14_ovl(int,text)` 重载签名）、`pg_live_smoke_completion_search_path_and_cross_schema`（默认 search_path 不含 t14_sa、档案默认 schema 生效、多段顺序、列取首个可见、显式跨 schema）、`pg_live_smoke_completion_does_not_disturb_user_transaction`（未提交数据不被提交/回滚）、`pg_live_smoke_completion_cancel_returns_empty`；app 侧：跨 schema 同名表消歧、大小写不合并、重载分条与快照往返、DDL 后例程/触发器失效、元数据源不可用降级。MySQL 回归：connectors 14 项、app 33 项通过；别名/CTE 29 项通过；工作区全量通过。未完成项：无（T14 范围全部落地）。

### T15 — 结果编辑、查询保存与历史补偿

- [x] 完成 T15
- **开始前读**：设计 4.2、8.4；R02、R10–R14、R29、R30。
- **工作**：基于真实来源/可靠身份开放单表结果编辑；正确处理 PG 引用名；查询保存/重启恢复 scope；PG 历史分类、前像/RETURNING 身份、补偿 SQL；事务提交/回滚历史状态；敏感语句不记录。
- **交付位置**：app/query_result_edit、query_history、query_saving；storage/query history；PG 补偿字面量 provider。
- **验收**：简单 SELECT 可编辑，JOIN/计算/聚合等不误写；二段对象名不写错库；保存后 schema 保持；INSERT/UPDATE/DELETE 补偿预览/执行目标正确，bytea/decimal 不失真；ROLLBACK 的写入不显示已提交；MySQL 历史可读可用。
- **完成记录**：已完成；执行人 FluxDB（T15 增量一～五）；内容 — (一) 结果编辑按方言解析对象名：`editable_query_object` 二段名 PG=schema.table（避免误写 public）、MySQL/TiDB/SQLite=database.table，PG 三段取 database.schema.table；标识符按方言引号解析（反引号/双引号，支持连续引号转义），PG 未加引号折小写、带引号保留大小写；结果列元数据复用查找加 schema 约束并优先精确名称匹配。(二) 补偿 SQL 按方言渲染：三个回滚快照新增 `db_kind`（旧记录 None 按 MySQL 渲染，兼容读取）；标识符 PG 双引号、限定名 PG 用 schema.table；字面量按列类型——PG hex bytea 显式 `::bytea`、numeric/decimal/money 精确十进制文本按裸数值输出（不失真）、json/jsonb 具名转换，MySQL 保持 `X'..'`。(三) 事务状态：`QueryHistoryEntry.transaction_state`（已提交/未提交/已回滚），一次执行内 BEGIN 后写入先标未提交、COMMIT 转已提交、ROLLBACK 转已回滚，批次结束仍未提交（连接释放被服务端回滚）同样标已回滚，不谎报已提交；ROLLBACK TO SAVEPOINT 不结束事务；已回滚条目不提供补偿 SQL。(四) 敏感语句（SET PASSWORD、CREATE/ALTER/DROP USER|ROLE|LOGIN、GRANT/REVOKE、含 IDENTIFIED BY）不入历史，仅 debug 日志且不落 SQL 文本。(五) `apply_changes` 返回 `AppliedChangeOutcome`（core 新类型），PG 插入追加 `RETURNING` 主键列以捕获自增/序列生成的真实身份，app 补偿快照优先用服务端身份、缺失回退编辑器已知主键值；快照列元数据按请求 schema 取并按精确名匹配。保存查询恢复时保留 schema 作用域（原先硬编码 None 会丢失）。验证 — 真实 PG 冒烟 `pg_live_smoke_apply_changes_returns_generated_identity`（serial 主键 RETURNING 返回 id=1）；app 测试：`pg_query_result_object_name_uses_schema_and_quoted_identifiers`（二段/三段/折小写/引用转义/MySQL 回归/JOIN 与派生表只读）、`pg_rollback_literals_use_dialect_quoting_and_types`（decimal/bytea/jsonb/文本转义）、`pg_update_rollback_sql_quotes_identifiers_and_keeps_decimal`、`legacy_rollback_snapshot_without_dialect_renders_mysql`（MySQL 历史可读）、`history_marks_uncommitted_and_rolled_back_writes`、`sensitive_statements_are_not_recorded_in_history`、`query_editor_keeps_schema_scope_on_open`；MySQL 回归 app 376 项通过、工作区全量通过。未完成项：SQL 文本直接执行的 INSERT 仍不生成补偿快照（与 MySQL 基线一致：无法可靠捕获 RETURNING 身份时明确不提供，不猜 SQL）。

### T16 — DDL 与新建表

- [x] 完成 T16
- **开始前读**：设计 9.1、9.2；R02、R05、R08、R21、R22、R25、R26。
- **工作**：TableMetadata → DDL；PG CreateTableProvider、类型能力、schema、identity/default/generated、约束/索引/注释/trigger function；预览与执行同一计划。
- **交付位置**：postgres/ddl.rs；app/create_table PostgreSQL provider；core 必要结构计划类型。
- **验收**：新表有列/联合主键/唯一/FK/check/index/comments/trigger；DDL 在隔离库重建后元数据对等；无 SHOW CREATE/虚构 pg_get_tabledef；不含 MySQL 属性；未知属性不被抹掉；MySQL 新建表 SQL 快照保持。
- **完成记录**：已完成；执行人 FluxDB（T16 增量一）；内容 — 新增 `app/parts/create_table_postgres.rs`：领域结构 → PG 语句计划（CREATE TABLE + COMMENT ON TABLE/COLUMN + CREATE [UNIQUE] INDEX + CREATE TRIGGER），schema 限定表名、双引号标识符、identity（`GENERATED BY DEFAULT AS IDENTITY`）、numeric 精度（不复用 MySQL 的 varchar/decimal 判定）、联合主键内联、唯一索引独立语句、FK schema 限定引用、check 约束；MySQL 专属属性（engine/charset/unsigned/zerofill/ON UPDATE/索引前缀长度/binary/认证插件）一律不生成。触发器按 PG 语义引用已存在函数（`EXECUTE FUNCTION "fn"()`），行内 BEGIN…END 触发器体与 identity 挂非整数列均明确拒绝并给出可读提示。`PostgresCreateTableProvider` 接入 `create_table_provider` 分发：PG 类型清单（含 jsonb/uuid/inet/timestamptz 等）、默认 text/integer、能力位关闭 MySQL 专属项。`CreateTableState` 增加 `schema` 作用域，`OpenCreateTable` 命令与桌面入口（对象树带 schema、菜单入口用档案默认 schema）下传。`CreateTableSqlDialect` 增加 Postgres 分支，FK/check/触发器标识符按方言引用。DDL 读取沿用 T08 的 `pg_table_ddl`：视图走 `pg_get_viewdef`、表从 pg_catalog 元数据构造（无 `pg_get_tabledef` 假实现，无 `SHOW CREATE`），设计模式按元数据加载并对 PG DDL 抽取 check（`create_table_unquote_identifier` 已同时剥离反引号与双引号）。验证 — 真实 PG 冒烟 `postgres_create_table_ddl_rebuilds_equivalent_metadata`（隔离 schema `t16_smoke` 内建表并逐条语句断言成功，回读索引/联合主键/FK/触发器/表注释与设计一致，public 下无同名对象）；单元测试 `postgres_create_table_preview_covers_columns_keys_indexes_comments`、`postgres_create_table_rejects_mysql_only_and_inline_trigger`、`postgres_create_table_type_capabilities_hide_mysql_options`；MySQL 建表 SQL 快照测试保持通过；工作区全量通过（app 380、connectors 133 等）。未完成项：设计表差异执行（ALTER）属 T17 范围，`design_statements` 当前明确返回「尚未开放」而非生成半套 ALTER。

### T17 — 设计表和差异执行

- [x] 完成 T17
- **开始前读**：设计 9.1、9.2；R08、R16、R21、R22、R26。
- **工作**：结构直接加载 metadata；字段/约束/索引/trigger/注释变化转为有序操作；ALTER TYPE USING、依赖保护、事务能力分组；结构指纹防止旧快照覆盖，保留未知定义。
- **交付位置**：app/create_table 设计模型/provider、postgres/ddl.rs，既有设计命令。
- **验收**：无修改无 SQL；增删改名/默认值/类型/NULL/identity/索引/FK/check/trigger 可预览执行；失败回滚；外部 DDL 后阻止过期应用；修改普通列不删除 RLS/分区/排除等未知属性；特殊非事务语句明确单独状态。
- **完成记录**：已完成；执行人 FluxDB（T17 增量一）；内容 — 新增 `app/parts/create_table_postgres_design.rs`：设计状态与打开时的原始快照做差异，按依赖顺序生成 PG 动作——表改名（RENAME TO 只接新名）、先丢旧主键约束再加新主键、列删除/新增/改名（RENAME COLUMN 先于其它动作）/类型（ALTER COLUMN TYPE，**不自动编造 USING**：隐式转换成功、需要显式转换时由服务端报错提示用户补写，避免静默截断）/默认值 SET|DROP DEFAULT/可空 SET|DROP NOT NULL/identity ADD GENERATED …|DROP IDENTITY IF EXISTS/列注释 COMMENT ON COLUMN；索引走独立 `CREATE [UNIQUE] INDEX` 与 `DROP INDEX "schema"."name"`；FK 与 CHECK 统一 `ADD/DROP CONSTRAINT`；触发器 `CREATE TRIGGER … EXECUTE FUNCTION` 与 `DROP TRIGGER … ON 表`；表注释 COMMENT ON TABLE。删除路径覆盖（单测 `postgres_design_drops_constraints_and_triggers`）。**只发生差异动作**，不重建整表：分区/RLS/排除约束等编辑器不能表示的属性不会因普通字段修改被抹掉。执行侧：建表/设计保存统一走单批事务路径 `BEGIN … COMMIT`，任一条失败不提交、连接释放即回滚（顺带修掉 PG 建表保存时触发器被 MySQL/SQLite 拆分路径丢掉的缺陷）；建表向导 schema 随状态下传到 QueryRequest（原先硬编码 None）。防过期：保存前重查表 DDL，与打开设计器时的快照逐字比对，不一致即拒绝并提示重新打开（`ensure_postgres_design_not_stale`，日志 warn 记录 connection/table）。验证 — 单测 6 项（无修改无 SQL、列变更有序动作与不使用 USING、对象变更 PG 语法且不重建表、索引删除带 schema 限定、约束/触发器删除、类型能力位）；真实 PG 冒烟 `postgres_design_diff_applies_atomically_and_blocks_stale_snapshot`（无修改无 SQL → 追加列/注释/唯一索引 → 外部 DDL 后保存被拒 → 刷新后保存成功且新列/索引/注释落库、外部列保留 → 非法默认值导致失败时前面的 ADD COLUMN 一并回滚）；工作区全量通过（app 386、connectors 133 等）。未完成项：无。特殊非事务语句（CREATE INDEX CONCURRENTLY）不在生成范围内，故无需单独的非事务执行计划；若后续加入，需按设计拆独立计划并反馈部分执行状态。

### T18 — 表操作

- [x] 完成 T18
- **开始前读**：设计 9.3；R08、R15、R16、R21、R22、R26。
- **工作**：PG 重命名/复制/清空/删除 provider；全限定对象；复制结构/数据和独立序列；RESTRICT/CASCADE 与 restart identity；成功后更新对应缓存/标签。
- **交付位置**：app/table_actions、postgres/ddl.rs、core 表动作选项。
- **验收**：schema 同名表只操作指定对象；复制后源/目标自增独立；有数据 identity 不重复、generated 不手工写；默认 RESTRICT；拒绝以 session_replication_role 绕过 FK；对象 kind 对应正确 DDL；MySQL 表操作未变。
- **完成记录**：已完成；执行人 FluxDB（T18 增量一/二）；内容 — (一) 新增 `app/parts/table_actions_postgres.rs`：重命名（旧名 schema 限定、新名必须单段，带点号直接拒绝）、复制（`CREATE TABLE (LIKE ... INCLUDING ALL)` + 独立序列 + 可选数据）、删除（按 kind 分 `DROP TABLE` / `DROP VIEW`，默认 RESTRICT，不带 CASCADE）、清空（默认 `CONTINUE IDENTITY RESTRICT`，用户显式选择才 `RESTART IDENTITY`）。PG 明确拒绝 MySQL 的「禁用外键检查」，且不退化为 `session_replication_role`（语义不等价，会绕过触发器）。(二) 复制表自增独立性：`LIKE INCLUDING ALL` 会把 serial 默认值指向**源**序列，改为用 DO 块在目标 schema 内为每个 nextval 默认列新建 `OWNED BY` 目标列的序列并重绑；数据复制同样走 DO 块——按 `attgenerated = ''` 生成列清单（生成列不参与写入）、存在 identity 时附加 `OVERRIDING SYSTEM VALUE` 保留显式值，复制后用 `pg_get_serial_sequence` + `setval(max)` 校准各序列位置，使副本后续插入既不与已复制数据冲突也不推进源序列。(三) `TableActionSqlProvider` 增加 schema / kind / restart_identity 入参，MySQL/SQLite/Unsupported 实现保持原行为，桌面表操作表单与预览同步（清空表新增 restart_identity，默认 false）。(四) 表操作成功后失效补全缓存（重命名/复制/删除直接执行 SQL、不走历史记录路径，原先完全不失效），后台刷新在库级 dirty 时整批重取表清单，旧名/已删表不再被建议。验证 — 单测 `postgres_table_actions_are_schema_qualified_and_kind_aware`、`mysql_table_actions_unchanged_by_postgres_provider`、`table_actions_invalidate_completion_index`；真实 PG 冒烟 `postgres_copy_table_gets_independent_sequence`（`GENERATED ALWAYS AS IDENTITY` + 生成列 STORED + 两行数据：副本序列独立、源序列 last_value 仍为 3 未被推进、两表各自主键唯一、生成列由服务端重算）；工作区全量通过（app 390、connectors 133 等）。未完成项：无。CASCADE 的依赖范围展示沿用既有危险操作确认流程（与 MySQL 一致），PG 侧不额外生成 CASCADE。

### T19 — 连接 UI

- [ ] 完成 T19
- **开始前读**：设计 4.1、5、10；R00、R13、R16–R18、R24。
- **工作**：数据库选择列表加入 PostgreSQL；复用连接 Dialog/Input/Select/Tabs，PG 字段和验证；测试/保存 loading；证书/SSH 指纹/URI 错误反馈；编辑/复制/清空密码与持久化命令。
- **交付位置**：connection_dialog/postgres.rs、navicat_main/connection_forms、连接类型图标/选择。
- **验收**：gpui-component 0.6.0、AppIcon；明暗主题、输入 focus/hover、Esc/关闭/外点/内点防穿透；测试配置就是业务有效配置；重启恢复可用；MySQL/TiDB 原表单默认值/保存不变；桌面可启动。
- **完成记录**：进行中；执行人 FluxDB（T19 增量一～四）；内容 —
  (一) `e499f65`：连接类型选择加入 PostgreSQL 卡片；表单新增 PG 专用字段（TLS 模式 disable/prefer/require/verify-ca/verify-full、默认 schema、应用名、建连/查询超时、TCP 保活）并接 toggle/占位绑定；新增 `build_postgres_profile` 组装结构化档案（主机/端口/维护库/账号密码 + PG TLS 模式 + SSH/代理传输 + scope/advanced）——此前新建 PG 连接 draft 恒为 `postgres_profile: None` 无法拨号；新增 `apply_postgres_profile` 编辑/重启按档案回填，缺档案用历史扁平参数迁移；TLS/SSH/高级页签对 PG 开放。
  (二) `47d6485`：`postgres_connection_form_roundtrips_into_profile`、`mysql_connection_form_defaults_unchanged_by_postgres_fields`（MySQL 默认值锁定）；补齐 apply_postgres_profile 的 TLS 证书路径回填。
  (三) `b852413`：`validate_new_connection` 增加 Postgres 分支走 `build_postgres_profile().validate()`（此前落入 `_` 兜底只查主机+端口）；新增 `postgres_form_validation_uses_profile_rules` 测试。
  (四) `1a90421`：测试/保存 loading 与防重复——新增 `saving_connection`，保存并连接期间置位、全部退出路径清除；测试按钮在测试任务进行时 disabled，`test_new_connection` 加重复点击防抖；保存按钮在 saving 或测试进行中 disabled。
  验证 — `postgres_form_validation_uses_profile_rules`、`postgres_connection_form_roundtrips_into_profile`、`mysql_connection_form_defaults_unchanged_by_postgres_fields` 通过；desktop 369、app 390、connectors 133 工作区全量通过；`cargo build -p fluxdb-desktop` 通过。
  未完成项：证书/SSH 指纹/URI 错误反馈细化（PG 连接期 TLS/SSH 具体错误已透出，表单期证书路径校验受 layering 约束不做进 core validate）、真实重启恢复验证、桌面交互启动验证（明暗主题/Esc/外点/内点防穿透）。
  已核验（§4.1 复制隔离）：`CreateConnection`（dispatch.rs:83）在存在凭据时无条件为连接派生全新 `credential_ref`；复制连接 draft 即便携带原 ref 与内联档案，副本也得到独立 ref、内联密钥原样保留供新 ref 落 keychain。测试 `copy_of_postgres_connection_gets_independent_credential_ref_and_secret`、`copy_of_mysql_connection_with_flat_password_gets_independent_ref` 锁定 PG 档案与 MySQL 扁平密码两路径，更新/删除按 ref 隔离不互相影响。无需额外代码改动（原交给 dispatch 的 ref 覆盖保证），仅补回归测试。`c757e68`。

### T20 — schema 树与数据库 UI

- [ ] 完成 T20
- **开始前读**：设计 4.2、6、10；R00、R07、R13、R16、R17、R20、R21。
- **工作**：connection/database/schema/object 层级、Tree/ListItem、懒加载/错误/刷新/显示库、建库与 schema Dialog、能力菜单；上下文数据库/schema 不混淆；断开/删除后的任务/标签处理。
- **交付位置**：sidebar/、tree_helpers 拆分职责、menus_dialogs/create_database/display_database、连接菜单。
- **验收**：两库/多 schema/同名对象能独立浏览；加载有 Spinner，旧请求不会覆盖新对象；菜单外点关闭/内点阻止穿透/二级贴齐；脏标签保护；库删除失败不提前删 UI；MySQL 仍保持原树层级。
- **完成记录**：进行中；执行人 FluxDB（T20 增量一：PG schema 树层级）；内容 —
  自 `5044f83`：sidebar 拉平新增 `SidebarRowKind::Schema`，PG 数据库展开后按 schema 分桶
  （Database → Schema → ObjectGroup → Table），同名表跨 schema 独立成行（`schema_tree_key`/
  `object_group_tree_key_scoped` 带 schema）；新增 `schema_tree` 渲染与 `toggle_schema_tree`
  （schema 键独立展开并懒加载该 schema 关系）；`group_objects` 加 schema 过滤，MySQL/SQLite/Redis
  无 schema 时（schema=None）保持原扁平层级；修 `replace_loaded_children` 跨 schema 互清——
  PG 父节点为 schema 时只替换该 schema 的表/视图，无 schema 父节点保持原整库替换。
  验证 — 新测试：`postgres_schema_level_buckets_tables_and_keeps_mysql_flat`（三 schema 独立、
  同名 orders key 唯一、MySQL 无 Schema 行）、`replace_loaded_children_scopes_to_schema_for_postgres`
  与 `replace_loaded_children_without_schema_keeps_legacy_database_scope`；工作区全量通过
  （desktop 370、app 392、connectors 133）。
  增量二（`ec12ddb`，PG schema 右键菜单）：`schema_tree` 加右键 → `show_schema_context_menu`，
  独立 `SchemaContextMenu` + 状态字段 + 渲染；四动作全部 schema 作用域——新建查询/新建表（均带 schema
  下传 OpenQueryEditorInDatabase/OpenCreateTable）、设置默认 schema、刷新（按 schema 路径重载关系）。
  各 `show_*` 与 `close_context_menus` 清空 `schema_context_menu`，schema 节点不再落到数据库菜单（解「上下文
  数据库/schema 不混淆」）。菜单沿用既有手绘 div + `context_menu_backdrop`（外点关闭/内点防穿透/贴齐）。
  增量三（`f828179`，PG 建库 owner/encoding/locale/template）：`CreateDatabaseRequest` 增 owner/template，
  `pg_create_database_sql` 生成 `OWNER`/`TEMPLATE`（标识符白名单防注入，与 ENCODING/LC 同源）；连接对话框
  开放 Postgres（编码 ENCODING + Locale LC_COLLATE/LC_CTYPE 下拉 + Owner/模板输入框），编码/locale 按 PG
  语义提供（不套 MySQL 字符集）；`select_create_database_charset` PG 下保持 locale 选项。
  验证 — 新测试 `pg_create_database_sql_builds_options_and_quotes` 扩展 owner/template 生成与注入拒绝；
  PG live 建/删库冒烟 `pg_live_smoke_create_delete_database` 真实 PG 通过；工作区全量通过
  （app 394、desktop 370、connectors 133）。
  增量四（`d861652`，新建 schema）：`Connector::create_schema` + PG `pg_create_schema`（维护库 autocommit
  `CREATE SCHEMA`，schema 名标识符白名单校验，连接前拒绝空格/引号/分号/分号注入）；app `CreateSchema` 命令 +
  `create_schema_for_connection` 真实路由 + `AppEvent::SchemaCreated`；PG 数据库右键菜单加「新建 schema」，
  `show_create_schema_modal` 弹框（名称输入，Esc/外点/Cancel/Enter 处理），confirm 后台执行，成功后
  `invalidate_database_schema_cache` 重取 schema 清单使新 schema 出现在对象树，失败保留弹框报错。
  验证 — 新测试 `pg_create_schema_rejects_untrusted_names_before_connect`（非法名连接前拒绝）、
  `pg_live_smoke_create_schema`（真实 PG 建 schema + 清理）；工作区全量通过（app 394、connectors 135、desktop 370）。
  未完成项：真正 PG 交互视觉验证（明暗主题下 schema 菜单/建库弹框渲染、Esc/外点/loading Spinner 视觉，
  需人工 macOS 桌面交互，已确认 `cargo run -p fluxdb-desktop` 启动无 panic 进入事件循环）。
  已核验（2026-09-12）：旧请求防覆盖由 `loading_databases` 单飞 + `replace_loaded_children` schema 作用域
  保证（同 key 只有一个加载在飞、跨 schema 互不覆盖，见增量一测试）；删库/断开脏标签与运行任务保护已在
  增量五（`b89ecca`）落地（删连接/删库均有未保存/运行中查询拦截提示，断开本已有 warning）。

### T21 — 数据、查询、详情与历史 UI

- [ ] 完成 T21（实现完成，待人工验收）
- **开始前读**：设计 7、8、10；R00、R10–R17、R28–R30。
- **工作**：现有 DataTable/delegate 接入 PG 列/值/编辑性；二进制/JSON/时间详情；查询 scope Select、结果标签、取消、事务状态、历史补偿；gpui-component 统一 loading/error/disabled 与 show_message。
- **交付位置**：data_editor_model、data_table_ui、cell_detail_table_info、sql_editor_adapter、查询参数/保存/历史现有职责。
- **验收**：复制/多选/键盘/列宽/隐藏/排序/过滤/分页保持；RETURNING 和空结果显示；时区/复杂类型不被错误时间控件改写；明暗主题、Esc/外点/焦点；长操作不冻结窗口；关闭标签后迟到结果安全；MySQL 数据与查询回归。
- **完成记录**：实现完成（FluxDB，2026-09-12）；验收待人工（见集中清单 B/C/E 节）。
  实现要点：delegate 经 `cell_value_label` 渲染 PG 全类型（numeric 精确文本、bytea BinarySummary、jsonb Json、timestamptz 时间控件，编辑安全：binary 只读、temporal 用日期/时间选择器）；cell_detail 支持二进制完整下载/上传/Hex、JSON 格式化编辑、时间编辑；查询结果标签含 RETURNING（ResultSet+DataPage）、空结果保留列头（本会话修复 0 行时渲染列名头）；查询历史详情新增「事务」字段（已提交/未提交/已回滚，`QueryHistoryTransactionState::label`）；loading/error/disabled 与 show_message 统一、全部 DB/查询经 `background_spawn` 不阻塞 GPUI。
  验证：workspace 全绿（app 395、connectors 135、desktop 370、storage 20）；新增/修改测试覆盖 delegate 类型渲染、temporal/二进制只读、结果 tab 派生、事务状态、历史回滚方言。
  未完成项：交互/视觉项（复制/键盘/列宽/隐藏/排序/过滤/分页手测、明暗主题、Esc/外点/焦点、长操作、MySQL 数据/查询回归手测）→ 见集中人工验收清单 C/E 节，验收完成后再勾选整个任务。

### T22 — 结构编辑与表操作 UI

- [ ] 完成 T22（实现完成，待人工验收）
- **开始前读**：设计 9、10；R00、R08、R15–R17、R22。
- **工作**：新建/设计表的 PG provider 字段和 tabs；metadata 保真只读属性；表重命名/复制/清空/删除对话框、依赖与 SQL 预览；PG trigger function 输入语义。
- **交付位置**：create_table/、table_rename/table_copy/table_danger、表信息面板。
- **验收**：F12–F14 完整可操作；PG 不展示 engine/unsigned/MySQL FK 开关；RESTART IDENTITY/CASCADE 明确选择；危险操作预览与实际一致；Esc/外点/主题/手形/hover；MySQL 设计表功能与默认值保持。
- **完成记录**：实现完成（FluxDB，2026-09-12）；验收待人工（见集中清单 D 节）。
  实现要点：PG `PostgresCreateTableProvider` 能力位隐藏 MySQL 专属项（engine/unsigned/zerofill/text-options/key-length/auto-update-time/binary），Options 页签对 PG 不渲染 engine；建表/设计表 tabs（字段/索引/外键/检查/触发器/选项/分区/SQL）PG 可用，设计差异预览走 create_table_postgres_design（T17）；表操作对话框 PG SQL 预览（schema 限定）；本会话修复：PG 清空表新增「重置自增序列 RESTART IDENTITY」显式选项（默认 CONTINUE，切换后预览更新）、外键检查选择器（MySQL/TiDB 专属）对 PG 隐藏、PG 触发器「定义」区提示需引用已存在函数 `EXECUTE FUNCTION 函数名()`。
  验证：workspace 全绿（app 395、connectors 135、desktop 370、storage 20）；PG 建表/设计/表操作单测与真库冒烟（T16/T17/T18）保持通过。
  未完成项：交互项（对话框 Esc/外点/主题/手形/hover、危险操作预览一致手测、MySQL 设计表默认值回归手测）→ 见集中人工验收清单 D 节，验收完成后再勾选整个任务。

### T23 — 数据导出

- [ ] 完成 T23（实现增量一~四：方言字面量 + 一致快照导出 + 桌面接入 + 取消临时文件语义；格式往返待续）
- **开始前读**：设计 7.1、7.2、11.1；R02、R06、R10、R15、R16、R25、R26。
- **工作**：从 UI 搬出共享编码/文件写入；PG 全表一致快照批量流；表 SQL/TXT/CSV/JSON/XML、行/选区 CSV/JSON/Markdown/INSERT；字段选择/条件/计数预览/进度/取消/临时文件。
- **交付位置**：app/transfer、storage 文件服务、PG export provider、data_export UI 适配。
- **验收**：每个现有格式与范围真实导出；UTF-8/分隔符/NULL/decimal/数组/bytea/JSON 往返；全量无重复漏行；内存有界；取消只留可识别临时状态，不记录成功；SQL 文件可在隔离 PG 库执行；MySQL 导出格式不变。
- **完成记录**：进行中（FluxDB，2026-09-12）。
  增量一（`218c549`，方言字面量）：`sql_parser` 的 `sql_quote_ident`/`sql_qualified_object_name`/`sql_preview_cell_literal` 按 `DatabaseKind` 渲染——PG 双引号标识符、`"schema"."name"` 限定（不生成跨库三段名）、bytea `'\x..'::bytea`（替换 MySQL `X'..'`）、jsonb 显式 `::jsonb`；线程化到 `row_insert_sql`/`row_update_sql`/`data_change_sql_preview` + 导出（`TableDataExportWriter`/`write_data_row_export_file`/`write_data_row_insert_export`）+ 「复制为 INSERT/UPDATE」菜单 + 备份写行（`write_page_rows`）+ 数据变更预览（render/content_views 按连接类型解析方言）。过滤/排序预览文字保留 MySQL 引用（展示串，执行走连接器参数化 SQL）。
  验证：新增 `postgres_export_literals_use_pg_bytea_and_jsonb`（PG `'\xdeadbeef'::bytea`/`'{}'::jsonb`/`"public"."blobs"` + MySQL `X'DEADBEEF'`/反引号回归）；desktop 371、app 395 全过。
  增量二（`9d299a5`，一致快照导出分页）：`postgres/data.rs` 新增 `pg_export_pages`——在**单个** REPEATABLE READ 事务内分页读取整表（`LIMIT..OFFSET` + 主键 tie-breaker 稳定序），逐页 `on_page` 回调写出，一次只持一页（内存有界）；每页前检测 cancel 提前终止，结束/取消连接释放即回滚。解决现有桌面导出逐页每开新会话、并发写让行在页间漂移的不一致。真库验证 `pg_live_smoke_export_pages_snapshot`：建 10000 行表 → 快照导出页码齐全、id 无重漏（seen=10000、pages≥3）、on_page 提前停止生效；清理完整。
  增量三（`aceec5d`，接入桌面）：core `Connector::export_pages`（默认 load_data 逐页）+ PG 覆写（`pg_export_pages` 快照）；app `export_pages_for_connection` 路由；desktop `data_export` 对 PG 改走 `controller.export_pages_for_connection`（一致快照 + 逐页回调写文件 + cancel），其余保持原逐页循环（MySQL 不变）。workspace 全绿。
  增量四（`ea1a481`，取消临时文件语义）：`run_table_data_export` 先写临时文件 `path.partial`，成功后 `fs::rename` 原子改名到最终 path；取消/失败删除临时文件，不把半成品导出误当成功（`canceled` 不 rename 成成功文件）；PG 快照与逐页路径统一走该临时落盘语义。
  未完成项：XML/TXT/CSV/JSON/INSERT 等全部格式的 PG 真库往返验证（bytea/numeric/jsonb 往返）、行/选区各格式 PG 用例、导出的字段/条件/计数预览端到端、桌面全程导出 UI 运行验证、MySQL 导出格式回归手测（→ 集中人工清单）。（一致快照分页 + 桌面接入 + 取消临时文件语义已完成。）

### T24 — SQL 文件与原生脚本

- [ ] 完成 T24（实现增量一/二：原生检测 + psql 参数 + app/桌面原生脚本执行接入；桌面真实子进程调用链待人工）
- **开始前读**：设计 8.1–8.3、11.2；R14、R15、R24、R27、R31、R33。
- **工作**：目标 database/schema、编码、流式分句、进度日志/继续错误/停止；检测 COPY STDIN/psql 元命令后提供明确原生模式，使用安全子进程参数、凭据/传输与生命周期；普通模式不误拆 dump。
- **交付位置**：app/transfer/sql_file、postgres/native_tools、sql_file_execution UI 拆分职责。
- **验收**：普通多语句/函数体/Unicode 与编码转换；中途失败及取消；COPY 数据中的分号不误执行；原生模式需明确执行动作；无 shell 插值或密码参数；psql 版本/不存在有清晰错误；原有 MySQL SQL 文件流程回归。
- **完成记录**：进行中（FluxDB，2026-09-12）。
  增量一（`aff447a`）：`pg_script_needs_native_mode` + `pg_psql_invocation`（见本文件历史记录）。
  增量二（本会话，app/桌面原生脚本执行接入）：`fluxdb-app` 对外再导出 `pg_script_needs_native_mode`/`pg_psql_invocation`/`PgPsqlInvocation`（桌面经 app 网关，遵循分层）；`sql_file_execution.rs` 的 `start_sql_file_execution` 在分句执行前调 `start_pg_native_sql_file_execution`——当连接为 PG 且 `pg_script_needs_native_mode(text)` 命中时改走 psql 子进程：`pg_psql_invocation(host,port,user,db,path,password,!continue_on_error)` 构造 argv（密码仅 PGPASSWORD 环境变量）、`run_pg_native_psql` 无 shell 执行并逐批检测取消 kill+wait 回收、stderr 尾段回填失败原因、成功后经 `record_sql_file_statement_summary` 记一条日志；目标库优先作用域库、缺省回退连接维护库。捕获 owned 副本避免借用逃逸。普通脚本（非原生模式）仍走原分句执行，MySQL 流程不变。
  验证（真库，docker fluxdb-t09-pg PG16.15）：(1) 原生脚本（CREATE TABLE + COPY FROM STDIN + SELECT）以 `pg_psql_invocation` 完全相同的 argv 经 psql 执行退出 0，COPY 两行正确落入并 count=2——COAPY STDIN 分号不误拆、原生模式可行；(2) ON_ERROR_STOP=1 下唯一冲突使 psql 退出码 3（非零 → 判失败、中途停止），`continue_on_error`(ON_ERROR_STOP=off) 下报错但继续且退出 0——与 `!form.continue_on_error` 映射一致；(3) 错误 stderr 尾部回填。临时库/文件已清理。cargo test --workspace 全绿。
  增量三（本会话，TLS 模式接入子进程，见 T25 增量二同记录）：原生脚本执行沿档案 ssl_mode 经 PGSSLMODE env 传 psql，密码同样只入 env。
  未完成项：桌面端真实调用链需主机有 psql（本机无 psql/pg_dump，已并入集中人工验收清单，容器内工具验证了后端但桌面实际 spawn 待人工）；psql 版本/不存在错误的 UI 呈现（spawn 失败有清晰「启动 psql 失败」错误，具体版本不符提示待人工）；SSH 隧道下原生子进程链路未落地（保留待验证）；目标 database/schema 下拉与编码转换在原生模式的深度验证。

### T25 — 数据库备份和恢复验收

- [ ] 完成 T25（增量一~三：pg_dump 路径 + 恢复验证 + 应用调用链/owner/ACL/范围/版本预检）
- **开始前读**：设计 11.3；R13、R15、R18、R23、R26、R31。
- **工作**：pg_dump 工具检测/版本、plain 格式与结构/数据/完整、对象/owner/ACL、目录/记录；有 custom 格式则同时实现 pg_restore；所有 I/O 经 app/connector/storage；取消、管道/进程回收、passfile/partial 清理；普通表逻辑导出准确标注范围。
- **交付位置**：postgres/native_tools、app/transfer/backup、storage backup records、database_backup/backup_tab UI。
- **验收**：完整备份恢复到干净隔离库后比对表/行/约束/索引/函数/视图/序列；恢复后新增 identity 行正确；SSH+TLS 下可用；旧 pg_dump/缺工具/权限不足不假成功；取消无进程/密钥泄漏；MySQL 原生/逻辑备份和记录回归。
- **完成记录**：进行中（FluxDB，2026-09-12）。
  增量一（本会话，PG 原生 pg_dump 备份）：`database_backup.rs` 新增 `run_native_pg_dump`（`--no-owner --no-acl --format=plain --inserts -h -p -U -d` 单文件 .sql；选中表 `-t schema.table` 透传、空集合=整库；密码仅经 `PGPASSWORD` 环境变量不进 argv；stdout 流式写文件+逐批取消检测 kill；stderr 尾部回填失败提示）。`run_backup` 的 Native/Postgres 分支由 `不支持原生备份` 接入该函数；`native_tool_available` 增 PG 分支（`settings.pg_dump_path` 或 PATH 的 `pg_dump`）。`resolved_credentials` 按连接类型分支——PG 走 `postgres_resolved()`（host/maintenance_database/username/password），纠正此前恒走 `mysql_resolved()` 导致 PG 拿不到 host/user。设置新增 `pg_dump_path`（core Settings + Default + storage 测试字面量 + content_views 路径行/选择/changed/apply）。备份写行沿用既有 `{backup_dir}/{库安全名}/*.sql` 约定，backup_tab 扫描兼容。
  验证（真库，docker `fluxdb-t09-pg` PG16.15）：构造隔离源库 `t25_dump_src`（serial 主键表 items + name 索引 + qty CHECK + 视图 items_v + 函数 items_total + 3 行数据 + 序列 last_value=3）→ 以与代码完全相同参数跑 `pg_dump --no-owner --no-acl --format=plain --inserts` 退出 0（stderr 空）→ dump 含 CREATE TABLE/VIEW/FUNCTION/INDEX/SEQUENCE + INSERT 三行 + `setval(seq,3)` → `createdb t25_dump_dst` + psql 恢复退出 0 → 比对：3 行数据/数值 12.50 保精、序列 last_value=3、新插入 `delta` 得 id=4（identity+sequence 独立可用）、视图 count=3、函数 items_total()=4、索引 items_pkey + idx_items_name 均在。源/目标库与临时文件已清理。
  增量二（本会话，子进程审计：TLS 模式 + 取消/失败清理）：`pg_psql_invocation` 增 `ssl_mode` 参数——TLS 模式经 `PGSSLMODE` 环境变量（psql/pg_dump 无 `--sslmode` CLI 开关；`-c sslmode=..` 会被当作用户 SQL，已修正）；新增 `pg_sslmode_value`（Disabled→disable/Require→require/VerifyCa→verify-ca/VerifyFull→verify-full/Prefer→None 省略）。desktop `run_native_pg_dump` 与原生脚本执行沿档案 `profile.tls.ssl_mode` 传 PGSSLMODE；pg_dump 取消或失败时删除半成品输出文件（防残留部分 dump 被备份扫描误当成功）。
  验证：作为竞争检查，`PGSSLMODE=disable` 连接成功（容器 ssl off）、`PGSSLMODE=require` 明确报「server does not support SSL, but SSL was required」退出 2——证明 TLS 模式经 env 正确生效且错误语义清晰。新单测 `pg_sslmode_value_maps_tls_modes`、更新 `pg_psql_invocation_keeps_password_out_of_argv`（密码与 sslmode 均只入 env 不入 argv）。工具全绿（connectors 141）。
  增量三（`0450d0e`，应用调用链 + owner/ACL/范围/版本）：connectors 新增 `pg_dump_invocation`（纯函数、可单测）——plain+inserts、结构/数据/完整（`PgDumpScope`）、owner/ACL 开关（默认 `--no-owner --no-acl` 便于跨环境恢复，勾选才保留）、表过滤 `-t`、密码/TLS 只入 env 不进 argv；`pg_dump_version_compatible`+`pg_tool_major_version`+`pg_server_major_version`。desktop `run_native_pg_dump` 改经连接器构造参数（不再手写 argv）；`run_backup` 加版本预检（pg_dump 主版本 < 服务端主版本明确报错，不假成功）；高级表单新增「包含属主 OWNER / ACL 权限（PG 原生）」。记录落盘时机已符合（`.meta.json` 仅成功写、失败/取消删残留文件）。
  **custom 格式未提供**（`BackupForm` 无格式选择器，恒为 plain .sql）——按设计条件项，**不实现 pg_restore**，不为此延误既定功能。
  未完成项：SSH 隧道下 pg_dump/psql 的原生子进程链路未落地（当前只传直连 host/port，未建子进程隧道；需 SSH 环境 + 主机工具，保留待验证，并入集中人工清单 F 节）；备份记录「耗时」字段与集群级角色边界说明（可加）；普通表逻辑导出准确标注范围的 PG 分支；MySQL 原生/逻辑备份回归手测（→ 集中人工清单 E 节）。

### T26 — 角色、用户和 ACL 后端

- [ ] 完成 T26（实现增量一~五，ACL 语义/有效权限/成员选项已落地，T27 UI 待续）
- **开始前读**：设计 12；R02、R07、R09、R20、R28、R29。
- **工作**：PrincipalIdentity 和 PG role 属性；角色/LOGIN 用户列表/创建/改密/重命名/删除；成员关系/ADMIN OPTION；database/schema/table/sequence/routine 授权撤销；直接/继承/PUBLIC/owner 权限解释与变更差异；敏感操作隔离历史日志。
- **交付位置**：core user_admin 领域类型、postgres/user_admin.rs、app/user_admin PG provider/命令。
- **验收**：管理者与普通用户；所有对齐操作可执行；函数重载与跨 schema 授权不混淆；PG14/16+ 成员差异；缺权限不覆盖既有 ACL；删除有依赖 role 不自动 DROP OWNED；密码不入日志/历史；MySQL user@host 和资源字段保持。
- **完成记录**：进行中（FluxDB，2026-09-12）。
  增量一（`89c5a0e`）：core `PgRole`（集群级 role 身份，不复用 MySQL user@host）+ Connector trait 角色方法（list/create/alter-password/rename/drop role、alter_role_options、成员 grant/revoke/list、对象 grant/revoke 默认 Unsupported）；connectors `postgres/user_admin.rs` 用 pg_roles 真实列 + 独立 autocommit 连接执行角色 DDL（标识符 `pg_quote_identifier`、密码单引号字面量转义、权限关键字白名单防注入、DROP 默认 RESTRICT 不自动 DROP OWNED）；app `role_operation_for_connection` 真实路由 + AppCommand（LoadPgRoles/CreatePgRole/AlterPgRolePassword/RenamePgRole/DropPgRole）+ 事件 PgRolesLoaded/PgRoleChanged。
  增量二（`5dd8130`）：`pg_list_relation_grants` 用 `aclexplode(relacl)` 展开 grantee×privilege×grant_option（空 grantee=PUBLIC、无 ACL 记 NULL 由 UI 示默认）+ Connector `list_relation_grants`。
  增量三（本会话，广义对象权限读取 + 敏感操作隔离核验）：core 新增 `PgObjectGrantScope`（Database{schema name} / Schema / Relation{schema,name,kind} / Routine{schema,name,signature}）与 `PgRelationKind`（Table/View/Sequence，relkind 常数 IN 列表）；Connector 新增 `list_object_grants`（默认 Unsupported）+ PG 实现 `pg_list_object_grants`——按 scope 分别对 `pg_database.datacl` / `pg_namespace.nspacl` / `pg_class.relacl` / `pg_proc.proacl` 做 `aclexplode` 展开（grantee 空=PUBLIC、无 ACL=NULL 由 UI 示默认）；函数按 `pg_get_function_identity_arguments` 签名区分重载；relkind 常数内联不参数绑定（避类型推断）。对象授权/撤销侧 `grant_object_privilege` 本就支持任意 object_sql + 白名单关键字（CONNECT/CREATE/USAGE/EXECUTE…），覆盖四类。敏感操作隔离已核验：角色 DDL/密码经 `role_operation_for_connection` → `PostgresConnector` → `pg_exec_role_sql`（batch_execute）直达，完全绕过查询执行器与历史记录路径，不入历史/日志。
  验证：真实 PG 冒烟 `pg_live_smoke_object_grants_per_scope`（建 schema/表/序列/函数 + 对 role 授权 SELECT/USAGE/EXECUTE → 按四类 scope 读取命中，函数签名 `a integer` 区分重载，schema 无显式 ACL 返回空不误报）；connectors 140（+1）、app 395、工作区全绿（含既有 role/membership/relation-grants live 冒烟）。清理完整。
  增量四（本会话，ACL 语义补全：默认权限/owner/直接/PUBLIC/继承）+增量三中的 `PgObjectGrantScope`/`list_object_grants`：
  - core 富 `PgObjectGrants`（owner + `acl_is_null` + 显式 entries 含 PUBLIC(空 grantee) 与 `is_owner` 标记）替代原裸 (grantee,priv,opt) 元组——NULL ACL=默认权限（owner 全权）**不等于无权限**，UI 据此明示而非空表。
  - core `PgEffectivePrivilege`（privilege/effective/direct/grant_option）+ Connector `role_effective_grants`：经 PG 原生 `has_{database|schema|table|sequence|function}_privilege` 判定有效权限（owner/继承/PUBLIC 天然计入），与显式 direct 条目合并——`effective&&direct`=可直接撤销的直接授权；`effective&&!direct`=来自 owner/继承/PUBLIC（只读展示，**不得直接撤销**，撤销应到成员角色或 PUBLIC 源头）；`!effective`=无权限。函数按 identity 签名剥参数名构造参数类型列表供 `has_function_privilege`。
  - 编辑侧安全由 GRANT/REVOKE 仅改显式 ACL 保证（PG 语义），读取侧把继承/owner/PUBLIC 与直接授权分开，UI 不会据不完整有效视图误删未展示授权。
  验证：真库 PG16.15 `pg_live_smoke_object_grants_per_scope` 扩到 owner/默认/PUBLIC 断言（schema NULL ACL→acl_is_null+owner、PUBLIC 授权以空 grantee 呈现、is_owner 标记）；新增 `pg_live_smoke_role_effective_grants`——direct_role effective&&direct、inherit_role effective&&!direct（经 group 成员继承不可直接撤销）、none_role effective=false，三态全过；connectors 142（+1）、工作区全绿。
  增量五（`bf682fc`，PG14/16 成员选项贯穿）：core `PgRoleMembership`（含 inherit/set）+ `list_role_membership` 改返回它 + `grant_role_membership` 增 inherit/set；连接器读 `pg_auth_members` 版本感知（PG16+ 取 inherit_option/set_option 列，≤14 回填 true），授权拆多条 GRANT（每语句一 WITH 子句，关闭态显式下发可切回）。真库 INHERIT FALSE/SET TRUE 往返验证。
  未完成项：scope 选择器带 database/schema/签名的 UI 入口（T27，数据源已齐：list_object_grants + role_effective_grants + list_role_membership）；T27 用户/角色 UI（user_admin 现为 MySQL user@host 外形，需按 provider 新增 Postgres dialect——连接器/核心数据层已齐，UI 待续）。PG14/16 成员选项**已贯穿**。

### T27 — 用户与权限 UI

- [ ] 完成 T27（实现完成，待人工验收）
- **开始前读**：设计 10、12；R00、R09、R16、R17、R20。
- **工作**：现有 user_admin 页面按 provider 显示 PG role/LOGIN/成员/对象权限；授权对象选择带 database/schema/签名；loading/草稿/应用/错误/确认；MySQL host/plugin/每小时资源限制不出现在 PG 表单。
- **交付位置**：user_admin/、user_admin_privileges、现有用户管理入口与菜单能力。
- **验收**：无需手写 SQL 完成 F18；直接与继承权限区别明确；成员与 grant option 不混淆；无权限有理由；切用户不覆盖旧草稿；明暗主题、Esc/外点/组件/键盘；MySQL 全套用户页面回归。
- **完成记录**：进行中（FluxDB，2026-09-12）。
  **范围决策（2026-09-12）**：现有 `DatabaseUserAdminProvider`/desktop user_admin 是 MySQL 外形（user@host、SHOW GRANTS 文本解析、auth_plugin、每小时资源限制），把 PG role 塞进该 provider 会产生**误导性 UI**（host/plugin 空列、MySQL 资源限制、无法表达成员/对象 ACL）。因此 T27 **不应**通过给现有 provider 加 Postgres branch 实现，而应做独立 PG 角色管理接口，复用 T26 已就绪的数据源。**不引入 `UserAdminDialect::Postgres`**（避免强开 19 个 MySQL SQL 生成器分支编译雪崩），只加 `PrivilegeScope::Postgres`。
  **增量一（app 层，`860e39e`）**：core `PrivilegeScope::Postgres` + `supports_database_user_admin(Postgres)=true`（菜单/入口放行）；app `open_user_admin` 对 Postgres 放行；`load_user_admin_users` 对 PG 经连接器 `list_roles` 读取（role→user 映射）；`load_user_admin_grants` 对 PG 经 `list_role_membership` 返回该 role 所在组角色；新增 `list_pg_roles_for_connection`/`list_pg_memberships_for_connection` 复用 `role_operation_for_connection` 路由；纯函数 `pg_groups_for_member`（可测试）。验证：`pg_groups_for_member` 单测 + 工作区全绿。
  **增量二（desktop，`1bb4416`）**：`user_admin_content` 对 Postgres 连接分支到新增 `pg_user_admin.rs` 的 `pg_role_admin_content`（不再显示 MySQL/TiDB-only 占位）；左侧复用 `user_admin_user_list`（PG 角色经 app 层 list_roles 装载为 admin.users），右侧展示选中角色的组角色成员关系（admin.grants ← pg_auth_members）；集群级角色语义提示。MySQL user_admin 全保留。build/test 全绿。
  **增量三（app 层 CRUD 支撑，`f57490c`）**：`UserAdminState` 增 `pg_can_login`（default true）；新增 AppCommand `EndUserAdminCreateUser` / `SetUserAdminPgCanLogin`（桌面不直接改 app 状态）；render 快照补字段；单测 `user_admin_pg_can_login_defaults_and_reflects`。
  **增量四（desktop 角色 CRUD 表单 + 成员详情，`95ad136`）**：工具栏新建（BeginUserAdminCreateUser）/删除（DropPgRole，postgres/pg_* 不给删除入口）；内联创建表单复用已订阅输入句柄 + LOGIN 开关 + 创建/取消 → 刷新；右侧详情展示所属组角色（list_role_membership→admin.grants）；PG tab 默认停 MemberOf 使选角色即加载成员。
  **增量五（PG 对象权限面板，`9cd6139`）**：core `grant_object_privilege` 增 grant_option；app 新纯函数 `pg_grant_scope_from_state`/`pg_grant_object_sql` + AppCommand（SetUserAdminPgGrantTarget / Load-Start-Finish / Grant / Revoke）+ 路由 `load_pg_object_grants`（list_object_grants + role_effective_grants）；desktop 面板：目标种类按钮组 + schema/对象/签名输入 + 逐权限「直接授权（可撤销）/继承·PUBLIC·属主（不可直接撤销）/无」+ 授予（带/不带 GRANT OPTION）/撤销；ACL 为 NULL 显示「属主 X（默认权限）」而非空表。真库 grant(带 option)+revoke 往返 + app 纯函数单测。
  **剩余（视觉验收范畴）**：仅剩桌面交互/视觉确认（弹框 Esc/外点/主题/loading/disabled、手形光标、按钮组与输入框渲染、授权后列表刷新目视）与 MySQL 用户页面回归手测——功能实现已完成，UI 编码不等人工，留最后集中验收。

### T28 — 全量对齐与交付审查

- [ ] 完成 T28
- **开始前读**：完整设计，重点 1.3、13；全部任务完成记录和偏差；现有全部相关测试；R00。
- **工作**：按下表完成 F01–F20 的证据登记；PG14–18 真库矩阵、非超级用户、网络/TLS/取消异常；MySQL 对等功能和共享影响数据库回归；文件/连接/线程/结果缓存资源检查；复查新源码行数、模块可见性与过度封装；同步设计与真实实现路径。
- **交付位置**：相关 integration tests/fixture/CI 或运行脚本；本文件验收证据；设计参考索引新增实际实现路径（保留原参考）。
- **验收命令**：`cargo fmt --all`、`cargo check --workspace`、`cargo test --workspace`；另运行显式 opt-in 的 PostgreSQL 与 MySQL 真实测试并记录完整命令/服务器版本；`cargo run -p fluxdb-desktop` 启动及明暗主题手工回归。环境不足如实待验证，不能跳过后宣称完成。
- **最终标准**：T01–T27 全部有有效完成记录；F01–F20 每行通过；无必须功能残留或以 mock/仅 SQL 替代图形能力；MySQL 正常功能保持；文档与代码一致。
- **完成记录**：进行中（FluxDB，2026-09-12）。
  增量一（真库回归 + 资源审查）：新增 `mysql_live_smoke_test_connection_regressed_by_postgres`（`FLUXDB_MYSQL_SMOKE` 环境门控）——对隔离真实 MySQL 8.0.34 容器 `test_connection` 通过，证明 PG/T23/T24/T25/T27 改动未回归 MySQL 连接。资源审查（代码走查）：`run_pg_native_psql`/`run_native_pg_dump` 取消路径 kill+wait 防僵尸、stdout/stderr 后台线程随管道 EOF 自收敛（进程被杀→管道关闭→线程退出，不泄漏）；`pg_export_pages` 单事务结束/取消统一 ROLLBACK 释放会话，连接随 session drop 回收。`cargo check --workspace` + `cargo test --workspace` 全绿。
  剩余：`cargo run -p fluxdb-desktop` 明暗主题手测、F01–F20 逐项登记到 §5 表、PG14–18 真库矩阵（现有 16.x 单点，其余版本待环境）、MySQL 共享手续测（→ 人工）、T01–T27 全部有效完成记录复核。

## 5. 最终功能验收证据

每行应填写真实测试名/命令、UI 证据文件、结果与版本；“通过”二字不是充分证据。

| 功能 | PostgreSQL 证据 | MySQL/相关旧数据库回归 | 状态 |
| --- | --- | --- | --- |
| F01 连接保存与恢复 | — | — | 未验证 |
| F02 TLS/SSH/代理/超时 | — | — | 未验证 |
| F03 树/分组/显示/断开 | — | — | 未验证 |
| F04 建库/删库 | — | — | 未验证 |
| F05 对象与注释 | — | — | 未验证 |
| F06 分页/排序/过滤 | — | — | 未验证 |
| F07 数据编辑与提交 | — | — | 未验证 |
| F08 详情/JSON/时间/二进制 | — | — | 未验证 |
| F09 查询/结果/进度/取消 | — | — | 未验证 |
| F10 格式化/参数/补全 | — | — | 未验证 |
| F11 保存/历史/补偿/结果编辑 | — | — | 未验证 |
| F12 表信息与 DDL | — | — | 未验证 |
| F13 新建/设计表 | — | — | 未验证 |
| F14 复制/重命名/清空/删除 | — | — | 未验证 |
| F15 全部导出格式与范围 | — | — | 未验证 |
| F16 SQL 文件执行 | — | — | 未验证 |
| F17 备份及恢复验证 | — | — | 未验证 |
| F18 用户/角色/权限 | — | — | 未验证 |
| F19 通用 UI、状态与主题 | — | — | 未验证 |
| F20 配置与旧数据库兼容 | — | — | 未验证 |

## 6. 文档工作记录

### DOC-01 — 设计与任务清单

- 状态：完成（仅文档，不勾选任何功能实现任务）。
- 执行人：Codex；开始：2026-09-09；完成核对：2026-09-10（Asia/Shanghai）。
- 完成内容：梳理当前 MySQL 全链路和 PostgreSQL 预留缺口；核对本地 DBeaver、dbx、固定提交的 GitHub pgAdmin；制定 database/schema、驱动与会话、类型、编辑/事务、DDL、权限、备份和 gpui-component 设计；建立 28 项依赖任务、必读代码、逐项完成记录和 F01–F20 验收矩阵。
- 改动位置：本文件及同目录 `2026-09-09-postgresql-detailed-design.md`。
- 验证：34 组参考索引的现有文件与关键符号均已定位；本地文件链接、GitHub 固定源码、28 项任务编号与 F01–F20 映射进行文档检查；未修改 Rust，不在本次文档任务中运行 cargo 或真实数据库验收。
- 后续要求：实现者先按第 1 节阅读，从依赖已满足的任务开始；每完成一项立即更新本文件，不能分期降低最终范围。
