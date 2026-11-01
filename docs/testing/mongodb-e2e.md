# MongoDB 测试

> 关联实现说明：[MongoDB 支持](../mongodb.md)
>
> 通用测试规范见 [Rust 风格与单测](rust-style-and-unit-tests.md)。

## 1. 分层

| 层 | 位置 | 是否需要真实 MongoDB |
| --- | --- | --- |
| URI 解析 / 档案往返 | `crates/fluxdb-core/src/parts/mongo_profile.rs` 内 `#[cfg(test)]` | 否 |
| 错误文本脱敏、Extended JSON | `crates/fluxdb-connectors/src/parts/mongo/client.rs` 内 `mongo_wire_tests` | 否 |
| 导入：格式嗅探 / 类型推断 / 单元格转换 / JSON 错误定位 / 非 ASCII 与宽表预览 | `crates/fluxdb-connectors/src/parts/mongo/import.rs` 内 `mongo_import_engine_tests`（8 个用例） | 否 |
| 刷新替换语义 | `crates/fluxdb-app/src/parts/table_info.rs` 内 `replace_loaded_children_tests` | 否 |
| 连接器端到端 | `crates/fluxdb-connectors/tests/mongo_live.rs` | 是 |
| 查询面板与嵌套条件构建（生产视图） | `apps/fluxdb-desktop/src/main_parts/i18n_tests/mongo_query_panel.rs` | 否 |
| 聚合管道卡片视图（生产视图） | `apps/fluxdb-desktop/src/main_parts/i18n_tests/mongo_aggregation_cards.rs` | 否 |
| UI 端到端（生产视图） | `apps/fluxdb-desktop/src/main_parts/mongo_sidebar_tests.rs` | 是 |

## 2. 凭据注入

真实用例只从 `FLUXDB_MONGO_URI` 读连接串，未设置时**自动跳过**（打印 `跳过：未设置 FLUXDB_MONGO_URI`，用例仍算通过）。

凭据由调用方从自己的凭据文件注入，**不得写进仓库、测试夹具或日志**：

```bash
set -a; . /path/to/atlas-credentials.env; set +a
export FLUXDB_MONGO_URI="$MONGODB_URI"
export FLUXDB_MONGO_DB=fluxdb_demo
```

目标库解析顺序：URI 默认库 → `FLUXDB_MONGO_DB` → `fluxdb_demo`。

## 3. 连接器用例（`crates/fluxdb-connectors/tests/mongo_live.rs`）

```bash
cargo test --locked -p fluxdb-connectors --test mongo_live -- --test-threads=1
```

| 用例 | 覆盖 |
| --- | --- |
| `mongo_live_ping_create_query_metadata_and_cleanup` | 认证 → 建集合 → 插入（ObjectId/日期/嵌套）→ 查询 → 条件查询 → 投影 → 取消标志 → **整篇替换并复查** → **拒绝改 `_id`** → 集合信息 → 重复创建幂等 → **删除文档并复查（含重复删除必须失败）** → **新增文档（按返回 `_id` 复查、Decimal128 类型保留、自带 ObjectId 必须原样采用、非 ObjectId 的 `_id` 必须拒绝）** → 删集合 → 列表校验 |
| `mongo_live_aggregate_and_indexes` | 聚合分组求和、默认 `_id` 索引 |
| `mongo_live_server_info_reports_real_topology` | `buildInfo.version`、拓扑判定、`listDatabases` |
| `mongo_live_import_documents_from_csv_and_json` | 导入链路：CSV 列类型推断（`_id`→objectId、int32/double/boolean/date）→ 重复 `_id` 只失败一条（`ordered:false`）→ 按 `$oid` 复查确认主键是 ObjectId → JSON 数组（`$oid`/`$date` 还原）→ JSONL（一行坏 JSON 跳过，错误带行号） |
| `mongo_live_operations_survive_small_caller_stack` | **栈回归**：192 KiB 栈线程里跑建连 + 集合列表 + `find` 游标解码 |
| `mongo_live_shell_statements` | shell 语法全链路：`show dbs` / `show collections` / `use` 切库 / `find`（带 filter、sort、projection、limit）/ `findOne` / `countDocuments` / `distinct` / `aggregate` / `getIndexes` / `stats` / `getCollectionNames`；`ObjectId("…")` 等构造器改写；写方法在解析层被拒（用例实测 `deleteMany`，其余写方法由 `mongo_shell_tests` 单测覆盖） |

