struct NavicatMain {
    ai_chat: AiChatUi,
    query_files: QueryFilesUi,
    focus_handle: FocusHandle,
    controller: AppController,
    storage: FileStorage,
    theme_mode: ThemeMode,
    update_ui: UpdateUiState,
    update_auto_check_pending: bool,
    update_cancel: Arc<AtomicBool>,
    update_downloaded_bytes: Arc<AtomicU64>,
    update_total_bytes: Arc<AtomicU64>,
    _update_check_task: Option<Task<()>>,
    _update_download_task: Option<Task<()>>,
    _update_progress_task: Option<Task<()>>,
    _update_install_task: Option<Task<()>>,
    new_connection_kind: Option<DatabaseKind>,
    new_connection_tab: NewConnectionTab,
    new_connection_form: NewConnectionForm,
    new_connection_inputs: NewConnectionInputs,
    /// 模型新建/编辑弹框；`None` 表示未打开（T013）。
    ai_model_dialog: Option<AiModelDialog>,
    /// 插件「管理」弹框；`None` 表示未打开。字段来自插件自己的配置声明。
    ai_plugin_config_dialog: Option<AiPluginConfigDialog>,
    /// 调试用：首帧后自动打开第一个可配置插件的管理弹框（仅 debug 构建读取）。
    /// 调试用：请求打开插件的「管理」弹框；空串表示"第一个可配置插件"。
    ai_plugin_config_dialog_debug_pending: Option<String>,
    /// 弹框内「获取模型列表」面板的搜索框（弹框关闭后保留实体但清空文本）。
    ai_model_dialog_filter_input: Entity<InputState>,
    _ai_model_dialog_filter_subscription: Subscription,
    ai_model_dialog_filter: String,
    /// 调试用：首帧后自动打开模型弹框（仅 debug 构建读取）。
    ai_model_dialog_debug_pending: bool,
    /// 「大模型」页搜索框与过滤文本（T013）。
    ai_model_filter_input: Entity<InputState>,
    _ai_model_filter_subscription: Subscription,
    ai_model_filter: String,
    /// 正在测试连接的模型 id（行内显示「测试中…」）。
    ai_model_testing_id: Option<String>,
    ai_model_deleting_id: Option<String>,
    /// AI 设置列表的加载态与错误态（设计 §39.5：错误不能被空列表掩盖）。
    ai_settings_loading: bool,
    ai_settings_error: Option<String>,
    /// 「工具与 MCP」页签当前停留的子页。
    ai_tools_tab: AiToolsTab,
    /// 本地 MCP 客户端注册状态快照（Codex/Claude Code），渲染期只读。
    ai_mcp_install_status: Vec<fluxdb_app::McpInstallClientStatus>,
    ai_mcp_clients_busy: Option<String>,
    ai_mcp_clients_error: Option<String>,
    ai_mcp_connection_details: Option<fluxdb_app::McpConnectionDetails>,
    ai_mcp_connection_error: Option<String>,
    ai_mcp_details_open: bool,
    /// 注册时使用的 `fluxdb-mcp` 二进制路径覆盖（测试注入；生产留空走 bundled_mcp_binary）。
    ai_mcp_binary_override: Option<String>,
    /// 「Agent 配置」编辑草稿；`None` 表示未选中任何 Agent。
    ai_agent_editor: Option<AiAgentEditorState>,
    /// 调试用：首帧后自动打开 Agent 新建草稿（仅 debug 构建读取）。
    ai_agent_editor_debug_pending: bool,
    /// Agent 刚被删除时置位；下一帧自动选中剩余列表的第一项。
    ai_agent_delete_just_removed: String,
    /// 「会话历史」页的搜索框与过滤文本（按标题过滤）。
    ai_session_filter_input: Entity<InputState>,
    _ai_session_filter_subscription: Subscription,
    ai_session_filter: String,
    /// 「安全审计」页签的筛选与搜索（仅作用于当前快照，不查库）。
    ai_audit_filter: Option<fluxdb_app::AiAuditDecision>,
    ai_audit_filter_input: Entity<InputState>,
    _ai_audit_filter_subscription: Subscription,
    ai_audit_filter_text: String,
    /// 展开查看详情的审计行 id。
    ai_audit_expanded: Option<String>,
    /// 正在归档的会话 id（行内 disabled + loading）。
    ai_session_archiving_id: Option<String>,
    /// 产物报告弹框（T040）：只读快照，dialog builder 在渲染期同步读取共享 RefCell。
    ai_report_modal: Rc<std::cell::RefCell<AiReportModalData>>,
    /// 打开产物报告的读快照任务；保持到完成，避免后台任务被提前丢弃。
    _ai_report_task: Option<Task<()>>,
    /// 报告导出（选择路径 + 安全写文件）任务。
    _ai_report_export_task: Option<Task<()>>,
    /// AI 面板「诊断」弹框：只读快照 + 加载/错误态 + 展开行。
    ai_diagnostics_modal: Rc<std::cell::RefCell<AiDiagnosticsModalData>>,
    /// 诊断读取任务；保持到完成，避免后台任务被提前丢弃。
    _ai_diagnostics_task: Option<Task<()>>,
    /// AI 关系建议逐条审阅弹框。
    er_proposal_modal: Rc<std::cell::RefCell<ErProposalModalData>>,
    /// 关系建议采纳任务；保持到完成，避免后台任务被提前丢弃。
    _er_proposal_task: Option<Task<()>>,
    /// 会话重命名弹框的输入与待保存对象。
    ai_session_rename_input: Entity<InputState>,
    _ai_session_rename_subscription: Subscription,
    pending_ai_session_rename: Option<AiSessionRename>,
    /// 调试用：首帧后自动对第一个模型发起测试连接（仅 debug 构建读取）。
    ai_model_test_debug_pending: bool,
    new_connection_password_visible: bool,
    editing_connection_id: Option<ConnectionId>,
    rename_group_input: Entity<InputState>,
    _rename_group_subscription: Subscription,
    table_folder_rename_input: Entity<InputState>,
    _table_folder_rename_subscription: Subscription,
    sidebar_search_input: Entity<InputState>,
    _sidebar_search_subscription: Subscription,
    sidebar_search: String,
    // 连接浏览器树：虚拟化滚动句柄。滚动偏移/视口保留在句柄内跨帧不掉；
    // 可见行列表只在连接树输入变化时重算，表格滚动期间复用纯数据缓存。
    sidebar_tree_scroll: VirtualListScrollHandle,
    sidebar_tree_cache: std::cell::RefCell<Option<SidebarTreeCache>>,
    field_filter_search_input: Entity<InputState>,
    _field_filter_search_subscription: Subscription,
    field_filter_search: String,
    field_filter_popover: Option<TabId>,
    visible_table_fields: BTreeMap<TabId, BTreeSet<String>>,
    data_filter_value_input: Entity<InputState>,
    _data_filter_value_input_subscription: Subscription,
    data_filter_batch_input: Entity<InputState>,
    _data_filter_batch_input_subscription: Subscription,
    local_filter_value_input: Entity<InputState>,
    _local_filter_value_input_subscription: Subscription,
    local_filter_search_input: Entity<InputState>,
    _local_filter_search_subscription: Subscription,
    data_filter_search_input: Entity<InputState>,
    _data_filter_search_subscription: Subscription,
    data_filter_text_input: Entity<gpui_component::input::EditorState>,
    _data_filter_text_subscription: Subscription,
    data_sort_text_input: Entity<InputState>,
    _data_sort_text_subscription: Subscription,
    data_sql_panel_input: Entity<InputState>,
    _data_sql_panel_subscription: Subscription,
    data_sql_footer_selection: SqlTextSelection,
    data_page_input: Entity<InputState>,
    _data_page_subscription: Subscription,
    data_cell_edit_input: Entity<InputState>,
    _data_cell_edit_subscription: Subscription,
    data_cell_editing: Option<DataCellEditState>,
    temporal_part_input: Entity<InputState>,
    _temporal_part_subscription: Subscription,
    temporal_part_editing: Option<TemporalPartEditState>,
    cell_detail_input: Entity<InputState>,
    _cell_detail_subscription: Subscription,
    data_search_input: Entity<InputState>,
    _data_search_subscription: Subscription,
    row_detail_search_input: Entity<InputState>,
    _row_detail_search_subscription: Subscription,
    row_detail_search: String,
    data_search_panels: BTreeSet<TabId>,
    data_search_queries: BTreeMap<TabId, String>,
    data_search_active_matches: BTreeMap<TabId, DataSearchMatch>,
    data_search_highlight_all_tabs: BTreeSet<TabId>,
    redis_search_input: Entity<InputState>,
    _redis_search_subscription: Subscription,
    redis_type_select: Entity<SelectState<SearchableVec<RedisTypeFilterOption>>>,
    _redis_type_select_subscription: Subscription,
    redis_search_drafts: BTreeMap<TabId, String>,
    redis_search_queries: BTreeMap<TabId, String>,
    redis_type_filters: BTreeMap<TabId, String>,
    // Redis Key 列表展示模式（平铺 / Folder），按 tab 隔离；默认平铺以保持既有行为。
    redis_key_list_modes: BTreeMap<TabId, RedisKeyListMode>,
    // Redis Key 列表 Folder 模式下已展开的 folder 前缀集合，按 tab 隔离；
    // 切回平铺时不清空，仅不使用，切回 Folder 时继续生效。
    redis_key_list_expanded: BTreeMap<TabId, BTreeSet<String>>,
    // Redis Key 列表 Folder 模式下当前选中的叶子键名：详情抽屉跟随叶子键，folder 节点不打开详情。
    redis_key_list_selected_leaf: BTreeMap<TabId, String>,
    // Redis Key 列表 Folder 模式下 hover 的叶子原始行下标（用于浮现删除按钮，避免与表格行下标纠缠）。
    redis_key_list_folder_hovered: BTreeMap<TabId, usize>,
    // Redis Key 平铺列表的虚拟滚动句柄：只保留视口附近的行，避免 Key 数量增长后整页布局。
    redis_key_list_uniform_scroll: BTreeMap<TabId, UniformListScrollHandle>,
    // Redis Key Folder 拍平结果缓存：仅在数据或展开状态变化时重建前缀树。
    redis_key_folder_visible_cache: BTreeMap<TabId, Rc<Vec<RedisKeyVisibleRow>>>,
    // Redis Key 列表当前已加载（已扫描）的键数，按 tab 隔离：Folder 每批拉到 10000，
    // 平铺随滚动触底累加。用于状态栏展示「已扫描 N 个键」并作为每次懒加载的基准。
    redis_key_list_loaded: BTreeMap<TabId, u64>,
    // Redis Key 元信息惰性补全「请求中」的键名集合，按 tab 隔离：用于并发去重，
    // 避免同一批可见行被重复触发补全请求。
    redis_key_metadata_pending: BTreeMap<TabId, BTreeSet<String>>,
    // 新建 Key 后待选中的键名，按 tab 隔离：键列表刷新完成（DataLoaded）后再按键名定位并选中，
    // 避免像 RedisInsight 那样为单个键另开标签页。刷新结果里没有该键时不保留。
    redis_key_pending_selection: BTreeMap<TabId, String>,
    // Redis Key 搜索历史：按 (连接 ID, DB 索引) 隔离，队首最新，上限 200。
    // `redis_key_search_history_loaded` 标记该 (连接, DB) 是否已从存储惰性加载一次，
    // 避免用户从未打开的连接历史被提前读入。
    redis_key_search_history: BTreeMap<(ConnectionId, String), Vec<String>>,
    redis_key_search_history_loaded: BTreeMap<(ConnectionId, String), bool>,
    // 上下按钮触发的搜索历史下拉开关
    redis_key_search_history_open: bool,
    redis_data_refresh_times: BTreeMap<TabId, Instant>,
    redis_refresh_time_task: Option<Task<()>>,
    // ER 关系图画布数据：按 tab 缓存；首次打开自动后台加载（见 er/canvas.rs）。
    er_graphs: BTreeMap<TabId, fluxdb_core::ErGraphData>,
    // 整库表目录（阶段 1 读取）。当前表关联 ER 视图据此在关系就绪后扩展邻域；
    // 整库视图直接用它作为展示集。
    er_full_tables: BTreeMap<TabId, Vec<fluxdb_core::ErTableNode>>,
    // ER 图加载失败的错误文案：有值则渲染错误+重试，而非空图。
    er_errors: BTreeMap<TabId, String>,
    // 进行中的 ER 加载任务；存在即「请求中」，避免渲染重复触发加载。
    er_load_tasks: BTreeMap<TabId, Task<()>>,
    er_generation: BTreeMap<TabId, u64>,
    // ER 关系索引后台加载任务（按 tab，共享 app 层索引）。存在即请求中，避免重复发起。
    er_relation_tasks: BTreeMap<TabId, Task<()>>,
    // 当前表关联 ER 的展开深度（跳数）：按 tab 隔离，默认 1。
    // 切换深度会清图缓存并重新加载 neighborhood（不在 tab 去重键内，同一表一个 tab）。
    er_depths: BTreeMap<TabId, u8>,
    // 单节点式展开：显式点过展开的表集合；这些表作为额外种子与中心同层扩 1 跳。
    er_expanded: BTreeMap<TabId, BTreeSet<String>>,
    // ER 画布视图状态：Rc 共享直改（§perf 8 画布实体隔离阶段一；实体层待阶段二按需引入）。
    er_canvas: Rc<std::cell::RefCell<ErCanvasShared>>,
    // ER 画布场景缓存（布局+连线端点预计算）：随图加载一次构建，渲染复用避免每帧重算。
    er_scenes: BTreeMap<TabId, Rc<ErScene>>,
    // 用户已操作画布（平移/拖动/选择/字段滚动）的 tab：关系就绪后不再自动重排。
    er_user_interacted: BTreeSet<TabId>,
    // 关系布局已应用过一次的 tab（首次关系就绪自动应用只此一次，§6.2）。
    er_layout_applied: BTreeSet<TabId>,
    // 自动落位（新增表/冲突表避让）已执行过一次的 tab：与用户是否已交互无关。
    // 平移/缩放在关系就绪前也会置 er_user_interacted，若复用该标记会把重叠留在画布上。
    er_auto_placed: BTreeSet<TabId>,
    /// 首次显示已做过「自动适配居中」的 tab：只做一次，之后尊重用户视口（§六.23）。
    er_fit_done: BTreeSet<TabId>,
    // 本帧可见折线缓存（供空白点击的关系线命中，有界）。（已迁入 ErCanvasState）
    // 最近一次就绪的「整范围」关系索引全边集（按 tab）：供局部 ER 深度切换/单节点展开
    // 无需重新读库即可同步重算邻域，避免撕图后出现空画布，同时保留人工坐标（§修复 1）。
    er_all_edges: BTreeMap<TabId, Vec<fluxdb_core::ErForeignKeyEdge>>,
    // 局部 ER 中心表身份（按 tab）：单节点展开按钮需用它同步重算邻域（node.rs 无 ErDiagramState）。
    er_center_refs: BTreeMap<TabId, fluxdb_core::ErTableRef>,
    // ER 隐藏字段定位后的高亮（按 tab → (表名, 字段名)）：从汇总端口定位到精确字段后高亮该行。
    // （已迁入 ErCanvasState：er_field_highlights）
    // ER 最近一次成功元数据加载的时间（按 tab，展示「更新时间」）。
    er_last_updated: BTreeMap<TabId, String>,
    // 手动刷新进行中的 tab：刷新时保留当前可用图，仅重新读取元数据（§3.3 手动刷新）。
    er_refreshing: BTreeSet<TabId>,
    // ER 字段按需加载的待请求表集合（去抖合并）：同一批可见表合并为一次批量请求。
    /// 待请求字段的表集合（结构化身份，缓存/结果归并据此进行，不从展示名反推）。
    er_pending_columns: BTreeMap<TabId, BTreeSet<fluxdb_core::ErTableRef>>,
    /// 最近一次 ER JSON 导入结果（校验/差异预览；本轮仅用于展示，未做「应用」）。
    er_last_import: BTreeMap<TabId, ErImportReport>,
    // ER 字段按需加载去抖任务：短暂窗口内合并需求，避免每个鼠标/渲染事件启动请求。
    er_column_debounce_tasks: BTreeMap<TabId, Option<Task<()>>>,
    // ER 表搜索（§五.6）：按 tab 的输入框实体 / 订阅 / 当前查询 / 选中索引。
    er_search_input: BTreeMap<TabId, Entity<InputState>>,
    er_search_subs: BTreeMap<TabId, Subscription>,
    er_search_query: BTreeMap<TabId, String>,
    er_search_sel: BTreeMap<TabId, usize>,
    // ER 业务分组（§五.9-12）：按 schema 分组进入/返回全部；None=全部。
    er_group: BTreeMap<TabId, Option<String>>,
    er_custom_groups: BTreeMap<TabId, BTreeMap<String, BTreeSet<String>>>,
    er_group_inputs: BTreeMap<TabId, Entity<InputState>>,
    // 最近一次据以构建场景的分组（用于分组切换时触发重建，不因普通 pan/滚动重建）。
    er_group_built: BTreeMap<TabId, Option<String>>,
    // ER 小地图（§六.25）：小地图在窗口中的 (x,y,w,h)，供点击导航归一化换算（paint 回报）。
    // （已迁入 ErCanvasState：er_minimap_rect）
    // ER 导出对话框（§十）：true 显示 JSON/DBML/Mermaid/SVG 导出到文件动作 + 导入 JSON。
    er_export_open: BTreeSet<TabId>,
    // ER 导出对话框当前键盘高亮项索引（0..=4：JSON/DBML/Mermaid/SVG/导入 JSON）。
    er_export_sel: BTreeMap<TabId, usize>,
    // ER 导出对话框焦点句柄（打开时聚焦，使键盘上下/Enter/Esc 路由到对话框）。
    er_export_focus: BTreeMap<TabId, FocusHandle>,
    // ER 视图持久化已恢复标记（按 tab）：首次渲染本 tab 时应用上次保存的分组/坐标/固定一次。
    er_view_restored: BTreeSet<TabId>,
    er_view_save_failed: BTreeSet<TabId>,
    er_previous_layout:
        BTreeMap<TabId, (BTreeMap<String, (f32, f32)>, BTreeSet<String>, ErViewport)>,
    // ER 作用域持久化 key（按 tab）：打开时由 ErDiagramState 生成，保存/恢复据此定位。
    er_scope_keys: BTreeMap<TabId, String>,
    er_relationship_scope_keys: BTreeMap<TabId, String>,
    // ER 本地逻辑关系目录显示缓存与后台请求状态；按 tab 隔离，结果回填前校验 scope。
    er_relationships: BTreeMap<TabId, Vec<fluxdb_core::ErRelationship>>,
    er_relationship_loading: BTreeSet<TabId>,
    er_relationship_tasks: BTreeMap<TabId, Task<()>>,
    er_relationship_errors: BTreeMap<TabId, String>,
    er_relationship_panel_open: BTreeSet<TabId>,
    er_relationship_panel_focus: BTreeMap<TabId, FocusHandle>,
    er_relationship_form_open: BTreeSet<TabId>,
    er_relationship_form_editing: BTreeMap<TabId, Option<String>>,
    er_relationship_form_role_inputs: BTreeMap<TabId, Entity<InputState>>,
    er_relationship_form_description_inputs: BTreeMap<TabId, Entity<InputState>>,
    er_relationship_form_left_tables:
        BTreeMap<TabId, Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>>,
    er_relationship_form_right_tables:
        BTreeMap<TabId, Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>>,
    er_relationship_form_pairs: BTreeMap<TabId, Vec<ErRelationshipPairControls>>,
    er_relationship_form_filters: BTreeMap<TabId, Vec<ErRelationshipFilterControls>>,
    /// 基数选择：`1:1 / 1:N / N:1 / N:N / 未知`（映射到 ErMatchCardinality，basis=UserAssertion）。
    er_relationship_form_cardinality:
        BTreeMap<TabId, Entity<SelectState<SearchableVec<ErRelationshipSelectOption>>>>,
    er_relationship_form_subscriptions: BTreeMap<TabId, Vec<Subscription>>,
    /// 关系面板宽度（按 tab，可拖动左缘调整）。
    er_relationship_panel_width: BTreeMap<TabId, f32>,
    /// 关系面板中高亮选中的关系 id（点击画布逻辑边后定位，供编辑/确认）。
    er_relationship_panel_selected: BTreeMap<TabId, Option<String>>,
    /// 结构刷新重绑待处理项（§5.2/D9）：刷新后比对新快照产出的 unresolved/needs_review，
    /// 供关系面板顶部列出，人工重绑。不自动改关系。
    er_rebind_pending: BTreeMap<TabId, Vec<ErRebindPendingItem>>,
    /// 可自动重绑的关系（§5.2 第 1 步稳定标识命中的"改名但同对象"）：刷新后由
    /// `er_rebind_scan` 计算，`apply_er_rebind_auto` 逐个经应用服务写回并持久化。
    er_rebind_auto: BTreeMap<TabId, Vec<fluxdb_core::ErRelationship>>,
    /// 自动重绑后台写回任务（不占 `er_relationship_tasks`，避免关系面板误显"提交中"）。
    er_rebind_tasks: BTreeMap<TabId, Task<()>>,
    er_rebuild_review_tasks: BTreeMap<TabId, Task<()>>,
    /// 关系面板拖动起点（全局拖动，无 tab_id 语义，参照 sidebar 模式）。
    er_relationship_panel_resize_start: Option<SidebarResizeStart>,
    er_relationship_delete_pending: BTreeMap<TabId, (String, u64)>,
    er_relationship_delete_undo: BTreeMap<TabId, fluxdb_core::ErRelationship>,
    /// 左右表下拉当前已渲染选项的表 id 列表签名。表单在面板每次 render 都会重建
    /// （ensure_er_relationship_form_controls），若每次都 set_items 重置 Select 的选项，
    /// 搜索过滤结果会被重置为全量、搜索文本被清空（set_selected_value 会 clear_query），
    /// 导致表下拉搜索"输入了但列表不动"。仅当该签名变化时才刷新左右表选项。
    er_relationship_form_table_options: BTreeMap<TabId, Vec<String>>,
    // Redis 连接级概览（版本/内存/CPU）的定期刷新任务与进行中的单次拉取任务
    redis_overview_refresh_task: Option<Task<()>>,
    redis_overview_refresh_tasks: BTreeMap<u64, Task<()>>,
    // Redis 服务端版本缓存（字段级 TTL 编辑等能力开关）；None 表示未知/探测失败。
    // 惰性填充，连接重连成功后清除、下次打开 Hash 面板自动重新探测。
    redis_server_versions: BTreeMap<ConnectionId, Option<RedisServerVersion>>,
    // Redis 服务端版本探测中的任务（按连接去重，避免重复渲染重复发起）
    redis_server_version_tasks: BTreeMap<ConnectionId, Task<()>>,
    redis_key_list_hovered_tab: Option<TabId>,
    // 当前被 hover 的 Redis Workbench 记录（(tab_id, execution_id)）；用于「复制命令」按钮
    // hover 态浮现（不常驻抢标题行空间）。记录 id 为每标签页各自分配，跨标签可能重号，故按
    // (tab_id, execution_id) 唯一标识。None 表示未 hover 任何记录；会话内 UI 展示用，不持久化。
    redis_workbench_hovered_record: Option<(TabId, u64)>,
    redis_stream_table_state: Entity<TableState<RedisStreamTableDelegate>>,
    // Hash 明细数据表：序列/Field/Value/TTL 四列，比例宽度由 canvas 测量后换算
    redis_hash_table_state: Entity<TableState<RedisHashTableDelegate>>,
    // List 明细数据表：序号/Value 两列（只读展示），比例宽度由 canvas 测量后换算
    redis_list_table_state: Entity<TableState<RedisListTableDelegate>>,
    redis_key_value_input: Entity<InputState>,
    _redis_key_value_subscription: Subscription,
    _redis_json_key_value_subscription: Subscription,
    // Redis JSON 值组件：内部持有多行编辑框，维护权威全文、诊断与折叠状态。
    redis_json_editor: JsonComponent,
    // JSON 编辑器当前归属的 (tab, key)；切换 key 或关闭抽屉时据此清理并重置折叠/诊断。
    redis_json_editor_active: Option<(TabId, String)>,
    // JSON 编辑器是否为「保存中/加载中」，期间禁用编辑、保存、格式化等修改文本的操作。
    redis_json_editor_busy: bool,
    redis_key_value_active: Option<(TabId, String)>,
    redis_key_value_drafts: BTreeMap<(TabId, String), String>,
    redis_key_value_syncing: bool,
    // String 详情值状态：按 (tab, key) 缓存已加载值（preview 或完整值），
    // 让「值是否完整」脱离列表 200 字符 preview，作为格式化/复制/编辑/保存的统一 gating。
    redis_string_values: BTreeMap<(TabId, String), RedisStringValueState>,
    redis_string_format: BTreeMap<(TabId, String), RedisStringFormat>,
    redis_string_editing: Option<(TabId, String)>,
    // 在飞的 string 值加载：(tab, key, full)；full=true 表示「加载全部」。
    redis_string_loading: Option<(TabId, String, bool)>,
    // 在飞的 string 值下载（去重，同一 (tab, key) 只允许一个进行中的下载）。
    redis_string_downloading: BTreeSet<(TabId, String)>,
    // 当前详情抽屉正在展示的 string key：离开（切换 key / 关闭抽屉）时据此清理缓存，下次进入重新预览。
    redis_string_active: Option<(TabId, String)>,
    redis_key_name_input: Entity<InputState>,
    _redis_key_name_subscription: Subscription,
    redis_key_ttl_input: Entity<InputState>,
    _redis_key_ttl_subscription: Subscription,
    redis_key_meta_active: Option<(TabId, String)>,
    redis_key_name_drafts: BTreeMap<(TabId, String), String>,
    redis_key_ttl_drafts: BTreeMap<(TabId, String), String>,
    redis_key_meta_editing: Option<RedisKeyMetaField>,
    redis_key_meta_syncing: bool,
    pending_redis_key_delete: Option<RedisKeyDeleteConfirm>,
    pending_redis_stream_entry_add: Option<RedisStreamEntryAddForm>,
    pending_redis_stream_entry_delete: Option<RedisStreamEntryDeleteConfirm>,
    redis_stream_entry_id_input: Entity<InputState>,
    redis_stream_entry_field_rows: Vec<RedisStreamEntryFieldInputs>,
    // Stream「新增 Entry」抽屉字段区滚动句柄：滚动区与常显滚动条共用同一偏移
    redis_stream_entry_drawer_scroll: ScrollHandle,
    redis_set_member_search_input: Entity<InputState>,
    _redis_set_member_search_subscription: Subscription,
    redis_set_member_search_active: Option<(TabId, String)>,
    redis_set_member_search_queries: BTreeMap<(TabId, String), String>,
    redis_set_member_search_pages: BTreeMap<(TabId, String, String), RedisSetMemberPage>,
    redis_set_member_search_loading: Option<(TabId, String, String)>,
    redis_set_member_search_more_loading: Option<(TabId, String, String)>,
    redis_set_member_search_generation: BTreeMap<u64, u64>,
    redis_set_member_search_debounce: Option<Task<()>>,
    redis_set_member_search_debounce_until: Option<Instant>,
    redis_set_member_search_syncing: bool,
    redis_set_member_rows: Vec<Entity<InputState>>,
    // Set 详情面板成员列表滚动句柄：供成员列表滚动区与常显滚动条共用同一偏移
    redis_set_member_panel_scroll: ScrollHandle,
    redis_set_member_active: Option<(TabId, String)>,
    pending_redis_set_member_drawer: Option<RedisSetMemberDrawerForm>,
    redis_set_member_drawer_rows: Vec<Entity<InputState>>,
    redis_set_member_drawer_scroll: ScrollHandle,
    pending_redis_set_member_delete: Option<RedisSetMemberDeleteTarget>,
    redis_hash_field_search_input: Entity<InputState>,
    _redis_hash_field_search_subscription: Subscription,
    redis_hash_field_search_active: Option<(TabId, String)>,
    redis_hash_field_search_queries: BTreeMap<(TabId, String), String>,
    redis_hash_field_search_pages: BTreeMap<(TabId, String, String), RedisHashFieldPage>,
    redis_hash_field_search_loading: Option<(TabId, String, String)>,
    redis_hash_field_search_more_loading: Option<(TabId, String, String)>,
    redis_hash_field_search_generation: BTreeMap<u64, u64>,
    redis_hash_field_search_debounce: Option<Task<()>>,
    redis_hash_field_search_debounce_until: Option<Instant>,
    redis_hash_field_search_syncing: bool,
    redis_hash_value_edit_input: Entity<InputState>,
    _redis_hash_value_edit_subscription: Subscription,
    redis_hash_ttl_edit_input: Entity<InputState>,
    _redis_hash_ttl_edit_subscription: Subscription,
    redis_hash_field_rows: Vec<RedisHashFieldRow>,
    redis_hash_field_hovered: Option<RedisHashFieldEditingState>,
    redis_hash_field_editing: Option<RedisHashFieldEditingState>,
    // 完整值弹框（查看/编辑 >1MB 截断 hash 字段）；懒加载 task id 用 redis_hash_field_mutation_task_id。
    // 用 Rc<RefCell> 而非裸字段：gpui-component 的 dialog builder 在 `Root` 插件层渲染 dialog layer
    // 时被同步执行，此时 NavicatMain 处于 lease 状态，builder 里 `view.read(cx)`
    // 会触发 double-lease panic，因此 builder 必须改读共享状态而不是 re-enter 视图本身。
    redis_hash_full_value_viewer: Rc<std::cell::RefCell<Option<RedisHashFullValueViewer>>>,
    pending_redis_hash_field_drawer: Option<RedisHashFieldDrawerForm>,
    redis_hash_field_drawer_rows: Vec<RedisHashFieldDrawerInputs>,
    redis_hash_field_drawer_scroll: ScrollHandle,
    pending_redis_hash_field_delete: Option<RedisHashFieldDeleteTarget>,
    redis_zset_member_search_input: Entity<InputState>,
    _redis_zset_member_search_subscription: Subscription,
    redis_zset_member_search_active: Option<(TabId, String)>,
    redis_zset_member_search_queries: BTreeMap<(TabId, String), String>,
    redis_zset_member_search_pages: BTreeMap<(TabId, String, String), RedisZSetMemberPage>,
    redis_zset_member_search_loading: Option<(TabId, String, String)>,
    redis_zset_member_search_more_loading: Option<(TabId, String, String)>,
    redis_zset_member_search_generation: BTreeMap<u64, u64>,
    redis_zset_member_search_debounce: Option<Task<()>>,
    redis_zset_member_search_debounce_until: Option<Instant>,
    redis_zset_member_search_syncing: bool,
    redis_zset_member_rows: Vec<RedisZSetMemberInputs>,
    redis_zset_member_panel_scroll: ScrollHandle,
    // ZSet 详情表 score 列「行内编辑」状态：hover 显示编辑图标，点击进入编辑（输入框 + x/勾），
    // 与 member 列（只读纯文本）区分。编辑态独立于 rows 存储，避免同步重建行时丢失编辑焦点。
    redis_zset_member_score_hover: Option<usize>,
    redis_zset_member_score_editing: Option<usize>,
    redis_zset_member_score_edit_input: Option<Entity<InputState>>,
    // 编辑输入失焦订阅：点击编辑框以外区域（焦点转移）时自动取消编辑。
    redis_zset_member_score_blur_sub: Option<Subscription>,
    pending_redis_zset_member_drawer: Option<RedisZSetMemberDrawerForm>,
    redis_zset_member_drawer_rows: Vec<RedisZSetMemberInputs>,
    redis_zset_member_drawer_scroll: ScrollHandle,
    pending_redis_zset_member_delete: Option<usize>,
    redis_list_item_search_input: Entity<InputState>,
    _redis_list_item_search_subscription: Subscription,
    redis_stream_entry_active: Option<(TabId, String)>,
    /// Stream 时间范围过滤的输入框（本地时间，格式 YYYY-MM-DD HH:MM:SS，留空表示不限）。
    redis_stream_since_input: Entity<InputState>,
    redis_stream_until_input: Entity<InputState>,
    /// 生效中的时间范围（毫秒时间戳），按 (tab, key) 记住。
    redis_stream_ranges: BTreeMap<(TabId, String), (Option<u64>, Option<u64>)>,
    /// 新增条目时的 MAXLEN 近似裁剪输入框，留空表示不裁剪。
    redis_stream_maxlen_input: Entity<InputState>,
    /// 消费者组概览：(组名, 消费者数, 未确认数, 最后投递 ID, [(消费者名, 未确认, 空闲毫秒)])
    redis_stream_groups: BTreeMap<(TabId, String), Vec<RedisStreamGroupRow>>,
    redis_stream_groups_loading: Option<(TabId, String)>,
    /// 消费者组面板是否展开。
    redis_stream_groups_expanded: bool,
    redis_stream_entry_pages: BTreeMap<(TabId, String), RedisStreamEntryPage>,
    redis_stream_entry_loading: Option<(TabId, String)>,
    redis_stream_entry_more_loading: Option<(TabId, String)>,
    redis_stream_entry_generation: BTreeMap<u64, u64>,
    redis_list_item_search_active: Option<(TabId, String)>,
    redis_list_item_search_queries: BTreeMap<(TabId, String), String>,
    redis_list_item_search_pages: BTreeMap<(TabId, String, String), RedisListItemPage>,
    redis_list_item_search_loading: Option<(TabId, String, String)>,
    redis_list_item_search_more_loading: Option<(TabId, String, String)>,
    redis_list_item_search_generation: BTreeMap<u64, u64>,
    redis_list_item_search_syncing: bool,
    pending_redis_list_item_drawer: Option<RedisListItemDrawerForm>,
    redis_list_item_drawer_rows: Vec<RedisListItemInputs>,
    redis_list_item_drawer_scroll: ScrollHandle,
    // List 元素删除抽屉（LPOP/RPOP 按数量）：表单态 + 数量输入 + 位置选择（Select）+ 二次确认目标
    redis_list_item_remove_drawer: Option<RedisListItemRemoveDrawerForm>,
    redis_list_item_remove_confirm: Option<RedisListItemRemoveConfirmTarget>,
    redis_list_item_remove_count_input: Entity<InputState>,
    _redis_list_item_remove_count_subscription: Subscription,
    redis_list_item_remove_select:
        Entity<SelectState<SearchableVec<RedisListRemovePositionOption>>>,
    _redis_list_item_remove_select_subscription: Subscription,
    // 「新增 Key」抽屉（对齐 RedisInsight AddKey）：公共字段 entity + 按类型子表单状态。
    redis_add_key_drawer: Option<RedisAddKeyDrawerForm>,
    redis_add_key_type_select: Entity<SelectState<SearchableVec<String>>>,
    _redis_add_key_type_select_subscription: Subscription,
    redis_add_key_name_input: Entity<InputState>,
    _redis_add_key_name_subscription: Subscription,
    redis_add_key_ttl_input: Entity<InputState>,
    _redis_add_key_ttl_subscription: Subscription,
    // 「新增 Key」抽屉中间可滚动表单区的滚动句柄。
    redis_add_key_scroll: ScrollHandle,
    // 类型专用子表单：String/JSON 单值多行输入；Hash 每字段带 TTL；ZSet/Stream 名称=值行；Set/List 成员行 + List 方向。
    redis_add_key_string_input: Entity<InputState>,
    _redis_add_key_string_subscription: Subscription,
    // JSON 值复用 JSON 组件（Edit 模式内部持有带语法高亮的多行编辑器）。
    redis_add_key_json_editor: JsonComponent,
    _redis_add_key_json_subscription: Subscription,
    redis_add_key_hash_rows: Vec<RedisAddKeyHashRow>,
    redis_add_key_zset_rows: Vec<RedisAddKeyNameValueRow>,
    redis_add_key_set_rows: Vec<RedisAddKeySingleRow>,
    redis_add_key_list_rows: Vec<RedisAddKeySingleRow>,
    redis_add_key_list_direction: RedisListDirection,
    redis_add_key_stream_id_input: Entity<InputState>,
    _redis_add_key_stream_id_subscription: Subscription,
    redis_add_key_stream_rows: Vec<RedisAddKeyNameValueRow>,
    redis_add_key_applying: bool,
    // List 元素行内编辑（LSET）：编辑输入框、hover 行、正在编辑的行
    redis_list_value_edit_input: Entity<InputState>,
    _redis_list_value_edit_subscription: Subscription,
    redis_list_item_hovered: Option<RedisListItemEditingState>,
    redis_list_item_editing: Option<RedisListItemEditingState>,
    _redis_key_value_apply_tasks: BTreeMap<u64, Task<()>>,
    tab_switcher_search_input: Entity<InputState>,
    _tab_switcher_search_subscription: Subscription,
    tab_switcher: Option<TabSwitcherKind>,
    tab_switcher_search: String,
    query_history_search_input: Entity<InputState>,
    _query_history_search_subscription: Subscription,
    query_history_search: String,
    query_history_quick_search_input: Entity<InputState>,
    _query_history_quick_search_subscription: Subscription,
    query_history_quick_search: String,
    query_history_quick_open: bool,
    query_history_quick_selected: usize,
    query_history_quick_kind_filter: QueryHistoryKindFilter,
    /// 快速打开表弹层：输入框、当前搜索串与候选列表。
    quick_open_table_input: Entity<InputState>,
    _quick_open_table_subscription: Subscription,
    quick_open_table_search: String,
    quick_open_table_open: bool,
    quick_open_table_selected: usize,
    /// 只来自侧边栏已加载对象树，不额外查询数据库 catalog。
    quick_open_table_entries: Vec<ObjectPath>,
    query_history_connection_select:
        Entity<SelectState<SearchableVec<QueryHistoryConnectionFilterItem>>>,
    _query_history_connection_select_subscription: Subscription,
    query_history_database_select: Entity<SelectState<SearchableVec<QueryHistoryTextFilterItem>>>,
    _query_history_database_select_subscription: Subscription,
    query_history_table_select: Entity<SelectState<SearchableVec<QueryHistoryTextFilterItem>>>,
    _query_history_table_select_subscription: Subscription,
    sql_file_connection_select: Entity<SelectState<SearchableVec<SqlFileConnectionItem>>>,
    _sql_file_connection_select_subscription: Subscription,
    sql_file_database_select: Entity<SelectState<SearchableVec<SqlFileDatabaseItem>>>,
    _sql_file_database_select_subscription: Subscription,
    sql_file_encoding_select: Entity<SelectState<SearchableVec<SqlFileEncodingItem>>>,
    _sql_file_encoding_select_subscription: Subscription,
    sql_file_path_input: Entity<InputState>,
    _sql_file_path_subscription: Subscription,
    backup_file_name_input: Entity<InputState>,
    _backup_file_name_subscription: Subscription,
    /// 新建备份弹框：备份目录输入（默认填充设置 backup_dir，可更换）。
    backup_target_dir_input: Entity<InputState>,
    _backup_target_dir_subscription: Subscription,
    backup_object_search_input: Entity<InputState>,
    _backup_object_search_subscription: Subscription,
    /// 新建备份弹框：备注输入（保存到 .meta.json）。
    backup_note_input: Entity<InputState>,
    _backup_note_subscription: Subscription,
    /// 备份 tab：查看「备份表」弹框（None = 未打开）。tables 为 None 表示该备份无表清单记录。
    backup_tables_modal: Option<BackupTablesModal>,
    /// 备份 tab：查看「恢复记录」弹框（None = 未打开）。
    restore_records_modal: Option<RestoreRecordsModal>,
    /// 备份 tab：备注编辑弹框对应的备份文件路径（None = 未打开）。
    backup_note_modal_path: Option<PathBuf>,
    /// 备注编辑弹框的输入控件。
    backup_note_edit_input: Entity<InputState>,
    /// 运行中备份任务的元数据暂存：task_id → 表清单/备注；成功后写入 .meta.json 并移除。
    backup_pending_metas: BTreeMap<u64, BackupFileMeta>,
    /// 备份 tab：删除确认弹框对应的备份文件路径（None = 未打开）。
    pending_delete_backup: Option<PathBuf>,
    /// Windows 关闭确认弹框：点击自绘关闭按钮后先确认再退出（true = 弹框打开）。
    pending_exit_confirm: bool,
    user_admin_search_input: Entity<InputState>,
    _user_admin_search_subscription: Subscription,
    user_admin_create_user_input: Entity<InputState>,
    _user_admin_create_user_subscription: Subscription,
    user_admin_create_host_input: Entity<InputState>,
    _user_admin_create_host_subscription: Subscription,
    user_admin_auth_plugin_select: Entity<SelectState<SearchableVec<String>>>,
    _user_admin_auth_plugin_select_subscription: Subscription,
    user_admin_password_expiry_select: Entity<SelectState<SearchableVec<String>>>,
    _user_admin_password_expiry_select_subscription: Subscription,
    user_admin_create_password_input: Entity<InputState>,
    _user_admin_create_password_subscription: Subscription,
    user_admin_new_password_input: Entity<InputState>,
    _user_admin_new_password_subscription: Subscription,
    // ===== PG 用户与角色工作台（改版）输入与选择器 =====
    /// 常规页：角色名。
    pg_role_name_input: Entity<InputState>,
    _pg_role_name_subscription: Subscription,
    /// 常规页：新密码（脱敏输入；确认密码不一致时保存禁用）。
    pg_password_input: Entity<InputState>,
    _pg_password_subscription: Subscription,
    /// 常规页：确认密码（仅桌面层校验一致性，明文不进 App 状态）。
    pg_password_confirm_input: Entity<InputState>,
    _pg_password_confirm_subscription: Subscription,
    /// 高级页：连接数限制文本（"-1" = 不限）。
    pg_connection_limit_input: Entity<InputState>,
    _pg_connection_limit_subscription: Subscription,
    /// 常规页：自定义密码截止时间输入（选择「自定义截止时间」时显示）。
    pg_valid_until_input: Entity<InputState>,
    _pg_valid_until_subscription: Subscription,
    /// 常规页：密码有效期模式选择（保持/清除/自定义）。
    pg_valid_until_mode_select: Entity<SelectState<SearchableVec<String>>>,
    _pg_valid_until_mode_select_subscription: Subscription,
    /// 常规页：密码操作选择（保持不变/设置新密码/清除密码）。
    pg_password_op_select: Entity<SelectState<SearchableVec<String>>>,
    _pg_password_op_select_subscription: Subscription,
    /// 权限页：目标数据库选择。
    pg_grant_db_select: Entity<SelectState<SearchableVec<String>>>,
    _pg_grant_db_select_subscription: Subscription,
    /// 权限页：schema 选择。
    pg_grant_schema_select: Entity<SelectState<SearchableVec<String>>>,
    _pg_grant_schema_select_subscription: Subscription,
    /// 权限页：对象选择（随对象种类变化）。
    pg_grant_object_select: Entity<SelectState<SearchableVec<String>>>,
    _pg_grant_object_select_subscription: Subscription,
    /// 成员关系页：待添加的组角色（所属方向）选择。
    pg_member_of_role_select: Entity<SelectState<SearchableVec<String>>>,
    _pg_member_of_role_select_subscription: Subscription,
    /// 成员关系页：ADMIN OPTION 开关（仅作用于新添加的成员授权草稿）。
    pg_member_new_admin: bool,
    /// 成员关系页：选项列表的数据指纹（避免每次渲染重建 select items）。
    pg_select_items_fingerprint: String,
    // ===== SQL Server 用户与权限第一阶段输入 =====
    sqlserver_security_database_select: Entity<SelectState<SearchableVec<String>>>,
    _sqlserver_security_database_subscription: Subscription,
    sqlserver_security_database_items: Vec<String>,
    sqlserver_security_name_input: Entity<InputState>,
    _sqlserver_security_name_subscription: Subscription,
    sqlserver_security_password_input: Entity<InputState>,
    _sqlserver_security_password_subscription: Subscription,
    sqlserver_security_confirm_password_input: Entity<InputState>,
    _sqlserver_security_confirm_password_subscription: Subscription,
    sqlserver_security_login_select: Entity<SelectState<SearchableVec<String>>>,
    _sqlserver_security_login_subscription: Subscription,
    sqlserver_security_default_select: Entity<SelectState<SearchableVec<String>>>,
    sqlserver_security_form_options: (Vec<String>, Vec<String>),
    _sqlserver_security_default_subscription: Subscription,
    sqlserver_security_permission_input: Entity<InputState>,
    _sqlserver_security_permission_subscription: Subscription,
    sqlserver_security_permission_target_input: Entity<InputState>,
    _sqlserver_security_permission_target_subscription: Subscription,
    _sqlserver_security_load_task: Option<Task<()>>,
    _sqlserver_security_permissions_task: Option<Task<()>>,
    _sqlserver_security_preview_task: Option<Task<()>>,
    _sqlserver_security_apply_task: Option<Task<()>>,
    // ===== Oracle 用户与权限 =====
    oracle_security_name_input: Entity<InputState>,
    _oracle_security_name_subscription: Subscription,
    oracle_security_password_input: Entity<InputState>,
    _oracle_security_password_subscription: Subscription,
    oracle_security_confirm_password_input: Entity<InputState>,
    _oracle_security_confirm_password_subscription: Subscription,
    /// 改密码口令：明文只进内存草稿（`OracleUserAdminState::new_password`），保存时随计划执行。
    oracle_security_new_password_input: Entity<InputState>,
    _oracle_security_new_password_subscription: Subscription,
    /// 确认框只用于桌面层的一致性校验，不入状态。
    oracle_security_new_password_confirm_input: Entity<InputState>,
    _oracle_security_new_password_confirm_subscription: Subscription,
    oracle_security_default_ts_input: Entity<InputState>,
    _oracle_security_default_ts_subscription: Subscription,
    oracle_security_temp_ts_input: Entity<InputState>,
    _oracle_security_temp_ts_subscription: Subscription,
    oracle_security_profile_input: Entity<InputState>,
    _oracle_security_profile_subscription: Subscription,
    oracle_security_quota_ts_input: Entity<InputState>,
    _oracle_security_quota_ts_subscription: Subscription,
    oracle_security_quota_bytes_input: Entity<InputState>,
    _oracle_security_quota_bytes_subscription: Subscription,
    oracle_security_grant_name_input: Entity<InputState>,
    _oracle_security_grant_name_subscription: Subscription,
    oracle_security_grant_owner_input: Entity<InputState>,
    _oracle_security_grant_owner_subscription: Subscription,
    oracle_security_grant_object_input: Entity<InputState>,
    _oracle_security_grant_object_subscription: Subscription,
    _oracle_security_load_task: Option<Task<()>>,
    _oracle_security_apply_task: Option<Task<()>>,
    user_admin_max_queries_input: Entity<InputState>,
    _user_admin_max_queries_subscription: Subscription,
    user_admin_max_updates_input: Entity<InputState>,
    _user_admin_max_updates_subscription: Subscription,
    user_admin_max_connections_input: Entity<InputState>,
    _user_admin_max_connections_subscription: Subscription,
    user_admin_max_user_connections_input: Entity<InputState>,
    _user_admin_max_user_connections_subscription: Subscription,
    user_admin_ssl_type_select: Entity<SelectState<SearchableVec<String>>>,
    _user_admin_ssl_type_select_subscription: Subscription,
    user_admin_ssl_cipher_input: Entity<InputState>,
    _user_admin_ssl_cipher_subscription: Subscription,
    user_admin_ssl_issuer_input: Entity<InputState>,
    _user_admin_ssl_issuer_subscription: Subscription,
    user_admin_ssl_subject_input: Entity<InputState>,
    _user_admin_ssl_subject_subscription: Subscription,
    user_admin_privilege_database_menu: Option<(TabId, u64)>,
    query_history_connection_filter: Option<ConnectionId>,
    query_history_database_filter: Option<String>,
    query_history_table_filter: Option<String>,
    query_history_kind_filter: QueryHistoryKindFilter,
    query_history_detail: Option<QueryHistoryEntry>,
    user_admin_password_visible: bool,
    pinned_tabs: BTreeSet<TabId>,
    tab_order: Vec<TabId>,
    workspace_tab_order: Vec<FirstLevelScope>,
    // 当前鼠标悬停的标签页：仅用于浮现标签操作按钮，不持久化。
    hovered_tab: Option<TabId>,
    hovered_first_level_tab: Option<FirstLevelScope>,
    display_database_search_input: Entity<InputState>,
    _display_database_search_subscription: Subscription,
    create_database_name_input: Entity<InputState>,
    _create_database_name_subscription: Subscription,
    create_database_charset_select: Entity<SelectState<SearchableVec<String>>>,
    _create_database_charset_select_subscription: Subscription,
    create_database_collation_select: Entity<SelectState<SearchableVec<String>>>,
    _create_database_collation_select_subscription: Subscription,
    create_database_owner_input: Entity<InputState>,
    _create_database_owner_subscription: Subscription,
    create_database_template_input: Entity<InputState>,
    _create_database_template_subscription: Subscription,
    /// MongoDB「首个集合」输入框（建库时隐式建集合）。
    create_database_first_collection_input: Entity<InputState>,
    _create_database_first_collection_subscription: Subscription,
    danger_table_foreign_key_check_select:
        Entity<SelectState<SearchableVec<ForeignKeyCheckOption>>>,
    _danger_table_foreign_key_check_select_subscription: Subscription,
    rename_table_input: Entity<InputState>,
    _rename_table_subscription: Subscription,
    copy_table_input: Entity<InputState>,
    _copy_table_subscription: Subscription,
    column_choice_value_input: Entity<InputState>,
    column_choice_label_input: Entity<InputState>,
    query_save_name_input: Entity<InputState>,
    _file_picker_task: Option<Task<()>>,
    _connection_tasks: BTreeMap<u64, Task<()>>,
    /// 侧边栏「刷新连接树」的后台任务。只保留最近一次：连点刷新时旧任务被丢弃即取消，
    /// 避免多轮刷新结果交错回写。
    _tree_refresh_task: Option<Task<()>>,
    _database_tasks: BTreeMap<String, Task<()>>,
    _data_load_tasks: BTreeMap<u64, Task<()>>,
    // —— MongoDB 集合工作台 ——
    /// 查询面板输入框（按集合标签页缓存；标签页关闭时释放）。
    pub(crate) mongo_query_inputs: BTreeMap<TabId, Entity<MongoQueryInputs>>,
    /// 查询面板 Options（投影/排序/条数）的显式展开态。
    ///
    /// 只在用户点过 Options 后写入：没记录时按「已生效查询是否带投影/排序」决定默认值，
    /// 免得存下来的条件生效了却在收起的面板里看不见。
    pub(crate) mongo_query_options_expanded: BTreeMap<TabId, bool>,
    /// 「高级查询」第二层折叠的显式展开态（排序规则 / 起始 / 索引提示 / 执行上限）。
    ///
    /// 与 [`Self::mongo_query_options_expanded`] 分开记：两层各自展开/收起，
    /// 点过之后互不影响；未记录时按已生效查询是否带这些低频条件推默认值。
    pub(crate) mongo_query_advanced_expanded: BTreeMap<TabId, bool>,
    /// 「条件构建」的显式展开态。
    ///
    /// 与上面两层分开记：构建器是 Filter 的另一种输入法，不是第三层筛选项。
    /// 未记录时默认收起——它只在用户主动要图形化编辑条件时才占高度。
    pub(crate) mongo_query_builder_expanded: BTreeMap<TabId, bool>,
    /// 条件构建器的条件行（按标签页缓存，切走再切回来不丢草稿）。
    pub(crate) mongo_query_builders: BTreeMap<TabId, MongoQueryBuilder>,
    /// 聚合管道的卡片视图（按标签页缓存；卡片/文本模式与 stage 参数编辑器都在这里）。
    pub(crate) mongo_aggregation_builders: BTreeMap<TabId, MongoAggregationBuilder>,
    /// 已折叠的文档卡片，按「标签页 → 文档身份（`_id`，缺失时 `#下标`）」记录。
    pub(crate) mongo_collapsed_documents: BTreeMap<TabId, BTreeSet<String>>,
    /// 已展开的嵌套字段，键为 `{文档身份}/{字段路径}`；默认全收起，与原型一致。
    pub(crate) mongo_expanded_fields: BTreeMap<TabId, BTreeSet<String>>,
    /// 索引页里已展开定义的索引（按标签页 → 索引名），默认全部收起。
    pub(crate) mongo_expanded_indexes: BTreeMap<TabId, BTreeSet<String>>,
    /// 索引表的排序（按标签页 → 列 + 是否降序）；缺省保持服务端返回的名称顺序。
    pub(crate) mongo_index_sort: BTreeMap<TabId, (MongoIndexSort, bool)>,
    /// JSON 展示下被用户点过的折叠节点，键为 `{文档身份}\u{1e}{折叠路径}`。
    ///
    /// 记录「与默认态相反」的节点而不是折叠结果：默认只有根节点展开、嵌套节点全部收起
    /// （与 Compass 一致），点击即翻转该节点，因此空集合就代表完整默认态，无需额外初始化。
    pub(crate) mongo_json_fold_toggles: BTreeMap<TabId, BTreeSet<String>>,
    /// 文档区的渲染缓存（解析结果、表格列、虚拟列表滚动状态），按标签页持有。
    /// 只在查询结果（`load_generation`）变化时重建，逐帧渲染只读。
    pub(crate) mongo_document_cache: BTreeMap<TabId, MongoDocumentCache>,
    /// 文档编辑抽屉的一次会话（切标签页/关标签即丢弃，不写回标签页状态）。
    mongo_document_editor: Option<MongoDocumentEditor>,
    _mongo_document_editor_task: Option<Task<()>>,
    /// 有未保存草稿时要切换的目标文档：等用户在确认框里选完再决定丢弃还是留下。
    pending_mongo_document_switch: Option<PendingMongoDocumentSwitch>,
    /// 等待确认的文档删除（删除不可撤销，必须二次确认）。
    pending_mongo_document_delete: Option<PendingMongoDocumentDelete>,
    _mongo_document_delete_task: Option<Task<()>>,
    /// 有未保存草稿时点了「复制文档」或结果条的「+」：等用户在确认框里选完再决定丢弃还是留下。
    pending_mongo_document_open: Option<PendingMongoDocumentOpen>,
    _mongo_document_insert_task: Option<Task<()>>,
    /// 文档导入弹窗会话；关闭即丢弃，不写回标签页状态。
    pending_mongo_import: Option<MongoImportForm>,
    _mongo_import_task: Option<Task<()>>,
    _mongo_load_tasks: BTreeMap<u64, Task<()>>,
    _mongo_info_tasks: BTreeMap<u64, Task<()>>,
    _mongo_index_tasks: BTreeMap<u64, Task<()>>,
    _mongo_drop_index_tasks: BTreeMap<u64, Task<()>>,
    /// 补全字段的抽样任务：每个集合标签页只跑一次，失败不重试（补全降级为仅运算符）。
    _mongo_field_sample_tasks: BTreeMap<u64, Task<()>>,
    _query_execute_tasks: BTreeMap<u64, Task<()>>,
    /// Redis Workbench 命令的后台执行任务，按标签页 id 去重：同一标签页执行中再次触发
    /// （Run / 重跑 / 历史「运行此条」）时直接提示，不并发发起第二条网络请求。
    _redis_workbench_tasks: BTreeMap<u64, Task<()>>,
    /// MongoDB 命令工作台的后台执行任务，按标签页 id 去重（同一标签页执行中不并发再发一条）。
    _mongo_workbench_tasks: BTreeMap<u64, Task<()>>,
    /// MongoDB 命令工作台的输入编辑器（按 tab 缓存，随 tab 关闭移除）。
    pub(crate) mongo_workbench_inputs: BTreeMap<TabId, Entity<editor_component::Editor>>,
    /// 工作台补全适配器：持有 shell 补全用的库 / 集合缓存，`use` 后需同步当前库。
    mongo_workbench_completion:
        BTreeMap<TabId, std::sync::Arc<MongoShellEditorAdapter>>,
    _mongo_workbench_input_subscriptions: BTreeMap<TabId, Subscription>,
    _sql_file_execute_tasks: BTreeMap<u64, Task<()>>,
    _sql_file_cancel_flags: BTreeMap<u64, Arc<AtomicBool>>,
    _query_completion_tasks: BTreeMap<(TabId, u64), Task<()>>,
    _completion_index_tasks: BTreeMap<(ConnectionId, Option<String>), Task<()>>,
    _table_info_tasks: BTreeMap<(TabId, TableInfoTab), Task<()>>,
    /// 设计表元数据后台加载任务，按表身份去重；完成后才在主线程安装标签页。
    _design_table_tasks: BTreeMap<String, Task<()>>,
    _design_table_detail_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_users_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_pg_membership_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_pg_target_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_pg_object_grant_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_grants_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_member_grants_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_apply_tasks: BTreeMap<TabId, Task<()>>,
    _user_admin_preview_tasks: BTreeMap<TabId, Task<()>>,
    _create_table_apply_tasks: BTreeMap<TabId, Task<()>>,
    _create_table_reference_columns_tasks: BTreeMap<(TabId, u64), Task<()>>,
    _cell_binary_download_tasks: BTreeMap<(TabId, usize, usize), Task<()>>,
    _cell_text_load_tasks: BTreeMap<(TabId, usize, usize), Task<()>>,
    /// 大字段分段读取任务，按单元格去重：连点「读取更多」不并发读同一窗口。
    _cell_content_chunk_tasks: BTreeMap<(TabId, String), Task<()>>,
    /// 大字段流式保存任务，按单元格去重。
    _cell_content_save_tasks: BTreeMap<(TabId, usize, usize), Task<()>>,
    _data_export_tasks: BTreeMap<u64, Task<()>>,
    _data_export_cancel_flags: BTreeMap<u64, Arc<AtomicBool>>,
    _backup_tasks: BTreeMap<u64, Task<()>>,
    _backup_cancel_flags: BTreeMap<u64, Arc<AtomicBool>>,
    _create_database_tasks: BTreeMap<u64, Task<()>>,
    _delete_database_tasks: BTreeMap<u64, Task<()>>,
    _rename_table_task: Option<Task<()>>,
    _copy_table_task: Option<Task<()>>,
    _routine_ddl_task: Option<Task<()>>,
    _copy_table_ddl_task: Option<Task<()>>,
    _copy_table_structure_task: Option<Task<()>>,
    _danger_table_task: Option<Task<()>>,
    _table_import_task: Option<Task<()>>,
    data_export_task_seq: u64,
    _test_connection_task: Option<Task<()>>,
    _create_schema_task: Option<Task<()>>,
    /// 连接保存/测试期间为 `true`，用于绑定“测试”“保存并连接”按钮 loading/禁用。
    saving_connection: bool,
    /// Redis 连接串导入 / 云自动发现的异步任务。
    _redis_discover_task: Option<Task<()>>,
    /// Redis 连接串导入成功后，输入框实体待与表单重新同步的标记
    /// （异步回调里没有 `Window`，真正同步放在 `render` 消费该标记）。
    redis_discovery_pending_sync: bool,
    connection_context_menu: Option<ConnectionContextMenu>,
    database_context_menu: Option<DatabaseContextMenu>,
    schema_context_menu: Option<SchemaContextMenu>,
    table_context_menu: Option<TableContextMenu>,
    collection_context_menu: Option<CollectionContextMenu>,
    table_group_context_menu: Option<TableGroupContextMenu>,
    table_folder_context_menu: Option<TableFolderContextMenu>,
    tab_context_menu: Option<TabContextMenu>,
    data_cell_context_menu: Option<DataCellContextMenu>,
    data_row_context_menu: Option<DataRowContextMenu>,
    group_context_menu: Option<GroupContextMenu>,
    pending_query_save: Option<TabId>,
    pending_connection_query_save: Option<TabId>,
    pending_rename_group: Option<PendingRenameGroup>,
    pending_rename_table_folder: Option<PendingRenameTableFolder>,
    pending_delete_connection: Option<ConnectionId>,
    pending_delete_database: Option<PendingDeleteDatabase>,
    /// MongoDB「显示连接信息」弹框：None 表示关闭；Some 为已加载（或加载中 / 失败）的服务端信息。
    mongo_info_dialog: Option<MongoInfoDialog>,
    /// MongoDB「创建集合」弹框：连接 + 库 + 集合名；None 表示关闭。
    pending_create_collection: Option<MongoCreateCollectionForm>,
    create_collection_running: bool,
    create_collection_input: Entity<InputState>,
    _create_collection_subscription: Subscription,
    _create_mongo_collection_task: Option<Task<()>>,
    pending_disconnect_connection: Option<PendingDisconnectConnection>,
    pending_close_workspace: Option<PendingCloseWorkspace>,
    pending_new_query_connection: Option<ConnectionId>,
    pending_delete_data_row: Option<DataCellContextMenu>,
    data_row_viewer: Option<DataRowViewer>,
    pending_dirty_data_action: Option<PendingDirtyDataAction>,
    pending_apply_data_changes: Option<TabId>,
    /// 正在后台提交改动的标签（`TabId.0` → 任务）；键存在即表示该标签提交中，
    /// 用于提交按钮 loading/disabled 与重复提交拦截。
    data_change_apply_tasks: BTreeMap<u64, Task<()>>,
    pending_query_parameters: Option<PendingQueryParameterPrompt>,
    pending_dangerous_query: Option<PendingDangerousQuery>,
    pending_dangerous_redis_command: Option<PendingDangerousRedisCommand>,
    /// AI 提交的持久化配置提案；确认前不落库。
    pending_config_proposal: Option<fluxdb_app::ConfigProposal>,
    config_proposal_busy: bool,
    config_delete_confirmed: bool,
    // 「执行 SQL 文件」弹框共享状态：dialog builder 在 NavicatMain::render 期间被同步执行，
    // 此时主视图处于 lease 状态，builder 内不能 view.read(cx)，实时数据统一放 Rc<RefCell<..>>
    // （先例：redis_hash_full_value_viewer）。
    sql_file_modal: Rc<std::cell::RefCell<SqlFileModalData>>,
    /// 任务中心：跨功能的统一任务视图与跨弹窗保留的任务记录。
    task_center: TaskCenterState,
    // 恢复弹框挂载状态:UI 实体 + 配置快照,运行时数据在 RestoreModal::runtime(Rc 共享)。
    restore_modal: Option<RestoreModal>,
    pending_data_export: Option<TableDataExportForm>,
    pending_data_import: Option<TableDataImportForm>,
    _data_import_task: Option<Task<()>>,
    /// 结构对比 / 数据传输弹框；`None` 表示未打开。
    compare_transfer: Option<CompareTransferState>,
    compare_transfer_inputs: CompareTransferInputs,
    _compare_transfer_task: Option<Task<()>>,
    _compare_transfer_source_options_task: Option<Task<()>>,
    _compare_transfer_target_options_task: Option<Task<()>>,
    data_export_custom_conditions_open: bool,
    data_export_preview: Option<TableDataExportPreviewState>,
    data_export_preview_seq: u64,
    _data_export_preview_task: Option<Task<()>>,
    pending_table_data_export_after_load: Option<TabId>,
    pending_rename_table: Option<PendingRenameTable>,
    pending_rename_collection: Option<PendingRenameCollection>,
    rename_collection_input: Entity<InputState>,
    _rename_collection_subscription: Subscription,
    _rename_mongo_collection_task: Option<Task<()>>,
    pending_drop_collection: Option<PendingDropCollection>,
    _drop_mongo_collection_task: Option<Task<()>>,
    pending_mongo_export: Option<MongoExportForm>,
    _mongo_export_task: Option<Task<()>>,
    pending_copy_table: Option<PendingCopyTable>,
    pending_routine_ddl: Option<PendingRoutineDdl>,
    pending_column_choices: Option<PendingColumnChoices>,
    pending_danger_table_action: Option<PendingDangerTableAction>,
    data_export_log_task: Option<u64>,
    data_export_tasks: Vec<TableDataExportTaskState>,
    pending_backup_modal: Option<BackupForm>,
    // 对象选择表列表：虚拟化滚动句柄（滚动偏移/视口跨帧保留，仅渲染视口附近的行）。
    backup_objects_scroll: VirtualListScrollHandle,
    backup_log_task: Option<u64>,
    backup_tasks: Vec<BackupTaskState>,
    backup_task_seq: u64,
    query_parameter_history: BTreeMap<String, String>,
    display_database_connection: Option<ConnectionId>,
    display_database_selection: BTreeSet<String>,
    display_database_search: String,
    display_database_show_system: bool,
    pending_create_database: Option<CreateDatabaseForm>,
    create_database_running: BTreeSet<ConnectionId>,
    /// 新建 schema 弹框：连接 id + 所属库 ObjectPath + schema 名。
    pending_create_schema: Option<(ConnectionId, ObjectPath, String)>,
    create_schema_running: bool,
    create_schema_name_input: Entity<InputState>,
    _create_schema_name_subscription: Subscription,
    new_connection_target_group: Option<ConnectionGroupId>,
    connecting_connections: BTreeSet<ConnectionId>,
    loading_databases: BTreeSet<String>,
    loaded_database_children: BTreeSet<String>,
    pinned_databases: BTreeSet<String>,
    pinned_tables: BTreeSet<String>,
    table_folders: BTreeMap<String, Vec<String>>,
    table_folder_assignments: BTreeMap<String, (String, String)>,
    selected_table_folder: Option<(String, String)>,
    expanded_databases: BTreeMap<String, bool>,
    expanded_object_groups: BTreeMap<String, bool>,
    data_table_states: BTreeMap<TabId, Entity<TableState<DataPageTableDelegate>>>,
    data_table_column_widths: BTreeMap<DataTableWidthKey, BTreeMap<String, Pixels>>,
    _data_table_width_subscriptions: BTreeMap<TabId, Subscription>,
    create_table_inputs: BTreeMap<CreateTableInputKey, Entity<InputState>>,
    _create_table_input_subscriptions: BTreeMap<CreateTableInputKey, Subscription>,
    create_table_comment_editor_sizes: BTreeMap<(TabId, u64), (f32, f32)>,
    create_table_check_expression_editor_sizes: BTreeMap<(TabId, u64), (f32, f32)>,
    create_table_index_field_dropdown: Option<CreateTableIndexFieldDropdownKey>,
    create_table_index_field_selection: Option<CreateTableIndexFieldSelectionKey>,
    create_table_foreign_key_field_selection: Option<CreateTableForeignKeyFieldSelectionKey>,
    create_table_foreign_key_fields_draft: Option<CreateTableForeignKeyFieldsDraft>,
    create_table_foreign_key_referenced_fields_draft:
        Option<CreateTableForeignKeyReferencedFieldsDraft>,
    create_table_comment_editor_resize_start: Option<CreateTableCommentEditorResizeStart>,
    create_table_check_expression_editor_resize_start:
        Option<CreateTableCheckExpressionEditorResizeStart>,
    create_table_type_selects:
        BTreeMap<(TabId, u64), Entity<SelectState<SearchableVec<CreateTableTypeOption>>>>,
    _create_table_type_select_subscriptions: BTreeMap<(TabId, u64), Subscription>,
    create_table_custom_type_editing: Option<(TabId, u64)>,
    create_table_enum_scrolls: BTreeMap<(TabId, u64), ScrollHandle>,
    create_table_selects:
        BTreeMap<CreateTableSelectKey, Entity<SelectState<SearchableVec<CreateTableSelectOption>>>>,
    _create_table_select_subscriptions: BTreeMap<CreateTableSelectKey, Subscription>,
    data_export_object_selects:
        BTreeMap<DataExportObjectSelectKey, Entity<SelectState<SearchableVec<String>>>>,
    _data_export_object_select_subscriptions: BTreeMap<DataExportObjectSelectKey, Subscription>,
    query_editors: BTreeMap<TabId, Entity<editor_component::Editor>>,
    /// 查询页语句执行状态仓库（按 tab 缓存；与 SqlAdapter 共享，供装饰还原行背景）。
    query_statement_statuses: BTreeMap<TabId, sql_editor_adapter::SqlStatusStore>,
    /// 最近一次从编辑器发起的执行语句快照，与结果摘要同序。
    /// 仅用于失败修复定位，不能用于执行或刷新 SQL。
    query_execution_sources: BTreeMap<TabId, Vec<SqlStatementRun>>,
    query_save_targets: BTreeMap<TabId, QuerySaveTarget>,
    saved_queries: Vec<SavedQuery>,
    /// 启动后是否已完成持久化历史/保存查询的一次性异步恢复（防重复触发）。
    persisted_history_loaded: bool,
    _query_editor_subscriptions: BTreeMap<TabId, Subscription>,
    /// 查询页查找/替换输入框（按 tab 缓存；宿主渲染 find 面板用）。
    #[allow(dead_code)]
    // 查找面板（table_state::query_find_state 等）尚未接入宿主渲染，保留字段。
    query_find_inputs: BTreeMap<TabId, Entity<InputState>>,
    #[allow(dead_code)] // 同上，替换输入框随查找面板一并预留。
    query_replace_inputs: BTreeMap<TabId, Entity<InputState>>,
    _query_find_input_subscriptions: BTreeMap<TabId, Subscription>,
    _query_replace_input_subscriptions: BTreeMap<TabId, Subscription>,
    /// workbench 命令编辑器（按 tab 缓存）。
    /// 迁移到通用 `editor_component::Editor` 后，命令输入、补全、执行全部经本实体。
    /// `pub(crate)`：供 crate 根作用域读取，用于事件订阅与生命周期管理。
    pub(crate) redis_workbench_inputs: BTreeMap<TabId, Entity<editor_component::Editor>>,
    _redis_workbench_input_subscriptions: BTreeMap<TabId, Subscription>,
    // Redis CLI 终端会话：按 tab 缓存复用同一个 PTY 会话。
    // 仅在通过数据库右键菜单「Redis CLI」打开时创建，生命周期随 tab 关闭而销毁。
    terminal_sessions: BTreeMap<TabId, Entity<TerminalComponent>>,
    // 周期 pumping 各终端 PTY 输出的后台任务（首开终端时懒创建，drop 即停止）。
    _terminal_session_pump: Option<gpui::Task<()>>,
    // Redis Pub/Sub 会话：按 tab 缓存复用同一个订阅连接，生命周期随 tab 关闭而销毁。
    pubsub_sessions: BTreeMap<TabId, PubSubSessionModel>,
    // 周期消费各 Pub/Sub 会话消息并合并进 UI 的后台任务（首开会话时懒创建，drop 即停止）。
    _pubsub_pump: Option<gpui::Task<()>>,
    // Workbench 上下分栏拖动状态（全局，Redis / MongoDB 共用；目标决定写回哪个比例）。
    // 注：Workbench 历史持久化已移除，只保留当前会话态；后期恢复历史时在此
    // 新增「按连接 + DB 分组」的会话历史与持久化入口，不复用 SQL 历史。
    workbench_panel_resize_start: Option<WorkbenchPanelResizeStart>,
    query_output_tabs: BTreeMap<TabId, QueryOutputTab>,
    /// 执行计划页签当前选中的节点下标（按标签页记住）。
    query_plan_selected_nodes: BTreeMap<TabId, usize>,
    /// 执行计划的展示模式：图形树 / 原始结果表格。
    query_plan_view_modes: BTreeMap<TabId, QueryPlanViewMode>,
    collapsed_query_outputs: BTreeSet<TabId>,
    results_placement: ResultsPlacement,
    query_result_display_pages: BTreeMap<QueryResultDisplayKey, SortedQueryResultPage>,
    settings_panel_section: SettingsPanelSection,
    settings_editor_draft: Settings,
    /// 设置面板「危险 SQL 操作清单」折叠区是否展开（UI 瞬时状态，不进渲染快照）。
    settings_dangerous_actions_collapsed: bool,
    /// 设置「数据」页备份/客户端分组的折叠状态（UI 瞬时状态，不进渲染快照）。
    settings_data_groups_collapsed: SettingsDataGroupsCollapsed,
    settings_font_size_slider: Entity<SliderState>,
    _settings_font_size_slider_subscription: Subscription,
    settings_line_height_input: Entity<InputState>,
    _settings_line_height_subscription: Subscription,
    settings_radius_input: Entity<InputState>,
    _settings_radius_subscription: Subscription,
    pg_client: NativeClientState,
    mysql_client: NativeClientState,
    /// 备份对话框打开时的预检结果：PostgreSQL 连接但本机缺少 pg_dump。
    backup_pg_client_missing: bool,
    /// 备份对话框打开时的预检结果：MySQL/TiDB 连接但本机缺少 mysqldump/mariadb-dump。
    backup_mysql_client_missing: bool,
    query_output_heights: BTreeMap<TabId, f32>,
    query_output_widths: BTreeMap<TabId, f32>,
    query_output_resize_start: Option<QueryOutputResizeStart>,
    query_result_sort_rules: BTreeMap<QueryResultSortKey, Vec<DataSortRule>>,
    query_result_cell_detail: BTreeMap<TabId, QueryResultCellDetail>,
    /// 详情面板打开中的大字段分段窗口（键 = 标签页）。
    large_value_windows: BTreeMap<TabId, LargeValueWindows>,
    data_change_sql_preview_tabs: BTreeSet<TabId>,
    data_filter_panels: BTreeSet<TabId>,
    data_filter_rules: BTreeMap<TabId, Vec<DataFilterRule>>,
    data_sort_rules: BTreeMap<TabId, Vec<DataSortRule>>,
    data_filter_draft_rules: BTreeMap<TabId, Vec<DataFilterRule>>,
    data_sort_draft_rules: BTreeMap<TabId, Vec<DataSortRule>>,
    /// 最近一次清空排序前的草稿，用于标签区的单步撤销。
    data_sort_clear_undo: BTreeMap<TabId, Vec<DataSortRule>>,
    data_filter_grouped_tabs: BTreeSet<TabId>,
    data_filter_modes: BTreeMap<TabId, DataFilterMode>,
    data_filter_texts: BTreeMap<TabId, String>,
    data_filter_condition_completion_signatures:
        BTreeMap<TabId, (Vec<String>, sql_editor_adapter::SqlDialect, DatabaseKind)>,
    data_sort_texts: BTreeMap<TabId, String>,
    data_filter_panel_heights: BTreeMap<TabId, f32>,
    data_filter_panel_resize_start: Option<DataFilterPanelResizeStart>,
    cell_detail_drawer_heights: BTreeMap<TabId, f32>,
    cell_detail_drawer_resize_start: Option<CellDetailDrawerResizeStart>,
    data_filter_popover: Option<DataFilterPopover>,
    local_filter_popover: Option<LocalFilterPopover>,
    local_filter_manager_popover: Option<TabId>,
    local_filter_value: String,
    local_filter_search: String,
    local_filter_draft_values: BTreeSet<String>,
    local_filter_manager_field: Option<String>,
    local_filter_manager_draft_filters: BTreeMap<String, BTreeSet<String>>,
    local_filter_manager_field_open: bool,
    local_filter_manager_values_open: bool,
    local_table_filters: BTreeMap<TabId, BTreeMap<String, BTreeSet<String>>>,
    query_history_open: bool,
    /// Redis Workbench 历史抽屉是否展开。
    pub redis_history_open: bool,
    /// 打开 Redis 历史抽屉时锚定的作用域（连接 + 逻辑库）。
    pub redis_history_scope: Option<WorkbenchHistoryScope>,
    /// Redis 历史抽屉搜索框：输入实体 / 订阅 / 当前搜索文本（仅对已 load 的历史做本地过滤）。
    redis_history_search_input: Entity<InputState>,
    _redis_history_search_subscription: Subscription,
    redis_history_search: String,
    data_filter_value_input_text: String,
    data_filter_value_search: String,
    data_filter_value_search_loading_until: Option<Instant>,
    data_filter_value_search_task: Option<Task<()>>,
    data_filter_value_draft: Option<DataFilterValueDraft>,
    data_filter_applying_tabs: BTreeSet<TabId>,
    _data_filter_apply_tasks: BTreeMap<u64, Task<()>>,
    /// 每个表页当前筛选加载对应的 generation；旧任务回调不得清理新任务状态。
    _data_filter_apply_generations: BTreeMap<u64, u64>,
    app_message: Option<AppMessage>,
    _app_message_task: Option<Task<()>>,
    table_hover_color: Option<Hsla>,
    table_hover_blocked_by_overlay: bool,
    show_connection_browser: bool,
    connection_browser_width: f32,
    connection_browser_resize_start: Option<SidebarResizeStart>,
    table_info_resize_start: Option<SidebarResizeStart>,
}
