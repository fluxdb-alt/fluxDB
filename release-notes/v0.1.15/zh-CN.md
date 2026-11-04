## 新功能

- 新增 MariaDB 数据源：独立连接类型复用 MySQL 协议链路并持久化数据库类型，支持对象浏览、建库删库、SQL 查询与补全、表结构管理与设计、数据编辑、导入导出、备份恢复、用户与权限管理，以及 AI 页面上下文与受控执行。
- MariaDB 使用独立只读守卫与 AI SQL 策略；账号 DDL 使用 `IDENTIFIED VIA ... USING PASSWORD` 与 `SET DEFAULT ROLE ... FOR`，角色按无 host 账号处理；备份恢复优先使用 `mariadb-dump` / `mariadb`，TLS 参数使用 `--ssl`。
- 新增连接级只读模式：高级设置增加“只读连接”，落盘为 `options["read_only"]`。SQL 按语句解析拦截写入，Redis 按命令白名单拦截，MongoDB 按只读命令白名单拦截；SQLite 同时以驱动只读方式打开文件，侧边栏标记只读连接。
- Redis CLI 在只读连接下改连本机 RESP 代理：读命令经现有连接器执行，写命令返回 `READONLY` 且不触达服务端；代理只监听 `127.0.0.1`，使用 `REDISCLI_AUTH` 注入随机令牌。
- 新增 `⌘P` / `Ctrl+P` 快速打开：模糊搜索侧边栏已加载的表与视图，支持高亮、键盘选择和直接打开；没有活动页签时自动选择可用的 SQL 连接。
- 新增 AI 内置文档工具：`docs.search` 检索随应用编译的产品文档，`docs.read` 按 `section_id` 分页读取正文；两个工具只读本地静态内容，不访问用户文件、数据库、凭据或网络。
- 新增完整中文操作手册，覆盖连接、SQL 编辑器、数据编辑、结构管理、导入导出、备份恢复、权限、Redis、MongoDB、设置、快捷键与常见问题，并配实际操作截图。
- MongoDB 命令行终端与库级命令工作台支持 `insertOne` / `insertMany` / `insert`、`updateOne` / `updateMany`、`replaceOne`、`deleteOne` / `deleteMany` / `remove`、索引创建与删除、`db.createCollection` 和集合删除；聚合管道仍拒绝 `$out` / `$merge` 写回。

## 改进

- 设置页的数据库支持列表补齐 MySQL、MariaDB、PostgreSQL、SQL Server、Oracle、SQLite、TiDB、达梦 DM8、MongoDB 与 Redis 的真实连接器状态，移除过时的 Mock 文案。
- 退出应用时，如存在未保存修改或运行中的查询，先弹出确认框；覆盖快捷键、托盘和关闭窗口三种退出入口。
- 连接框、对象树、数据编辑、导入、恢复、建删库及 Redis / MongoDB 写入口统一接入只读拦截，并在界面与 AI 工具中同步只读状态。
- AI SQL 策略为 MariaDB 增加独立方言与只读判定；MongoDB 写方法、原生终端和命令工作台共用同一套只读边界。
- SQL 补全在 MySQL、PostgreSQL、Oracle、SQL Server 上补齐多类派生表、CTE、集合运算、JSON 表函数、限定符与大小写场景。
- Redis Workbench 的执行失败改为与成功记录同构展示，已有历史记录时仍能看到本次失败原因。

## 修复

- 修复 MariaDB 建表校验错误套用 MySQL `CHECK ... NOT ENFORCED` 限制的问题。
- 修复 SQL 补全在普通 `schema.` 限定符后不触发的问题；带引号标识符不再依赖回退逻辑。
- 修复 MySQL 补全缺少 CTE 投影列、集合运算派生列、命名窗口、`JSON_TABLE` 列清单、库限定 UDF 与跨库表元数据的问题。
- 修复 PostgreSQL 补全不支持 `LATERAL` 表值函数列别名、已有调用括号时重复插入 `()`、引号列丢失 `d.` 限定符的问题。
- 修复 Oracle 补全未引用小写表名、CTE / 派生表别名、`UNION` 投影与 `JSON_TABLE` 列推导的问题；执行前剥离前导注释，避免 `ORA-00900`。
- 修复 SQL Server 补全对 `OPENJSON ... WITH` 列清单与 `PIVOT` 输出列的推导缺失。
- 修复 MongoDB 补全缓存复用后再次选择集合候选时出现 `db.db["customers"].` 的问题：复用结果按光标后新增后缀平移替换区间。
- 修复主键列就地编辑后脏格不高亮、删除行不置灰的问题；行身份同时参考本次改动的新值。
- 修复 Redis Workbench 整次执行失败时不生成失败记录、错误提示被历史记录顶掉的问题。
- 修复鼠标点击或拖选移出函数调用后，旧函数签名浮层残留的问题。
- 修复取消更新下载后更新弹框未立即关闭、后台返回取消时重复提示的问题。
- 修复只读连接下 Redis Pub/Sub 发布仍可能入队执行的问题，发布入口直接拒绝。

## 已知限制

- 连接只读是客户端拦截，不能替代数据库账号权限；外部客户端或绕过 FluxDB 的连接仍受服务端授权控制。
- MariaDB 复用 MySQL 协议链路，未单独实现 MariaDB 独有的全部服务端特性；备份恢复需要本机安装 `mariadb-dump` / `mariadb` 或兼容的 MySQL 客户端。
- AI 文档工具只覆盖随应用编译的产品文档，不读取在线文档、用户文件或外部知识库。
- MongoDB 写方法仅覆盖终端与命令工作台已列出的命令；未实现的写方法会返回明确提示，不冒充成功。

## 验证

- 通过 `cargo fmt --all -- --check`、`cargo check --workspace --locked` 与 `cargo clippy --workspace --all-targets --locked`。
- 通过 `cargo test -p fluxdb-app --lib`、文档解析与检索测试、工具目录覆盖测试。
- 补齐 MySQL / PostgreSQL / Oracle / SQL Server 补全真实库用例；用例按环境变量启用，未配置时自动跳过。
- 发布前通过 GitHub Actions 三平台（macOS、Windows、Linux）构建预检。
