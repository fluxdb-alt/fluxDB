# PostgreSQL 用户与权限界面改版方案

日期：2026-09-15  
状态：**已实现（本期范围），实现记录见文末 §11。**  
依据：用户提供的 PostgreSQL/MySQL 两张截图、当前仓库实现，以及既有 PostgreSQL 详细设计 §12。

## 1. 设计结论

采用图二的「统一工具栏 + 左侧身份列表 + 右侧五页签 + 对齐表单」结构，将 PostgreSQL 页面调整为 **用户与角色** 工作台。沿用 MySQL 页面的视觉节奏和保存习惯，字段、权限范围、成员关系和执行逻辑按 PostgreSQL 建模。

右侧页签固定为：**常规 / 高级 / 成员关系 / 权限 / SQL 预览**。新建、改名、改密均在右侧完成，移除横跨页面的临时表单；所有编辑先进入草稿，再通过顶部「保存」提交。

本次方案不要求修改 MySQL 页面，也不引入新的通用权限框架。优先复用已有组件、PG 领域模型和连接器能力。

## 2. 当前问题与代码依据

| 当前表现 | 已核对的实现 | 改版要求 |
| --- | --- | --- |
| PG 列表出现 `角色名@`，新建项出现 `@%` 和 `caching_sha2_password` | App 将 `PgRole` 转为 `DatabaseUserIdentity`；共用列表和草稿渲染按 MySQL 拼标签 | PG 状态直接保留角色数据；列表和草稿只显示角色名及 LOGIN/NOLOGIN |
| 新建、重命名、密码表单占据顶部并推动整个内容区 | `pg_role_admin_content` 动态插入创建/编辑表单 | 统一进入常规页，保持导航和内容位置稳定 |
| 登录开关立即执行，授权/撤销也是单项操作 | `pg_role_login_toggle`、PG 权限面板直接派发执行命令 | 全部改为草稿，保存一次提交；避免与 MySQL 保存习惯冲突 |
| 成员关系只有组名文本 | `pg_groups_for_member` 把完整成员记录压成字符串 | 使用已有 `PgRoleMembership`，呈现方向和成员选项 |
| 无成员角色存在错误路径 | `load_user_admin_grants` 在 PG 组名为空时继续进入 MySQL provider 路径 | PG 空结果直接作为成功空态，不能回退 MySQL |
| 权限目标依赖手填 schema、对象、函数签名，所有字段同时出现 | `pg_role_privileges_panel` | 使用按对象类型变化的选择器；从元数据取得签名 |
| 权限种类切换值得优先修正 | 当前种类按钮仅在 `selected` 分支绑定点击事件 | 验收所有非选中种类都可切换 |
| `postgres` 与 `pg_*` 被统一判为只读 | 工具栏和 LOGIN 开关按名称判断 | 区分预定义角色、当前会话身份、普通管理员角色及实际操作权限 |

上述为源码可见的设计/实现问题，不代表本次已运行数据库复现。图一中的加载状态也不能仅凭截图认定为死锁。

## 3. 必须保留的数据库差异

| 维度 | MySQL 界面语义 | PostgreSQL 设计 |
| --- | --- | --- |
| 身份 | 账号以 `user@host` 区分 | role 为集群级身份；LOGIN 允许登录，NOLOGIN 常用于权限分组。两者都能拥有对象、授予权限、参加成员关系 |
| 主机与认证 | Host、认证插件是账号相关设置 | 不显示 Host、MySQL 插件或 SSL REQUIRE 字段；客户端来源和认证由 `pg_hba.conf` 等服务端配置控制 |
| 密码期限 | DEFAULT、EXPIRE NOW、INTERVAL 等策略 | `VALID UNTIL` 是密码认证的有效截止时间，不是账号有效期；不影响非密码认证 |
| 登录控制 | ACCOUNT LOCK/UNLOCK | LOGIN/NOLOGIN；NOLOGIN 不终止已有会话 |
| 管理能力 | 全局权限等 | SUPERUSER、CREATEDB、CREATEROLE、REPLICATION、BYPASSRLS 等角色属性，不放进对象授权矩阵 |
| 资源限制 | 每小时查询/更新/连接限制 | 本期只提供 `CONNECTION LIMIT`；不虚构 PG 不支持的同名限额 |
| 成员关系 | 可包含默认角色选择 | PG 使用角色成员关系；无 MySQL「默认角色」列。ADMIN、INHERIT、SET 按服务端版本展示 |
| 授权范围 | 数据库/表等层级 | 数据库权限不等于其内所有表权限；schema USAGE、表权限、序列权限分别管理 |
| 特殊来源 | 账号与角色授权 | 明确区分直接 ACL、成员继承、PUBLIC、属主和超级用户；PUBLIC 不作为可登录角色 |
| 默认权限 | 不照搬 MySQL 操作 | `ALTER DEFAULT PRIVILEGES` 针对指定创建者以后创建的对象，不修改现有对象 |

