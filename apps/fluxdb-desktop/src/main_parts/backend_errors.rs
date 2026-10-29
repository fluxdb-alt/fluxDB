// 应用层/Core 错误仍以中文消息返回；这里在展示边界做精确匹配。
// 未命中的消息原样保留，避免吞掉或改写未知错误。

fn localize_backend_error_text(text: &str) -> String {
    let key = match text {
        "page_query_invalid" => "app.ai.page_query.invalid",
        "page_query_unavailable" => "app.ai.page_query.unavailable",
        "page_query_unsupported" => "app.ai.page_query.unsupported",
        "page_query_failed" => "app.ai.page_query.failed",
        "page_query_conflict" => "app.ai.page_query.conflict",
        "page_query_busy" => "app.ai.page_query.busy",
        "page_query_existing" => "app.ai.page_query.existing",
        "page_query_cancelled" => "app.ai.page_query.cancelled",
        "page_query_timeout" => "app.ai.page_query.timeout",
        "page_query_already_run" => "app.ai.page_query.already_run",
        "sql_edit_invalid" => "app.ai.sql_edit.invalid",
        "sql_edit_unavailable" => "app.ai.sql_edit.unavailable",
        "sql_edit_conflict" => "app.ai.sql_edit.conflict",
        "sql_edit_cancelled" => "app.ai.sql_edit.cancelled",
        "sql_edit_timeout" => "app.ai.sql_edit.timeout",
        "config_proposal_expired" => "app.ai.config_proposal.expired",
        "config_proposal_stale" => "app.ai.config_proposal.stale",
        "config_proposal_already_applied" => "app.ai.config_proposal.already_applied",
        "config_proposal_invalid" => "app.ai.config_proposal.invalid",
        "config_proposal_missing" => "app.ai.config_proposal.missing_target",
        "config_proposal_self_agent" => "app.ai.config_proposal.self_agent",

        "用户与权限标签页不存在" => "app.backend_error.user_admin_tab_missing",
        "连接不存在" => "app.backend_error.connection_missing",
        "新建表标签页不存在" => "app.backend_error.create_table_tab_missing",
        "数据结果不存在" => "app.backend_error.data_result_missing",
        "Redis Workbench 标签页不存在" => "app.backend_error.redis_workbench_tab_missing",
        "数据编辑器标签页不存在" => "app.backend_error.data_editor_tab_missing",
        "查询编辑器标签页不存在" => "app.backend_error.query_editor_tab_missing",
        "Redis 数据编辑器标签页不存在" => "app.backend_error.redis_data_editor_tab_missing",
        "工具入参不是合法 JSON" => "app.backend_error.tool_args_invalid_json",
        "缺少 target" => "app.backend_error.missing_target",
        "数据页未加载" => "app.backend_error.data_page_not_loaded",
        "行不存在" => "app.backend_error.row_missing",
        "附件授权损坏" => "app.backend_error.attachment_auth_corrupted",
        "未选择单元格" => "app.backend_error.no_cell_selected",
        "当前单元格不是二进制字段" => "app.backend_error.not_binary_cell",
        "模型凭据不可用" => "app.backend_error.model_credential_unavailable",
        "附件已变更，请移除后重新授权" => "app.backend_error.attachment_changed",
        "附件偏移无效" => "app.backend_error.attachment_offset_invalid",
        "文件读取已取消" => "app.backend_error.file_read_cancelled",
        "产物操作已取消" => "app.backend_error.artifact_cancelled",
        "字段元数据来自数据页" => "app.backend_error.field_meta_from_page",
        "元数据描述已取消" => "app.backend_error.metadata_cancelled",
        "暂不支持该连接的用户与权限管理" => "app.backend_error.user_admin_unsupported",
        "列不存在" => "app.backend_error.column_missing",
        "单元格不存在" => "app.backend_error.cell_missing",
        "二进制字段不支持就地编辑，请使用上传替换或设为 NULL" => {
            "app.backend_error.binary_edit_unsupported"
        }
        "超大字段不支持就地编辑；请使用分段查看或上传替换" => {
            "app.backend_error.large_edit_unsupported"
        }
        "无法匹配原始行，请刷新后重试" => "app.backend_error.row_match_failed",
        "任务目标不是合法 JSON" => "app.backend_error.task_target_invalid_json",
        "模型返回错误结束" => "app.backend_error.model_error_finish",
        "数据库引用已失效，请移除后重新选择" => "app.backend_error.database_ref_expired",
        "附件读取失败" => "app.backend_error.attachment_read_failed",
        "工具未开放" => "app.backend_error.tool_disabled",
        "附件未授权或已移除" => "app.backend_error.attachment_not_authorized",
        "附件状态不可读" => "app.backend_error.attachment_state_unreadable",
        "结果仓库不可用" => "app.backend_error.result_store_unavailable",
        "标签页不存在" => "app.backend_error.tab_missing",
        "运行快照损坏" => "app.backend_error.runtime_snapshot_corrupted",
        "已取消" => "app.backend_error.cancelled",
        "草稿超过 64 KiB" => "app.backend_error.draft_too_large",
        "附件路径缺失" => "app.backend_error.attachment_path_missing",
        "该审批没有绑定工具调用" => "app.backend_error.approval_not_bound",
        "当前 Agent 已停用，无法恢复任务" => "app.backend_error.agent_disabled_resume",
        "任务配置快照损坏" => "app.backend_error.task_config_corrupted",
        "任务快照缺少原模型，拒绝改用当前模型" => "app.backend_error.task_model_missing",
        "原任务模型已停用，无法恢复任务" => "app.backend_error.task_model_disabled",
        "该任务还没有助手消息" => "app.backend_error.task_no_assistant_message",
        "当前页面摘录超限" => "app.backend_error.page_excerpt_too_large",
        "会话使用的 Agent 已不可用，请新建会话使用默认助手" => "app.backend_error.session_agent_unavailable",
        "当前 Agent 已停用" => "app.backend_error.agent_disabled",
        "请在输入框下方选择模型，或在大模型设置中配置默认主模型" => "app.backend_error.select_model",
        "会话目标损坏" => "app.backend_error.session_target_corrupted",
        "会话目标连接已不存在，请重新选择引用或连接" => "app.backend_error.session_target_connection_missing",
        "该连接类型不支持 SQL 工具" => "app.backend_error.sql_tools_unsupported",
        "提交或回滚结果未知，禁止自动重试，请人工核实" => "app.backend_error.commit_unknown",
        "自动审批影响验证失败或已回滚，需要重新人工审核" => "app.backend_error.auto_approval_invalidated",
        "执行已取消" => "app.backend_error.execution_cancelled",
        "取消信号在写操作返回后才到达，是否已提交无法确定" => "app.backend_error.cancel_after_write",
        "SQL 无法解析，无法校验作用域" => "app.backend_error.sql_parse_failed",
        "SQL 引用的 database/schema 超出当前 target 作用域" => "app.backend_error.sql_target_out_of_scope",
        "SQL 调用的函数所在 database/schema 超出当前 target 作用域" => "app.backend_error.sql_function_out_of_scope",
        "检索已取消" => "app.backend_error.retrieval_cancelled",
        "对象不在授权范围内" => "app.backend_error.object_out_of_scope",
        "DDL 超过结果大小上限" => "app.backend_error.ddl_result_too_large",
        "累计上下文（含工具定义与回执）超出模型输入预算，未发送请求；请缩小读取范围或调整上下文" => "app.backend_error.context_budget_exceeded",
        "重复模型请求，拒绝再次发送" => "app.backend_error.duplicate_model_request",
        "target 必须是内核句柄（conn:<id>[/db:<database>][/schema:<schema>]）" => "app.backend_error.target_handle_invalid",
        "object_ref 必须是对象" => "app.backend_error.object_ref_not_object",
        "object_ref 缺少 name" => "app.backend_error.object_ref_missing_name",
        "object_ref 缺少 kind" => "app.backend_error.object_ref_missing_kind",
        "object_ref 的 database 超出当前 target 作用域" => "app.backend_error.object_ref_database_out_of_scope",
        "object_ref 的 schema 超出当前 target 作用域" => "app.backend_error.object_ref_schema_out_of_scope",
        "缺少 sql" => "app.backend_error.missing_sql",
        "sql 不能为空" => "app.backend_error.sql_empty",
        "parameters 必须是 tagged 值数组" => "app.backend_error.parameters_tagged_array",
        "执行式查询计划（EXPLAIN ANALYZE）默认不开放，需走修改入口另行审批" => "app.backend_error.explain_analyze_disabled",
        "结果仓库锁已损坏" => "app.backend_error.result_store_lock_corrupted",
        "缺少标准 SQL 工具定义: db.search_objects" => "app.backend_error.missing_tool_search_objects",
        "缺少标准 SQL 工具定义: db.describe_object" => "app.backend_error.missing_tool_describe_object",
        "缺少 object_ref" => "app.backend_error.missing_object_ref",
        "缺少标准 SQL 工具定义: db.run" => "app.backend_error.missing_tool_run",
        "计划参数无效" => "app.backend_error.plan_params_invalid",
        "连接未授权" => "app.backend_error.connection_not_authorized",
        "无法检查数据库类型" => "app.backend_error.database_type_check_failed",
        "SQL 超出作用域" => "app.backend_error.sql_out_of_scope",
        "scripted provider 状态锁已损坏" => "app.backend_error.scripted_provider_state_lock_corrupted",
        "scripted provider 脚本锁已损坏" => "app.backend_error.scripted_provider_script_lock_corrupted",
        "scripted provider 没有更多脚本步骤" => "app.backend_error.scripted_provider_no_steps",
        "决策脚本状态锁已损坏" => "app.backend_error.decision_script_state_lock_corrupted",
        "工具记录锁已损坏" => "app.backend_error.tool_record_lock_corrupted",
        "本轮结果快照数量已达上限" => "app.backend_error.result_snapshots_limit",
        "本轮结果快照字节数已达上限" => "app.backend_error.result_snapshot_bytes_limit",
        "工具输出快照超过容量限制" => "app.backend_error.tool_output_snapshot_too_large",
        "未读取到当前数据库用户" => "app.backend_error.db_user_not_read",
        "请先选择角色" => "app.backend_error.select_role",
        "PostgreSQL 连接档案缺失" => "app.backend_error.pg_profile_missing",
        "没有待保存的草稿" => "app.backend_error.no_draft_to_save",
        "角色名不能为空" => "app.backend_error.role_name_empty",
        "连接数限制必须是不限（-1）或非负整数" => "app.backend_error.connection_limit_invalid",
        "新建可登录角色未设置密码（如需无密码请选择「不设置密码」）" => "app.backend_error.login_role_password_missing",
        "原始行身份缺失" => "app.backend_error.original_row_identity_missing",
        "查询结果不存在" => "app.backend_error.query_result_missing",
        "查询结果不可编辑" => "app.backend_error.query_result_not_editable",
        "原始数据页未加载" => "app.backend_error.original_data_page_not_loaded",
        "当前数据库暂不支持流式写入大字段，请改用较小的文件" => {
            "app.backend_error.streamed_write_unsupported"
        }
        "新增行不支持流式写入，请先提交后再修改该字段" => {
            "app.backend_error.streamed_write_insert_unsupported"
        }
        "大字段内容已失效（结果已关闭或已翻页），请重新执行查询" => {
            "app.backend_error.cell_content_expired"
        }
        "大字段内容存储未初始化，无法读写完整内容" => {
            "app.backend_error.cell_content_store_missing"
        }
        "当前单元格不支持上传替换，请使用文本或二进制大字段" => {
            "app.backend_error.upload_cell_unsupported"
        }
        "文件不是合法的 UTF-8 文本，无法写入文本字段" => {
            "app.backend_error.upload_file_not_utf8"
        }
        "待写入文本不是合法的 UTF-8，已取消提交" => {
            "app.backend_error.streamed_text_not_utf8"
        }
        "文本分块过小，无法切在字符边界上；请调大分块阈值后重试" => {
            "app.backend_error.streamed_text_chunk_too_small"
        }
        "无唯一键且包含二进制摘要，无法可靠定位原始行" => "app.backend_error.row_identity_binary_ambiguous",
        "行身份包含未完整加载的大字段，无法安全定位行；请刷新后重试或改用主键定位" => {
            "app.backend_error.row_identity_large_value_unlocatable"
        }
        "无法定位行身份" => "app.backend_error.row_identity_locate_failed",
        "原始行身份不唯一，已取消提交" => "app.backend_error.row_identity_not_unique",
        "工具入参必须是对象" => "app.backend_error.tool_args_not_object",
        "缺少 request_id" => "app.backend_error.missing_request_id",
        "宿主未提供 request 仓储，操作状态不可用" => "app.backend_error.request_store_unavailable",
        "诊断读取参数不是合法 JSON" => "app.backend_error.diagnostics_args_invalid_json",
        "detail 只能是 summary 或 stages" => "app.backend_error.detail_scope_invalid",
        "产物目录不可读取" => "app.backend_error.artifact_dir_unreadable",
        "产物目录不得是符号链接" => "app.backend_error.artifact_dir_symlink",
        "SCAN 回包不是数组" => "app.backend_error.scan_reply_not_array",
        "SCAN 回包结构异常" => "app.backend_error.scan_reply_invalid",
        "SCAN 游标格式异常" => "app.backend_error.scan_cursor_invalid_format",
        "SCAN 游标不是无符号整数" => "app.backend_error.scan_cursor_not_unsigned",
        "SCAN 元素格式异常" => "app.backend_error.scan_element_invalid",
        "Redis 长度回包异常" => "app.backend_error.redis_length_reply_invalid",
        "范围游标必须是非负整数" => "app.backend_error.range_cursor_invalid",
        "扫描游标必须是非负整数" => "app.backend_error.scan_cursor_negative",
        "Redis 成员/值配对结构异常" => "app.backend_error.redis_pair_invalid",
        "COUNT 非硬上限，本批超过 limit；请提高 limit 至预算允许值或使用指定字段读取。本批未推进游标。" => "app.backend_error.count_exceeds_limit",
        "Stream 游标必须是 entry ID" => "app.backend_error.stream_cursor_invalid",
        "Stream entry ID 格式异常" => "app.backend_error.stream_entry_id_invalid",
        "XRANGE 回包结构异常" => "app.backend_error.xrange_reply_invalid",
        "Stream ID 不是文本" => "app.backend_error.stream_id_not_text",
        "Stream ID 格式异常" => "app.backend_error.stream_id_invalid",
        "Stream entry 结构异常" => "app.backend_error.stream_entry_invalid",
        "值预览超过字节预算；请缩小范围" => "app.backend_error.value_preview_too_large",
        "模型连接提前结束，未收到完整结束事件" => "app.backend_error.model_stream_incomplete",
        "Hook 不得改变执行目标" => "app.backend_error.hook_target_changed",
        "工具不可用" => "app.backend_error.tool_unavailable",
        "连接不在本轮授权集合内，未执行" => "app.backend_error.connection_not_authorized_turn",
        "连接配置已变化或连接已删除，未执行" => "app.backend_error.connection_changed_not_executed",
        "连接已设为只读，未执行" => "app.backend_error.connection_read_only_not_executed",
        "无法核实未知写入状态" => "app.backend_error.unknown_write_unverifiable",
        "该连接有尚未核实的未知写入，禁止继续修改" => "app.backend_error.unknown_write_blocked",
        "审批期间策略变化或调用已取消，未执行" => "app.backend_error.approval_policy_changed",
        "模型未声明工具能力却返回工具调用" => "app.backend_error.model_tool_capability_missing",
        "模型返回了未注册的工具" => "app.backend_error.model_unregistered_tool",
        "工具 ID 重复" => "app.backend_error.duplicate_tool_id",
        "内置能力目录包含重复 ID" => "app.backend_error.duplicate_builtin_id",
        "已下线的内置能力不得登记能力或配置项" => "app.backend_error.retired_builtin_registration",
        "内置能力目录包含空能力项" => "app.backend_error.empty_builtin_capability",
        "该 Provider 暂不支持对话" => "app.backend_error.provider_chat_unsupported",
        "模型缺少 API Key" => "app.backend_error.model_missing_api_key",
        "数据库资源路径无法核验" => "app.backend_error.db_resource_path_unverifiable",
        "配置目录无法核验" => "app.backend_error.config_dir_unverifiable",
        "配置数据库不能作为 AI 数据资源" => "app.backend_error.config_db_as_ai_resource",
        "任务目标损坏" => "app.backend_error.task_target_corrupted",
        "引用目标损坏" => "app.backend_error.reference_target_corrupted",
        "本轮引用跨多个连接、数据库或 schema；请保留同一执行目标的引用后发送" => "app.backend_error.references_cross_target",
        "产物工具 ID 重复" => "app.backend_error.artifact_tool_id_duplicate",
        "已取消执行" => "app.backend_error.cancelled_execution",
        "等待结果超时，已停止等待；请重试或检查目标可用性" => "app.backend_error.wait_result_timeout",
        "引用定位或连接已失效，请移除后重新选择" => "app.backend_error.reference_location_invalid",
        "引用连接已移除" => "app.backend_error.reference_connection_removed",
        "引用对象校验失败，请确认连接并刷新" => "app.backend_error.reference_object_validation_failed",
        "引用快照不可用，请重新选择" => "app.backend_error.reference_snapshot_unavailable",
        "保存查询已删除或修改，请重新选择" => "app.backend_error.saved_query_changed",
        "引用读取已取消" => "app.backend_error.reference_read_cancelled",
        "引用不属于本轮任务" => "app.backend_error.reference_not_in_turn",
        "引用未授权或不属于本轮" => "app.backend_error.reference_unauthorized",
        "引用偏移不在 UTF-8 边界" => "app.backend_error.reference_offset_not_utf8",
        "读取长度不足一个 UTF-8 字符" => "app.backend_error.read_length_too_short",
        "附件不是 UTF-8 文本或偏移切分字符，请调整范围" => "app.backend_error.attachment_offset_not_utf8",
        "无法读取附件信息" => "app.backend_error.attachment_info_unreadable",
        "决策模型缺少 API Key" => "app.backend_error.decision_model_missing_api_key",
        "该模型不是 System One 决策协议" => "app.backend_error.decision_model_protocol_invalid",
        "决策模型未配置请求地址" => "app.backend_error.decision_model_url_missing",
        "重复决策请求，拒绝再次发送" => "app.backend_error.duplicate_decision_request",
        "不是备份恢复命令" => "app.backend_error.not_backup_restore_command",
        "请至少选择一个备份对象" => "app.backend_error.backup_object_required",
        "pg_dump 版本低于服务端版本" => "app.backend_error.pg_dump_version_too_old",
        "备份文件名无效" => "app.backend_error.backup_file_name_invalid",
        "备份文件已存在，拒绝覆盖" => "app.backend_error.backup_file_exists",
        "此数据库尚不支持恢复" => "app.backend_error.restore_unsupported",
        "MCP 服务已关闭" => "app.backend_error.mcp_service_closed",
        "该数据源为只读或连接已标记只读，禁止写入" => "app.backend_error.datasource_read_only",
        "该调用需要人工确认，但 MCP 未启用审批队列" => "app.backend_error.mcp_approval_queue_disabled",
        "无法登记执行审计" => "app.backend_error.execution_audit_register_failed",
        "无法占用请求执行权" => "app.backend_error.request_claim_failed",
        "请求已执行或失效，不会重复执行" => "app.backend_error.request_already_executed",
        "执行审计提交失败" => "app.backend_error.execution_audit_commit_failed",
        "工具回执归属与调用不符，结果不可确认" => "app.backend_error.tool_receipt_mismatch",
        "MCP 服务已删除" => "app.backend_error.mcp_service_deleted",
        "MCP 服务配置已变化，请重新调用" => "app.backend_error.mcp_service_config_changed",
        "工具已被全局关闭，请重新调用" => "app.backend_error.tool_globally_disabled",
        "连接配置已变化，请重新调用" => "app.backend_error.connection_config_changed_retry",
        "缺少命令参数" => "app.backend_error.missing_command_args",
        "缺少写入目标" => "app.backend_error.missing_write_target",
        "该连接类型不支持 SQL 写工具" => "app.backend_error.sql_write_tools_unsupported",
        "SQL 无法解析，无法核对写范围" => "app.backend_error.sql_parse_write_scope_failed",
        "写工具只接受单条语句" => "app.backend_error.write_single_statement_only",
        "Hook 参数补丁冲突" => "app.backend_error.hook_patch_conflict",
        "Hook 参数超过上限" => "app.backend_error.hook_params_too_large",
        "after-tool Hook 不允许改写执行事实" => "app.backend_error.hook_after_tool_rewrite_forbidden",
        "禁止递归调用 Hook" => "app.backend_error.hook_recursive_call_forbidden",
        "Hook 绑定超过上限" => "app.backend_error.hook_bindings_too_many",
        "Hook 已取消" => "app.backend_error.hook_cancelled",
        "Hook 超过执行时限" => "app.backend_error.hook_timeout",
        "计划参数不是合法 JSON" => "app.backend_error.plan_params_invalid_json",
        "计划缺少非空 title" => "app.backend_error.plan_title_missing",
        "计划标题过长" => "app.backend_error.plan_title_too_long",
        "计划缺少 steps 数组" => "app.backend_error.plan_steps_missing",
        "计划至少需要一个步骤" => "app.backend_error.plan_steps_empty",
        "渲染快照超过单文件上限，请减少块数或缩小来源范围" => "app.backend_error.render_snapshot_too_large",
        "无法解析用户主目录，无法定位客户端配置" => "app.backend_error.home_dir_unresolved",
        "Redis DB 超出 0-15 的范围" => "app.backend_error.redis_db_out_of_range",
        "该目标不是 Redis 连接" => "app.backend_error.not_redis_connection",
        "Redis Cluster 只支持 DB 0" => "app.backend_error.redis_cluster_db_unsupported",
        "缺少 pattern" => "app.backend_error.missing_pattern",
        "pattern 过长" => "app.backend_error.pattern_too_long",
        "SCAN 游标必须是非负整数" => "app.backend_error.scan_cursor_negative_ascii",
        "已停止客户端扫描；不宣称撤销已发送命令" => "app.backend_error.scan_stopped_no_revocation",
        "SCAN Key 回包结构异常" => "app.backend_error.scan_key_reply_invalid",
        "扫描遇到二进制 Key，不能静默跳过；需二进制引用支持" => "app.backend_error.scan_binary_key_unsupported",
        "COUNT 非硬上限，本批超过 max_keys；本批未返回续页游标，请降低 count 或增加 max_keys" => "app.backend_error.scan_count_exceeds_max_keys",
        "缺少 key" => "app.backend_error.missing_key",
        "已停止后续读取；不宣称服务端撤销" => "app.backend_error.reads_stopped_no_revocation",
        "sections 最多 8 个" => "app.backend_error.sections_too_many",
        "channels 必须是 1-16 个通道名" => "app.backend_error.channels_invalid",
        "通道名过长或为空" => "app.backend_error.channel_name_invalid",
        "已取消，未发送 SUBSCRIBE" => "app.backend_error.subscribe_cancelled",
        "客户端已停止接收；已投递消息无法撤回，请核对频道实际状态" => "app.backend_error.subscribe_stopped_no_recall",
        "Redis 命令参数无效" => "app.backend_error.redis_command_args_invalid",
        "argv 必须是 1-64 个字符串" => "app.backend_error.argv_invalid",
        "argv 单项过长" => "app.backend_error.argv_entry_too_long",
        "已取消，未发送 Redis 命令" => "app.backend_error.redis_command_cancelled",
        "客户端停止与已发送命令竞态；请核对实际 Key 状态，不能宣称撤销写入" => "app.backend_error.redis_stop_race_unknown",
        "文件参数无效" => "app.backend_error.file_arg_invalid",
        "二进制附件不提供正文搜索" => "app.backend_error.binary_attachment_search_unsupported",
        "附件搜索读取失败" => "app.backend_error.attachment_search_read_failed",
        "附件不是 UTF-8 文本，无法按行搜索" => "app.backend_error.attachment_search_not_utf8",
        "任务附件快照损坏" => "app.backend_error.task_attachment_snapshot_corrupted",
        "AI 任务异常结束，请核对工具执行状态" => "app.backend_error.ai_task_abnormal_end",
        "Jev 配置声明缺少决策模型字段" => "app.backend_error.jev_decision_model_missing",
        "上下文压缩配置声明缺少压缩决策模型字段" => "app.backend_error.compaction_decision_model_missing",
        "BI 与报表输出能力未启用或已被当前 Agent 禁用；请启用该能力后重试，三个产物工具本身仍可用" => "app.backend_error.bi_output_disabled",
        "来源不属于本任务" => "app.backend_error.source_not_in_task",
        "来源结果不可用" => "app.backend_error.source_result_unavailable",
        "来源归属或执行状态不符" => "app.backend_error.source_ownership_mismatch",
        "来源回执损坏" => "app.backend_error.source_receipt_corrupted",
        "来源缺少列结构" => "app.backend_error.source_columns_missing",
        "来源列名重复，请通过 SQL 别名消除歧义" => "app.backend_error.source_duplicate_columns",
        "产物不属于本任务" => "app.backend_error.artifact_not_in_task",
        "产物来源必须给出 result_ref（结果句柄）" => "app.backend_error.artifact_result_ref_required",
        "ToolCallRef 与实际产出调用不一致" => "app.backend_error.artifact_tool_call_ref_mismatch",
        "产物没有内容块，不能完成" => "app.backend_error.artifact_blocks_empty",
        "renderer 输出超过宿主上限" => "app.backend_error.renderer_output_too_large",
        "产物快照超过宿主上限，请缩小来源范围" => "app.backend_error.artifact_snapshot_too_large",
        "产物快照内容核对不一致" => "app.backend_error.artifact_snapshot_mismatch",
        "该块已在产物中存在，无需重复添加；请以回执中的 blocks 为准，如需不同维度请显式声明另一个字段或图表类型" => "app.backend_error.artifact_block_duplicate",
        "产物没有可用来源，不能完成" => "app.backend_error.artifact_source_missing",
        "产物来源缺少可解析的结果句柄" => "app.backend_error.artifact_source_handle_invalid",
        "来源句柄与创建时版本不一致" => "app.backend_error.source_handle_version_mismatch",
        "没有需要提交的更改" => "app.backend_error.no_changes_to_commit",
        "该 Provider 不支持拉取模型列表，请手工填写模型 ID" => "app.backend_error.model_list_unsupported",
        "请先填写 API Key 再拉取模型列表" => "app.backend_error.model_list_api_key_required",
        "获取模型能力超时" => "app.backend_error.model_capabilities_timeout",
        "读取响应失败" => "app.backend_error.response_read_failed",
        "响应不是合法 JSON" => "app.backend_error.response_not_json",
        "未找到可用模型列表路由" => "app.backend_error.model_list_route_missing",
        "请求地址必须是合法 HTTP/HTTPS URL" => "app.backend_error.request_url_invalid",
        "请求地址不能包含凭据、查询串或片段，且必须使用 HTTP/HTTPS" => "app.backend_error.request_url_forbidden_parts",
        "未返回模型列表" => "app.backend_error.model_list_empty",
        "模型列表超过上限" => "app.backend_error.model_list_too_large",
        "未返回任何模型 ID，请检查 Endpoint 与 API Key" => "app.backend_error.model_ids_missing",
        "SQLite 数据库名称已存在" => "app.backend_error.sqlite_db_name_exists",
        "SQLite 新建数据库需要文件路径" => "app.backend_error.sqlite_db_path_required",
        "Redis Workbench 记录不存在" => "app.backend_error.redis_workbench_record_missing",
        "刷新结果页没有返回结果表" => "app.backend_error.refresh_result_table_missing",
        "刷新结果页没有返回结果摘要" => "app.backend_error.refresh_result_summary_missing",
        "结果页不存在" => "app.backend_error.result_page_missing",
        "没有待关闭的标签页" => "app.backend_error.no_tab_to_close",
        "表结构已在外部变化，请重新打开设计器并确认预览后再保存" => "app.backend_error.table_structure_changed",
        "表结构读取没有返回 DDL" => "app.backend_error.table_ddl_missing",
        "数据页尚未加载" => "app.backend_error.data_page_not_ready",
        "当前页找不到 Redis Key" => "app.backend_error.redis_key_not_on_page",
        "Redis Key 没有需要保存的修改" => "app.backend_error.redis_key_no_changes",
        "NULL 二进制没有可下载内容" => "app.backend_error.null_binary_no_content",
        "MySQL 用户与权限 provider 缺失" => "app.backend_error.mysql_user_admin_provider_missing",
        "外键不存在" => "app.backend_error.foreign_key_missing",
        "请选择目标表" => "app.backend_error.target_table_required",
        "AI 存储尚未初始化，请重启应用" => "app.backend_error.ai_storage_uninitialized",
        "该模型没有可显示的 API Key" => "app.backend_error.api_key_not_displayable",
        "该内置能力已下线，暂不可启停" => "app.backend_error.builtin_capability_retired",
        "该插件没有配置声明，无法保存配置" => "app.backend_error.plugin_config_missing",
        "完整 URL 模式必须填写请求地址" => "app.backend_error.full_url_mode_requires_url",
        "名称不能为空" => "app.backend_error.name_required",
        "默认主模型必须是生成模型" => "app.backend_error.default_model_not_generation",
        "Server Profile 名称不能为空" => "app.backend_error.server_profile_name_required",
        "数据源必须归属一个 Server Profile" => "app.backend_error.datasource_profile_required",
        "数据源引用的连接不存在" => "app.backend_error.datasource_connection_missing",
        "该目标暂不支持命令执行器" => "app.backend_error.command_executor_unsupported",
        "非表操作命令不应分发至表操作处理器" => "app.backend_error.non_table_command_dispatch_invalid",
        "会话已被其他窗口修改，请重新加载" => "app.backend_error.session_modified_elsewhere",
        "暂不支持新建数据库" => "app.backend_error.create_database_unsupported",
        "暂不支持新建 schema" => "app.backend_error.create_schema_unsupported",
        "暂不支持角色管理" => "app.backend_error.role_management_unsupported",
        "暂不支持创建角色" => "app.backend_error.create_role_unsupported",
        "暂不支持修改角色密码" => "app.backend_error.change_role_password_unsupported",
        "暂不支持重命名角色" => "app.backend_error.rename_role_unsupported",
        "暂不支持删除角色" => "app.backend_error.delete_role_unsupported",
        "暂不支持修改角色属性" => "app.backend_error.alter_role_unsupported",
        "暂不支持成员授权" => "app.backend_error.grant_membership_unsupported",
        "暂不支持撤销成员关系" => "app.backend_error.revoke_membership_unsupported",
        "暂不支持对象授权" => "app.backend_error.grant_object_unsupported",
        "暂不支持对象撤销" => "app.backend_error.revoke_object_unsupported",
        "暂不支持读取成员关系" => "app.backend_error.read_membership_unsupported",
        "暂不支持探测成员选项版本" => "app.backend_error.membership_options_probe_unsupported",
        "暂不支持读取对象权限" => "app.backend_error.read_object_privileges_unsupported",
        "暂不支持读取该对象的权限" => "app.backend_error.read_this_object_privileges_unsupported",
        "暂不支持读取角色有效权限" => "app.backend_error.read_effective_role_privileges_unsupported",
        "暂不支持渲染角色变更计划" => "app.backend_error.render_role_plan_unsupported",
        "暂不支持应用角色变更计划" => "app.backend_error.apply_role_plan_unsupported",
        "暂不支持读取权限目标列表" => "app.backend_error.read_privilege_targets_unsupported",
        "暂不支持删除数据库" => "app.backend_error.delete_database_unsupported",
        "暂不支持导出预览" => "app.backend_error.export_preview_unsupported",
        "该连接尚不支持有界写入提交" => "app.backend_error.bounded_write_commit_unsupported",
        "该连接暂不支持参数绑定；请改用内联字面量" => "app.backend_error.parameter_binding_unsupported",
        "该连接暂不支持命令执行器" => "app.backend_error.connection_command_executor_unsupported",
        "暂不支持二进制单元格读取" => "app.backend_error.binary_cell_read_unsupported",
        "暂不支持索引元数据" => "app.backend_error.index_metadata_unsupported",
        "暂不支持外键元数据" => "app.backend_error.foreign_key_metadata_unsupported",
        "暂不支持触发器元数据" => "app.backend_error.trigger_metadata_unsupported",
        "暂不支持 DDL 元数据" => "app.backend_error.ddl_metadata_unsupported",
        "产物标题不能为空" => "app.backend_error.artifact_title_required",
        "产物标题超过长度上限" => "app.backend_error.artifact_title_too_long",
        "renderer ID 缺失或超过长度上限" => "app.backend_error.renderer_id_invalid",
        "产物必须声明至少一个来源" => "app.backend_error.artifact_source_required",
        "产物来源条数超过上限" => "app.backend_error.artifact_sources_too_many",
        "产物来源必须至少包含 result_ref 或 tool_call_ref" => "app.backend_error.artifact_source_ref_required",
        "产物来源重复" => "app.backend_error.artifact_source_duplicate",
        "产物块数超过上限" => "app.backend_error.artifact_blocks_too_many",
        "可访问性描述超过长度上限" => "app.backend_error.accessibility_description_too_long",
        "过滤条件条数超过上限" => "app.backend_error.filters_too_many",
        "时间范围必须使用有效 RFC3339" => "app.backend_error.time_range_rfc3339_invalid",
        "时间范围起始晚于结束" => "app.backend_error.time_range_reversed",
        "产物类型与块类型不一致" => "app.backend_error.artifact_block_type_mismatch",
        "表格块至少要有一列" => "app.backend_error.table_block_columns_required",
        "表格块列数超过上限" => "app.backend_error.table_block_columns_too_many",
        "表格块列名重复" => "app.backend_error.table_block_column_duplicate",
        "说明块正文不能为空" => "app.backend_error.note_block_body_required",
        "说明块正文超过长度上限" => "app.backend_error.note_block_body_too_long",
        "影响范围或回滚能力无法核实，需要人工审核" => "app.backend_error.impact_scope_unverifiable",
        "自动放行阈值必须在 0–1 之间" => "app.backend_error.auto_approval_threshold_invalid",
        "查询失败" => "app.backend_error.title_query_failed",
        "保存失败" => "app.backend_error.title_save_failed",
        "刷新失败" => "app.backend_error.title_refresh_failed",
        "加载失败" => "app.backend_error.title_load_failed",
        "元信息加载失败" => "app.backend_error.title_metadata_load_failed",
        "下载失败" => "app.backend_error.title_download_failed",
        "新建失败" => "app.backend_error.title_create_failed",
        "ER 关系加载失败" => "app.backend_error.title_er_load_failed",
        "ER 关系创建失败" => "app.backend_error.title_er_create_failed",
        "ER 关系更新失败" => "app.backend_error.title_er_update_failed",
        "ER 自动重绑失败" => "app.backend_error.title_er_rebind_failed",
        "ER 关系确认失败" => "app.backend_error.title_er_confirm_failed",
        "ER 关系拒绝失败" => "app.backend_error.title_er_reject_failed",
        "ER 关系删除失败" => "app.backend_error.title_er_delete_failed",
        "连接失败" => "app.backend_error.title_connection_failed",
        "连接超时" => "app.backend_error.title_connection_timeout",
        "结果未知" => "app.backend_error.title_outcome_unknown",
        "摘要未正常结束，拒绝提交" => "app.backend_error.app_msg_summary_unfinished",
        "解析器已结束" => "app.backend_error.app_msg_parser_finished",
        "模型 token 用量无效" => "app.backend_error.app_msg_model_token_usage_invalid",
        "choices 不是数组" => "app.backend_error.app_msg_choices_not_array",
        "choice 不是对象" => "app.backend_error.app_msg_choice_not_object",
        "delta 不是对象" => "app.backend_error.app_msg_delta_not_object",
        "模型 token 用量溢出" => "app.backend_error.app_msg_model_token_usage_overflow",
        "外键名称不能为空" => "app.backend_error.app_msg_fk_name_required",
        "外键名称不能重复" => "app.backend_error.app_msg_fk_name_duplicate",
        "外键至少需要选择一个字段" => "app.backend_error.app_msg_fk_columns_required",
        "外键字段不能为空" => "app.backend_error.app_msg_fk_column_required",
        "外键引用了不存在的字段" => "app.backend_error.app_msg_fk_column_missing",
        "外键字段不能重复" => "app.backend_error.app_msg_fk_column_duplicate",
        "目标字段不能为空" => "app.backend_error.app_msg_fk_target_column_required",
        "目标字段数量需要与外键字段一致" => "app.backend_error.app_msg_fk_target_column_count_mismatch",
        "目标字段不能重复" => "app.backend_error.app_msg_fk_target_column_duplicate",
        "摘要为空" => "app.backend_error.app_msg_summary_empty",
        "摘要疑似被截断" => "app.backend_error.app_msg_summary_truncated",
        "索引名称不能为空" => "app.backend_error.app_msg_index_name_required",
        "索引名称不能为 PRIMARY" => "app.backend_error.app_msg_index_name_primary_reserved",
        "PostgreSQL 索引不支持前缀长度" => "app.backend_error.app_msg_pg_index_prefix_unsupported",
        "页面数据所属连接不可用" => "app.backend_error.app_msg_page_connection_unavailable",
        "页面快照登记失败" => "app.backend_error.app_msg_page_snapshot_register_failed",
        "重命名只接受单段新表名（不带 schema）" => "app.backend_error.app_msg_rename_single_table_name",
        "当前连接类型暂不支持重命名表" => "app.backend_error.app_msg_rename_table_unsupported",
        "当前连接类型暂不支持复制表" => "app.backend_error.app_msg_copy_table_unsupported",
        "当前连接类型暂不支持删除表" => "app.backend_error.app_msg_drop_table_unsupported",
        "当前连接类型暂不支持清空表" => "app.backend_error.app_msg_truncate_table_unsupported",
        "未读取到可复制的 SQLite 表结构 DDL" => "app.backend_error.app_msg_sqlite_copy_ddl_missing",
        "未读取到 SQLite 表名" => "app.backend_error.app_msg_sqlite_table_name_missing",
        "SQLite 表名引用未闭合" => "app.backend_error.app_msg_sqlite_table_name_unclosed",
        "未提交（事务进行中）" => "app.backend_error.app_msg_transaction_pending",
        "已回滚（未提交或显式 ROLLBACK）" => "app.backend_error.app_msg_transaction_rolled_back",
        "结果待核实" => "app.backend_error.app_msg_result_pending_verification",
        "执行失败" => "app.backend_error.app_msg_execution_failed",
        "单个条目超过本页字节预算，请增大 max_bytes 或缩小来源字段" => "app.backend_error.app_msg_artifact_entry_exceeds_budget",
        "饼图不适用于含负值的数值列，未渲染。" => "app.backend_error.app_msg_pie_negative_values_unsupported",
        "饼图数值合计为 0，无法计算占比，未渲染。" => "app.backend_error.app_msg_pie_zero_total",
        "产物标记为就绪但缺少快照路径。" => "app.backend_error.app_msg_artifact_snapshot_path_missing",
        "产物快照内容与记录不一致，拒绝展示可能被篡改的数据。" => "app.backend_error.app_msg_artifact_snapshot_tampered",
        "表 DDL 不存在" => "app.backend_error.app_msg_table_ddl_missing",
        "没有需要保存的变更" => "app.backend_error.app_msg_no_changes_to_save",
        "该任务或工具调用已被处理，拒绝重复执行" => "app.backend_error.app_msg_task_already_processed",
        "当前 Agent 已停用或不再允许该工具" => "app.backend_error.app_msg_agent_tool_revoked",
        "批准后的参数未通过校验" => "app.backend_error.app_msg_approved_params_failed",
        "执行策略已改为只读，旧审批不得放行" => "app.backend_error.app_msg_policy_now_read_only",
        "工具没有可执行实现" => "app.backend_error.app_msg_tool_no_executable_impl",
        "目标存在尚未核实的未知执行结果，拒绝冲突写入" => "app.backend_error.app_msg_unknown_write_conflict",
        "参数、目标或工具版本已变化，请重新核实并重新审批" => "app.backend_error.app_msg_plan_verification_changed",
        "当前安全审计或权限拒绝恢复执行" => "app.backend_error.app_msg_audit_denies_resume",
        "批准与当前 plan_hash 不匹配或已过期，拒绝执行" => "app.backend_error.app_msg_approval_plan_hash_mismatch",
        "该任务已结束或执行结果未知，不能再批准或重放" => "app.backend_error.app_msg_task_finished_or_unknown",
        "审批已过期，请重新核实" => "app.backend_error.app_msg_approval_expired",
        "参数必须匹配且仅匹配一种声明形状" => "app.backend_error.app_msg_params_shape_ambiguous",
        "参数不匹配声明的固定取值" => "app.backend_error.app_msg_params_fixed_value_mismatch",
        "安装已取消" => "app.backend_error.app_msg_install_cancelled",
        "SQLite 暂不支持通过设计表修改已有字段类型或约束" => "app.backend_error.app_msg_sqlite_design_type_change_unsupported",
        "PostgreSQL 不支持 NOT ENFORCED 检查约束" => "app.backend_error.app_msg_pg_not_enforced_unsupported",
        "触发器名称不能为空" => "app.backend_error.app_msg_trigger_name_required",
        "触发器定义不能为空" => "app.backend_error.app_msg_trigger_definition_required",
        "请输入表名" => "app.backend_error.app_msg_table_name_required_hint",
        "请至少填写一个字段" => "app.backend_error.app_msg_columns_required",
        "字段名不能重复" => "app.backend_error.app_msg_column_name_duplicate",
        "字段类型不能为空" => "app.backend_error.app_msg_column_type_required",
        "自增字段必须是整数主键" => "app.backend_error.app_msg_auto_increment_integer_pk_required",
        "字符串默认值需要用引号包裹" => "app.backend_error.app_msg_string_default_quoted_required",
        "主键字段名不能为空" => "app.backend_error.app_msg_primary_key_column_required",
        "检查名称不能重复" => "app.backend_error.app_msg_check_name_duplicate",
        "检查表达式不能为空" => "app.backend_error.app_msg_check_expression_required",
        "当前连接类型不支持不强制实施检查" => "app.backend_error.app_msg_not_enforced_unsupported",
        "触发器名称不能重复" => "app.backend_error.app_msg_trigger_name_duplicate",
        "触发器触发时机无效" => "app.backend_error.app_msg_trigger_timing_invalid",
        "请选择触发器事件" => "app.backend_error.app_msg_trigger_event_required",
        "当前连接类型不支持分区" => "app.backend_error.app_msg_partition_unsupported",
        "请填写分区定义 SQL" => "app.backend_error.app_msg_partition_sql_required",
        "分区定义需要以 PARTITION BY 开头" => "app.backend_error.app_msg_partition_by_required",
        "当前连接类型暂不支持创建表" => "app.backend_error.app_msg_create_table_unsupported",
        "当前连接类型暂不支持设计表" => "app.backend_error.app_msg_design_table_unsupported",
        "PostgreSQL identity 只能用于整数类型列" => "app.backend_error.app_msg_pg_identity_integer_required",
        "PostgreSQL 触发器需要引用已存在的函数，不支持行内 BEGIN…END 触发器体" => "app.backend_error.app_msg_pg_trigger_function_required",
        "SQLite 自增字段必须是整数主键" => "app.backend_error.app_msg_sqlite_autoincrement_integer_pk",
        "SQLite 自增字段只能是单列整数主键" => "app.backend_error.app_msg_sqlite_autoincrement_single_pk",
        "未读取到源表达梦 DDL，无法复制表" => "app.backend_error.app_msg_dm_copy_ddl_missing",
        "未读取到可复制的达梦表结构 DDL" => {
            "app.backend_error.app_msg_dm_copy_structure_missing"
        }
        "达梦表结构 DDL 引号未闭合" => "app.backend_error.app_msg_dm_ddl_quote_unclosed",
        "达梦 DDL 缺少 CREATE TABLE" => {
            "app.backend_error.app_msg_dm_ddl_create_table_missing"
        }
        "达梦 CREATE TABLE 缺少字段定义" => {
            "app.backend_error.app_msg_dm_ddl_columns_missing"
        }
        "达梦索引 DDL 缺少 INDEX" => "app.backend_error.app_msg_dm_ddl_index_missing",
        "达梦索引 DDL 缺少索引名" => "app.backend_error.app_msg_dm_ddl_index_name_missing",
        "达梦索引 DDL 缺少 ON" => "app.backend_error.app_msg_dm_ddl_index_on_missing",
        "达梦索引 DDL 缺少索引字段" => {
            "app.backend_error.app_msg_dm_ddl_index_columns_missing"
        }
        "达梦 DDL 标识符未引用" => "app.backend_error.app_msg_dm_ddl_identifier_unquoted",
        "达梦 DDL 标识符引用未闭合" => {
            "app.backend_error.app_msg_dm_ddl_identifier_unclosed"
        }
        "达梦自增字段不能同时设置默认值" => {
            "app.backend_error.app_msg_dm_identity_default_conflict"
        }
        "达梦建表向导暂不支持创建触发器" => "app.backend_error.app_msg_dm_trigger_unsupported",
        "达梦建表向导暂不支持分区" => "app.backend_error.app_msg_dm_partition_unsupported",
        "达梦设计表暂不支持指定哈希索引" => {
            "app.backend_error.app_msg_dm_hash_index_unsupported"
        }
        "达梦索引不支持前缀长度" => "app.backend_error.app_msg_dm_index_prefix_unsupported",
        "达梦设计表暂不支持修改触发器" => {
            "app.backend_error.app_msg_dm_trigger_design_unsupported"
        }
        "达梦设计表暂不支持移除自增属性" => {
            "app.backend_error.app_msg_dm_identity_remove_unsupported"
        }
        "达梦不支持 NOT ENFORCED 检查约束" => {
            "app.backend_error.app_msg_dm_not_enforced_unsupported"
        }
        "达梦不支持以连接级开关临时禁用外键检查" => {
            "app.backend_error.app_msg_dm_fk_check_unsupported"
        }
        "导入文件格式必须是 TXT、CSV、JSON 或 XML" => {
            "app.backend_error.app_msg_import_format_unsupported"
        }
        "表名不能为空" => "app.backend_error.app_msg_table_name_required",
        "表名没有变化" => "app.backend_error.app_msg_table_name_unchanged",
        "新表名不能和原表相同" => "app.backend_error.app_msg_table_name_same_as_original",
        "当前连接类型不支持外键检查选项" => "app.backend_error.app_msg_fk_check_option_unsupported",
        "搜索词不能为空" => "app.backend_error.app_msg_search_term_required",
        "搜索游标不在 UTF-8 边界" => "app.backend_error.app_msg_search_cursor_utf8_boundary",
        "正文搜索已取消" => "app.backend_error.app_msg_content_search_cancelled",
        "正文搜索参数不是合法 JSON" => "app.backend_error.app_msg_content_search_args_invalid",
        "页面读取已取消" => "app.backend_error.app_msg_page_read_cancelled",
        "页面快照不属于本轮任务" => "app.backend_error.app_msg_page_snapshot_wrong_turn",
        "参数无效" => "app.backend_error.app_msg_invalid_parameters",
        "页面引用未授权" => "app.backend_error.app_msg_page_reference_unauthorized",
        "偏移超限" => "app.backend_error.app_msg_offset_out_of_range",
        "偏移超出已加载快照" => "app.backend_error.app_msg_offset_exceeds_snapshot",
        "列索引无效" => "app.backend_error.app_msg_column_index_invalid",
        "列索引超出快照" => "app.backend_error.app_msg_column_index_exceeds_snapshot",
        "单行超过输出预算，请用 columns 指定更少的列" => "app.backend_error.app_msg_row_exceeds_output_budget",
        "开关只能取 true 或 false" => "app.backend_error.app_msg_switch_boolean_required",
        "缺少 decision 答案" => "app.backend_error.app_msg_decision_answer_missing",
        "答案缺少 type" => "app.backend_error.app_msg_decision_answer_type_missing",
        "choice 不是字符串" => "app.backend_error.app_msg_decision_choice_not_string",
        "probabilities 不是对象" => "app.backend_error.app_msg_decision_probabilities_not_object",
        "概率不是有限数" => "app.backend_error.app_msg_decision_probability_not_finite",
        "概率超出 0–1" => "app.backend_error.app_msg_decision_probability_out_of_range",
        "score 不是有限数" => "app.backend_error.app_msg_decision_score_not_finite",
        "confidence 不是有限数" => "app.backend_error.app_msg_decision_confidence_not_finite",
        "confidence 超出 0–1" => "app.backend_error.app_msg_decision_confidence_out_of_range",
        "决策请求没有候选" => "app.backend_error.app_msg_decision_no_candidates",
        "响应缺少 answers" => "app.backend_error.app_msg_decision_answers_missing",
        "下载已取消" => "app.backend_error.app_msg_download_cancelled",
        "编辑器续读已取消" => "app.backend_error.app_msg_editor_continuation_cancelled",
        "编辑器续读参数不是合法 JSON" => "app.backend_error.app_msg_editor_continuation_args_invalid",
        "offset 超出范围" => "app.backend_error.app_msg_editor_offset_out_of_range",
        "byte_offset 不在起始行的 UTF-8 边界" => "app.backend_error.app_msg_byte_offset_utf8_boundary",
        "编辑器搜索已取消" => "app.backend_error.app_msg_editor_search_cancelled",
        "编辑器搜索参数不是合法 JSON" => "app.backend_error.app_msg_editor_search_args_invalid",
        "编辑器全文登记锁已损坏" => "app.backend_error.app_msg_editor_registry_lock_corrupted",
        "决策未给出稳定选择" => "app.backend_error.app_msg_decision_no_stable_choice",
        "决策选择不在候选集合内" => "app.backend_error.app_msg_decision_choice_not_candidate",
        "决策缺少被选候选的概率" => "app.backend_error.app_msg_decision_missing_candidate_probability",
        "候选概率无明显优势" => "app.backend_error.app_msg_decision_no_clear_advantage",
        "决策置信度不足" => "app.backend_error.app_msg_decision_confidence_low",
        "未确认的口径假设；仅按来源原始字段展示，不视为业务事实" => "app.backend_error.app_msg_metric_assumption_unconfirmed",
        "ER 存储尚未初始化" => "app.backend_error.app_msg_er_storage_uninitialized",
        "字段不存在或已删除" => "app.backend_error.app_msg_column_missing_or_deleted",
        "完整 URL 模式不自动获取模型列表，请手工填写模型 ID" => "app.backend_error.app_msg_full_url_model_list_manual",
        "IPv6 地址缺少结尾 `]`" => "app.backend_error.app_msg_ipv6_closing_bracket_missing",
        "主机不能为空" => "app.backend_error.app_msg_host_required",
        "请填写主机" => "app.backend_error.app_msg_host_required_hint",
        "端口不合法" => "app.backend_error.app_msg_port_invalid",
        "verify-full 模式需提供主机名或显式 TLS server_name" => "app.backend_error.app_msg_verify_full_server_name_required",
        "请填写 SSH 隧道主机" => "app.backend_error.app_msg_ssh_host_required",
        "SSH 端口不合法" => "app.backend_error.app_msg_ssh_port_invalid",
        "请填写 SSH 用户名" => "app.backend_error.app_msg_ssh_username_required",
        "请填写 SSH 私钥文件路径" => "app.backend_error.app_msg_ssh_key_path_required",
        "高级超时值不能超过 86400 秒" => "app.backend_error.app_msg_advanced_timeout_too_large",
        "强制 TLS 模式需先启用 TLS" => "app.backend_error.app_msg_tls_require_enabled_first",
        "代理传输暂未支持，请先关闭代理或仅使用 SSH 隧道" => "app.backend_error.app_msg_proxy_transport_unsupported",
        "无法识别的连接字符串" => "app.backend_error.app_msg_connection_string_unrecognized",
        "缺少主机地址" => "app.backend_error.app_msg_host_address_missing",
        "请填写 Sentinel 主库名" => "app.backend_error.app_msg_sentinel_master_name_required",
        "请至少填写一个 Sentinel 节点" => "app.backend_error.app_msg_sentinel_node_required",
        "请至少填写一个 Cluster 节点" => "app.backend_error.app_msg_cluster_node_required",
        "上下文接近上限，但暂无可安全压缩的完整历史" => {
            "app.backend_error.ctx_near_limit_no_history"
        }
        "上下文已超出模型窗口，但没有可安全压缩的新增历史，本轮已暂停" => {
            "app.backend_error.ctx_over_limit_no_history"
        }
        "输出预留空间不足" => "app.backend_error.ctx_output_reserve_insufficient",
        "AI 运行时繁忙，请稍后重试" => "app.backend_error.ai_runtime_busy",
        "AI 运行时已关闭" => "app.backend_error.ai_runtime_closed",
        "AI 运行时未在运行" => "app.backend_error.ai_runtime_not_running",
        "读取超时" => "app.backend_error.read_timeout",
        "连接提前结束" => "app.backend_error.connection_ended_early",
        "超时" => "app.backend_error.timeout",
        "传输失败" => "app.backend_error.transfer_failed",
        "模型未返回正文。" => "app.backend_error.model_no_content",
        "已停止后续生成。" => "app.backend_error.ai_generation_stopped",
        "不允许链接附件" => "app.backend_error.attachment_link_forbidden",
        "需要绝对文件路径" => "app.backend_error.absolute_path_required",
        "路径含空字符" => "app.backend_error.path_null_char",
        "数据来源的结果句柄 ID，须已在 sources 中声明" => {
            "app.backend_error.data_source_handle_declared"
        }
        "~/.claude.json 根不是对象，拒绝改写" => {
            "app.backend_error.claude_json_root_not_object"
        }
        _ => {
            return localize_backend_dynamic_error(text)
                .or_else(|| localize_backend_connector_error_text(text))
                .or_else(|| localize_backend_storage_error_text(text))
                .or_else(|| localize_backend_template_error(text))
                .or_else(|| localize_backend_external_error_text(text))
                .unwrap_or_else(|| text.to_string());
        }
    };
    tr(key).to_string()
}

