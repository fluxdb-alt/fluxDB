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
| T01 | 按职责机械拆分需扩展的大文件、冻结 MySQL 基线 | 无 | 未开始 / — |
| T02 | PostgreSQL 核心类型、配置和凭据持久化 | T01 | 未开始 / — |
| T03 | database/schema 身份、查询上下文、缓存与历史迁移 | T02 | 未开始 / — |
| T04 | 驱动、runtime、会话和唯一拨号入口 | T02、T03 | 未开始 / — |
| T05 | TLS、SSH、代理、超时、资源清理 | T04 | 未开始 / — |
| T06 | database/schema/对象浏览与真实路由 | T03、T05 | 未开始 / — |
| T07 | 创建/删除数据库与 schema 操作 | T06 | 未开始 / — |
| T08 | 列、索引、约束、触发器和类型元数据 | T06 | 未开始 / — |
| T09 | PostgreSQL 值转换、参数编码和 bytea | T08 | 未开始 / — |
| T10 | 数据分页、排序、筛选与预览 | T09 | 未开始 / — |
| T11 | 数据编辑、可靠定位、原子提交和冲突处理 | T10 | 未开始 / — |
| T12 | 统一 PG 方言、分句和参数解析 | T03 | 未开始 / — |
| T13 | SQL 执行、多结果、会话事务、进度和取消 | T05、T09、T12 | 未开始 / — |
| T14 | PostgreSQL 补全、元数据索引和语义提示 | T08、T12、T13 | 未开始 / — |
| T15 | 查询结果编辑、保存查询和历史补偿 | T11、T13、T14 | 未开始 / — |
| T16 | DDL 读取和 PostgreSQL 新建表 provider | T08、T12、T13 | 未开始 / — |
| T17 | 设计表差异计划和结构修改执行 | T16 | 未开始 / — |
| T18 | 复制/重命名/清空/删除表 | T11、T16、T17 | 未开始 / — |
| T19 | PostgreSQL 连接对话框 | T02、T05 | 未开始 / — |
| T20 | schema 树、数据库对话框和能力路由 | T06、T07、T19 | 未开始 / — |
| T21 | 数据/查询/详情与历史 UI 接入 | T10–T16、T20 | 未开始 / — |
| T22 | 新建/设计表及危险操作 UI | T17、T18、T21 | 未开始 / — |
| T23 | 所有现有数据导出格式与范围 | T10、T11、T13、T21 | 未开始 / — |
| T24 | SQL 文件执行与 PostgreSQL 原生脚本路径 | T12、T13、T20、T21 | 未开始 / — |
| T25 | 数据库备份、原生工具、记录和恢复验证 | T05、T16、T20、T23、T24 | 未开始 / — |
| T26 | PostgreSQL 用户/角色/ACL provider 与命令 | T03、T05、T08、T13 | 未开始 / — |
| T27 | 用户/角色/权限 UI 与完整交互 | T20、T26 | 未开始 / — |
| T28 | 全矩阵联调、MySQL 回归与交付审查 | T01–T27 | 未开始 / — |

## 4. 可执行任务

### T01 — 机械拆分与 MySQL 基线

