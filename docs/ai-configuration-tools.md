# AI 配置管理工具

## 全局工具

`toolbox.inspect`、`toolbox.search` 和 `toolbox.load` 不依赖设置页或数据库连接。
`toolbox.inspect` 只读取非敏感实时状态；`toolbox.search` 只返回长尾操作的摘要，
不携带参数 Schema；`toolbox.load` 在本轮加载 1–5 个操作，下一轮模型请求才会看到
这些操作的完整 Schema。加载不等于授权，配置写入工具只提交待确认提案。

`toolbox.search` 的选择顺序固定为：

1. 有可用 Jev 决策插件时，用 `query + 工具摘要` 让 Jev 选一个候选。
2. Jev 不可用、失败或低置信度时，用当前对话模型做一次隔离请求，只发送
   `query + 工具摘要`，不带历史、页面正文、完整 Schema，也不允许工具调用。
3. 两者都不可用时返回该领域的候选摘要，不返回空列表，由主模型从候选中选择。

选择器返回的 ID 必须属于本次候选集合；非法 ID 会被丢弃。搜索结果增加
`recommended_ids`、`recommended_tool`、`selection_source` 和 `next_step`，
主模型应使用推荐 ID 调用 `toolbox.load`，不要把推荐当成已授权执行。

| 工具/动作 | 行为 |
| --- | --- |
| `toolbox.inspect` | 读取设置、模型、Agent、全局工具开关、MCP、会话摘要 |
| `config.propose` | 配置写入能力开关；本身不是模型可见工具，启用后才允许加载配置动作 |
| `config.setting.update` | 修改白名单内的普通设置 |
| `config.model.create` / `config.model.update` | 确认后打开预填好的本地模型表单，用户完成凭据输入并保存 |
| `config.model.delete` | 删除模型，复用引用检查与对象级 CAS |
| `config.agent.create` / `config.agent.update` | 创建或局部修改 Agent，保留未指定字段 |
| `config.agent.delete` | 复用既有软删除，保留历史会话身份 |
| `config.tool.set_enabled` | 修改全局开关，与 Agent 白名单取交集，不清空白名单 |
| `config.session.archive` / `config.session.purge` | 归档或永久删除会话，永久删除仍要求先归档且无活动任务 |
| `config.mcp.server_set_enabled` | 启停已登记的本地 MCP Server Profile |
| `config.mcp.sync_client` | 为 Codex / Claude Code 注册或移除本应用的 MCP 条目；不是安装客户端软件 |

模型/Agent 编辑只传 `changes` 内明确修改的字段和读取到的 `expected_revision`。
模型和 Agent 的内部 ID 应从 `toolbox.inspect` 获取，不能按名称猜测。
运行中的 Agent 不能通过提案编辑或删除自身，仍可在设置页手动操作。

## 内置文档工具

`docs.search` 与 `docs.read` 只读随应用编译的操作手册，不读取用户文件、不联网，
也不需要附件授权。`docs.search` 返回小节 ID、章节名和短摘要；`docs.read` 按
`section_id` 分页读取正文，单次最多 16 KiB。两个工具始终进入 AI 工具目录，
受全局工具开关和 Agent 白名单约束。

遇到 FluxDB 功能、界面、配置、快捷键、支持范围或操作步骤问题时，AI 应先检索
文档，再依据正文回答并引用章节名；`docs.search` 使用简短关键词而不是整句提问。
正文被截断时按 `next_offset` 继续读取；文档未覆盖时明确说明，不编造操作路径。

## 确认与保存

1. 用户可在任意页面提出配置需求。AI 先读取摘要，再搜索并加载具体动作；
   每个动作的 Schema 只描述该动作字段，不再重复传 `action`。
2. 确认框展示修改内容。删除与客户端配置写入需要额外勾选影响确认。
3. 用户取消不落库；用户确认后，由应用层命令在后台执行。
4. 执行前重新检查版本、过期时间与目标状态。提案有效期为十分钟。
5. 保存成功才更新界面并显示成功；失败保留明确错误，不冒充成功。
6. 已尝试的提案不能再次执行。发生中断时先核实状态，再重新生成提案。

模型表单中的 API Key 只通过既有 Keep/Set/Clear 路径进入加密存储。
配置工具没有 API Key、密码、Token、密文或 nonce 字段。AI 只能看到凭据状态。
模型编辑打开表单只是准备阶段，用户点击该表单的保存按钮后才真正修改模型。

## 设置草稿与工具目录

设置修改统一经 `toolbox.search` / `toolbox.load` 加载 `config.setting.update`，
不再提供单独的页面草稿预填工具。用户仍可手动编辑设置页；
已有未保存的草稿时，不允许配置提案覆盖不同的修改。
旧 Agent 白名单中的已移除工具 ID 不再生效，也不会自动换成配置写入权限。

设置中的工具列表、Agent 的可选工具和 `toolbox.inspect(kind="tools")` 共用应用层
`ai_tool_management.rs` 的管理目录。目录包含条件装配工具，不代表这些工具在当前
任务都有授权。全局关闭仍由执行边界检查，发现工具不等于获得权限。

内部与对外 MCP 的 SQL 执行都只使用 `db.run`；`db.query`、`db.execute`、
`db.explain` 已从模型、MCP 和设置目录移除。启动迁移把旧 Agent 白名单归并为
`db.run`，并删除旧全局开关；旧 `config.inspect` 同样迁移到 `toolbox.inspect`。
Redis 工具仍保留各自的领域入口，不合并为通用执行器。