每个用例自建 `fluxdb_smoke_*` 集合并在结束时删除，不触碰既有数据。

### 3.1 栈回归用例的意义

`Runtime::block_on` 在调用方线程 poll future，gpui 后台任务在 macOS 上是约 512 KiB 栈的 GCD worker。曾经在 `listCollections` 序列化时溢栈并 `abort()`（SIGILL / SIGABRT，不可捕获）。

该用例用 192 KiB 栈线程复现同样环境：把 `mongo_run` / `mongo_block_on` 换成直接 `mongo_runtime().block_on`，测试进程会直接崩（`fatal runtime error: stack overflow, aborting`），而不是断言失败。恢复放大栈后稳定通过。

## 4. 查询面板与嵌套条件构建（无需真实数据库）

```bash
FLUXDB_PROFILE=dev cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop \
  mongo_query_panel_keeps_filter_row_and_collapses_options -- --exact
```

由生产 View 的无头点击驱动：打开「更多选项 → 条件构建」→ 添加条件 / 子组 → 点分段控件的 `OR` 段（`mongo-query-builder-logic:0:or`，五段控件里的一段，不是键盘选下拉项）→ 验证同字段两条条件都写入 Filter → 删除子组 → 收起抽屉。

断言里还量了两格的几何：两格都是**代码编辑器**（字段挂路径补全、值挂同行字段的取值补全），宽度不许越出格子——它们曾因多行编辑器溢出而长出拖不动的滚动条；垂直位置量的是编辑器光标行的中心与格子中线之差（≤ 1.5px），锁住「加了补全后字不在格子正中」这个回归。

同一段还断言 `builder.scroll.max_offset() == 0`：只有一个条件时树不该有可滚区间。滚动容器自带内边距时 gpui 会把它算两遍，单行也判成溢出并画出滚动条。

运算符下拉也在这段里走真实点击：点 `mongo-query-builder-operator:0` 开菜单，`down down enter` 选第二项（菜单打开时 `selected_index` 是 `None`，第一个 `down` 落在第 0 项），再断言模型里那条条件的运算符变成 `$ne`。格子画的与模型读的是同一份 `condition.operator`（行里不再存副本，改完调 `cx.notify()` 重绘），所以模型断言即覆盖显示。选完点回第 0 项还原，后面「空行不进 Filter」的断言才成立。

同一段还断言两格的 `lsp().completion_provider` 都存在：候选内容依赖真实集合的字段目录，无库环境是空的，所以这里只锁「接线没断」，筛选规则由 `mongo_completion_tests` 覆盖（含值格只取同行字段取值、目录外路径给空候选）。

工具条是单行流式布局，所以在点「添加子组」前先断言它的中心落在 `window.bounds()` 内：内容超宽会把右侧按钮推出窗口，而窗口外的点击被 gpui 静默丢弃，症状是「点了没反应」而不是报错。

五种组类型的语义（`$nor` / `$elemMatch` / `$not+$elemMatch` 的往返与字段作用域）由同 crate 的纯模型单测覆盖，不依赖视图：

```bash
cargo test --locked -p fluxdb-desktop mongo_query_builder_model_tests
```

它锁的是「解析 → 写回」逐字相等，以及数组组缺字段时不产出条件；另有 `a_wide_group_is_not_capped_per_group` 锁「单组八条条件照常展开」——组内卡条数会让「添加条件」在第 7 条时无声置灰。

面板用例里也连点七次「添加条件」再逐条移除：同一组超过六条仍要落成行。上限只剩总数 24 条，中间的溢出交给条件树滚动。

默认态与收起态不渲染构建器，查询面板高度不被嵌套条件长期占用；展开后条件树封顶 240px，再多也在树内滚动，不挤占结果区。

封顶这条用例的树从 Filter 文本反向展开（根 `$or` 四个子组、每组三条条件），不逐条点「添加条件」：加满第一个子组后，后一个子组的按钮已被滚出封顶视口，窗口外的点击被 gpui 静默丢弃，用例会卡在「按钮点了没反应」而不是报错。树高用 `mongo-query-builder-tree` 的实测边界量，再发三次滚轮事件断言最后一行上移——滚动条与滚轮同属原生 `overflow_y_scroll`，滚不动就说明树被压成 0 高或没有可滚区间。

滚完还要断言 `scrollbar-overlay` 的边界**没变**、且高度等于树的可视区：覆盖层是视口覆盖层，跟内容一起滚时拇指就在原地不动，正是「往下滚滚动条不动」那个 bug。