NOLOGIN 与 LOGIN 不是两种互斥的业务身份类型；「登录角色」「组角色」只作为易读提示，不限制它们参与成员关系的方向。

## 4. 页面结构与视觉规范

### 4.1 常规页线框

```text
┌ 用户与角色  [开发环境] [PostgreSQL]        [保存] [刷新] [删除] ┐
├─────────────────────┬────────────────────────────────────────┤
│ 搜索角色名称    [+]  │ 常规  高级  成员关系  权限  SQL 预览    │
│ [全部角色      ▾]   ├────────────────────────────────────────┤
│                     │ app_user  [可登录] [集群级]             │
│ app_user            │                                        │
│ 可登录              │ 角色名        [app_user             ]  │
│                     │ 允许登录      [开]                     │
│ readonly            │ 密码操作      [保持不变            ▾]  │
│ 不可登录            │ 密码有效期    [永不过期            ▾]  │
│                     │                                        │
│ pg_monitor          │ 允许登录仍受服务端认证配置约束。        │
│ 预定义 · 不可登录    │                                        │
├─────────────────────┴────────────────────────────────────────┤
│ 12 个角色 · 3 个可登录                     未保存的更改       │
└──────────────────────────────────────────────────────────────┘
```

布局参考 MySQL，但数值按 GPUI 逻辑像素实现，不直接套用截图物理像素：

- 顶部工具栏高约 44px；左侧沿用现有约 320px 宽度，窄窗口可收至 260px。创建入口仅保留搜索框旁的加号。
- 搜索占一行，筛选占紧凑次行；筛选为全部、可登录、不可登录、预定义。默认展示全部，搜索覆盖全部角色，避免系统角色被误认为不存在。
- 列表项约 48–56px，名称为主行、身份为弱化次行；选中背景走主题色。长名称省略并提供完整 Tooltip。
- 页签高约 36px；详情内边距 20–24px。表单标签宽约 120px，字段宽约 360–480px，输入高约 34px，行间距约 16px。减少横向拉满输入框和大面积阴影。
- 常规表单保持单列，权限表格利用剩余宽度；窄窗口标签可置于输入上方，内容独立滚动，工具栏和页签固定可见。
- 明暗主题统一走 `UiColors`；危险操作使用主题危险色。状态同时使用文字/图标，不能只靠颜色区分。

### 4.2 组件落地

已确认本机 `gpui-component 0.6.0` 有 List、Select、Table/DataTable、Dialog 等对应组件。实现时复用该组件库的 Button、Input、Switch、Checkbox、Tab/TabBar、Tooltip、Spinner 和 TextView；普通容器才使用 `div()`。左侧旧自绘列表应迁移到 List/ListItem，不能直接复制其问题样式。

输入复用项目外框规范：`appearance(false)`、`focus_bordered(false)`、约 13px 字号；focus 边框优先于 hover。Button 默认 `.rounded_md()`，可点击控件提供手形、hover 和组件支持的 active；图标统一使用 AppIcon 的 lucide SVG。

弹框必须支持关闭按钮、Esc、外部点击关闭，内部阻止穿透。关闭确认框等于取消待执行动作，不丢弃底层草稿。轻量反馈统一 `show_message(...)`。

## 5. 五个页签的内容

### 5.1 常规

| 字段 | 交互与规则 |
| --- | --- |
| 角色名 | 创建时必填；保留 catalog 大小写，不静默转小写。现有角色改名进入同一草稿；当前会话用户不能重命名，由后端校验 |
| 允许登录 | Switch；新建默认开启，关闭即 NOLOGIN。未知/加载失败不能默认显示开启 |
| 密码操作 | 现有角色默认「保持不变」；可选「设置新密码」「清除密码」。新建默认「不设置密码」，可切换设置；不设置并不保证能使用密码认证登录 |
| 新密码 / 确认密码 | 仅设置新密码时显示，两次一致才可保存；支持显隐。不读取、不回填旧密码 |
| 密码有效期 | 「永不过期」或带明确时区的截止时间；保存为服务端可解析的绝对时间。清除已有截止时间必须生成显式 `infinity`，不能用“不修改”代替 |