- [ ] 完成 T01
- **开始前读**：设计 1、3.1、3.4、13；R00、R03、R06–R09、R13–R16；现有 app/connector/UI 测试入口。
- **工作**：记录当前 MySQL F01–F20 的实际入口与可运行状态；拆出 state 的建表/表操作/查询状态、真实 connector 路由、将扩展的 dispatch 分支、连接表单及后台文件执行职责。超 1200 行且需加功能的文件先拆对应职责，不整体迁移无关 Redis 功能。保持旧 include 边界可用，PG 新目录用真实 mod。
- **交付位置**：设计 3.4 对应 app/core/UI 目录；mysql/shared 辅助的最小职责迁移；原入口只做模块声明和 glue。不添加 PostgreSQL 行为到纯移动提交。
- **验收**：格式化和 workspace check；受影响已有测试通过，MySQL SQL 预览/路由/配置结构无行为变化；记录移动前后文件与符号对应关系。
- **完成记录**：进行中（未勾选）；执行人 Claude Code。
- **开始**：2026-09-10（Asia/Shanghai）。
- **完成内容（已做）**：① 冻结 MySQL 基线——`cargo check --workspace` 干净；修复 1 处预置失效断言（`mock_objects` 现返回 4 张联合表，`tests.rs:list_objects_returns_mock_database_then_tables` 表断言 2→4）；全 workspace 1133 tests 通过。② 按职责拆 state——`state.rs` 5291 行先拆建表域，再细拆 6 职责文件（model/state/metadata/sql/actions/design_statements，均 <1200 行）；`state.rs` 降至 1921 行（含 AppCommand/AppEvent/AppController）。③ 将详细设计、任务清单、AGENTS.md 纳入版本控制（`docs/design` 被忽略）。
- **改动位置（纯移动）**：`crates/fluxdb-connectors/src/parts/tests.rs`（断言 2→4，对齐 mock 数据）；`crates/fluxdb-app/src/parts/state.rs` → `create_table_model.rs`/`create_table_state.rs`/`create_table_metadata.rs`/`create_table_sql.rs`/`create_table_actions.rs`/`create_table_design_statements.rs`；lib.rs include 相应调整。
- **必读确认**：已读 AGENTS.md、设计 1/3.1/3.4/1.3、R01–R09 对应本地文件（F01–F20 入口映射见下方）。
- **验证**：`cargo fmt --all`、`cargo check --workspace`、`cargo test --workspace` 全绿（app 357 / connectors 93 等）；提交 `01ee37a`、`1c8f793`。
- **MySQL 回归**：建表/设计表相关 357 app tests 通过；后续拆分每步后重跑证明行为不变。
- **偏差/剩余**：验收不满足故不勾选——
  - dispatch.rs（3724 行）巨型 match 未按域提取（open-create-table/table-action 块 65 arm，含跨行 struct 头与 34 处内部 `return`，纯手写路由 arm 风险高；选定延后到新增 PG 命令时一并路由，避免纯移动阶段引入行为风险）。
  - shared.rs（1837 行）未拆：MySQL/SQLite 专属 helper 保留，真正可复用分页/校验/结果 helper 将迁 design 3.4 `relational/mod.rs`。
  - desktop 大文件 connection_dialog.rs 3656 / tree_helpers.rs 2432 / app_boot.rs 2641 未拆；`AGENTS.md` 引用 `docs/2026-09-04-gpui-component-ui-migration.md` 当前缺失。
  - 未做真实 MySQL 全量验收（仅冻结测试基线）。

#### T01 附加：MySQL F01–F20 当前入口映射（`/crates` 路径）

> 基线为设计 1.3 各行；“入口”为当前 MySQL 功能实际落点与关键符号（本次只做记录，未做真库验收）。