## 4.1 聚合管道卡片视图（无需真实数据库）

```bash
FLUXDB_PROFILE=dev cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop \
  mongo_aggregation_cards_edit_the_pipeline -- --exact
```

同样由生产 View 的无头点击驱动：切到聚合分节 → 断言默认管道摊成一张卡 → 点 `mongo-aggregation-add` 追加 `$match` 并检查管道文本；新卡的参数框已是「说明注释 + `{}`」，且注释**没有**跟着进管道文本 → 停用第一张卡（卡片仍在、`$group` 从文本里消失）→ 下移换序 → 上移在首位是空操作 → 断言预览区（右栏，常驻）与卡头运行按钮都在窗口内 → 手动摆一个「已有预览」再点停用触发管道重写，断言预览被置回 `NotLoaded` → 折叠/展开卡片（卡身消失又回来，卡片与管道都不动）→ 删除 → 运算符下拉：点 `mongo-aggregation-key:1` 开下拉，敲 `l i m` 再回车，断言选中的是 `$limit`（筛选没生效的话回车会落在第一项 `$match`）且没动过的默认参数跟着换成 `$limit` 的模板；再把参数框改成手写 JSON，换成 `$skip`，断言参数原样保留。

下拉是 `Select`（可输入筛选），键盘直接打进弹出的搜索框；点开卡片区的第二张卡之前先收起第一张——卡片区封顶 320px，两张展开的卡会把第二张卡头挤出视口，而窗口外的点击被 gpui 静默丢弃。

预览结果（`LoadState::Loaded`）摆进用例后断言 `mongo-aggregation-preview-card:0` 与 `mongo-document-fields:0` 同时存在：预览每条文档渲染成文档页同一套字段列表卡片，而不是整段 JSON 文本。

预览请求本身在这里**不点**：它会真的建连，无服务时驱动要等服务端选择超时（默认 30s）才失败。预览与正式聚合共用 `mongo_aggregate`，链路归连接器用例；追加 `$limit: 10` 与 `$out` / `$merge` 拒绝预览在 `run_mongo_aggregation_stage_preview` 里，走到即需要真实连接，本地不覆盖。

管道 ↔ stage 数组的纯转换由同 crate 单测覆盖，不依赖视图：

```bash
cargo test --locked -p fluxdb-desktop mongo_aggregation_model_tests
```

锁的是：空文本等价空管道、往返不改数字类型（`$limit: 10` 仍是整数）、嵌套参数缩进、停用 stage 保留但不写进管道、非法参数原样内联（不静默丢 stage）、多键对象与非 `$` 键被拒绝、参数编辑器高度夹在 2–8 行、默认参数是「注释 + 可跑的合法 JSON」且注释不进管道、每个运算符的说明 key 在英文与中文语言包里都存在。

## 5. UI 端到端（`apps/fluxdb-desktop/src/main_parts/mongo_sidebar_tests.rs`）

```bash
cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop \
  mongo_database_expands_and_collection_opens_workspace
```

走**真实生产视图**（`navicat_main_new` + `Root`），全部通过无头事件驱动，不直接调处理函数：

0. 文档卡片走虚拟列表，视口外的卡片**根本不构建**，选择器查不到。需要第 N 张卡片的按钮时先用 `scroll_card_into_view` 像用户一样滚过去（滚轮回滚到顶再逐格下滚），不能假设整页 DOM 都在。
	- 卡片下标取自上一步真实返回的 `page.documents` 里目标 `_id` 的位置，不要写死。刷新在途时渲染缓存仍是上一页快照，写死下标会点到别的文档的按钮（用例会等「待删探针文档落库」拿到下标，正是在防这一点）。
1. 双击 `connection-row:{id}` → 等 `database-row:{db}` 出现（真实建连）
2. 点击库行 → 等 `object-group-row:collections:{db}` 出现（懒加载集合列表）
3. 点击分组行 → 等集合行 `table-tree-row:{name}` 出现
4. 点击集合行 → 等 `mongo-collection-page:{db}/{collection}` 出现
5. 等标签页 `MongoCollectionState.page` 有真实返回文档
6. 等 `mongo-document-fields:0` 出现（字段列表，不是整段 JSON）→ 点 `mongo-document-toggle:0`（`mongo-document-toggle:{下标}` 卡片折叠箭头）→ 选择器消失 → 再点恢复
	- 整条卡头可点：再点 `mongo-document-head:0`（卡头本体，不是箭头）同样能收起/展开