NOLOGIN 时密码字段默认折叠，允许展开管理已有密码，不因关闭 LOGIN 自动清除密码或有效期。空白密码输入不等于清除密码；清除使用独立操作并预览 `PASSWORD NULL`。

重命名可能使旧 MD5 密码失效，连接器/应用层应给出重设密码提示，且不读取密码散列来实现 UI；普通 SCRAM 场景也不能承诺所有认证行为保持不变。

### 5.2 高级

分为「角色能力」和「连接限制」两组，采用紧凑开关行，每行中文名称、SQL 属性和一句解释：

- 创建数据库（CREATEDB）、创建与管理角色（CREATEROLE）。提示 CREATEROLE 不代表可任意管理所有角色，能力受版本、目标和 ADMIN 等规则约束。
- 继承权限（INHERIT）。PG 15 及以前控制自动继承；PG 16+ 是新成员授权的 INHERIT 默认值，不能把它当成已有成员关系的总开关。
- 超级用户（SUPERUSER）、复制能力（REPLICATION）、绕过行级安全（BYPASSRLS）放在独立的敏感能力组，不默认开启。
- 连接数限制：不限或非负整数；不限对应 -1。说明它针对普通连接，非精确限流器，超级用户等有例外。

预定义 `pg_*` 角色不提供属性编辑/删除，但仍能查看权限和按授权能力管理成员。`postgres` 通常是初始化超级用户的名称，不是通用的「不可修改」判据。当前身份、目标属性、版本及服务端授权共同决定可执行操作；无法确认能力时不伪装为已授权，执行以服务端判定为准并保留错误说明。

### 5.3 成员关系

页内两组表格：「所属角色」和「此角色的成员」。例如选中 app_user，在所属角色中添加 readonly，含义是 `GRANT readonly TO app_user`。反向表格展示 app_user 授予了哪些成员；LOGIN 角色也允许出现在两侧。

| 列 | 说明 |
| --- | --- |
| 角色名 / 成员名 | 可搜索选择，排除自身；后端校验循环及合法性 |
| 已授予 | 只编辑直接成员关系；间接关系放在只读详情中 |
| 管理成员（ADMIN） | 能否向其他角色授予该成员资格；不等于对象的 GRANT OPTION |
| 自动继承（INHERIT） | PG 16+ 的成员级开关 |
| 允许切换（SET） | PG 16+，能否通过该成员关系 SET ROLE |

PG 15 及以前不显示可编辑的成员 INHERIT/SET 列，提示「继承由角色属性控制」。不能将连接器补的 `true` 当成实际可编辑字段，更不能据此断言 NOINHERIT 角色自动继承。

仅调整 ADMIN/INHERIT/SET 时修改该选项，不通过先撤销整个成员关系再授予来实现。撤销 ADMIN 的依赖失败展示服务端详情，不静默 CASCADE。若服务端存在不同 grantor 的成员记录，读模型须保留来源，编辑只作用于当前可管理记录，不能合并布尔值后覆盖。

### 5.4 权限

使用「对象选择区 + 权限表格」，将现有手填表单换成元数据导航：

```text
数据库 [appdb ▾]   对象类型 [表 ▾]   Schema [public ▾]
对象   [orders                                      ▾]
当前目标：appdb / public.orders        属主：app_owner

权限       直接授予     可再授权     当前有效     来源
SELECT        □            □           是        readonly（继承）
INSERT        ☑            □           是        app_user（直接）
UPDATE        □            □           否        —
```

选择完整目标后自动异步读取；保留刷新入口，不再要求每次点击「读取权限」。Database 类型只显示数据库；Schema 类型不显示对象名；函数/过程由列表展示服务端规范签名，区分重载，不能要求用户手写参数签名。表、视图、物化视图根据真实种类标注。

| 对象 | 基础权限集合 |
| --- | --- |
| 数据库 | CONNECT、CREATE、TEMPORARY |
| Schema | USAGE、CREATE |
| 表 / 视图 | SELECT、INSERT、UPDATE、DELETE、TRUNCATE、REFERENCES、TRIGGER；按对象种类及版本限制可用项 |
| 序列 | USAGE、SELECT、UPDATE |
| 函数 / 过程 | EXECUTE |

