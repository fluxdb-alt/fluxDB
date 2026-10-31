# UI 样式规范

适用于新增和修改桌面 UI，是根目录 [AGENTS.md](../AGENTS.md) 的专项细则。修改 UI、页面 AI 上下文或 GPUI 依赖前阅读相关章节；验证范围见根文件“测试与验收”。

## 组件与依赖
- 修改前核对 `Cargo.lock` 和已有封装。当前锁定 `gpui-component 0.7.1`、`gpui-kit 0.7.1`、`gpui-pre 0.3.8`；以实际发布版接口为准，不照搬上游 main。
- 通用控件必须复用组件，不能用 `div()` 替代或新增重复本地控件；仅当组件缺失或经验证无法满足语义时自行组合，并记录原因。普通布局容器可用 `div()`。
- `actions!`、`#[derive(Action)]` 等 GPUI 生态宏由 `gpui-component` / `gpui-base` 的过程宏经 `proc_macro_crate` 解析，优先 `gpui-kit`，其次回退 `gpui-pre`。`gpui-kit` 必须是生产依赖；只放开发依赖会导致普通构建展开出无法解析的 `::gpui_kit::…`。
- macOS 需要在 `gpui-platform` 显式开启 `runtime_shaders`：`gpui-base 0.6.x` 曾无条件开启，0.7.x 改为使用方自选；不开启则 build.rs 调用 `xcrun metal`，把构建机强制绑定到完整 Xcode。
- GPUI 升级须额外运行 `cargo tree -d` 确认没有重复 GPUI 来源，且 macOS、Windows、Linux 原生环境各完成一次 `cargo check`；不能用单平台或跨目标编译替代三端验证。

## 主题、交互与反馈
- 颜色统一走 `UiColors`，兼容明暗主题；图标使用 `AppIcon` / `app_icon(...)` / `app_icon_box(...)` 与 lucide 风格 SVG，不用文字字符代替。
- 弹框接关闭按钮、`Esc` 和遮罩关闭；弹框、浮层、下拉菜单支持外部点击关闭，内部点击阻止穿透。
- 按钮及按钮式控件提供 `cursor_pointer()`、hover，组件支持时提供 active；具体圆角、输入框与菜单样式见后续章节。
- 数据请求、数据库和文件操作等耗时任务显示 loading/disabled，不阻塞 UI；数据区域刷新也要明确 loading。
- 轻量反馈统一使用主窗口 `show_message(...)`，单槽替换并重新计时，不散落 `push_notification` 或自绘提示。

## 当前页面 AI 上下文
- 新增页面或功能时检查是否接入，并向用户说明 AI 可见的定位、结构、文本、数据与状态，以及未接入/不可读内容。
- 页面尚未接入时先询问是否需要接入，确认前不得静默扩大读取或上传范围。
- 复用当前页面引用协议，按内容选择全文、摘录或结构化摘要，明确截断、版本和续读能力；不另造上下文协议。
- 密码、API Key 等敏感配置始终禁止进入上下文；业务数据值只能经有界只读工具按需读取，不得塞进首条消息。

## 按钮
- `gpui-component::Button` 默认使用 `.rounded_md()`；明确的图标按钮或组件语义要求可例外。

## 表单输入框
- 使用 `gpui-component::Input`，放在自定义外框内；外框高度约 34px，宽度按表单语义稳定约束。
- 外框使用 `rounded_md`、`border_1`、`colors.input_bg`，默认边框为 `colors.border`，hover 使用主题适配的增强边框。
- focus 边框：明亮主题 `rgb(0x111111)`，暗黑主题 `rgb(0x8ab4ff)`；hover 不得覆盖 focus。
- 内部 Input 使用 `appearance(false)`、`focus_bordered(false)`、`w_full()`、`h_full()`，字号约 13px，保证光标垂直居中。

## 详情页页签
- 用户与权限面板（MySQL / PostgreSQL / SQL Server / Oracle）的右侧页签条统一由 `user_admin_detail_tab_strip` 构建：
  高 36px、`flex_1` 等分、文字居中、页签间 `border_r_1`。
- 不要按文字宽度撑开，也不要用固定 `px_3` 让总宽超出窄窗口；两者都会让同一功能在不同库间宽度参差、末页签被裁掉。
- 每个页签传稳定英文 key（`general` / `member-of` / `privileges` / `sql-preview` 等）作 `debug_selector` 后缀，key 不进语言包。
- 页签集合按库独立：`UserAdminDetailTab` 是四个枚举（`UserAdminMySqlTab` / `PgUserAdminTab` /
  `SqlServerUserAdminTab` / `OracleUserAdminTab`）的包装，不再五库共用一个。给某一库加页签只改
  它自己的枚举与面板，其它库的穷尽 `match` 不受影响。Oracle / 达梦是五页：常规、角色、
  系统权限、对象权限、SQL 预览（`system-privileges` / `object-privileges`）。
- 面板按库分派只经 `user_admin_panel_for(kind)` 一个分支点（`main_parts/user_admin/shared.rs`
  里的 `UserAdminPanel` trait），公共壳里不再写按库的 `if` 链。