7. 点 `mongo-display-tab:Json` → 等 `mongo-document-json:0` 出现，且卡片头 `mongo-document-toggle:0` 仍在、`mongo-document-fields:0` 消失（JSON 是逐条卡片正文，不是整页数组）
	- 默认折叠：标签页的 `mongo_json_fold_toggles` 必须为空（空 = 根展开、嵌套全收起的默认态），数出此时可见行数
	- 展开：从第 1 行起找窗口内的 `mongo-json-fold:0:{行}`（该选择器只出现在可折叠节点行；普通行是 `mongo-json-fold:0-plain:{行}`，注意 `-plain` 在前缀之后），点击 → 等可见行数增加
	- 点 `mongo-display-tab:Documents` 还原
8. 点 `mongo-display-tab:Table` → 等 `mongo-document-table` 出现，且首行 `mongo-table-cell:0:id` 渲染、`mongo-document-fields:0` 消失；首行的 `mongo-document-delete:0` 与 `mongo-document-copy:0` 必须渲染且中心点在窗口内（固定操作列）→ 点复制按钮，等 `app-message-overlay` 出现（剪贴板写入必须有可观察反馈）→ 切回文档视图
9. 从真实返回的文档里挑一个落在窗口内的对象/数组字段 → 断言 `mongo-field-children:{_id}/{字段}` 默认不存在 → 点摘要行后出现
10. 删除文档：先插入一篇固定 `_id`（`fluxdb-e2e-delete`）的探针文档并重查 → 收起展开的卡片 → 点该行 `mongo-document-delete:{下标}` → 等 `mongo-document-delete-modal` / `mongo-document-delete-confirm` 出现，并断言此时探针文档仍在（没点确认不能删）→ 点 `mongo-document-delete-cancel`：确认框消失且文档仍在 → 再点删除并确认 → 等本页不再含该 `_id`。用例只删自己插入的探针，绝不删演示数据。
10.1. 复制文档：插入固定 `name` 的探针文档 → 点该行 `mongo-document-duplicate:{下标}` → 等 `mongo-document-duplicate-drawer`（与编辑共用的右侧抽屉）出现，断言初值**不含 `_id`** → 点 `mongo-document-editor-cancel`：抽屉消失且不写入 → 再点复制并 `mongo-document-duplicate-confirm` → 等本页出现两条同名文档，且其中一条 `id` 不等于源 `_id`（副本 `_id` 由服务端生成）
10.2. 新增文档：点结果条 `mongo-results-insert`（先断言它落在窗口内，越界点击会被静默丢弃）→ 等 `mongo-document-insert-drawer` 出现，断言初值只有 `_id` 一个字段且是 `$oid` 包装 → 把草稿补成「同一个 `_id` + 一个探针 `name`」→ 点 `mongo-document-insert-confirm` → 等本页出现 `id` **等于预填 `_id`** 的文档（预填的 id 必须真的落库，不能只是界面假象）→ 断言抽屉已关闭
10.3. 导入文档：点结果条 `mongo-results-import`（先断言落在窗口内）→ 等 `mongo-import-modal` 出现 → 断言此时 `pending_mongo_import.source/preview` 都为空（未选文件不出预览）→ 断言 `mongo-import-empty-null` 开关与 `mongo-import-delimiter-field`（「标签 + 下拉框」两行）**底部对齐**（差值 ≤ 2px，居中会让开关浮在中间）→ 点 `import-dialog-tab:Log` 切到消息日志页：`mongo-import-copy-log` 出现且 `mongo-import-start` 消失（写入动作只在常规页）→ 点 `import-dialog-tab:General` 切回，开始按钮回来 → 点 `mongo-import-close` → 等弹窗消失。
	- 选文件走系统原生对话框（`prompt_for_paths`），无头环境无法驱动，因此本步只验证入口、弹窗与关闭；解析与写入链路由连接器用例 `mongo_live_import_documents_from_csv_and_json` 覆盖，不在这里假装验证。