较新版本的额外权限（如 PG 17+ 的 MAINTAIN）通过能力表扩展，不能对旧版本生成未知关键字。拥有某权限也不代表视图一定可更新、物化视图可直接写入。

权限表遵守以下规则：

1. 「直接授予」「可再授权」是草稿；「当前有效」是已读取的服务端状态，保存前不冒充已生效。可另标「待新增/待撤销」。
2. 来源可以有多个：直接、成员继承、PUBLIC、属主、超级用户。现有 `effective/direct` 两个布尔值不足以准确归因；未补齐来源查询前显示「其他来源，待展开核实」，不能一概写“继承”。
3. 撤销直接授权后可能仍有有效权限。对来源为 PUBLIC/成员角色的条目提供定位入口，不在当前角色里自动撤销公共或组授权。
4. GRANT OPTION 单独编辑；取消它使用 `REVOKE GRANT OPTION FOR`，不撤销基础权限。存在下游授权时默认 RESTRICT，展示依赖冲突，不自动级联。
5. 属主身份带来不可撤销的管理权，但属主也能撤销自己的普通对象权限；不能把所有 owner ACL 一律禁用，也不能强制把所有普通权限显示为真。以原生权限检查和 ACL 为依据。
6. ACL 为 NULL 表示使用对象类型的默认 ACL，不等于空授权；例如数据库、函数可能存在默认 PUBLIC 权限。对象当前默认 ACL 与「未来对象默认权限」是不同概念。
7. 原生 `has_*_privilege` 只能说明相应对象权限，不保证某条 SQL 成功：还可能需要数据库 CONNECT、schema USAGE、序列权限，或受 RLS 限制。表格旁保留简短说明，按需提示缺失前置权限，不自动补授。

**作用域**：角色/成员关系影响整个集群；schema、关系、函数 ACL 必须在选中数据库连接中读写。权限页始终显示数据库，不静默使用连接默认库。第一期一次编辑会话只保留一个数据库的对象授权草稿；切换数据库若有权限草稿，先保存或放弃，避免跨库提交被误当成原子操作。

**本期范围**：支持单个现有对象的读取和授权差异编辑；可以依次编辑同一数据库中的多个对象。列级授权本期只明确提示未覆盖，不能把列级 ACL 误判为表级无权或覆盖清除。PUBLIC 不进入角色列表，也不在本期提供公共权限批量编辑。

**后续范围**：未来对象默认权限单独设计，必须同时选择「数据库、对象创建者、可选 schema、对象类型、受权角色」，解释仅对该创建者未来创建的对象生效，不自动使用其所属组的默认权限。对现有全部对象的批量 GRANT 是另一操作，不能与默认权限合并。本期不展示不可用的空壳编辑页。

### 5.5 SQL 预览

显示本次所有页签草稿的差异 SQL，附连接、数据库、影响角色和变更摘要；无改动时显示「暂无待执行变更」。SQL 只读，使用 PostgreSQL 高亮。

预览与执行使用同一结构化变更计划和连接器渲染规则；UI 不拼接 SQL，也不调用 MySQL provider。密码语句显示明确的脱敏占位符并标记「脱敏预览，不能直接执行」，复制也不含明文密码；实际执行从临时秘密字段取值，不反向执行预览文本。

示例（无密码的可读变更）：

```sql
ALTER ROLE "app_user" LOGIN CONNECTION LIMIT 20;
GRANT "readonly" TO "app_user";
GRANT SELECT ON TABLE "public"."orders" TO "app_user";
```

## 6. 创建、保存、删除与异常行为

### 创建

点击加号 → 左侧生成「新角色 · 待创建」草稿项 → 常规页聚焦角色名。角色名留空，避免自动误建 `new_user`；可以先设置其他页签。成员关系和权限都引用草稿身份，保存时先创建角色再应用其余变更。取消创建仅清理本地草稿。

### 保存

工具栏保存覆盖所有页签，未修改/加载失败尚无可靠基线/输入无效/正在提交时禁用。修改页签显示未保存标记；Cmd/Ctrl+S 与按钮一致。

普通变更直接提交；删除、授予敏感能力、影响当前登录身份等操作在应用内给出具体影响确认。服务端执行成功并重新读取后清除草稿，使用全局消息反馈。

PG 同一数据库会话中的角色 DDL 与本期 GRANT/REVOKE 变更使用单连接事务。**现有每个操作独立连接的接口不能直接循环调用来承诺原子性**：需新增批量执行边界，失败回滚整批，保留草稿和字段错误。执行顺序为创建/改名 → 属性与密码 → 成员 → 对象授权；改名后所有操作引用新身份。执行结果不确定（例如提交时断线）显示「结果待核实」，先重新读取，不能直接重试。