| 编号 | MySQL 功能 | 当前入口（文件 + 关键符号） |
| --- | --- | --- |
| F01 | 建/编/复制连接、测试、保存、重启恢复 | `core/parts/connection.rs` ConnectionConfig/ConnectionDraft；`core/parts/mysql_profile.rs` MySqlConnectionProfile；`app/parts/mock_data.rs:101` test_connection、`app/parts/state.rs` AppCommand::CreateConnection/UpdateConnection/TestConnection；`storage/lib.rs:49` FileStorage |
| F02 | TLS、SSH、代理、超时 | `connectors/parts/mysql/connector.rs`（dial）、`mysql/connection_url.rs`；`redis/ssh_tunnel.rs`（SSH 桥，R18）；配置见 mysql_profile.rs |
| F03 | 分组/排序/显示库/展开/刷新/断开/删除 | `app/parts/state.rs` OpenConnection/DisconnectConnection/RefreshObject/OpenObjectList/DeleteConnection；`mysql/connector.rs:71` list_objects |
| F04 | 建/删库 | `mysql/connector.rs:82,93` create_database/delete_database；`shared.rs:537,567` mysql_create/delete_database_sql；`app/mock_data.rs:132,155` *_for_connection |
| F05 | 对象列表、表/视图、列、注释 | `mysql/connector.rs` list_objects/table_ddl；`mysql/metadata.rs`；`app/mock_data.rs:110,691` list_objects/load_table_info_for_connection |
| F06 | 分页/多列排序/过滤/搜索/字段隐藏 | `mysql/connector.rs:104` load_data；`shared.rs` data_order_by_clause/push_data_filter_clause/data_export_preview_sql；`app/mock_data.rs:529` load_data_for_connection |
| F07 | 增/复制/改/删行、批量提交、撤销 | `mysql/connector.rs:139` apply_changes；`mysql/apply_changes.rs`；`shared.rs` validate_data_changes/non_null_insert_values/push_mysql_bind/push_identity_where；`app/data_editor.rs` edit_data_cell/insert_data_row/apply_data_editor_edit |
| F08 | 单元格详情、JSON、时间、二进制 | `mysql/connector.rs:150` load_cell_binary；`shared.rs` mysql_binary_summary/binary_summary；`app/data_editor.rs:98` binary_preview；UI main_parts/data_editor_model、json_editor |
| F09 | 全部/当前/选区执行、结果标签、多语句、进度/停止 | `mysql/connector.rs:166,184` execute/_with_progress；`shared.rs` mysql_execute_query/_with_progress、split_sql_statements/query_statements_for_execution；`app/parts/controller/dispatch.rs` ExecuteQuery/ExecuteQueryText |
| F10 | SQL 格式化、参数输入、补全、文档/语义提示 | `app/sql_format.rs`；editor-core SqlDialect::Postgres（R14 未接通宿主映射）；`app/completion_index.rs`、`app/query_completion.rs`、`app/parts/controller/query_completion.rs`、`mysql/completion.rs` |
| F11 | 保存查询、历史、补偿、结果集编辑 | `app/query_history.rs`、`app/query_result_edit.rs`；`storage/lib.rs` QueryHistoryRecord、FileStorage::save/load_query_history |
| F12 | 表信息：列、索引、FK、触发器、DDL | `mysql/connector.rs:341,349,369,376` list_indexes/list_foreign_keys/list_triggers/table_ddl；`mysql/metadata.rs`；`app/table_info.rs`、`app/mock_data.rs:691` |
| F13 | 新建/设计表、字段/索引/FK/check/触发器/选项 | `app/parts/create_table_*.rs`（本次拆分）；`app/create_table_provider.rs`、`create_table_foreign_keys.rs`；dispatch.rs OpenCreateTable/OpenDesignTable/ApplyCreateTable 等 65 arm；UI main_parts/create_table |
| F14 | 复制/重命名/删除/清空表 | `app/parts/create_table_actions.rs` MySqlTableActionSqlProvider/rename/copy/drop/truncate_table_sql_preview；dispatch.rs Rename/Copy/Drop/TruncateTable |
| F15 | 导出 SQL/TXT/CSV/JSON/XML、行/选区 CSV/JSON/MD/INSERT | `app/mock_data.rs:614` preview_data_export_for_connection；UI main_parts/menus_dialogs/data_export.rs（含文件 I/O 与 SQL 生成，R15 需迁移） |
| F16 | SQL 文件编码/目标库/拆分/继续错误/日志/取消 | `shared.rs` split_sql_statements/query_statements_for_execution；UI main_parts/menus_dialogs/sql_file_execution.rs |
| F17 | 数据库备份、结构/数据、原生/逻辑、记录、取消 | UI main_parts/menus_dialogs/database_backup.rs、database_backup/ui.rs、backup_tab.rs |
| F18 | 用户/角色、密码、授权撤销、成员、权限列表 | `core/parts/user_admin.rs`；`app/parts/user_admin.rs`（impl AppController）；dispatch.rs LoadUserAdmin* arm；UI main_parts/user_admin.rs |
| F19 | 标签/脏状态/关闭保护、全局反馈、明暗主题 | `app/parts/state.rs` CloseTab/CloseTabs/ConfirmCloseDirtyTab、TaskState、AppEvent；UI main_parts/（gpui-component） |
| F20 | 旧数据库路由及配置兼容 | `DatabaseKind` 枚举（core，暂无 Postgres）；`app/mock_data.rs` 真实路由集；`storage/lib.rs` strip_plaintext_secrets/profile_secret_slots |

- **提交或差异**：`01ee37a`、`1c8f793`；分支 `pg`。未运行真实 MySQL 验收。

### T02 — 配置、核心类型与凭据

- [ ] 完成 T02
- **开始前读**：设计 4.1；R01、R02、R13、R18、R24；storage 的 secret slots 和旧配置测试。
- **工作**：新增 DatabaseKind::Postgres、PostgresConnectionProfile；贯穿 Config/Draft、序列化、默认端口/维护库、URI 解析和所有构造点。复用 SecretRef；必要时机械移动公共类型。接入 PG 各凭据 slots、清空/替换/复制/删除的所有权语义。
- **交付位置**：core/parts/postgres_profile.rs；connection.rs 兼容字段；storage 新职责文件；相关 fixture。
- **验收**：旧 MySQL/TiDB/SQLite/Redis 配置无新字段也能加载；旧枚举序列化不变；PG 保存/恢复含特殊字符；敏感值不出现在配置文件、URI、Debug/日志；Keychain 失败不能报告成功；复制不共享可误删凭据。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T03 — 对象身份与作用域贯穿