10.4. 创建集合后自动刷新：库行右键（`MouseDownEvent` + `MouseButton::Right`）→ 等 `database-menu-item:Create collection` → 点开 `mongo-create-collection-modal` → 在 `mongo-create-collection-input` 输入探针集合名 → 点 `mongo-create-collection-confirm` → **不点刷新**，直接等 `table-tree-row:{集合}` 出现。这是「创建集合后没刷新该库」的回归点：控制器只替换第一层库列表，界面必须自己重取该库子节点，只标记「已加载」会把过期快照当成最新。结束后用 `MongoRunCommand` 发 `{"drop": …}` 清理探针集合。
11. 文档编辑抽屉：断言 `mongo-document-editor` 初始不存在 → 先临时插入一篇固定 `_id`（`fluxdb-e2e-switch`）的探针文档并重查（演示库集合可能只剩一篇，未保存切换需要两张卡片），把展开着的卡片逐个收起让卡头都落进窗口 → 点卡片头 `mongo-document-detail:0` → 等抽屉出现
	- 断言编辑框初值等于会话基线 `base_json`（未改动时保存按钮禁用）
	- 取本页首篇文档，把第一个顶层标量字段改掉（字符串追加 `[fluxdb-e2e]`，没有字符串才退回数值 +1；`_id` 一律不动），通过真实编辑框 `set_value` 替换全文（不直接调保存函数）
	- 未保存切换：点另一张卡片的编辑按钮 → 等 `mongo-document-discard` 出现 → 点 `mongo-document-discard-cancel` → 确认框消失且草稿仍是改后的文本
	- 放弃并切换：再点另一张卡片 → 点 `mongo-document-discard-confirm` → 编辑会话切到目标文档（`editor.index` 为目标下标）
	- 再点回首篇、写入同一份草稿 → 点 `mongo-document-editor-save` → 等抽屉消失 → 等本页同 `_id` 文档的 JSON 与期望语义相等（重新查询带回新值）
	- 还原：用同一条 `SaveMongoDocument` 命令写回原文档并重查，断言已回到原值；随后删除探针文档并重查确认消失——用例只借真实库验证写回，不留改动；第 12 步按 `_id` 清 switch 探针、按 `name` 清复制探针（副本 `_id` 由服务端生成，只能按内容定位）

卡片 / 字段 / 表格行的纯逻辑（BSON 包装判类型、`ObjectId("…")` 文本、卡片身份取 `_id`、表格列序与类型、JSON 折叠键按文档隔离）另有 `mongo_documents_tests` 五个单测、`mongo_document_delete_tests` 两个删除提示单测、`mongo_document_duplicate_tests` 五个复制草稿单测（剥 `_id`、字符串 `_id`、**字段顺序不变**、BSON 类型保留、非法输入透传）、`mongo_document_insert_tests` 三个新增草稿单测（`_id` 是 24 位小写十六进制、同进程不重复、初值只有 `$oid` 包装的 `_id`）覆盖，不需要连库。

断言既有可观察反馈（debug 选择器边界），也有业务结果（返回条数 > 0）。第 9–11 步都要先比对目标中心点是否在窗口内——越界点击会被 gpui 静默丢弃；卡片正文很高，所以要先把展开的卡片收起，否则第二张卡片只有卡头露出视口外。第 11 步的保存由真实按钮触发，但替换全文走编辑器实体（`set_value` 不产生 Change 事件，按钮可用性由编辑框的 Change 订阅驱动重绘）；若当前集合没有可改的顶层标量字段，用例打印原因后提前返回，不谎报通过。未保存切换的断言不再允许跳过：探针文档保证窗口内至少有两只可点的编辑按钮，缺任何一只都会让用例失败。

## 6. 命令行终端

```bash
cargo test --locked -p fluxdb-desktop --bin fluxdb-desktop \
  mongo_cli_runs_shell_statements_from_keyboard
```

同样走真实生产视图，输入全部是无头键盘事件（`simulate_input` + `simulate_keystrokes("enter")`），断言落在终端 grid 的可视文本与标签页状态上：

1. 库行右键 → 等 `database-menu-item:Command line`（界面文案来自语言包，按钮按真实文案定位）→ 点击 → 等标签页状态出现指向该连接 / 该库的 `TabKind::MongoCli`
2. 点 `terminal-surface` 取焦点，再画一帧——输入 handler 只在本帧绘制时挂到窗口，不重画则 `simulate_input` 无处可去
3. 等 grid 出现 `<库名>> ` 提示符（开场不打 banner，第一行就是提示符）
4. Tab 补全：敲 `sh` → **等 `> sh` 回显上屏** → `tab` → 等 `terminal-completion-popup` 出现 → 回车 → 等输入行变成 `show dbs` 且浮层消失 → 再回车 → 等输出含 `admin`（来自真实服务端）。这条同时证明 Tab 没被 Root 的焦点切换绑定抢走。
	- 等回显不能省：补全读的是 grid 已回显的输入行，REPL 回显走 30ms 泵。立刻按 Tab 会按空行算候选（全量），回车写回的首个候选是 `use ` 而不是 `show dbs`。