切换角色、刷新、关闭页面遇到草稿时提供保存/放弃/取消；切换页签不触发确认。保存前重读相关基线，发现外部修改则提示重新核对，不整表覆盖授权；数据库仍是最终并发与授权判定方。

### 删除

仅对已存在且可删除的角色启用，确认框显示角色名、集群影响及可能的对象依赖。服务端拒绝时展示依赖摘要；不自动执行 DROP OWNED、REASSIGN OWNED 或 CASCADE。依赖可能跨数据库，单库预检不能承诺已检查整个集群。保护当前会话身份，预定义角色不提供删除；不以字符串 `postgres` 代替权限判断。

### 加载与失败

- 角色列表、详情、成员、对象权限分别有 loading/error/empty 状态；失败不显示为“无权限/无成员”。错误区支持重试，旧数据可保留但标记未刷新。
- 请求带连接、数据库、角色、目标和请求代次；切换后丢弃过期响应，避免 A 角色权限写到 B 角色页面。
- 数据库访问、元数据查询和提交均异步，不阻塞 GPUI 渲染；提交中禁用重复动作。
- 日志记录操作类型、作用域、耗时与脱敏错误；密码和连接秘密不进入日志、普通 SQL 历史、保存查询或错误原文。

## 7. 实现分层与能力缺口

| 层 | 复用内容 | 必要调整 |
| --- | --- | --- |
| Core | `PgRole`、`PgRoleMembership`、`PgObjectGrantScope`、`PgObjectGrants`、`PgEffectivePrivilege` | 按需要补充数据库上下文、能力信息、来源/grantor、明确的密码操作和变更类型；避免把 secret 放入通用序列化读模型 |
| App | 现有 AppCommand/AppEvent、加载与错误反馈机制 | PG 专属 baseline/draft/dirty/save 状态；直接保留 PgRole，停止空 host 转换；以结构化计划组织预览和应用 |
| Connector | PG 角色 CRUD、属性修改、成员查询/授权、ACL 和原生权限检查 | 同会话事务执行、元数据目标枚举、准确版本语义、GRANT OPTION 单独撤销、密码清除、来源读取、数据库路由 |
| UI | MySQL 页面结构、项目表单封装、gpui-component | 共用纯布局与组件；PG 常规/高级/成员/权限/预览分别渲染，不共用 MySQL 身份草稿及 SQL |
| Storage | 现有连接配置 | 本功能不增加密码或授权草稿持久化 |

`pg_user_admin.rs` 当前约 923 行，`user_admin.rs` 约 2604 行，App `state.rs` 也是大文件。实施前先机械拆出清晰职责，避免继续追加：建议 PG UI 放在 `main_parts/pg_user_admin/`，按页面拆分；App PG 状态和控制器放在 `parts/` 对应职责文件。入口只保留模块/include 声明和必要 glue；结构移动与行为修改分开说明。

不重写已有 PgRole 和连接器，不把 MySQL 与 PG 强行合成一份字段齐全的万能表单。实现前进一步检查组件 API 与同层相似代码，确认复用位置。

## 8. 分阶段交付

1. **整理基础与身份**：机械拆分相关职责；PG 状态保留完整角色，修正列表/草稿文案、空成员结果、LOGIN 未知态和类型切换。保持 MySQL 行为。
2. **建立保存闭环**：先完成 PG 草稿、差异计划、脱敏预览、单会话事务及失败保留；再接入工具栏和常规/高级页。不可只换成五页签却保留部分开关即时写库。
3. **成员与权限完整交互**：版本化成员选项、元数据目标选择、明确数据库、直接权限与来源展示、单独撤销授权选项。完成所有在界面中宣称支持的操作。
4. **验收视觉与错误路径**：明暗主题、键盘、窄窗口、慢查询、权限不足、断线和外部变更回归。通过后才视为本期完成。

未来对象默认权限、跨库批量授权、列级权限编辑和对象所有权转移另立后续需求，不作为本次对标 MySQL 外观的前置条件。

## 9. 验收清单