- [ ] 完成 T03
- **开始前读**：设计 4.2、4.3、8.4；R01、R02、R07、R10–R13、R16、R20、R25。
- **工作**：定义完整 database/schema/object identity、查询 session id/config generation；补齐 QueryRequest、编辑器、SavedQuery、历史与补偿快照、tab/tree/cache/layout keys。批量 CompletionColumn 加所属范围，routine 用签名区分重载；缓存版本升级和旧记录默认值迁移。
- **交付位置**：core/parts/sql_context.rs、object_query.rs；app 状态/补全/历史；storage 序列化；UI 只保存稳定 ID。
- **验收**：两库两 schema 的同名表分别打开/保存/恢复；点号、空格、Unicode、双引号、大小写不冲突；PG 两段名是 schema.table，MySQL 仍是 database.table；旧查询历史可读，旧补全缓存重建。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T04 — 驱动、runtime 与会话

- [ ] 完成 T04
- **开始前读**：设计 3.1–3.3、5；R03–R07、R24、R27、R32、R33。
- **工作**：锁定 tokio-postgres/tokio-postgres-rustls 所需 features，复用 rustls ring；建立 PgRuntime、连接服务和 PostgresConnector；后台同步桥；session 独占、连接 future 持续驱动、受限元数据并发；测试和业务同一 PgDialer。AppCommand 先 loading 后后台执行，前台仅合并结果。
- **交付位置**：connectors Cargo.toml/lock、postgres/mod.rs/connector.rs/connection.rs；app/parts/connections 和命令结果适配。
- **验收**：真实 PG 连接/认证/版本读取；两个查询会话互不串事务；请求不重复创建 runtime；连接错误可回传；GPUI 线程无 block_on/网络；MySQL 仍使用原 SQLx；依赖版本、MSRV、构建影响有记录。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T05 — 传输、安全策略与生命周期

- [ ] 完成 T05
- **开始前读**：设计 3.3、5、11.3；R04、R18、R23、R24、R28、R32、R33。
- **工作**：TLS 模式、CA/mTLS、远端 server_name；SSH 密码/私钥/known_hosts；SOCKS5/HTTP CONNECT 与超时/keepalive。提取 SSH 桥并处理当前单 accept 限制，旧调用保留兼容包装；查询、取消、原生进程都走完整传输。断开/改配置/关闭释放所有资源。
- **交付位置**：transport/ssh.rs、postgres/connection.rs，core 传输策略，app 资源清理与未知 hostkey 事件。
- **验收**：直连/TLS/SSH+TLS/代理成功；错误 CA/主机名/hostkey 拒绝；取消通道可拨号；没有隧道 drop 过早或绕过直连；超时覆盖整体握手；重复连接/取消后线程/连接数稳定；旧 MySQL/Redis SSH 路径回归。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T06 — 对象树与真实路由

- [ ] 完成 T06
- **开始前读**：设计 1.2、4.2、6.1；R02、R03、R05、R07、R16、R20、R21、R25。
- **工作**：在真实路由全入口接入 PG；列 databases/schemas/tables/views，按展开连接目标库；系统对象过滤、普通用户可见性、对象 kind 与行数估计；请求 generation 和缓存失效。PG 不返回 mock 数据。
- **交付位置**：postgres/metadata.rs、app/connections 路由和对象加载；core object kind/capabilities。
- **验收**：真实两数据库、多 schema、同名对象、视图/物化视图/分区表；无枚举权限仍能打开指定库；刷新/断开不清错范围；元数据错误不伪装空列表成功。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T07 — 建库、删库与 schema 管理

- [ ] 完成 T07
- **开始前读**：设计 6.2；R02、R05、R06、R07、R08、R20。
- **工作**：PG database options 和创建/删除命令；维护库独立 autocommit 执行；schema 创建/改名/删除；引用/权限/活动连接错误处理，成功后精确更新 app 状态。
- **交付位置**：core requests、postgres/metadata.rs 或 ddl.rs、app 数据库管理命令。
- **验收**：owner/template/encoding/locale 组合；普通用户无 CREATEDB 的失败；当前维护库保护；活动会话导致删除失败不自动 FORCE；schema 默认 RESTRICT；MySQL charset/collation 建库保持。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T08 — 结构元数据

