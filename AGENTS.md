# AGENTS.md

本文件适用于整个仓库。

## 代码约定
- 修改代码时避免以临时补丁、局部兜底、硬编码特殊 case 的方式处理；先结合整体调用链、分层职责和已有实现判断根因，再选择最小且正确的修改位置。
- 在关键分支、异常返回、外部调用和异步/批量处理等适当位置补充日志；复杂规则或不直观实现需要添加简洁中文注释。
- 可以使用 TDD 推动复杂逻辑或 bugfix，但不要为简单改动机械套用 TDD。
- 开发过程中可以为了 TDD 或梳理逻辑临时封装小方法；代码完成后必须复查函数拆分是否合理，合并能合并的小方法，避免过度封装和碎片化函数。
- 不允许使用openspec和superpowers

## 项目结构

- `apps/fluxdb-desktop/`：GPUI 桌面 UI。
- `crates/fluxdb-core/`：领域模型、错误类型、Connector 接口。
- `crates/fluxdb-app/`：应用状态、命令、事件。
- `crates/fluxdb-connectors/`：数据库连接实现。
- `crates/fluxdb-storage/`：本地配置和连接持久化。
- `docs/design`：PRD、架构、详细设计和任务拆解。

## 文件和文件夹拆分规则

- `main.rs` / `lib.rs` 只保留 crate 入口职责：imports、模块声明、re-export、启动/bootstrap 和少量 glue code；不要继续把新功能直接堆进入口文件。
- 当前部分大文件已用 `include!` 做第一阶段物理拆分，保持 crate-root 作用域以降低重构风险；后续修改时应优先在现有职责文件内改动，并逐步迁移为真实 `mod` 模块边界。
- 新增 UI 功能优先放在 `apps/fluxdb-desktop/src/main_parts/` 下按页面或交互职责归类；如果一个职责继续变大，应新建子目录，例如 `data_table/`、`dialogs/`、`menus/`、`sidebar/`。
- 新增 App 层逻辑优先放在 `crates/fluxdb-app/src/parts/` 下，按 `state`、`controller`、`data_editor`、`query_completion`、`sql_format`、`table_info` 等职责拆分。
- 新增 Connector 能力优先放在 `crates/fluxdb-connectors/src/parts/` 下，按数据库类型或共享 SQL/数据转换能力拆分；不要把 MySQL、SQLite、MongoDB、Redis 的实现混进同一个新增文件。
- 新增 Core 类型优先放在 `crates/fluxdb-core/src/parts/` 下，按领域概念拆分；通用模型不要放进 UI 或 connector 文件。
- 单个源码文件超过约 800 行时应优先评估拆分；超过约 1200 行且仍要新增功能时，必须先拆出清晰职责文件再继续实现。
- 拆文件时优先做纯机械移动并保持行为不变；行为修改和结构拆分尽量分开提交/分开说明，方便 review。
- 真正模块化时要显式处理可见性，避免为了绕过编译把大量类型改成 `pub`；优先使用 `pub(crate)` 和小范围 helper。

## 开发前必读

- 修改代码前先看同层已有实现，优先复用现有模型、命令、组件和样式。
- 不要在本项目中使用 Superpower 或 OpenSpec 工作流；除非用户在当前请求中明确要求。

## 分层约定

- UI 层只负责渲染和交互，不能直接连接数据库、读写配置文件、保存密码或拼接数据库 SQL。
- UI 操作通过 `AppCommand` 进入 `fluxdb-app`，状态变化通过 `AppState`/`AppEvent` 表达。
- 通用领域类型放在 `fluxdb-core`，不要把 UI 类型传进 core/app/storage/connectors。
- 持久化放在 `fluxdb-storage`，连接实现放在 `fluxdb-connectors`。

## UI 约定