- [ ] PG 页面不出现系统拼接的 `@host`、`@%`、MySQL 插件、DEFAULT 密码策略或默认角色列；真实角色名含 `@` 时原样显示。
- [ ] 新建不改变整体布局；五页签与顶部保存一致，任意开关/勾选在保存前都不写库。
- [ ] LOGIN/NOLOGIN 都能参与两向成员关系；PG 15 与 PG 16+ 的继承和 SET 行为正确，敏感能力按授权失败可解释。
- [ ] 空成员、无搜索结果、读取失败、无直接授权、ACL NULL 均有不同表达。
- [ ] 同库不同 schema 同名对象、跨库同名对象、重载函数及带引号/Unicode 名称定位正确；切换目标不残留旧权限。
- [ ] 直接、PUBLIC、继承、owner、superuser 来源不会被混为可直接撤销的授权；撤销直接权限后仍有效时显示正确。
- [ ] GRANT OPTION 与 ADMIN OPTION 分开；取消再授权不误删基础权限；未覆盖的列级 ACL 不被覆盖。
- [ ] 密码保持/设置/清除含义明确，期限重置生成 infinity；预览、复制、历史、日志、错误均无秘密泄露。
- [ ] 单批事务中途失败无半完成状态；结果不确定先核实；重试、迟到响应、外部修改和丢弃草稿均验证。
- [ ] 删除有跨库依赖的角色只报告失败，不自动处理对象；预定义角色保护与普通管理员权限判断正确。
- [ ] 明暗主题、键盘焦点、Esc、外点关闭、手形/hover、窄窗口、长列表和 loading 均通过人工验收；MySQL 页面无回归。

实现时 Rust 改动至少运行 `cargo fmt`、`cargo check`；Core/App/Connector 逻辑执行相关测试，范围不清时运行 `cargo test`；用 `cargo run -p fluxdb-desktop` 验证主窗口启动。数据库集成测试使用专属测试角色/数据库，覆盖 PG 15 与 PG 16+，额外版本权限使用相应版本验证；无测试环境时明确记录未运行。

本次仅新增设计文档，不修改 Rust 或数据库，不将上述实现验收记为已通过。

## 10. 参考

仓库依据（相对本文件）：

- [PG 详细设计 §12](design/2026-09-09-postgresql-detailed-design.md)
- [MySQL 用户页面与共用列表](../apps/fluxdb-desktop/src/main_parts/user_admin.rs)
- [当前 PG 页面](../apps/fluxdb-desktop/src/main_parts/pg_user_admin.rs)
- [App 用户管理加载和授权](../crates/fluxdb-app/src/parts/user_admin.rs)
- [PG 身份与权限模型](../crates/fluxdb-core/src/parts/postgres_identity.rs)
- [PG 连接器](../crates/fluxdb-connectors/src/parts/postgres/user_admin.rs)

实施时按目标服务端版本复核官方语义：