- [ ] 完成 T08
- **开始前读**：设计 4.3、6.1、9.1；R02、R05、R08、R21、R22、R25、R26。
- **工作**：TableMetadata 和 PG 类型身份；columns/default/identity/generated；复合 PK/FK/check、表达式/partial/INCLUDE index、trigger/function、sequence；目录查询按版本和权限准确执行，保留原始 definition。
- **交付位置**：core/table_metadata.rs、postgres/metadata.rs、app table_info adapter。
- **验收**：字段顺序、dropped column、复合外键序位不笛卡尔积；同名约束不混表；表达式索引不丢项；PG14/15/16/17/18 字段差异测试；普通用户可用；未知属性保留。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T09 — 类型转换、绑定与二进制

- [ ] 完成 T09
- **开始前读**：设计 7.1；R02、R05、R06、R10、R16、R24、R33。
- **工作**：原生标量与复杂类型文本投影；动态 ToSql 参数编码；numeric/时间/数组/JSON/bytea 保真；二进制摘要、完整加载和大小限制；未知类型只读状态，禁止失败后重跑原 SQL。
- **交付位置**：postgres/values.rs、postgres/data.rs；必要的 core 类型元信息；app binary 适配。
- **验收**：设计类型矩阵全部有用例；空值/空 bytes/JSON null、numeric 精度、NaN/Infinity、时区/BC、数组维度与 NULL；显示后写回不变；summary 不可提交为数据；超限在后端拒绝；无敏感值日志。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T10 — 分页、排序、筛选与预览

- [ ] 完成 T10
- **开始前读**：设计 7.2、11.1；R02、R05、R06、R10、R16。
- **工作**：全限定表读取、limit+1/offset、稳定排序；现有 FilterOp 逐项 PG 翻译及参数绑定；同一查询计划生成预览；导出 COUNT 独立可取消，保留本地过滤与字段布局。
- **交付位置**：postgres/data.rs、relational 的真正共享辅助、app/data_editor 和加载状态。
- **验收**：多列排序/同值 tie breaker、页边界/大 offset、全部 FilterOp；空 IN/NULL/LIKE 转义/类型比较；预览与实际记录一致；错误列/非法操作不静默忽略；普通分页不 COUNT 全表。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T11 — 原子数据编辑与可靠行定位

- [ ] 完成 T11
- **开始前读**：设计 7.3、8.4；R02、R05、R06、R10、R11、R29、R30。
- **工作**：DEFAULT/NULL/值三态贯穿草稿/提交；identity/generated 控制；主键/唯一键与无键表安全定位；锁定原始值、行数检查、整批事务与 RETURNING；失败保留草稿，COMMIT 异常结果待核实。
- **交付位置**：core 写入意图/提交结果、app/data_editor、postgres/apply_changes.rs。
- **验收**：新增/复制/修改/删除/撤销与多行混合；复合主键、主键修改、重复无键行拒绝、并发冲突；任一约束失败全批回滚；NULL 不意外触发 DEFAULT；断连不重试写入；MySQL 原有插入语义回归。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T12 — PG 方言、分句与参数

- [ ] 完成 T12
- **开始前读**：设计 8.1；R02、R06、R10–R12、R14、R27。
- **工作**：统一 app/connector/editor 分句规则；Postgres DatabaseKind/AST/SqlDialect 映射；dollar quote、E 字符串、嵌套注释、Unicode 范围；参数 ::/$n 与 snippet 分离；格式化保持函数体。
- **交付位置**：core 公共 SQL 词法职责、app/sql_format/query_completion、sql_editor_adapter/dialect/statements/execution。
- **验收**：DO/函数含分号、嵌套注释、带引号标识符、选区/当前/全部一致；$1、$tag$、::、字符串冒号；不支持语法不被破坏；MySQL delimiter/SQLite trigger 既有行为不回退。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T13 — SQL 执行、事务与取消