- 所有弹框必须支持按 `Esc` 取消/关闭。
- 所有弹框、浮层、下拉菜单必须支持点击外部区域关闭；内部点击必须阻止事件穿透到后面的内容层。
- 每次新增或修改前端 UI 时，必须同时考虑明亮主题和暗黑主题，不要写死只适用于单一主题的颜色。
- `gpui-component` 是本项目桌面 UI 的默认控件层，不是可选的样式偏好。新增或修改 UI 前必须先检查 `gpui-component 0.6.0` 是否已有对应组件，并优先复用现有组件和项目内已有封装。
- Dialog、Button、Input、Select、Tabs、TabBar、Dropdown、PopupMenu、ContextMenu、Toast、Form、Table、DataTable、TextView、Sheet、Scrollable、Slider、Switch、Radio、Tree、List、Tooltip、Spinner、Progress 等通用控件必须使用 `gpui-component`；不得因自定义样式、尺寸或点击回调简单而重新用 `div()` 实现。
- 使用 `gpui-component::Button` 时默认调用 `.rounded_md()`，保持按钮统一的中等圆角；只有明确的图标按钮或组件语义要求时才例外。
- 表单输入框样式统一优先使用 `gpui-component::Input` 并放在自定义外框内：外框高度约 34px，宽度按表单语义稳定约束；`rounded_md`、`border_1`、`colors.input_bg`；默认边框使用 `colors.border`，hover 使用主题适配的增强边框，focus 明亮主题使用 `rgb(0x111111)`、暗黑主题使用 `rgb(0x8ab4ff)`，且 hover 不得覆盖 focus 边框；内部 Input 使用 `appearance(false)`、`focus_bordered(false)`、`w_full()`、`h_full()`，字号 13px 左右，保证光标垂直居中。
- 只有在 `gpui-component` 没有对应组件，或组件经过验证确实无法满足当前交互/布局语义时，才可以使用 GPUI 原生 `div()` 自行组合；普通布局容器可以使用 `div()`，但按钮、菜单项、弹框、抽屉、Tab、下拉框、选择器、开关和列表项不得仅以布局容器代替组件。自行组合时必须记录原因，并确保颜色走 `UiColors`。
- 不得复制或新增与现有 `Button`、`Input`、`Select`、`Checkbox`、`Popover`、`DataTable` 等组件功能重复的本地控件。现有自绘控件改动时，应优先评估迁移到 `gpui-component`，具体范围见 `docs/2026-09-04-gpui-component-ui-migration.md`。
- 新增或修改功能图标必须使用统一的 `AppIcon` / `app_icon(...)` / `app_icon_box(...)`，图标资源使用 lucide 风格 SVG；禁止直接用文字字符（如 `×`、`⌄`、`↻`、`⧉`）作为按钮或菜单图标，除非没有合适 SVG 且不是功能操作图标。
- 所有可点击按钮和按钮式控件必须显示手形光标，使用 `cursor_pointer()`，并提供 hover 状态；组件支持时同时提供 active 状态。
- 右键菜单应保持紧凑宽度，菜单文字使用略重字重；带二级菜单时二级浮层必须贴齐主菜单右侧边缘，不要出现明显水平缝隙或垂直错位。
- 新增弹框时必须同时接入关闭按钮、遮罩点击策略、`Esc` 快捷键和主题色。
- 所有需要请求数据、访问数据库、读写文件或可能耗时的操作必须显示 loading/disabled 状态；数据展示区发起刷新时也要有明确 loading，不要阻塞 UI 渲染路径。
- 轻量操作反馈（例如复制成功、保存成功、删除成功）统一使用主窗口全局 `show_message(...)`；不要直接散落 `push_notification` 或临时自绘提示。Message 为单槽替换式：新提示出现时直接替换旧提示并重新计时。

## 数据和安全

- 不要把明文密码保存到连接配置文件；配置里只保存 `credential_ref` 或非敏感选项。
- 数据库操作、连接测试、对象加载要通过 connector/app 层，不要从 UI 直接调用驱动。

## 验证

- Rust 代码改动后至少运行 `cargo fmt` 和 `cargo check`。
- 修改 core/app/storage/connectors 的逻辑后运行相关测试；不确定影响范围时运行 `cargo test`。
- UI 行为改动需要保证主窗口仍可通过 `cargo run -p fluxdb-desktop` 启动。


## 会话默认模式

- 每次开启新会话时，如当前 Agent 环境支持 `ponytail` skill/规则，则默认以 `ponytail` 沟通模式（默认 `full` 强度）工作；如不支持该 skill，则无需报错或额外说明，仍按同等原则保持回答极简、直接，并保留完整技术信息。ponytail 持续生效于整个会话，且通过 Agent 工具派生的子代理同样注入该规则集。
- ponytail 的原则：只在任务真正需要时写代码，绝不顺手过度构建。决策按阶梯依次停在第一个能满足诉求的档位——1) 这个功能真的需要存在吗（YAGNI，不需要就跳过）；2) 仓库里已有就复用，不重写；3) 标准库能做到就用标准库；4) 平台原生能力能做到就用原生能力；5) 已安装的依赖能做到就用依赖；6) 一行能解决就写一行；7) 然后才是「最小可用实现」。
- ponytail 是「懒于解法，从不懒于阅读」：动手前必须先读完本次改动涉及的相关代码并追踪真实数据流，在理解清楚之后再选择最简方案；信任边界校验、错误处理、数据丢失防护、安全与可访问性永不在砍掉之列，代码因「必要」而短，而非因「炫技」而短。
- 仅在用户显式要求切换 ponytail 强度（`/ponytail lite`、`full`、`ultra`、`off`）、`normal mode`，或当前回复涉及安全告警、不可逆操作确认、以及过度精简会造成技术歧义时，才允许临时退出或调整；临时退出后应在风险说明结束后恢复极简风格。
