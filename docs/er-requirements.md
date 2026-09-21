# FluxDB ER 需求追踪

> 本文件是 ER 功能的精简、可维护需求追踪文档，随每次实施更新。
> 状态取值：`未实现` / `部分实现` / `自动验证通过` / `待人工验收` / `验收通过`。
> 每项包含需求与来源、当前实现位置、状态、缺口/依赖/验证方式。
> 对应设计：[er-design.md](er-design.md)、[er-ui-relationship-canvas.md](er-ui-relationship-canvas.md)。
> 当前范围约束：Agent/MCP、ER 查询工具及其授权/快照协议暂不实施，后续统一规划；本阶段只覆盖本地 ER 模型、存储、画布和关系编辑闭环。

## 0. 验收口径

- 功能完成必须有真实端到端调用链，不能因存在按钮/字段/辅助函数/测试桩标为完成。
- 我无法识别图片；界面视觉与真实鼠标/键盘验收由人工完成，注释为「待人工验收」。
- 只有实际检查的平台才标验证通过；macOS 已实机，Windows/Linux 未验证。

## 一、正确性问题修复（四、来自审查）

| # | 需求 | 来源 | 实现位置 | 状态 | 缺口/依赖/验证 |
|---|------|------|----------|------|----------------|
| 1 | 局部 ER 深度 1/2/3 跳 + 逐节点展开实际可用；入/出向都含；复用目录/关系/字段缓存；保留坐标/固定/滚动/视口；快速切范围旧任务不覆盖新结果；收起/缩小范围不丢人工布局 | §四.1 | desktop `recompute_local_er_graph`/`er_all_edges`/`er_center_refs`；app `er_neighborhood_included_tables` | 部分实现 | 需自动化验证深度切换后图仍在、人工位置保留；旧任务竞争/取消需测 |
| 2 | PostgreSQL 身份与范围：center_table 结构化身份、跨 schema 双向邻居完整、含点标识符不靠字符串拆、MySQL/SQLite 明确、不支持给真实反馈 | §四.2 | core `ErTableRef`；app `er_service`/`dispatch`/`er_catalog` | 自动验证通过 | 字段缓存键改为结构化 `ErTableRef`，删除旧 `er_split_display`（rsplit 误拆含点表名）；逐 schema 批量读取 + 含点表名回归 |
| 3 | 失败重试真正重新调用 Connector；保留成功结果只重试所需范围；关系/字段都有恢复入口；不每帧自动重试、不查询风暴 | §四.3 | app `er_columns_invalidate_failed`/`er_relations_invalidate_failed` + retry；桌面工具栏/节点重试 | 自动验证通过 | 已有列/关系重试回归 |
| 4 | FK 标记：被引用端不标 FK；PK/FK/唯一/tooltip 语义正确 | §四.4 | core/桌面只标持有端 | 自动验证通过 | 需复核 |
| 5 | 命中坐标：绘制、鼠标命中、拖动、缩放、小地图统一坐标变换；覆盖侧栏/工具栏/窗口尺寸/DPI | §四.5 | `ErCanvas` 命中补画布原点 | 部分实现 | 待补齐已有缩放/小地图后统一复核 |

## 二、数据模型 D1–D9（三、五、七）