- [ ] 完成 T13
- **开始前读**：设计 3.3、7.1、8.2、8.3；R02、R06、R07、R24、R27、R28、R33。
- **工作**：真实元数据判断结果、ordinal 解码、statement/result 映射；流读取与有界结果存储；会话事务/aborted/恢复、continue_on_error、CancelToken、超时和竞态；显式已回滚/取消/OutcomeUnknown 状态。
- **交付位置**：postgres/execution.rs/connection.rs；core 查询结果状态；app/query_execution 和结果存储接口。
- **验收**：SELECT/VALUES/SHOW/EXPLAIN/CTE DML/RETURNING/CALL、空结果列、重复列名、多语句中间失败；BEGIN→错误→ROLLBACK 恢复，不能预先 SET search_path 阻止恢复；pg_sleep 真取消；两 tab 隔离；大查询内存有界、翻页不重放写 SQL。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T14 — 补全、索引与语义提示

- [ ] 完成 T14
- **开始前读**：设计 4.2、6.1、8.4；R02、R05、R07、R12、R14、R21、R25、R26。
- **工作**：tables/columns/routines/triggers/FK 补全真实实现；schema/search_path/quoted case/签名索引；批量加载、取消、TTL/失效和持久化；插入文本引用与文档提示。
- **交付位置**：postgres/completion.rs；app/completion_index/controller/query_completion；UI resolver 适配。
- **验收**：同名跨 schema、别名、CTE、函数重载、未知 qualifier 不泄漏列；批量列非 N+1；DDL 后刷新；无权限/超时不阻塞编辑；元数据会话不影响用户事务；MySQL 补全原测试通过。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T15 — 结果编辑、查询保存与历史补偿

- [ ] 完成 T15
- **开始前读**：设计 4.2、8.4；R02、R10–R14、R29、R30。
- **工作**：基于真实来源/可靠身份开放单表结果编辑；正确处理 PG 引用名；查询保存/重启恢复 scope；PG 历史分类、前像/RETURNING 身份、补偿 SQL；事务提交/回滚历史状态；敏感语句不记录。
- **交付位置**：app/query_result_edit、query_history、query_saving；storage/query history；PG 补偿字面量 provider。
- **验收**：简单 SELECT 可编辑，JOIN/计算/聚合等不误写；二段对象名不写错库；保存后 schema 保持；INSERT/UPDATE/DELETE 补偿预览/执行目标正确，bytea/decimal 不失真；ROLLBACK 的写入不显示已提交；MySQL 历史可读可用。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T16 — DDL 与新建表

- [ ] 完成 T16
- **开始前读**：设计 9.1、9.2；R02、R05、R08、R21、R22、R25、R26。
- **工作**：TableMetadata → DDL；PG CreateTableProvider、类型能力、schema、identity/default/generated、约束/索引/注释/trigger function；预览与执行同一计划。
- **交付位置**：postgres/ddl.rs；app/create_table PostgreSQL provider；core 必要结构计划类型。
- **验收**：新表有列/联合主键/唯一/FK/check/index/comments/trigger；DDL 在隔离库重建后元数据对等；无 SHOW CREATE/虚构 pg_get_tabledef；不含 MySQL 属性；未知属性不被抹掉；MySQL 新建表 SQL 快照保持。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T17 — 设计表和差异执行

- [ ] 完成 T17
- **开始前读**：设计 9.1、9.2；R08、R16、R21、R22、R26。
- **工作**：结构直接加载 metadata；字段/约束/索引/trigger/注释变化转为有序操作；ALTER TYPE USING、依赖保护、事务能力分组；结构指纹防止旧快照覆盖，保留未知定义。
- **交付位置**：app/create_table 设计模型/provider、postgres/ddl.rs，既有设计命令。
- **验收**：无修改无 SQL；增删改名/默认值/类型/NULL/identity/索引/FK/check/trigger 可预览执行；失败回滚；外部 DDL 后阻止过期应用；修改普通列不删除 RLS/分区/排除等未知属性；特殊非事务语句明确单独状态。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T18 — 表操作

- [ ] 完成 T18
- **开始前读**：设计 9.3；R08、R15、R16、R21、R22、R26。
- **工作**：PG 重命名/复制/清空/删除 provider；全限定对象；复制结构/数据和独立序列；RESTRICT/CASCADE 与 restart identity；成功后更新对应缓存/标签。
- **交付位置**：app/table_actions、postgres/ddl.rs、core 表动作选项。
- **验收**：schema 同名表只操作指定对象；复制后源/目标自增独立；有数据 identity 不重复、generated 不手工写；默认 RESTRICT；拒绝以 session_replication_role 绕过 FK；对象 kind 对应正确 DDL；MySQL 表操作未变。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T19 — 连接 UI