fn localize_backend_template(key: &'static str, pairs: &[(&str, &str)]) -> String {
    let mut out = tr(key).to_string();
    for (placeholder, value) in pairs {
        out = out.replace(placeholder, value);
    }
    out
}

fn localize_backend_single_slot(
    text: &str,
    key: &'static str,
    prefix: &str,
    suffix: &str,
) -> Option<String> {
    let rest = text.strip_prefix(prefix)?;
    let value = rest.strip_suffix(suffix)?;
    Some(localize_backend_template(key, &[("%{value}", value)]))
}

fn localize_backend_two_slot(
    text: &str,
    key: &'static str,
    prefix: &str,
    middle: &str,
    suffix: &str,
) -> Option<String> {
    let rest = text.strip_prefix(prefix)?;
    let (first, second) = rest.split_once(middle)?;
    let second = second.strip_suffix(suffix)?;
    Some(localize_backend_template(
        key,
        &[("%{first}", first), ("%{second}", second)],
    ))
}

fn localize_backend_parameter_error(text: &str) -> Option<String> {
    let rest = text.strip_prefix("parameters[")?;
    let (index, tail) = rest.split_once("] ")?;
    let (key, value) = match tail {
        "不是合法 base64" => ("app.backend_error.dyn_parameter_base64_invalid", None),
        "不是合法 i64 文本" => ("app.backend_error.dyn_parameter_i64_invalid", None),
        "不是合法 u64 文本" => ("app.backend_error.dyn_parameter_u64_invalid", None),
        "不是合法十进制文本" => ("app.backend_error.dyn_parameter_decimal_invalid", None),
        "不是布尔值" => ("app.backend_error.dyn_parameter_not_bool", None),
        "不是数值" => ("app.backend_error.dyn_parameter_not_number", None),
        "必须是有限数值" => ("app.backend_error.dyn_parameter_not_finite", None),
        "缺少字符串 value" => ("app.backend_error.dyn_parameter_missing_string", None),
        _ => {
            if let Some(value) = tail.strip_prefix("字节数超过 ") {
                ("app.backend_error.dyn_parameter_bytes_limit", Some(value))
            } else if let Some(value) = tail
                .strip_prefix("文本超过 ")
                .and_then(|value| value.strip_suffix(" 字节"))
            {
                ("app.backend_error.dyn_parameter_text_limit", Some(value))
            } else {
                let value = tail.strip_prefix("未知类型标记: ")?;
                ("app.backend_error.dyn_parameter_unknown_tag", Some(value))
            }
        }
    };
    let translated = match value {
        Some(value) => localize_backend_template(key, &[("%{value}", value)]),
        None => tr(key).to_string(),
    };
    Some(format!("parameters[{index}] {translated}"))
}