- [CREATE ROLE](https://www.postgresql.org/docs/current/sql-createrole.html)
- [角色成员关系](https://www.postgresql.org/docs/current/role-membership.html)
- [PG 15 角色成员关系](https://www.postgresql.org/docs/15/role-membership.html)
- [GRANT](https://www.postgresql.org/docs/current/sql-grant.html)
- [ALTER DEFAULT PRIVILEGES](https://www.postgresql.org/docs/current/sql-alterdefaultprivileges.html)
- [DROP ROLE](https://www.postgresql.org/docs/current/sql-droprole.html)

官方链接用于实施复核；本次未进行在线逐条核验，不能据此宣称所有目标版本已验证。

## 11. 实现记录（2026-09-15）

### 11.1 完成项

**Core（`crates/fluxdb-core`）**
- 新增 `parts/postgres_user_admin.rs`：`PgRoleDraft`（新建/编辑草稿，密码 `PgPasswordOp`、有效期 `PgValidUntilOp`）、`PgRoleChange`（Create/Rename/AlterAttributes/SetPassword/Grant·RevokeMembership/Grant·Revoke·RevokeGrantOption 八类结构化变更）、`PgRoleSavePlan`（单库作用域变更计划）、`PgGrantTargetLists`、`pg_role_attributes_diff`（基线 diff 纯函数）。`PgPasswordOp`/`PgRoleChange` 的 `Debug` 手工实现，密码恒脱敏（`Set(********)`）。
- Connector trait 新增 `render_role_plan`（脱敏/真实双态渲染）、`apply_role_plan`（单会话单事务应用）、`list_grant_targets`（权限目标枚举）、`supports_member_options`（PG16+ 版本探测）。

**Connector（`crates/fluxdb-connectors/src/parts/postgres/`）**
- `pg_render_role_plan`：按 Create→Rename→属性/密码→成员→对象授权稳定排序渲染；成员选项语句按服务端版本（PG16+ 才有 INHERIT/SET/ADMIN FALSE）；清除有效期显式 `VALID UNTIL 'infinity'`；密码 literal 专属转义；GRANT/REVOKE 的 grantee 一律双引号引用；`mask=true` 输出 `PASSWORD '********'` 占位。
- `pg_apply_role_plan`：单连接 `BEGIN → batch_execute → COMMIT`，任一语句失败 `ROLLBACK` 整批（与 apply_changes 同口径），返回脱敏审计语句。
- `pg_list_grant_targets`：数据库 / schema / 表·视图·序列（按 relkind 分类）/ 函数·过程（`pg_get_function_identity_arguments` 签名区分重载），排除系统 schema。

**App（`crates/fluxdb-app`）**
- `UserAdminState` 新增 PG 工作台字段：`pg_roles`（权威 `PgRole` 列表，PG UI 不再用空 host 身份）、`pg_selected_role`、`pg_draft`、`pg_memberships`（双向）、`pg_membership_edits`/`pg_grant_edits`（草稿变更）、`pg_grant_database`/`pg_grant_targets`、`pg_save_status`（Idle/Saving/NeedsVerify）、`pg_plan_preview`（脱敏）+ 脱敏标记、`pg_member_options_supported`、`pg_loaded_target`（对象权限过期指纹）等；删除 `pg_edit_mode`/`pg_rename_new`/`pg_can_login`/`pg_role_login` 等即时写库时代的字段。
- 移除即时写库命令：`SetPgRoleLogin`、`SetUserAdminPgCanLogin`、`BeginUserAdminPgRename/Password`、`SetUserAdminPgRenameNew`、`EndUserAdminPgEdit`、`GrantUserAdminPgPrivilege`、`RevokeUserAdminPgPrivilege`。
- 新增命令：`SelectPgRole`/`SetPgRoleSwitchPending`/`PgCancelSwitchRole`/`DiscardPgDraftAndSelect`（有草稿切换确认）、`PgBeginCreateRole`/`PgCancelDraft`、`SetPgDraft*`（名称/LOGIN/六项布尔属性/连接数/有效期/密码操作与明文）、`Start/Load/FinishUserAdminPgRolesLoad`、`Start/Load/FinishPgMembershipsLoad`（含版本探测）、`PgMembershipGrant/Revoke/RemoveEdit`、`SetPgGrantDatabase`、`Start/Load/FinishPgGrantTargetsLoad`、`PgToggleGrant`/`PgRemoveGrantEdit`、`Start/Load/FinishPgPlanPreview`、`StartPgPlanApply`/`ApplyPgRolePlan`/`FinishPgRolePlanApply`、`PgBeginDeleteRole`/`PgCancelDeleteRole`、`SetPgRoleFilter`。
- `build_pg_role_plan`：草稿 diff 出计划；改名后成员/授权草稿统一改写为最终身份；连接数/角色名/密码校验失败不产生半份计划。失败保留草稿；结果不确定标记 NeedsVerify（提示刷新核实，不直接重试）。
- 修复：`load_user_admin_grants` PG 空成员结果不再回退 MySQL provider（空即成功空态）；权限种类按钮全部可点击（旧版仅选中项绑定事件）。

**Desktop UI（`apps/fluxdb-desktop/src/main_parts/pg_user_admin/`，新目录 6 文件）**
- 统一工具栏：用户与角色 + 连接徽标 + PostgreSQL 徽标；右侧保存（无草稿/保存中/密码不一致/新建无名时禁用）、刷新（加载中禁用）、删除（预定义/未选中/新建禁用）；NeedsVerify 提示条。
- 左栏 320px：搜索 + 加号 + 全部/可登录/不可登录/预定义筛选；角色行主行角色名（含 `@` 等字符原样）、次行「可登录/不可登录/预定义 · …」；脏角色「未保存」角标；新建草稿行「新角色 · 待创建」（角色名留空待填）；loading/error/empty + 重试。
- 右侧固定五页签：常规（角色名/允许登录 Switch/密码操作三选/新密码+确认/有效期三选+自定义时间输入/MD5 改名提示/NOLOGIN 折叠提示/预定义只读提示）、高级（CREATEDB/CREATEROLE/INHERIT + 敏感组 SUPERUSER/REPLICATION/BYPASSRLS + 连接数限制，INHERIT 提示按 PG15/16+ 版本语义区分）、成员关系（所属角色/此角色的成员双向表 + ADMIN/INHERIT/SET 列按版本显隐 + PG15「继承由角色属性控制」提示 + 草稿变更清单可撤销）、权限（数据库/种类/schema/对象选择器来自元数据，目标完整自动读取，直接授予/可再授权为草稿勾选，当前有效/来源为服务端状态，其他来源标注「PUBLIC/继承/属主，待展开核实」，ACL NULL 显式标注默认权限，GRANT OPTION 单独撤销）、SQL 预览（脱敏渲染 + 「脱敏预览，不能直接执行」标注 + 变更摘要，复用 SQL 高亮代码视图）。
- 删除确认框（集群影响 + 依赖只报告不自动处理 + Esc/外点关闭）；切换角色确认框（确认丢弃草稿）；底部状态条（N 角色 · M 可登录 · 未保存标记）。
- 旧 `pg_user_admin.rs`（923 行）删除，职责拆入新目录；`render.rs` 快照、`app_state.rs` 输入句柄、`app_boot.rs` 订阅同步更新；MySQL 页面行为不变。

### 11.2 验证命令与结果

| 验证 | 结果 |
| --- | --- |
| `cargo fmt --all` | 通过（无 diff 残留） |
| `cargo check --workspace` | 0 error |
| `cargo test -p fluxdb-app` | 430 passed / 0 failed |
| `cargo test -p fluxdb-core`（含新属性 diff/脱敏 Debug 测试） | 全部通过 |
| `cargo test --workspace` | fluxdb-connectors 174 passed / **2 failed**：`pg_build_table_ddl_round_trips_clauses`、`pg_create_database_sql_builds_options_and_quotes` —— **已在干净 HEAD（a1e53b8）验证同样失败，属预存在问题**（建库 TEMPLATE 子句与表 DDL 生成列断言），与本次改动无关，未在本期顺手修改 |
| 新增计划渲染单测（顺序/脱敏/版本选项/infinity/CREATE 密码转义/非法名拒绝） | 4 passed |
| **真实 PG16（本机 fluxdb-t09-pg 容器，postgres:16.15）**：`pg_apply_role_plan_rolls_back_as_a_whole`（坏语句→整批回滚，角色不存在） | passed |
| **真实 PG16**：`pg_apply_role_plan_end_to_end_and_rename`（建角色+密码+属性→成员→对象授权→改名→list_roles/list_role_membership/role_effective_grants 核实→清理） | passed |
| `cargo run -p fluxdb-desktop` 主窗口启动 | 启动正常（两次冒烟） |
| 测试遗留清理 | 测试角色/表已从容器清除 |

### 11.3 剩余限制与未验证项（如实记录）

1. **PG15 未实测**：本机仅有 PG16 容器。PG≤15 的成员 INHERIT/SET 隐显与选项语句省略逻辑已按版本分支实现并有单测覆盖渲染差异，但未在真实 PG15 上运行验证。
2. **人工视觉验收未做**（无视觉能力）：明暗主题配色、列表/表单对齐（标签 120px/输入 360px/行距）、页签高度 36px、长名称省略 Tooltip、窄窗口标签换行、手形/hover/焦点态，需人工检查；代码层已按规范走 `UiColors`/gpui-component/Button(Input/Switch/Select)/AppIcon。
3. **成员关系选项编辑**：ADMIN/INHERIT/SET 以「新授权默认值 + 草稿清单 + SQL 预览核对」方式覆盖；既有成员关系的选项级修改（不撤销重建）未提供独立编辑入口；不同 grantor 的多份成员记录合并展示，编辑作用于直接记录。
4. **来源归因粒度**：effective && !direct 统一显示「其他来源（PUBLIC/继承/属主），待展开核实」，未提供逐来源（PUBLIC vs 成员继承）定位跳转；`has_*_privilege` 前置权限（CONNECT/USAGE/RLS）缺失提示未做。
5. **单库授权草稿**：切换数据库即清空目标缓存与授权草稿（未提供「保存或放弃」二次确认弹框）。
6. **列级权限、未来对象默认权限、跨库批量授权、所有权转移**：按计划留待后续，未提供空壳入口。
7. Cmd/Ctrl+S 保存快捷键沿用全局 `save_or_apply` 路径，未为 PG 页单独验证。
8. 预存在失败测试 2 项（见 11.2）建议另立修复任务。