- [ ] 完成 T19
- **开始前读**：设计 4.1、5、10；R00、R13、R16–R18、R24。
- **工作**：数据库选择列表加入 PostgreSQL；复用连接 Dialog/Input/Select/Tabs，PG 字段和验证；测试/保存 loading；证书/SSH 指纹/URI 错误反馈；编辑/复制/清空密码与持久化命令。
- **交付位置**：connection_dialog/postgres.rs、navicat_main/connection_forms、连接类型图标/选择。
- **验收**：gpui-component 0.6.0、AppIcon；明暗主题、输入 focus/hover、Esc/关闭/外点/内点防穿透；测试配置就是业务有效配置；重启恢复可用；MySQL/TiDB 原表单默认值/保存不变；桌面可启动。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T20 — schema 树与数据库 UI

- [ ] 完成 T20
- **开始前读**：设计 4.2、6、10；R00、R07、R13、R16、R17、R20、R21。
- **工作**：connection/database/schema/object 层级、Tree/ListItem、懒加载/错误/刷新/显示库、建库与 schema Dialog、能力菜单；上下文数据库/schema 不混淆；断开/删除后的任务/标签处理。
- **交付位置**：sidebar/、tree_helpers 拆分职责、menus_dialogs/create_database/display_database、连接菜单。
- **验收**：两库/多 schema/同名对象能独立浏览；加载有 Spinner，旧请求不会覆盖新对象；菜单外点关闭/内点阻止穿透/二级贴齐；脏标签保护；库删除失败不提前删 UI；MySQL 仍保持原树层级。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T21 — 数据、查询、详情与历史 UI

- [ ] 完成 T21
- **开始前读**：设计 7、8、10；R00、R10–R17、R28–R30。
- **工作**：现有 DataTable/delegate 接入 PG 列/值/编辑性；二进制/JSON/时间详情；查询 scope Select、结果标签、取消、事务状态、历史补偿；gpui-component 统一 loading/error/disabled 与 show_message。
- **交付位置**：data_editor_model、data_table_ui、cell_detail_table_info、sql_editor_adapter、查询参数/保存/历史现有职责。
- **验收**：复制/多选/键盘/列宽/隐藏/排序/过滤/分页保持；RETURNING 和空结果显示；时区/复杂类型不被错误时间控件改写；明暗主题、Esc/外点/焦点；长操作不冻结窗口；关闭标签后迟到结果安全；MySQL 数据与查询回归。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T22 — 结构编辑与表操作 UI

- [ ] 完成 T22
- **开始前读**：设计 9、10；R00、R08、R15–R17、R22。
- **工作**：新建/设计表的 PG provider 字段和 tabs；metadata 保真只读属性；表重命名/复制/清空/删除对话框、依赖与 SQL 预览；PG trigger function 输入语义。
- **交付位置**：create_table/、table_rename/table_copy/table_danger、表信息面板。
- **验收**：F12–F14 完整可操作；PG 不展示 engine/unsigned/MySQL FK 开关；RESTART IDENTITY/CASCADE 明确选择；危险操作预览与实际一致；Esc/外点/主题/手形/hover；MySQL 设计表功能与默认值保持。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T23 — 数据导出

- [ ] 完成 T23
- **开始前读**：设计 7.1、7.2、11.1；R02、R06、R10、R15、R16、R25、R26。
- **工作**：从 UI 搬出共享编码/文件写入；PG 全表一致快照批量流；表 SQL/TXT/CSV/JSON/XML、行/选区 CSV/JSON/Markdown/INSERT；字段选择/条件/计数预览/进度/取消/临时文件。
- **交付位置**：app/transfer、storage 文件服务、PG export provider、data_export UI 适配。
- **验收**：每个现有格式与范围真实导出；UTF-8/分隔符/NULL/decimal/数组/bytea/JSON 往返；全量无重复漏行；内存有界；取消只留可识别临时状态，不记录成功；SQL 文件可在隔离 PG 库执行；MySQL 导出格式不变。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T24 — SQL 文件与原生脚本