fn localize_backend_dynamic_error(text: &str) -> Option<String> {
    if let Some(out) = localize_backend_parameter_error(text) {
        return Some(out);
    }
    if let Some(reason) = text
        .strip_prefix("正在压缩上下文（")
        .and_then(|value| value.strip_suffix("）…"))
    {
        let reason = localize_backend_error_text(reason);
        return Some(localize_backend_template(
            "app.backend_error.ctx_compacting",
            &[("%{value}", &reason)],
        ));
    }
    if let Some(reason) = text
        .strip_prefix("已自动压缩上下文（")
        .and_then(|value| value.strip_suffix("）；原始历史与审计保持不变"))
    {
        let reason = localize_backend_error_text(reason);
        return Some(localize_backend_template(
            "app.backend_error.ctx_compacted",
            &[("%{value}", &reason)],
        ));
    }
    if let Some(notice) = text.strip_suffix(
        "；上下文已超出模型窗口，本轮不会发出超限请求，请调整输入或改用更大窗口的模型后重试",
    ) {
        let notice = localize_backend_error_text(notice);
        return Some(localize_backend_template(
            "app.backend_error.ctx_over_limit_retry",
            &[("%{value}", &notice)],
        ));
    }

    const TWO_SLOT: &[(&str, &str, &str, &str)] = &[
        (
            "app.backend_error.dyn_artifact_schema_unsupported",
            "不支持的产物 schema 版本 ",
            "（当前 ",
            "）",
        ),
        (
            "app.backend_error.dyn_command_keys_limit",
            "单条命令涉及 ",
            " 个 Key，超过上限 ",
            "",
        ),
        (
            "app.backend_error.dyn_renderer_unavailable_with_list",
            "没有可用的 renderer ",
            "（当前可用：",
            "）",
        ),
        (
            "app.backend_error.dyn_target_db_out_of_scope",
            "目标 DB ",
            " 不在授权范围（仅授权 DB ",
            "）",
        ),
        (
            "app.backend_error.dyn_key_length_limit",
            "Key 长度 ",
            " 超过上限 ",
            "",
        ),
        (
            "app.backend_error.dyn_reason_pause_save_failed",
            "",
            "（暂停状态保存也失败：",
            "）",
        ),
        (
            "app.backend_error.dyn_tool_input_schema_invalid",
            "工具 ",
            " 的 input_schema 不是合法 JSON: ",
            "",
        ),
        (
            "app.backend_error.dyn_tool_name_conflict",
            "工具名映射冲突: ",
            " 与 ",
            "",
        ),
        (
            "app.backend_error.dyn_source_field_missing",
            "来源 ",
            " 的结果里没有字段 ",
            "",
        ),
        (
            "app.backend_error.dyn_source_handle_unparsable",
            "来源句柄不可解析（",
            "）：",
            "",
        ),
        (
            "app.backend_error.dyn_plan_step_status_invalid",
            "第 ",
            " 步状态非法：",
            "",
        ),
        (
            "app.backend_error.ctx_compaction_unavailable_model",
            "上下文压缩未执行（摘要模型 ",
            " 不可用：",
            "）",
        ),
        (
            "app.backend_error.ctx_compaction_unavailable_connection",
            "上下文压缩未执行（摘要模型 ",
            " 无法建立连接：",
            "）",
        ),
    ];
    for (key, prefix, middle, suffix) in TWO_SLOT {
        if let Some(out) = localize_backend_two_slot(text, key, prefix, middle, suffix) {
            return Some(out);
        }
    }

    const SINGLE_SLOT: &[(&str, &str, &str)] = &[
        (
            "app.backend_error.dyn_upload_too_large",
            "上传文件超过 ",
            " MB，超过单个大字段的暂存上限，请拆分或缩小文件后重试",
        ),
        (
            "app.backend_error.dyn_readwrite_statement_rejected",
            "允许读写仅放行数据修改（INSERT/UPDATE/DELETE），当前语句被拒绝：",
            "",
        ),
        (
            "app.backend_error.dyn_builtin_capability_registry_mismatch",
            "内置能力 ",
            " 与存储层插件注册表不一致",
        ),
        (
            "app.backend_error.dyn_tool_receipt_save_failed",
            "工具已执行但回执保存失败，已标记为结果未知：",
            "",
        ),
        (
            "app.backend_error.dyn_key_prefix_out_of_scope",
            "Key「",
            "」不在授权的 prefix 范围内",
        ),
        (
            "app.backend_error.dyn_sse_frame_too_large",
            "SSE 帧超过 ",
            " 字节上限",
        ),
        (
            "app.backend_error.dyn_target_segment_invalid",
            "target 片段无法识别: ",
            "",
        ),
        (
            "app.backend_error.dyn_turns_range_invalid",
            "turns 必须是 1..=",
            " 的整数",
        ),
        (
            "app.backend_error.dyn_label_missing_or_too_long",
            "",
            "缺失或超过长度上限",
        ),
        (
            "app.backend_error.dyn_artifact_type_unsupported",
            "不支持产物类型 ",
            "",
        ),
        (
            "app.backend_error.dyn_object_type_unsupported",
            "不支持的对象类型: ",
            "",
        ),
        (
            "app.backend_error.dyn_artifact_blocks_limit",
            "产物块数已达上限 ",
            "",
        ),
        (
            "app.backend_error.dyn_artifact_completed",
            "产物已完成（",
            "），不能再增加块",
        ),
        (
            "app.backend_error.dyn_artifact_snapshot_read_failed",
            "产物快照无法回读: ",
            "",
        ),
        (
            "app.backend_error.dyn_artifact_status_cannot_complete",
            "产物状态 ",
            " 不能再完成",
        ),
        (
            "app.backend_error.dyn_builtin_capability_unregistered",
            "内置能力 ",
            " 未在存储层插件注册表中登记",
        ),
        (
            "app.backend_error.dyn_artifact_snapshot_write_failed",
            "写产物快照失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_decision_request_serialize_failed",
            "决策请求序列化失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_artifact_dir_create_failed",
            "创建产物目录失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_parameter_count_limit",
            "参数个数超过上限 ",
            "",
        ),
        (
            "app.backend_error.dyn_parameterized_execution_failed",
            "参数化执行失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_cancel_state_save_failed",
            "取消状态保存失败：",
            "",
        ),
        (
            "app.backend_error.dyn_readonly_statement_rejected",
            "只读入口拒绝该语句（",
            "）",
        ),
        (
            "app.backend_error.dyn_command_policy_rejected",
            "命令被策略拒绝（",
            "）",
        ),
        (
            "app.backend_error.dyn_response_body_too_large",
            "响应体超过 ",
            " 字节上限",
        ),
        (
            "app.backend_error.dyn_response_format_invalid",
            "响应格式错误: ",
            "",
        ),
        (
            "app.backend_error.dyn_chart_y_not_numeric",
            "图表 y 字段 ",
            " 不是数值列，不能作为指标",
        ),
        (
            "app.backend_error.dyn_block_source_undeclared",
            "块引用了未声明的来源 ",
            "；请先用 artifact.create 声明来源",
        ),
        (
            "app.backend_error.dyn_multi_statement_scope_failed",
            "多语句无法校验作用域（",
            " 条）",
        ),
        (
            "app.backend_error.dyn_tool_spec_missing",
            "工具 ",
            " 没有对应的 ToolSpec，拒绝注册",
        ),
        (
            "app.backend_error.dyn_tool_not_authorized",
            "工具 ",
            " 在当前授权下不可用",
        ),
        (
            "app.backend_error.dyn_tool_datasource_policy_unavailable",
            "工具 ",
            " 在当前数据源策略下不可用",
        ),
        (
            "app.backend_error.dyn_tool_no_implementation",
            "工具 ",
            " 没有可执行实现",
        ),
        (
            "app.backend_error.dyn_tool_execution_failed",
            "工具执行失败：",
            "",
        ),
        (
            "app.backend_error.dyn_approved_params_invalid",
            "批准后的参数未通过校验：",
            "",
        ),
        (
            "app.backend_error.dyn_model_list_fetch_failed",
            "拉取模型列表失败：HTTP ",
            "",
        ),
        (
            "app.backend_error.dyn_artifact_snapshot_commit_failed",
            "提交产物快照失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_plugin_config_invalid",
            "插件配置声明无效：",
            "",
        ),
        (
            "app.backend_error.dyn_database_execution_failed",
            "数据库执行失败：",
            "",
        ),
        (
            "app.backend_error.dyn_command_key_scope_undetermined",
            "无法静态确定该命令的 Key 范围，已拒绝执行（",
            "）",
        ),
        (
            "app.backend_error.dyn_timezone_invalid",
            "时区仅支持 UTC 或有效的 ±HH:MM 固定偏移；收到 ",
            "，IANA 时区名称尚不支持",
        ),
        (
            "app.backend_error.dyn_datasource_handle_invalid",
            "未授权或已失效的数据源句柄: ",
            "",
        ),
        (
            "app.backend_error.dyn_artifact_type_unknown",
            "未知产物类型: ",
            "",
        ),
        (
            "app.backend_error.dyn_tool_unknown",
            "未知工具: ",
            "",
        ),
        (
            "app.backend_error.dyn_renderer_unavailable",
            "没有可用的 renderer ",
            "",
        ),
        (
            "app.backend_error.dyn_test_request_serialize_failed",
            "测试请求序列化失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_plan_step_title_too_long",
            "第 ",
            " 步标题过长",
        ),
        (
            "app.backend_error.dyn_plan_step_title_missing",
            "第 ",
            " 步缺少非空 title",
        ),
        (
            "app.backend_error.dyn_result_save_failed",
            "结果保存失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_result_serialize_failed",
            "结果序列化失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_missing_redis_tool_definition",
            "缺少 Redis 工具定义: ",
            "",
        ),
        (
            "app.backend_error.dyn_missing_sql_tool_definition",
            "缺少标准 SQL 工具定义: ",
            "",
        ),
        ("app.backend_error.dyn_network_error", "网络错误: ", ""),
        (
            "app.backend_error.dyn_plan_steps_limit",
            "计划步骤数超过上限 ",
            "",
        ),
        (
            "app.backend_error.dyn_review_weight_invalid",
            "评审模型权重必须在 0–",
            " 之间",
        ),
        (
            "app.backend_error.dyn_command_kernel_denied",
            "该命令在内核拒绝列表中，未执行（",
            "）",
        ),
        (
            "app.backend_error.dyn_statement_policy_rejected",
            "语句被策略拒绝（",
            "）",
        ),
        (
            "app.backend_error.dyn_controlled_dir_read_failed",
            "读取受控目录失败: ",
            "",
        ),
        (
            "app.backend_error.dyn_connection_not_found_with_id",
            "连接不存在: ",
            "",
        ),
        (
            "app.backend_error.dyn_predefined_role_readonly",
            "预定义角色「",
            "」由系统管理，不允许修改名称、属性或密码",
        ),
        (
            "app.backend_error.ctx_over_limit_retry",
            "",
            "；上下文已超出模型窗口，本轮不会发出超限请求，请调整输入或改用更大窗口的模型后重试",
        ),
        (
            "app.backend_error.ctx_compacting",
            "正在压缩上下文（",
            "）…",
        ),
        (
            "app.backend_error.ctx_compaction_failed_summary",
            "上下文压缩未完成（摘要失败：",
            "）",
        ),
        (
            "app.backend_error.ctx_compaction_failed",
            "上下文压缩未完成（",
            "）",
        ),
        (
            "app.backend_error.ctx_compaction_not_submitted",
            "上下文压缩未提交（",
            "）",
        ),
        (
            "app.backend_error.ctx_compacted",
            "已自动压缩上下文（",
            "）；原始历史与审计保持不变",
        ),
        (
            "app.backend_error.ctx_reason_percent",
            "达到上下文 ",
            "%",
        ),
        (
            "app.backend_error.ctx_keep_recent",
            "保留最近 ",
            " 条历史",
        ),
        (
            "app.backend_error.ctx_into_summary",
            "进入摘要 ",
            " 条",
        ),
        (
            "app.backend_error.network_error_reason",
            "网络错误: ",
            "",
        ),
        (
            "app.backend_error.connector_create_failed",
            "无法创建连接器: ",
            "",
        ),
        (
            "app.backend_error.artifact_declaration_corrupted",
            "产物声明损坏: ",
            "",
        ),
        (
            "app.backend_error.download_client_failed",
            "下载客户端失败：",
            "",
        ),
        (
            "app.backend_error.reply_save_failed",
            "回复保存失败：",
            "",
        ),
        ("app.backend_error.command_failed", "命令失败: ", ""),
        (
            "app.backend_error.config_serialize_failed",
            "序列化配置失败: ",
            "",
        ),
        (
            "app.backend_error.json_parse_failed_prefix",
            "不是合法 JSON: ",
            "",
        ),
    ];
    for (key, prefix, suffix) in SINGLE_SLOT {
        if let Some(out) = localize_backend_single_slot(text, key, prefix, suffix) {
            return Some(out);
        }
    }
    None
}
