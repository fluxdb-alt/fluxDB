## 新功能

- 新增连接级「服务器」面板：在连接右键菜单打开「服务器」页签。SQL 连接（MySQL / MariaDB / TiDB、PostgreSQL / KingbaseES、SQL Server、Oracle、达梦）提供概览、会话与锁等待，可终止查询或会话，并执行 VACUUM / ANALYZE / OPTIMIZE TABLE / REINDEX 等维护操作；Redis 提供概览、客户端与慢日志，可终止客户端、清空慢日志；MongoDB 提供概览、当前操作（慢操作过滤、killOp）与会话内性能趋势。支持手动刷新与 5 秒自动刷新、「只看我的会话」过滤；只读连接在界面、应用层与连接器三层拒绝终止与维护，面板错误只显示在面板内。
- 查询工具栏支持切换目标数据库与 Schema：连接名只读，按连接类型显示可切换的数据库 / Schema 下拉，执行中禁止切换。切库会关闭旧会话并清空结果（未提交事务回滚），切 Schema 保留会话；PostgreSQL / KingbaseES 切回「默认」时发送 `RESET search_path`，SQL Server 每次执行前重置到工具栏选中的库。SQLite、DM8、Oracle 按能力只读显示对应选项，需要时在 SQL 中显式限定。
- 新增自定义主题与字体设置：可新建、编辑、删除自定义主题，调整常用 token 与 SQL 高亮全部分类，主题写入用户数据目录的 `themes/`；支持导入 Zed 与 VS Code 主题（含 JSONC），按 scope 推导语法高亮，明暗变体自动切换主题模式；界面字体 / 字号与代码字体 / 字号 / 行高独立设置，界面字号整体缩放菜单、标签、表格与对话框，拖动时实时预览。

## 改进

- 本地 MCP 从独立 stdio 进程改为应用进程内的 Streamable HTTP 端点（默认 `http://127.0.0.1:47821/mcp`），仅监听回环地址并逐请求校验 Bearer 令牌；客户端配置改写为地址 + `Authorization` 头，并兼容接管旧 stdio 槽位；关闭开关后端点停止并断开已有会话，重新打开后地址与令牌不变。
- 新增集中式本地 SQLite 迁移：以 `PRAGMA user_version` 门控，升级前用 `VACUUM INTO` 生成一致快照并保留最近 3 份，每个迁移步骤与版本号在同一事务内提交；遇到更高版本的数据库直接失败关闭，避免旧版本写入破坏结构。
- MCP 的 `connections_list` 返回库名提示，`search_objects` 在没有库上下文时返回库清单；目标 / 作用域错误补充合法格式与修正建议。
- 统一数据库边界语义：SQLite 表浏览保留真实值类型，与查询编辑器一致；SQL Server 负 DECIMAL 精确渲染；TiDB 只读入口改用事务回滚并拒绝非查询语句。
- 新增跨 SQL 引擎一致性测试与运行脚本：SQLite 默认运行，PostgreSQL、MySQL、MariaDB、TiDB、SQL Server、DM8、Kingbase 与 Oracle 按环境变量启用，并补充覆盖范围与容器 / 远程运行说明。
- README 默认展示英文版，中文版迁移到 `README.zh-CN.md`；本地门禁脚本新增硬编码 px 字号检查，界面文本统一走 rem 缩放。

## 修复

- 修复 Redis 数据库在侧栏按字符串排序导致 10 排在 2 前的问题，改为按数值升序。
- 修复 SQL Server 负 DECIMAL 渲染成 `-1.-2500` 的问题。

## 已知限制

- 服务器面板的会话 / 客户端列表最多读取 500 条；SQL Server 不提供「终止查询」，Oracle 受驱动限制也不提供，二者只能终止会话；MongoDB 性能趋势只在会话内采样，最多约 5 分钟且不落盘。
- 只读连接是客户端护栏，不替代服务端只读账号：存储函数副作用、高权限账号等仍可能绕过。
- 自定义主题暂不支持 `.tmTheme` 等 plist 主题；删除主题会移入 `themes/.trash/`，不会立即释放磁盘；外部主题按 scope 推导高亮，个别语言可能需要手工调整。
- 切换数据库会关闭旧会话，其中未提交的事务会回滚；SQLite 没有当前库概念，DM8 驱动不支持切换 Schema，Oracle 一个实例只有一个库，均需在 SQL 中显式限定。
- MCP 端点仅本机可用，不提供远程访问；关闭 MCP Server 不会删除已写入的客户端配置。
- 本地数据库升级前会生成 `VACUUM INTO` 备份（保留最近 3 份），会占用额外磁盘；使用新版本升级过的数据库，旧版本应用会拒绝打开并提示升级。