- [ ] 完成 T24
- **开始前读**：设计 8.1–8.3、11.2；R14、R15、R24、R27、R31、R33。
- **工作**：目标 database/schema、编码、流式分句、进度日志/继续错误/停止；检测 COPY STDIN/psql 元命令后提供明确原生模式，使用安全子进程参数、凭据/传输与生命周期；普通模式不误拆 dump。
- **交付位置**：app/transfer/sql_file、postgres/native_tools、sql_file_execution UI 拆分职责。
- **验收**：普通多语句/函数体/Unicode 与编码转换；中途失败及取消；COPY 数据中的分号不误执行；原生模式需明确执行动作；无 shell 插值或密码参数；psql 版本/不存在有清晰错误；原有 MySQL SQL 文件流程回归。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T25 — 数据库备份和恢复验收

- [ ] 完成 T25
- **开始前读**：设计 11.3；R13、R15、R18、R23、R26、R31。
- **工作**：pg_dump 工具检测/版本、plain 格式与结构/数据/完整、对象/owner/ACL、目录/记录；有 custom 格式则同时实现 pg_restore；所有 I/O 经 app/connector/storage；取消、管道/进程回收、passfile/partial 清理；普通表逻辑导出准确标注范围。
- **交付位置**：postgres/native_tools、app/transfer/backup、storage backup records、database_backup/backup_tab UI。
- **验收**：完整备份恢复到干净隔离库后比对表/行/约束/索引/函数/视图/序列；恢复后新增 identity 行正确；SSH+TLS 下可用；旧 pg_dump/缺工具/权限不足不假成功；取消无进程/密钥泄漏；MySQL 原生/逻辑备份和记录回归。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T26 — 角色、用户和 ACL 后端

- [ ] 完成 T26
- **开始前读**：设计 12；R02、R07、R09、R20、R28、R29。
- **工作**：PrincipalIdentity 和 PG role 属性；角色/LOGIN 用户列表/创建/改密/重命名/删除；成员关系/ADMIN OPTION；database/schema/table/sequence/routine 授权撤销；直接/继承/PUBLIC/owner 权限解释与变更差异；敏感操作隔离历史日志。
- **交付位置**：core user_admin 领域类型、postgres/user_admin.rs、app/user_admin PG provider/命令。
- **验收**：管理者与普通用户；所有对齐操作可执行；函数重载与跨 schema 授权不混淆；PG14/16+ 成员差异；缺权限不覆盖既有 ACL；删除有依赖 role 不自动 DROP OWNED；密码不入日志/历史；MySQL user@host 和资源字段保持。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T27 — 用户与权限 UI

- [ ] 完成 T27
- **开始前读**：设计 10、12；R00、R09、R16、R17、R20。
- **工作**：现有 user_admin 页面按 provider 显示 PG role/LOGIN/成员/对象权限；授权对象选择带 database/schema/签名；loading/草稿/应用/错误/确认；MySQL host/plugin/每小时资源限制不出现在 PG 表单。
- **交付位置**：user_admin/、user_admin_privileges、现有用户管理入口与菜单能力。
- **验收**：无需手写 SQL 完成 F18；直接与继承权限区别明确；成员与 grant option 不混淆；无权限有理由；切用户不覆盖旧草稿；明暗主题、Esc/外点/组件/键盘；MySQL 全套用户页面回归。
- **完成记录**：未开始；执行人 —；内容/验证 —。

### T28 — 全量对齐与交付审查

- [ ] 完成 T28
- **开始前读**：完整设计，重点 1.3、13；全部任务完成记录和偏差；现有全部相关测试；R00。
- **工作**：按下表完成 F01–F20 的证据登记；PG14–18 真库矩阵、非超级用户、网络/TLS/取消异常；MySQL 对等功能和共享影响数据库回归；文件/连接/线程/结果缓存资源检查；复查新源码行数、模块可见性与过度封装；同步设计与真实实现路径。
- **交付位置**：相关 integration tests/fixture/CI 或运行脚本；本文件验收证据；设计参考索引新增实际实现路径（保留原参考）。
- **验收命令**：`cargo fmt --all`、`cargo check --workspace`、`cargo test --workspace`；另运行显式 opt-in 的 PostgreSQL 与 MySQL 真实测试并记录完整命令/服务器版本；`cargo run -p fluxdb-desktop` 启动及明暗主题手工回归。环境不足如实待验证，不能跳过后宣称完成。
- **最终标准**：T01–T27 全部有有效完成记录；F01–F20 每行通过；无必须功能残留或以 mock/仅 SQL 替代图形能力；MySQL 正常功能保持；文档与代码一致。
- **完成记录**：未开始；执行人 —；内容/验证 —。

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