5. `db.`（等回显）→ 等 `terminal-completion-popup` 自己出现：这条走的是**服务端集合列表**，与方法白名单是两条完全不同的数据来源，只测方法名会让「集合名一直读不到」照样全绿。必须在切库前做——`use admin` 之后当前库换成 admin，它本来没有用户集合。→ 退格删空这半行，等浮层自己收起（空行不提示）
	- 浮层断言用 `visible_bounds`（非零尺寸 + 中心点在窗口内），不能用 `debug_bounds` 判存在：后者对零尺寸与窗口外元素同样返回 `Some`。曾经因此漏掉真 bug——候选算出来了，浮层却整块落在窗口下方被裁掉。这条在「面板比窗口高」的布局（`completion_popup_top` 的上限夹取、画布绝对定位）回退时会立刻失败。
	- 同时断言**单条候选的浮层高度落在 34–60px**：上限抓「空黑板」——只写 `max_h` 时绝对定位元素仍被拉到上限，一条候选也撑成 180×260；下限抓「空白细条」——外层高度小于实际内容高度时整块文字被裁掉。两条都真实发生过，所以在 `completion_popup_height` 被删、改回 `max_h`，或行高常量与候选行不一致时会失败。
	- 浮层左上角必须落在**光标所在行之后**（`bounds.top() > 终端内容区顶部`）：锚点早先钉在面板左边缘，光标跑到行尾时浮层离光标半个屏幕。这条在 `completion_popup_anchor` 退回固定左边缘时失败。
	- 还要断言**候选行整行落在浮层内**、**候选文字整块落在行框内**（`terminal-completion-row:0` / `terminal-completion-label:0`）：浮层没写 `text_size` 时会继承根字号 16px，比终端 12px 大，30px 的行框装不下，`overflow_y_scrollbar` 上下各切一刀——盒子和行都没问题，只有字被裁，截图上就是「use 只剩半截」。这条在 `completion_popup` 丢掉 `text_size` 时失败。
	- 终端没有绑 `ctrl-u`，清行只能用退格；这也是「浮层跟着输入行走」的断言。
6. 敲 `use admn` → `left` → 敲 `i` → 回车 → 等 `switched to db admin` 且提示符变成 `admin> `（同时验证 ← 移光标与中途插入；只靠行尾追加拼不出正确语句）
7. `db.demo.insertOne({…})` → 等 `read-only`（只读边界真的上屏；终端报错一律英文，对齐 mongosh）
8. `db.demo.unknownMethod()` → 等 `Unknown method`（单行提示，不是整块用法清单）
9. `db.demo.co`（等回显）→ **不按 Tab**，等 `terminal-completion-popup` 自己出现（边打字边提示是 Compass 的行为，也是这条最容易回退的点）→ `tab` → **`down` 后点第二行** → 等输入行变成方括号写法且浮层消失（鼠标点候选等于选中后回车）→ 退格清行 → 重新敲 `db.demo.co` 等浮层出现 → `escape` → 等浮层消失且输入行原样保留（候选来自方法白名单：`db["demo"].find(` 等方括号形式，不依赖服务端集合名）。
	- 点之前先断言第二行落在窗口内：越界点击会被 gpui 静默丢弃，用例会以「等待超时」失败而不是看清原因。
	- `down` 不能省：它证明浮层打开时方向键被浮层吃掉（移动高亮），而不是透传给 PTY 翻了历史行——透传时整行输入会被换掉，之后点候选写回的内容也对不上。

终端键位在用例里显式注册（`register_terminal_shortcuts`）：平时由 app boot 的 `register_shortcuts` 完成，用例直接建视图时缺这一步回车不会变成 `TermEnter`。`terminal-surface` 与 `terminal-completion-popup` 都是 `debug_selector`，只在调试构建记录，不改变生产行为。

## 7. 无凭据时

- `cargo test -p fluxdb-connectors --test mongo_live` → 5 个用例全部跳过（0.00s）。
- `cargo test -p fluxdb-desktop --bin fluxdb-desktop` → Mongo E2E 跳过，其余 117 个用例照常运行（本机实测 116 passed + 2 ignored）。

零用例或全跳过**不算** MongoDB 功能验证通过；需要真实链路结论时必须注入凭据跑一遍。