| # | 需求 | 来源 | 实现位置 | 状态 | 缺口/依赖/验证 |
|---|------|------|----------|------|----------------|
| D1 | 四层分离：结构快照 / 逻辑关系目录 / 图形视图 / Agent 投影 | core `er_relationship.rs` + storage `er_rel:{scope}` + app `er_model_service.rs` | 部分实现 | 逻辑关系目录、图形视图、按作用域 service 已接入；结构快照解析未做；Agent 投影按范围约束暂缓 |
| D2 | 关系含精确字段配对、required_filters、角色(对内唯一)、双向基数+依据、来源、确认、约束、有效、证据 | core `ErRelationship`（er_relationship.rs）+ desktop `er/canvas.rs` | 部分实现 | 模型和新建/编辑表单已接入；required_filters 支持结构化常量条件；绑定校验反馈仍待补 |
| D3 | 逻辑关系仅存本地；Agent 只能提议，用户确认后进入一般 JOIN 候选；不执行 DDL | ErModelService + `AppCommand/AppEvent` ER 命令 | 部分实现 | 本地 CRUD/确认/拒绝/删除/编辑均不执行 DDL；Agent 提议入口按范围约束暂缓 |
| D4 | 首版支持单列/复合等值 AND、字面量常驻谓词、自关联、桥表多对多；复杂谓词只记录 | core `column_pairs` + structured `ErRequiredFilter`/`ErLiteral` + 新建/编辑表单 | 待人工验收 | 单列/复合配对和结构化常量条件已可编辑；自关联/桥表可表达；基数选择（1:1/1:N/N:1/N:N/未知，basis=UserAssertion）已入表单并可编辑 |
| D5 | 全库 ER 分组概览、当前表 ER 默认一跳；同一模型 | §5.7/D5 | 部分（1 跳已有） | 部分实现 | 分组概览未做 |
| D6 | 原生 GPUI 画布（已按此实现） | §2 | desktop er/* | 验收通过方向 | 缩放需自绘评估 |
| D7 | 四个读取工具、统一关系服务、宿主增量、快照分页、usage、有界路径 | §6.4-6.10 | 暂缓 | Agent/MCP 统一规划时实施 |
| D8 | 授权边界仅连接可见性 + 外部 MCP 白名单；服务签名不带主体 | §6.4 | 暂缓 | Agent/MCP 统一规划时实施 |
| D9 | 刷新重绑四步（稳定标识→限定名→unresolved），无相似度猜测；hub 默认不作中转 | core `rebind_entity` + app `rebind_report` | 部分实现 | 重绑四步已落地+测（无第5步猜测；重建检测→needs_review；缺列→unresolved 不静默接）；hub 抑制未做 |

## 三、全库与局部 ER 探索（五）

| # | 需求 | 实现位置 | 状态 | 缺口 |
|---|------|----------|------|------|
| 1 | 数据库 ER 入口、当前表关联 ER 入口 | 侧边栏 ER 节点 + 表右键「关联 ER」 | 自动验证通过 | |
| 2 | 同一作用域重复打开复用标签；不替换数据/SQL 标签 | app dispatch 去重 | 自动验证通过 | |
| 3 | 全库图搜索定位表、切入局部图、返回保留视口 | `er_search_bar` + `er_center_on_table` | 部分实现 | 搜索定位已有；自动切入局部图/返回入口仍待补 |
| 4 | 局部图深度调整、逐节点展开、回到中心表 | 工具栏深度 + 展开按钮 | 部分实现 | |
| 5 | 显示范围、总表数、已加载、关系数、不完整状态 | 工具栏部分 | 部分实现 | 完成度/不完整状态 |
| 6 | 表列表与关系列表，键盘搜索/选择/打开详情 | `er_search_bar` + `er_search_dropdown` + `er_relationship_panel` | 部分实现 | 表搜索和本地关系列表已实现；键盘关系选择/打开详情仍待补 |
| 7 | 保留已打开表、设计表、新建查询能力 | 已有 | 通过 | |
| 8 | 无关系也能查看表，可加本地逻辑关系或请求建议 | `er_relationship_panel` + `er_relationship_form` | 部分实现 | 可查看并新建本地关系；Agent 建议入口未做 |
| 9 | 业务分组：schema/业务分组管理、折叠/展开、回全部 | er_group_bar + er_group_subset_graph + er_set_group | 部分实现 | schema 分组进入/返回全部；业务自定义分组与折叠态持久化未做 |
| 10 | 孤立表始终可搜索访问，不前 N 截断 | 已有全量可达 + 搜索 | 通过 | |
| 11 | 聚合边表关系数量，不伪装 JOIN | 分组内边为真实外键（两端在组内），不伪装 | 自动验证通过 | |
| 12 | 分组是可控功能，不默认隐藏大量表掩盖性能 | 无 | 未实现 | |

## 四、完整画布交互（六）

| # | 需求 | 实现位置 | 状态 | 缺口 |
|---|------|----------|------|------|
| 1 | 紧凑表卡、字段分列、真实短表高度、长表内部虚拟滚动 | er/node.rs | 自动验证通过 | |
| 2 | 正确表名/字段名/类型/已知约束 | er/node.rs | 自动验证通过 | |
| 3 | 明暗主题、主题色、图标、点阵背景 | UiColors + 纯色画布 | 部分实现 | 点阵背景未启用 |
| 4 | 文字省略有完整说明、长表底部不截字 | tooltip + 页脚 | 部分实现 | 待人工验收 |
| 5 | 字段端口连真实字段行中心、滚动同步 | er/scene.rs 锚点 | 自动验证通过 | |
| 6 | 正交圆角路由、稳定并行通道、自关联回路 | er/scene.rs | 自动验证通过 | |
| 7 | 相同表对多关系保持独立 | er/scene.rs | 自动验证通过 | |
| 8 | 障碍规避有预算，避免无限搜索/讽刺穿卡兜底 | er/scene.rs 外侧绕行 | 自动验证通过 | |
| 9 | 两端屏外但路径穿视口的边不提前裁掉 | er/scene.rs 折线包围盒 | 自动验证通过 | |
| 10 | hover/选中高亮路径、端点、两端字段或隐藏字段标记 | er/interaction.rs | 部分实现 | 需完整复核 |
| 11 | 展示约束名/来源/状态/完整字段配对/可证语义 | 关系说明浮层 | 部分实现 | |
| 12 | 复合关系完整展示，不把一对当整条 | er 边独立 | 自动验证通过 | |
| 13 | 同名约束按可靠身份区分 | ErTableRef/边独立 | 自动验证通过 | |
| 14 | 空白/Esc 取消、外部点击关闭、事件不穿透 | er/interaction.rs | 自动验证通过 | |
| 15 | 隐藏字段定位：上下方向+实际左右端口统计；多字段用组件菜单；选择滚入可见区并高亮 | hidden_field_scroll + 徽标菜单 | 部分实现 | 需复核多字段菜单 |
| 16 | Pending/Failed/Missing 明确原因和恢复入口；不伪装未知端口 | 加载状态行 | 部分实现 | |
| 17 | 表头拖单节点、空白拖画布 | er/interaction.rs | 自动验证通过 | |
| 18 | 拖动阈值、光标、选择反馈、事件隔离 | er/interaction.rs | 自动验证通过 | |
| 19 | 画布外释放、窗口失焦、切换/关闭标签清理拖动 | er/interaction.rs | 部分实现 | 待人工验收 |
| 20 | 固定节点保护、其他/新节点避让 | er_layout 保留 pinned；拖动 | 部分实现 | |
| 21 | 重排默认保留固定位置；显式重置至少恢复上次布局 | er_layout | 部分实现 | 恢复上次布局未做 |
| 22 | resize/主题/pan/字段滚动不重排 | 已实现 | 通过 | |
| 23 | 缩放、缩放比例、适配全部、适配当前范围、恢复视口 | ErViewport.scale + 工具栏 + er_fit_scope | 自动验证通过 | 视图变换缩放（卡片固定尺寸，仅坐标乘 scale）；适配当前范围已实现 |
| 24 | 缩放围绕鼠标/锚点，节点/文字/端口/线/命中一致 | zoom_around 锚点不动 + to_screen/to_world 统一 | 自动验证通过 | Cmd(mac)/Ctrl - 滚轮 + 工具栏 +/- |
| 25 | 小地图：分布/视口/导航，开销有界 | ErMinimap + er_minimap_view + er_minimap_center_world | 自动验证通过方向 | 右下角叠层：节点分布(聚合≤2000)+当前视口矩形；点击/拖动居中导航；节点多时采样有界。需人工视觉验收 |
| 26 | 缩小按层级隐藏细节 | 无 | 未实现 | 视图变换缩放未做层级隐藏 |
| 27 | 三平台滚轮/触控板/快捷键一致或明确平台适配 | secondary() 修饰键跨平台 | 部分实现 | Win/Linux 未实测 |
| 28 | 键盘与可访问性：焦点、表/关系选择、搜索定位、打开详情、关闭浮层、字段导航；不只用鼠标/颜色 | 部分键盘 | 部分实现 | 搜索定位未做 |

## 五、逻辑关系编辑与确认（七）

| # | 需求 | 实现位置 | 状态 | 缺口 |
|---|------|----------|------|------|
| 1 | 无物理外键创建本地逻辑关系 | `er_relationship_form` → `CreateErRelationship` | 待人工验收 | 已有真实 service/store 调用链；需人工操作确认 |
| 2 | 选两端表/字段、有序复合配对 | `ErRelationshipSelectOption` + `ErRelationshipPairControls` | 待人工验收 | 稳定 ID 保存、schema 限定名展示、换表清理校验已实现；修复「选表后字段下拉恒空」：字段加载结果同步回 `er_full_tables`，局部 ER 下选择不在图内表的字段也可选 |
| 3 | 编辑/删除/确认/拒绝/必要撤销 | `Update/Confirm/Reject/DeleteErRelationship` + 关系面板 | 待人工验收 | 更新、删除、确认、拒绝已接；删除含二次确认；撤销尚未单独提供 |
| 4 | 区分物理/人工/未确认建议 | `ErRelationshipOrigin` + `logic:` 画布边 + `er_relationship_effective` 过滤 | 部分实现 | 画布只投影「已确认且 validity=current」的逻辑关系，Proposed/Rejected/Stale/Unresolved/Invalid 不冒充实外键；物理边名 `fk_*`、逻辑边名 `logic:*` 可区分；局部邻域纳入有效逻辑关系使「仅逻辑关系相连」邻居可展开 |
| 5 | 候选默认不作为已确认 SQL JOIN | `ErReviewState::Proposed` + service usage | 自动验证通过 | 服务测试覆盖 proposed 不进入候选 |
| 6 | required_filters 结构化可验证表达 | `ErRelationshipFilterControls` + `ErRequiredFilter` | 待人工验收 | 表单只允许端点字段、固定操作符和常量字面量；复杂表达式未提供 |
| 7 | 修改前后可理解的状态与保存反馈 | `er_relationship_loading/errors` + 面板 | 待人工验收 | loading/提交中/失败/重试/修订冲突均有路径 |
| 8 | 物理外键只读展示；DDL 独立草稿/预览/明确执行 | `ErEnforcementKind` + service | 自动验证通过 | 本地关系操作不执行 DDL；物理外键仍只读 |

## 六、加载、刷新、错误、取消（八）

| # | 需求 | 实现位置 | 状态 | 缺口 |
|---|------|----------|------|------|
| 1 | 首次打开自动生成，无额外确认 | 已实现 | 通过 | |
| 2 | 先目录概览，再关系、字段详情按需 | er_catalog/er_service | 自动验证通过 | |
| 3 | 元数据能力分页/分批；PG 外键按能力批量 | 部分 | 部分实现 | PG 仍逐表 |
| 4 | 冷启动局部图优先中心表 | 部分 | 部分实现 | |
| 5 | 同范围请求合并、并发去重、消费者订阅 | er_catalog inflight | 自动验证通过 | 检查与 inflight 登记合并为单次加锁（并发去重回测：8 线程合并为 1 次读库）；新增逐 key 取消旗标，关闭标签/停止时置位，逐表读取中间停止、完成丢弃过期结果 |
| 6 | 限制并发，不无界每表任务 | 部分 | 部分实现 | |
| 7 | 手动刷新/配置化缓存过期/DDL 失效合并 | 手动刷新已实现 | 部分实现 | 缓存过期/DDL 未做 |
| 8 | 更新保留有效图/视口/人工布局/本地关系 | 部分 | 部分实现 | |
| 9 | 停止/继续有效；关闭最后消费者取消 | `er_columns_cancel` + TabClosed | 部分实现 | 关闭 ER 标签：取消该作用域在飞字段加载（置位旗标→逐表停止→丢弃结果，丢弃不写缓存）；阻塞式单次驱动调用无法即时中断（如实保留） |
| 10 | generation/连接修订/作用域校验覆盖所有异步写回 | 部分 | 部分实现 | 需复核 |
| 11 | 显示真实阶段/完成度/更新时间/不完整范围 | 部分 | 部分实现 | |
| 12 | 区分空库/无外键/权限不足/部分失败/已取消/连接不可用 | 部分 | 部分实现 | |
| 13 | 错误保留可理解原因，日志不泄露密码 | 部分 | 部分实现 | |
| 14 | 缓存有内存边界，关闭/连接修改清理 | 无界缓存 | 部分实现 | 无界待修 |
| 15 | 不承诺接口不具备的实时订阅/阻塞驱动取消 | 已如实 | 通过 | |

## 七、性能与后台布局（九）

测量 50/200/1843/10000 表。已有纯 CPU 探针（`er_probe_perf_scales`，ignored），非 UI FPS。

| # | 需求 | 实现位置 | 状态 | 缺口 |
|---|------|----------|------|------|
| 1 | 布局/场景构建移出 UI 线程 | 后台执行 | 自动验证通过 | |
| 2 | 任务代次保护、工作预算、进度、确定性降级 | 部分 | 部分实现 | |
| 3 | 避免 render 与字段需求重复全量 materialize | 已知每帧热点（物化 7.2ms） | 部分实现 | 增量路由缓存未做 |
| 4 | 自由坐标索引、路径索引、缓存增量维护 | ErNodeGrid + edge bands | 自动验证通过 | |
| 5 | 拖动/字段滚动只更新受影响几何；新障碍更新相关非邻接边 | 部分 | 部分实现 | |
| 6 | 不每帧全量遍历/重路由 | 已实现 | 通过 | |
| 7 | 不为长边/超大节点按覆盖面积无界复制索引项 | edge bands 有界 | 自动验证通过 | |
| 8 | 避免不必要主窗口重绘，按需隔离画布实体 | 需评估 | 未实现 | 画布独立实体 |
| 9 | 50/200/1843/10000 实测清单 | 仅布局 CPU 探针 | 部分实现 | 需完整测量 |

## 八、持久化、导入导出（十）

| # | 需求 | 实现位置 | 状态 | 缺口 |
|---|------|----------|------|------|
| 1 | 保存结构快照/本地关系/证据/视图/坐标/固定/分组/视口 | 无 | 未实现 | 依赖 D1-D9 |
| 2 | 再次打开/重启后恢复 | 无 | 未实现 | |
| 3 | 视图与关系模型分离，不复制独立目录 | 无 | 未实现 | |
| 4 | 数据版本/迁移/原子写入/失败恢复/损坏处理 | 无 | 未实现 | |
| 5 | 不存明文密码；不靠名称自动绑另一连接 | 无 | 未实现 | |
| 6 | 未保存/保存失败显示；退出按项目机制处理 | 无 | 未实现 | |
| 7 | FluxDB ER JSON 带 schema_version 导入导出 | `er_export_json` + `er_import_parse` | 部分实现 | 导出含 schema_version/连接绑定/坐标/固定；导入解析+校验已落地（round-trip 全还原 + 版本拒绝） |
| 8 | DBML/Mermaid/SVG 导出 | er/export.rs | 自动验证通过 | 已有并测 |
| 9 | 有损导出明确提示 | SVG/DBML 标注 + 导入 unresolved 边 | 部分实现 | SVG 展示型标注有损；导入时无法解析的边进 unresolved 列表（不静默丢弃） |
| 10 | 导入校验大小/结构/引用/版本/连接绑定；展示差异 | `er_import_parse` + `er_import_database_matches` + 菜单 | 部分实现 | 结构/引用/版本/连接绑定校验 + 差异（新增/缺失/未解析边）预览已落地；校验通过才允许后续应用 |
| 11 | 导入 confirmed ≠ 已批准；转义防注入 | — | 部分实现 | 当前 JSON 只含结构与物理边（无逻辑关系），故无“confirmed 自动批准”问题；转义与校验经 serde_json 结构化解析（非拼接） |
| 12 | 导入失败不破坏现有模型；重复导入明确 | `er_last_import` + 先校验后应用 | 部分实现 | 解析失败即报错、不改模型；`er_last_import` 暂存最近结果供后续应用；「应用到画布/视图」编排未做 |

## 九、ER 查询服务与 Agent/MCP（十一）

| # | 需求 | 实现位置 | 状态 | 缺口 |
|---|------|----------|------|------|
| 1 | 实体搜索/详情/邻域/有界 JOIN 路径 | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 2 | 稳定快照/修订、双向关系索引、分页/预算 | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 3 | 返回完整度/覆盖/未解析/来源 | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 4 | 完整保留复合配对/常驻条件/usage | core `ErJoinPlan` + `ErModelService::join_plans` | 部分实现 | 完整保留已落地（单测）；上节查询/路径未做 |
| 5 | 画布隐藏不影响模型查询 | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 6 | Agent 读结构化数据不经截图/不经 SQLite | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 7 | 内部复用应用服务；外部 MCP 用既有接入/授权边界 | 待核查现有 MCP | 暂缓 | Agent/MCP 统一规划时实施 |
| 8 | 建议关系显式触发，首次打开不自动调模型/外发 | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 9 | 工具输入验证；关系写入走本地确认 | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 10 | 未确认候选/失效关系不伪装可靠 JOIN | 无 | 暂缓 | Agent/MCP 统一规划时实施 |
| 11 | 核查现有 Agent/MCP 基础，优先复用 | 待核查 | 暂缓 | Agent/MCP 统一规划时实施 |

## 实施顺序（按依赖，可调）

1. ~~修复现有正确性、重试、身份、局部展开~~（工作区已大部分完成，需复核+补测）
2. 完成关系交互、精确定位、输入、布局保护（复核 + 补齐）
3. 后台计算、索引、刷新、取消、缓存
4. 缩放、小地图、搜索、分组探索
5. D1–D9 模型 + 持久化 + 逻辑关系编辑
6. 导入导出、ER 查询、Agent/MCP
7. 回归、性能、文档、人工验收

> 当前进度：已完成关系服务按作用域复用、后台 AppCommand/AppEvent CRUD/确认/拒绝/删除、关系列表面板、非模态新建/编辑表单（含复合字段配对和结构化常量条件）及本地逻辑边同步；字段缓存改为结构化身份、并发去重/取消链路、逻辑关系邻域与有效性过滤、ER JSON 导入解析/校验/差异预览已落地。仍未做：结构快照解析/重绑 UI（rebind 算法已测但未接入刷新，缺 DB 稳定标识 + 快照持久化）、导入「应用」编排、关系查询服务；Agent/MCP 按范围约束暂缓，后续统一规划；桌面视觉与交互仍需人工验收。

## 本轮新增（逻辑关系编辑闭环）
- 应用层：`AppController` 持有按 scope 缓存的 `ErModelService`，启动时注入 `FileStorage`；新增 `Load/Create/Update/Confirm/Reject/DeleteErRelationship` 命令及对应事件，桌面端通过后台 `Task` 调用。
- desktop：ER 画布右侧非模态关系面板，列表 loading/失败/重试/提交中/错误状态；确认、拒绝、删除、编辑均使用 expected revision，成功后同步同作用域所有 tab 的列表缓存和 `logic:` 画布边。
- desktop：删除增加二次确认，并明确只删除本地逻辑关系、不修改数据库物理外键；列表将稳定列 ID 解析为 schema 限定表名和字段名（无法解析时保留 ID 作为错误线索）。
- desktop：新建/编辑表单复用 `SelectState<SearchableVec>` 与 `InputState`；表选项显示 schema 限定名但保存稳定实体 ID，字段选项保存稳定列 ID；换表清空字段选择；支持完整有序复合字段配对。
- 验证：`cargo test -p fluxdb-app` 495 通过、5 项既有 PG 客户端下载测试受沙箱临时目录权限影响失败；`cargo test -p fluxdb-storage` 34 通过；`cargo check --workspace` 通过；macOS `cargo run -p fluxdb-desktop` 启动到主窗口。Windows/Linux 未验证。

## 本轮新增（持久化）
- `fluxdb-storage`：`ErViewScopeState`（group/pinned/positions）+ `FileStorage::load_er_view_states/save_er_view_states`（sqlite kv `er_views` key，JSON upsert），round-trip 单测。
- desktop：打开 ER tab 生成作用域 key（`connection_id:database:center_display`）并应用上次保存的分组/坐标/固定一次；分组变化、拖动结束、标签关闭时持久化。坐标/固定仅按展示名应用，表不匹配多余项忽略、缺失表回退布局（不靠名称跨连接误绑）。
- 分组折叠态：重启/重开同一 ER 作用域后保留之前分组与手动布局。

## 本轮新增（导出 + 键盘搜索）
- 搜索键盘导航（§五.6）：`er_search_bar` 挂 on_key_down——上/下移选择（`er_search_sel`）、Enter 定位选中表、Esc 清空关闭；下拉高亮当前选中项 + 匹配计数。
- ER 导出（§十）：工具栏「导出」按钮（AppIcon::Save）→ 菜单 JSON/DBML/Mermaid/SVG，生成文本写剪贴板 + show_message。均为纯函数（er/export.rs）单测覆盖；SVG 为展示型（表级连线，标注有损）；JSON 带 `schema_version` 与连接绑定，不含密码。

## 复合外键修复（§6.2）
- `ForeignKeyInfo` 增有序列对 `columns`/`ref_columns`（保持数据库原始序号）；PG `pg_foreign_keys_from_structure` 不再逗号拼接成不存在的单列「a, b」（旧行为会让 ER 连线指向错误列），改为保留有序列对 + 扁平首列兼容旧展示。
- ER 加载 `er_expand_fk_columns` 按有序列对整个外键逐列展开成边，复用约束名作身份（同表对多条约束区分）；列数不匹配回退单列（不 zip 错位）。
- 单测：`er_service_tests::*` 2 用例 + PG structure 映射保留有序列对。
- 验证：app 487 / connectors 225 / core 89 / storage 32 / desktop 138 全通过。

## 本轮已交付（画布高级交互）
- 缩放视图变换（§六.23-24）：`ErViewport` 增 `scale`；绘制 `world*scale+pan+origin`，命中逆变换 `/scale`，节点拖动增量 `/scale`；`zoom_around` 锚点世界点静止；`visible_world_bounds` 除以 scale。工具栏 -/百分比/+/适配当前范围；Cmd(mac)/Ctrl(win/linux) 滚轮围绕光标缩放。
- `er_fit_scope`：节点世界范围整体适配画布（留 32px 边距，居中），空图回起点。
- 表搜索/定位（§五.6）：per-tab `Entity<InputState>` + 订阅，搜索条按表名/注释（`ErNodeMeta` 增 `comment`）过滤；命中下拉浮层（画布后置兄弟叠加），点击 `er_center_on_table` 居中并选中。
- 业务分组（§五.9-12）：`er_group_bar` 按 schema 分组 chips；进入组用 `er_group_subset_graph` 过滤展示图（边仅两端都在组内者=真实外键），保留人工坐标重建场景不丢布局。纯逻辑测试覆盖。
- 小地图（§六.25）：右下角叠层 `ErMinimap` 自定义 paint Element，画节点分布（>2000 采样）+ 当前视口矩形；`er_minimap_center_world` 点击/拖动把世界点居中导航；`er_world_bbox` 与导航共用同一归一化映射。
- 回归测试：`er_zoom_*`、`er_search_text_matches_*`、`er_group_subset_*`；既有 ER 测试全通过。
- Tab 关闭清理泄漏：补 `er_canvas_origins/er_all_edges/er_center_refs/er_field_highlights/er_last_updated/er_refreshing` + `er_search_*` + `er_group*` + `er_minimap_rect`。
- 验证：`cargo check --workspace`、`cargo test -p fluxdb-desktop` 423 通过、`cargo build/run -p fluxdb-desktop` 启动到主窗口。

## 本轮新增（正确性修复 + 导入）
- 并发去重（§六.5）：`er_relations_core`/`er_columns_core` 的「缓存检查」与「inflight 登记」改为单次加锁原子完成，杜绝两个并发调用都判定缺失、各自重复读库。并发回归：8 线程并发 `er_relations_core` 只发生 1 次 DB 读取。
- 取消链路（§六.9/§八）：`ErCatalogCache` 增逐 key 取消旗标 `columns_cancel`；`AppController::er_columns_cancel` 置位在飞读取旗标，加载线程经 `should_cancel` 在默认逐表循环的「两表之间」停止后续工作，完成时丢弃该批结果（不写缓存、不标 Loaded/Failed）。桌面 `TabClosed` 时按 ER 作用域取消该标签在飞字段加载。阻塞式单次驱动调用无法即时中断（如实保留）。
- 结构化身份贯穿字段加载（§一.2）：`ErColumnKey` 改持 `ErTableRef`（database/schema/裸名），删除旧 `er_split_display`（`rsplit_once('.')` 会把含点表名如 schema `s`.表 `my.table` 误拆成 schema `s.my`+裸名 `table`）；`er_columns_for_tables/retry/invalidate/cancel` 改收 `&[ErTableRef]`；`read_columns_by_schema` 按真实 schema 逐组批量读取，结果按结构化身份归并（跨 schema 同名、含点标识符不串表不漏字段）。
- 逻辑关系语义（§二.3/§五.4）：新增 `er_relationship_effective`（review=Confirmed 且 validity=Current）过滤；`sync_er_local_relationship_edges` 只投影有效逻辑关系到画布，Proposed/Rejected/Stale/Unresolved/Invalid 不冒充实外键；局部 ER 邻域计算纳入有效逻辑关系，使「仅逻辑关系相连」的邻居可展开（`recompute_local_er_from_center` 与 `sync_er_relations` 均合入）。
- ER JSON 导入（§八.7-12）：`er_import_parse` 用 serde_json 校验 format/schema_version/结构/引用/连接绑定，返回差异（新增表/缺失表/未解析边，不静默丢），`er_import_database_matches` 处理跨库绑定；`er_export_menu` 增「导入 JSON」入口（读剪贴板→校验→绑定→差异预览→show_message），先校验后应用、失败不改模型。`er_last_import` 暂存最近导入供后续「应用」编排（应用未做）。
- 缩放（§六.23-24）：复核为文档记录的「视图变换缩放」（卡片固定屏幕尺寸、坐标乘 scale、命中/端口/拖动/小地图共用 `ErViewport::to_screen/to_world`），`er_zoom_transform_roundtrip_and_anchor_stable`/`er_zoom_out_reveals_more_nodes` 通过；非 100% 缩放时卡片重叠为已接受取舍（全自绘文字缩放未实施，待专门评估）。
- 结构刷新重绑（§二.4）：`rebind_report`/`rebind_entity` 四步算法已有并测，但**未接入实际刷新**——缺连接器稳定对象标识（如 PG attrelid）与持久化结构快照，无旧-新快照可比对，故本轮属实未接线（见剩余缺口）。
- 验证：`cargo check --workspace`；`cargo test -p fluxdb-app` 505 / `fluxdb-core` 104 / `fluxdb-storage` 34 / `fluxdb-desktop` 431 全通过（新增：并发去重、取消丢弃、含点表名结构化身份、逻辑关系邻域投影相关、导出导入 round-trip/版本拒绝/unresolved 边）；macOS `cargo build -p fluxdb-desktop` 通过，`cargo run` 启动到单实例守卫（已有实例在运行，未作为视觉验收，见人工清单）。

## 本轮新增（逻辑关系表单修复）
- 抽屉可拖宽：关系面板左缘增拖宽手柄（复用 `SidebarResizeStart`/`SidebarResizeDrag` 模式），宽度按 tab 记忆，钳制 280–560px（`er_relationship_panel_width`）。
- 选表后可选字段：字段加载结果同步回 `er_full_tables`（`flush_pending_columns` 写回时补齐 full 的 columns），局部 ER 下选择不在图内表的字段下拉不再恒空（`er_form_table_by_id` 从 full 读）。
- 基数选择：表单增基数下拉（1:1 / 1:N / N:1 / N:N / 未知），映射 `ErMatchCardinality`（用户选择时 basis=UserAssertion，未知则不声明）；新建默认未知、编辑预填当前值且可改。纯函数 `er_cardinality_to_option_id`/`er_option_id_to_cardinality` + 回归。
- 验证：`cargo test -p fluxdb-desktop` 432 全通过（含 `er_cardinality_option_roundtrip`）；fmt 干净。

## 本轮新增（关系交互/布局修复）
- 抽屉拖宽修复：拖宽手柄从面板首 child 移到**最末 child**（GPUI 后声明者在同层之上），避免被内容区覆盖点不到；抽成 `er_rel_panel_resize_handle`（280–560px，按 tab 记忆）。
- 点击逻辑边打开抽屉：`ErEdgeView` 增 `logical_rel_id`（由边名 `logic:{id}:{idx}` 提取，`er_logical_rel_id` 纯函数 + 回归）；interaction 命中逻辑边 → `er_select_relationship` 打开右侧抽屉并高亮该关系（列表卡片可点击选中/再点取消）；物理外键点击仍仅提示。
- 表单重排：分节（基础信息/字段配对/常驻条件），左/右表各占一行带标签、字段对两列表头对齐（`er_form_section_title`/`er_form_labeled_row`）。
- 验证：`cargo test -p fluxdb-desktop` 433 全通过（含 `er_logical_rel_id`）；fmt + `cargo check --workspace` 干净。

## 本轮新增（表单重排 + 刷新丢线修复）
- 新建关系表单按 `docs/mockups/local-relation.source.html` 重排：减弱灰底/边框，用细分隔线分区；标题行「新建本地逻辑关系 + 说明 + 关闭」；「基础信息」左表/右表并排（label 在上、窄宽退单列）、角色/说明各独占一行（说明标可选）；「关联字段」标题旁放整条关系基数下拉（一对多/多对一/多对多/未知，一对一），右侧「添加配对」，方向文案 `左表 → 右表 · 类型，所有字段配对共同生效`，字段对每行 `序号 | 左表字段 = 右表字段 | 删除`（等号用两条短横线组合，AppIcon 无 Equal）；「附加关联条件」（原常驻条件改名，可选）+ 辅助文案 + 空态；底部「仅保存本地逻辑关系」+「取消/创建关系」。
- 新增 `er_remove_relationship_pair`（至少保留一组），关联字段删除按钮、基础信息/分节/等号/字段助手。
- 刷新丢线修复：`sync_er_local_relationship_edges` 在字段未加载（刷新后列待加载）时用 column_id 末段占位仍投影逻辑边；字段加载完成后重投影用真实列名锚点。
- 面板拖宽手柄加宽至 10px + hover/active 反馈。
- 验证：`cargo test -p fluxdb-desktop` 433 全通过；fmt + `cargo check --workspace` 干净；macOS `cargo build` 通过、`cargo run` 到单实例守卫（表单拖宽/交互需人工确认）。

## 本轮新增（拖拽根因修复 + 布局/宽度）
- **拖拽根因**：连接栏 `connection_browser_resize_handle` 的 `on_drag_move` 无条件 `stop_propagation()`，且关系面板此前与连接栏共用 `SidebarResizeDrag` 类型 → 连接栏在 Capture 阶段抢走同类型拖拽事件，关系面板手柄拖不动。修：为关系面板新增独立拖拽类型 `ErRelationshipResizeDrag`（connection_state.rs），不再与连接栏互相抢占；补齐起点记录/持续更新/手柄内外松手清理（`on_mouse_up` 清 `er_relationship_panel_resize_start`）、面板关闭清理。
- **宽度**：默认 560 逻辑像素，拖宽上限 800，受 ER 内容区可用宽约束（`clamp_er_rel_panel_width`，窄窗收缩、下界 320、按 tab 记忆、窗口缩小重约束）；拖宽公式 = 起始宽度 + 起始 X - 当前 X。
- **布局**：去掉「外层面板 + 内层大卡片重复标题/关闭」，表单直接铺在面板内（面板 header 在表单打开时标题切为「新建本地逻辑关系」并承担关闭）；去掉表单内自标题/自关闭/大卡片边框，改为弱化说明行 + 分节分隔；左右表并排默认各半、窄宽退单列；控件占位改中文（`Select.placeholder` 控制未选中、`search_placeholder` 控制搜索框，两者分别设置）；未选表时字段/条件字段选择器禁用并提示「请先选择表」；附加条件空态紧凑带辅助说明；表单内容外层滚动、底部取消/创建始终可达。
- 验证：`cargo test -p fluxdb-desktop` 434 全通过（新增 `er_rel_panel_width_clamped_to_available_and_bounds`）；fmt + `cargo check --workspace` 干净；macOS `cargo build` 通过、`cargo run` 启动到主窗口（拖拽/布局交互需人工确认）。