- 切换时的额外加载（拉授权、生成预览）放在调用方给的回调里，不进共用组件。
- “保存”在所有库里都是**两段式**：点保存先弹出统一的 SQL 确认框，展示本次要执行的语句，点“执行”才提交。五种库（MySQL / PostgreSQL / SQL Server / Oracle / 达梦）共用同一个确认框，内容就是各库“SQL 预览”页同一份渲染结果（`admin.pending_sql`）。
- 表单只做草稿，不为单个库另建“加入变更 / 待执行列表”这类中间步骤。
- 不要为某个库单独改保存语义——五种库的保存与确认流程必须一致。确认框确认后的执行路径由 `UserAdminPendingKind` 区分（各库计划构建与提交方式不同）。
- “SQL 预览”内容框是 `flex_1`，必须放进高度受约束的 `flex_col` 容器（自己带背景和内边距），
  不能套 `oracle_panel_shell` 这类 `overflow_y_scrollbar` 外壳：滚动层内部高度由内容撑开，
  `flex_1` 拿不到确定高度会坍缩成 0，用户只看到一条空白细线。相关用例断言真实高度而不只是
  元素存在。

## 终端选区（Redis CLI / MongoDB Shell）
- 鼠标拖选必须挂在**窗口级** `window.on_mouse_event`（在自定义 Element 的 `paint` 里注册，命中区在 `prepaint` 用 `insert_hitbox` 插入）。元素自身的 `on_mouse_move` 只在指针悬停时回调，拖出终端边界就收不到事件，选区会冻结。
- 按下只记录锚点，拖动超过 2px 才算拖选；单击不留可见选区，也不覆盖剪贴板。
- 选区是**线性区间**（首行锚点列→行尾、中间整行、末行→端点列），不是矩形。高亮与复制必须用同一套区间判定，否则看到的高亮范围和复制出来的文本对不上。复制要按真实字符取（`row_real_chars`），不能按 cell 下标切：宽字符占两格、第二格是填充，下标切片会把 `中文` 复制成 `中 文`。
- 选区底色带 alpha 且画在字形**之前**；不透明色画在文字之上会把选中的字整块盖掉。输入行不要再单独铺整行背景，否则会盖掉选区。亮色主题 `rgba(0x2f6fbf4d)`、暗色主题 `rgba(0x2f5d9e66)`。
- 拖出可视区上下边界时按 16ms 一拍自动滚动，并同步搬选区行号（选区存视口坐标，内容滚动后同一段文字会换行号）。滚到顶/底或松开左键即停。
- 指针在窗口外松开可能收不到 `MouseUp`：下一次“无按键移动”要负责收尾，避免拖选状态卡住。
- 连击按 `MouseDownEvent.click_count` 分派：1 次=拖选、2 次=选词、3 次及以上=选行。词边界判定在 core（`select_word_at`，空白与常见标点视作分隔符，宽字符整字选中），桌面端只负责把点击次数传进去。双击/三击是固定区间，不进入拖选状态。
- 右键菜单用 `gpui-component` 的 `.context_menu(...)`：它自己挂窗口级右键、按窗口坐标定位、外部点击/Esc 关闭并归还焦点，不要自建弹层与遮罩。菜单项按“是否已有选区”变化：无选区时先把指针下的词选中（右键复制这个词），已有选区则保留用户选的内容；`Copy` 无选区时置灰。
- 键位与菜单项共用同一个 helper 不等于两条路径都通：菜单点击走 deferred 菜单实体和它自己的焦点/键盘导航，必须单独覆盖。菜单元件没有 debug selector，无头用例用菜单自己的导航驱动（`selected_index` 起始为 `None`，第一次 `down` 选第 0 项，之后跳到下一个可点击项，分隔符不参与），见 `terminal_menu_tests.rs`；目标 redis 取 `FLUXDB_REDIS_HOST` / `FLUXDB_REDIS_PORT`，探测不可达时跳过。
- 控制键语义按真实 REPL 对齐（node/mongosh 实测）：`Ctrl+C` 只回 `\r\n` 再重画提示符，**放弃的输入留在旧行**；`Ctrl+U` 就地清空当前行，回 `\r` 后重画。两者不能共用一个只回 `\r` 的分支（Ctrl+C 就不换行），也不能在 Ctrl+C 分支里加 `\x1b[K`（会把旧行擦成空白，用户以为输入被吞了）。进程内 REPL 没有信号语义，`0x03` 不能返回空切片——组件层收到空切片就不写后端，分支永远收不到。用例必须同时断言“提示符下移”和“旧行文字仍在”。

## 右键菜单
- 保持紧凑宽度，文字使用略重字重。
- 二级浮层贴齐主菜单右侧边缘，避免明显水平缝隙或垂直错位。

## 贴控件弹层（筛选/排序）
- 弹层位置由触发点的窗口坐标决定（`MouseDownEvent::position` → `anchored().position(...)`），
  不要写死 `top`/`left`：写死后控件滚动、换行或面板变高时弹层会浮在原处，指不到被点的字段/条件/值。
- 统一带 `snap_to_window_with_margin` 避让窗口边缘，并包 `deferred(...).with_priority(2)`
  让它画在表格与筛选面板之上。
