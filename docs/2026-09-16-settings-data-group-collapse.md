# 设置「数据」页分组折叠改造（2026-09-16）

## 背景

设置 → 数据页原本是纯垂直堆叠：数据表组、备份组（4 个路径行）、PostgreSQL 客户端组、MySQL 客户端组，页面过长且信息层级不清。

## 方案

- 备份、PostgreSQL 客户端、MySQL 客户端三个分组改为**可折叠分组**：标题整行可点击，ChevronRight/ChevronDown 指示方向。
- 默认状态：备份展开（常用配置），两个客户端组折叠。
- 客户端组折叠时标题右侧显示状态摘要：`下载中… > 检测中… > 已检测 > 未检测`，不展开也能看到关键状态。
- 状态存 `NavicatMain.settings_data_groups_collapsed`（`SettingsDataGroupsCollapsed`），UI 瞬时态，不持久化，与编辑器页「危险 SQL 操作清单」折叠状态同一模式。
- 追加：`pg_dump 路径`、`mysqldump 路径` 两行从「备份」组迁入对应的 PostgreSQL/MySQL 客户端组（`dump_path_row`），同一库的覆盖配置与自动发现配置归拢一处；「备份」组只留备份目录与 sqlite3 路径（SQLite 无客户端组）。
- 追加：去掉客户端组底部常驻的 `install_hint` 静态文案（检测失败时状态行已显示同一句，常驻属于重复提示）；折叠标题摘要区分「已检测 / 未找到」，检测失败不再误显示「已检测」。
- 追加：客户端目录行未设置时，占位文案显示本机默认搜索路径摘要（`pg_client_default_dirs_summary` / `mysql_client_default_dirs_summary`，取已存在的系统目录前 3 条，无则显示「默认搜索路径:系统 PATH」），让用户直接看到留空时程序会去哪里找。

## 为什么不用 gpui-component 的 Accordion

gpui-component 0.5.x 的 `Accordion`/`AccordionItem` 展开状态是组件内部 RenderOnce 临时态（`Rc<RefCell<HashSet>>`），设置页每次重渲染会重建组件导致展开状态丢失；外部受控需要靠 `on_toggle_click` 快照重建，复杂度高于沿用仓库已有折叠模式。`Collapsible` 结构体只是无标题无交互的哑容器，不满足需求。

## 涉及文件

- `apps/fluxdb-desktop/src/main_parts/settings/system.rs`：`SettingsDataGroupsCollapsed`/`SettingsDataGroup` 定义、`settings_collapsible_group`（可折叠分组构造器）、`settings_panel_group_el`（分组标题支持任意 element）、`settings_data_panel` 按折叠状态条件渲染子行。
- `apps/fluxdb-desktop/src/main_parts/settings/native_client.rs`：`settings_native_client_group` 接收 `collapsed`，折叠时提前返回不渲染内部行（含下载源输入框同步），标题挂状态摘要。
- `apps/fluxdb-desktop/src/main_parts/settings/navigation.rs`、`content_views.rs`：`settings_content`/`settings_panel_body` 透传折叠状态。
- `apps/fluxdb-desktop/src/main_parts/app_state.rs`、`app_boot.rs`：新增字段与默认值。

## 验证

- `cargo check -p fluxdb-desktop`、`cargo fmt` 通过。
- `cargo run -p fluxdb-desktop` 主窗口正常启动。
