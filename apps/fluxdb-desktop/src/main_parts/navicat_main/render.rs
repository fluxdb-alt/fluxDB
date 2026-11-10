impl Render for NavicatMain {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_title = tr("app.window_title");
        if window.window_title() != window_title {
            window.set_window_title(&window_title);
        }
        if self.update_auto_check_pending {
            self.update_auto_check_pending = false;
            self.start_update_check(false, cx);
        }
        // 调试入口：首帧后打开模型弹框（需要 window 创建 Input 实体；仅 debug 构建会置位）。
        if self.ai_model_dialog_debug_pending && self.ai_model_dialog.is_none() {
            self.ai_model_dialog_debug_pending = false;
            // `FLUXDB_DEBUG_AI_MODEL_DIALOG=edit` 打开第一个已有模型的编辑态（验证字段回填）。
            let edit_first = cfg!(debug_assertions)
                && std::env::var("FLUXDB_DEBUG_AI_MODEL_DIALOG").is_ok_and(|v| v == "edit");
            let first_id = edit_first
                .then(|| {
                    self.controller
                        .state()
                        .ai_models
                        .first()
                        .map(|m| m.id.clone())
                })
                .flatten();
            // `FLUXDB_DEBUG_AI_MODEL_DIALOG=invalid` 额外预置内联校验错误，便于截图核对错误态。
            let show_invalid = cfg!(debug_assertions)
                && std::env::var("FLUXDB_DEBUG_AI_MODEL_DIALOG").is_ok_and(|v| v == "invalid");
            self.open_ai_model_dialog(first_id, window, cx);
            if show_invalid {
                self.set_ai_model_dialog_error(
                    tr("app.ai.model_dialog.name_and_id_required").as_ref(),
                    cx,
                );
            }
        }
        // 调试入口：首帧后打开第一个可配置插件的管理弹框（截图核对用）。
        if let Some(target) = self.ai_plugin_config_dialog_debug_pending.clone()
            && self.ai_plugin_config_dialog.is_none()
        {
            self.ai_plugin_config_dialog_debug_pending = None;
            let id = if target.is_empty() {
                // 空串：第一个可配置插件。
                self.controller
                    .state()
                    .ai_plugin_instances
                    .iter()
                    .find(|plugin| !plugin.config_fields.is_empty())
                    .map(|plugin| plugin.id.clone())
            } else {
                Some(target)
            };
            if let Some(id) = id {
                self.open_ai_plugin_config_dialog(&id, window, cx);
            }
        }
        // 调试入口：首帧后自动对第一个模型跑一次测试连接（截图核对 Alert 与状态标签）。
        if self.ai_model_test_debug_pending {
            self.ai_model_test_debug_pending = false;
            if let Some(id) = self
                .controller
                .state()
                .ai_models
                .first()
                .map(|m| m.id.clone())
            {
                self.start_ai_model_test(id, cx);
            }
        }
        // 调试入口：首帧后打开 Agent 新建草稿（同样需要 window 创建 Input 实体）。
        if self.ai_agent_editor_debug_pending && self.ai_agent_editor.is_none() {
            self.ai_agent_editor_debug_pending = false;
            self.open_ai_agent_editor(None, window, cx);
        }
        // Agent 删除后自动选中剩余列表的第一项，避免右侧空白。
        if self.ai_agent_editor.is_none()
            && self.settings_panel_section == SettingsPanelSection::AiAgents
            && !self.ai_agent_delete_just_removed.is_empty()
        {
            let first = self
                .controller
                .state()
                .ai_agents
                .first()
                .map(|a| a.id.clone());
            self.ai_agent_delete_just_removed.clear();
            if let Some(id) = first {
                self.open_ai_agent_editor(Some(id), window, cx);
            }
        }
        // 首帧异步启动恢复；命令与后续 Load/Save 共用串行队列，不在 UI 线程扫描 SQLite。
        if self.ai_chat.recovery_pending {
            self.ai_chat.recovery_pending = false;
            self.ai_chat_request(
                fluxdb_app::AiConversationCommand::RecoverOnStartup,
                window,
                cx,
            );
        }
        if self.ai_chat.debug_open_pending {
            self.ai_chat.debug_open_pending = false;
            self.open_ai_chat(window, cx);
        }
        self.ai_chat_process(window, cx);
        let width = window.viewport_size().width;
        let compact = width < px(760.);
        let app_state = self.controller.state();
        let state = render_state_snapshot(app_state);
        let colors = ui_colors_from_theme(self.theme_mode, cx);
        // 任务中心关闭时不构建快照，避免每帧复制各任务日志。
        let task_center_rows = if self.task_center.open {
            self.task_center_rows()
        } else {
            Vec::new()
        };
        let task_center_selected = if self.task_center.open {
            self.selected_task_center_row()
        } else {
            None
        };
        let settings_for_config_proposal = self.controller.state().settings.clone();
        // 恢复弹框「现有库」目标下拉：目标连接切换时重载该连接下的库（渲染期消费，靠 target_db_loaded_for 防抖）。
        let restore_db_reload = self.restore_modal.as_ref().and_then(|modal| {
            let existing_mode = modal.kind != DatabaseKind::Sqlite
                && modal
                    .mode
                    .read(cx)
                    .selected_value()
                    .is_some_and(|v| *v == RestoreTargetMode::ExistingDatabase);
            if !existing_mode {
                return None;
            }
            let cid = restore_selected_connection_id(modal, cx)?;
            (modal.target_db_loaded_for != Some(cid)).then_some((modal.target_db.clone(), cid))
        });
        if let Some((db, cid)) = restore_db_reload {
            self.load_restore_target_databases(db, window, cx);
            if let Some(m) = &mut self.restore_modal {
                m.target_db_loaded_for = Some(cid);
            }
        }
        // 对象选择页「进入即探测」：现有库模式下后台检查目标存在性/备份内容，回填动作候选。
        if self.restore_modal.is_some() {
            self.sync_restore_probe(window, cx);
        }
        let show_status_bar = if self
            .controller
            .state()
            .tabs
            .iter()
            .any(|tab| matches!(&tab.kind, TabKind::Settings(_)))
        {
            self.settings_editor_draft.show_status_bar
        } else {
            state.settings.show_status_bar
        };
        // Redis 连接串导入 / 云自动发现：异步回调里只更新了表单数据（拿不到 `Window`），
        // 这里消费「待同步」标记，把表单值写回各输入框实体。
        if self.redis_discovery_pending_sync {
            self.redis_discovery_pending_sync = false;
            self.new_connection_inputs
                .sync_from_form(&self.new_connection_form, window, cx);
        }
        let connection_browser_width =
            clamp_connection_browser_width(self.connection_browser_width);
        let first_level_groups = first_level_tab_groups(&state);
        let active_scope = active_first_level_scope(&state);
        let two_level_tabs =
            !first_level_groups.is_empty() && first_level_has_second_row(active_scope.as_ref());
        let query_history_detail = self.query_history_detail.clone();
        let pending_apply_preview = self.pending_apply_data_changes.and_then(|tab_id| {
            self.data_change_preview_for_tab(tab_id)
                .map(|(page, changes)| {
                    let db_kind = self.connection_database_kind(changes.object.connection_id);
                    (
                        tab_id,
                        data_change_item_count(&changes),
                        data_change_statement_count(&changes),
                        data_change_sql_preview(&page, &changes, db_kind),
                    )
                })
        });
        let pending_apply_submitting = self
            .pending_apply_data_changes
            .is_some_and(|tab_id| self.data_change_apply_in_flight(tab_id));
        // 提示浮层在这里先构造好：它的配色要读主题（`cx`），放进 `.when_some` 闭包里会和
        // 同一表达式链上其它对 `cx` 的借用打架。
        let app_message_element = self
            .app_message
            .as_ref()
            .map(|message| app_message_overlay(message, window, cx, colors));
        let update_dialog = self.update_ui.open.then(|| self.update_dialog(colors, cx));
        let (sidebar_rows, sidebar_item_sizes) = if self.show_connection_browser {
            self.sidebar_tree_rows(
                &state,
                &self.loading_databases,
                &self.loaded_database_children,
                &self.pinned_databases,
                &self.pinned_tables,
                &self.table_folders,
                &self.table_folder_assignments,
                &self.expanded_databases,
                &self.expanded_object_groups,
                &self.saved_queries,
                &self.sidebar_search,
            )
        } else {
            (Rc::new(Vec::new()), Rc::new(Vec::new()))
        };

        let compare_transfer_overlay = self.compare_transfer.clone().map(|state| {
            let connections = self.controller.connection_configs();
            compare_transfer_modal(
                &state,
                &self.compare_transfer_inputs,
                &connections,
                colors,
                cx,
            )
        });
        div()
            .debug_selector(|| "fluxdb-main".into())
            .size_full()
            .relative()
            .bg(colors.app_bg)
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .key_context("NavicatMain")
            .on_action(cx.listener(Self::new_query))
            .on_action(cx.listener(Self::open_new_connection_action))
            .on_action(cx.listener(Self::open_settings_menu_action))
            .on_action(cx.listener(Self::open_query_history_action))
            .on_action(cx.listener(Self::toggle_theme_action))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::save_or_apply))
            .on_action(cx.listener(Self::close_current_tab))
            .on_action(cx.listener(Self::toggle_connection_browser))
            .on_action(cx.listener(Self::execute_or_apply))
            .on_action(cx.listener(Self::open_data_search))
            .on_action(cx.listener(Self::open_query_history_quick_search))
            .on_action(cx.listener(Self::query_history_quick_search_previous))
            .on_action(cx.listener(Self::query_history_quick_search_next))
            .on_action(cx.listener(Self::query_history_quick_search_confirm))
            .on_action(cx.listener(Self::open_quick_open_table_action))
            .on_action(cx.listener(Self::quick_open_table_previous))
            .on_action(cx.listener(Self::quick_open_table_next))
            .on_action(cx.listener(Self::quick_open_table_confirm))
            .on_action(cx.listener(Self::copy_data_selection))
            .on_action(cx.listener(Self::copy_footer_sql_selection))
            .on_action(cx.listener(Self::delete_connection_shortcut))
            .on_action(cx.listener(Self::cancel_dialog))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.close_context_menus(cx);
                    let mut changed = false;
                    if let Some(tab_id) = this.controller.state().active_tab
                        && let Some(editor) = this.query_editors.get(&tab_id).cloned()
                        && editor.read(cx).completion_visible()
                    {
                        editor.update(cx, |editor, cx| editor.hide_completion(cx));
                        changed = true;
                    }
                    if this.data_filter_popover.take().is_some() {
                        changed = true;
                    }
                    if this.field_filter_popover.take().is_some() {
                        changed = true;
                    }
                    if this.local_filter_popover.take().is_some() {
                        changed = true;
                    }
                    if this.local_filter_manager_popover.take().is_some() {
                        changed = true;
                    }
                    if this.query_history_open {
                        this.query_history_open = false;
                        changed = true;
                    }
                    if this.redis_history_open {
                        this.redis_history_open = false;
                        this.redis_history_scope = None;
                        this.close_redis_history_search(window, cx);
                        changed = true;
                    }
                    if this.query_history_quick_open {
                        this.query_history_quick_open = false;
                        changed = true;
                    }
                    if this.quick_open_table_open {
                        this.quick_open_table_open = false;
                        changed = true;
                    }
                    if this.tab_switcher.take().is_some() {
                        changed = true;
                    }
                    if changed {
                        this.sync_table_hover_overlay_block(cx);
                        cx.notify();
                    }
                }),
            )
            // 文本选择层：零尺寸，挂根节点首位，负责 MongoDB / JSON 展示区的双击、
            // 拖选与复制。必须在内容之前 prepaint（见 gpui-base `TextSelectionLayer`）。
            .child(gpui_kit::base::TextSelectionLayer)
            .child(topbar(
                &state,
                self.show_connection_browser,
                window,
                colors,
                cx,
            ))
            // 工作区只分配工具栏与状态栏之间的剩余高度，内容不得反向撑大窗口布局。
            .child(
                div()
                    .id("main-workspace")
                    .debug_selector(|| "main-workspace".into())
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(ai_chat_workspace(
                        div()
                            .debug_selector(|| "database-workspace".into())
                            .flex()
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_hidden()
                            .when(self.show_connection_browser, |this| {
                                this.child(sidebar(
                                    compact,
                                    connection_browser_width,
                                    &self.sidebar_tree_scroll,
                                    sidebar_rows.clone(),
                                    sidebar_item_sizes.clone(),
                                    self.table_folder_rename_input.clone(),
                                    self.rename_group_input.clone(),
                                    self.sidebar_search_input.clone(),
                                    &self.sidebar_search,
                                    colors,
                                    cx,
                                ))
                            })
                            .when(!self.show_connection_browser, |this| {
                                this.child(connection_browser_restore_button(&state, colors, cx))
                            })
                            .child(
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .h_full()
                                    .flex()
                                    .flex_col()
                                    .child(tabs(compact, &state, self, window, colors, cx))
                                    .child(
                                        content(&state, self, window, colors, cx)
                                            .debug_selector(|| "workspace-page".into()),
                                    )
                                    .when_some(self.tab_switcher, |this, kind| {
                                        this.child(tab_switcher_popup(
                                            kind,
                                            &state,
                                            active_scope.as_ref(),
                                            self.tab_switcher_search_input.clone(),
                                            &self.tab_switcher_search,
                                            two_level_tabs,
                                            &self.workspace_tab_order,
                                            &self.tab_order,
                                            &self.pinned_tabs,
                                            colors,
                                            cx,
                                        ))
                                    }),
                            )
                            .into_any_element(),
                        self,
                        window,
                        colors,
                        cx,
                    )),
            )
            .when(show_status_bar, |this| {
                this.child(statusbar(
                    &state,
                    // 「执行 SQL 文件」任务列表与 Dialog 共享（Rc<RefCell>），状态栏只读快照
                    &self.sql_file_modal.borrow().tasks,
                    &self.data_export_tasks,
                    &self.backup_tasks,
                    &self.task_center,
                    colors,
                    cx,
                ))
            })
            .when_some(app_message_element, |this, element| this.child(element))
            .when(
                self.connection_context_menu.is_some()
                    || self.database_context_menu.is_some()
                    || self.schema_context_menu.is_some()
                    || self.table_context_menu.is_some()
                    || self.collection_context_menu.is_some()
                    || self.table_group_context_menu.is_some()
                    || self.table_folder_context_menu.is_some()
                    || self.tab_context_menu.is_some()
                    || self.data_cell_context_menu.is_some()
                    || self.data_row_context_menu.is_some()
                    || self.group_context_menu.is_some(),
                |this| this.child(context_menu_backdrop(cx)),
            )
            .when_some(self.schema_context_menu.clone(), |this, menu| {
                this.child(schema_context_menu(menu, colors, cx))
            })
            .when_some(self.connection_context_menu, |this, menu| {
                this.child(connection_context_menu(menu, &state, colors, cx))
            })
            .when_some(self.database_context_menu.clone(), |this, menu| {
                // Redis CLI 只对 Redis 显示；新建 schema 对支持数据库内 schema 的类型显示。
                let is_redis = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind == DatabaseKind::Redis);
                let is_postgres = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind.is_postgres_family());
                let is_sqlserver = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind == DatabaseKind::SqlServer);
                let is_dameng = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind == DatabaseKind::Dameng);
                let is_oracle = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind == DatabaseKind::Oracle);
                let is_mongo = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .any(|c| c.config.id == menu.connection_id && c.config.kind == DatabaseKind::MongoDb);
                this.child(database_context_menu(
                    menu,
                    &self.pinned_databases,
                    is_redis,
                    is_postgres,
                    is_sqlserver,
                    is_dameng,
                    is_mongo,
                    is_oracle,
                    colors,
                    cx,
                ))
            })
            .when_some(self.table_context_menu.clone(), |this, menu| {
                this.child(table_context_menu(
                    menu,
                    &self.pinned_tables,
                    &self.table_folders,
                    &self.table_folder_assignments,
                    colors,
                    window,
                    cx,
                ))
            })
            .when_some(self.collection_context_menu.clone(), |this, menu| {
                this.child(collection_context_menu(menu, &self.pinned_tables, colors, cx))
            })
            .when_some(self.table_group_context_menu.clone(), |this, menu| {
                this.child(table_group_context_menu(menu, colors, cx))
            })
            .when_some(self.table_folder_context_menu.clone(), |this, menu| {
                this.child(table_folder_context_menu(menu, colors, cx))
            })
            .when_some(self.tab_context_menu, |this, menu| {
                this.child(tab_context_menu(
                    menu,
                    &state,
                    &self.pinned_tabs,
                    colors,
                    cx,
                ))
            })
            .when_some(self.data_cell_context_menu.clone(), |this, menu| {
                this.child(data_cell_context_menu(menu, colors, window, cx))
            })
            .when_some(self.data_row_context_menu.clone(), |this, menu| {
                this.child(data_row_context_menu(menu, &state, window, colors, cx))
            })
            .when_some(self.group_context_menu, |this, menu| {
                this.child(group_context_menu(menu, &state, colors, cx))
            })
            .when_some(self.pending_delete_connection, |this, connection_id| {
                this.child(delete_connection_modal(
                    connection_id,
                    &state,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_delete_database.clone(), |this, pending| {
                this.child(delete_database_modal(
                    pending,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_disconnect_connection, |this, pending| {
                this.child(disconnect_connection_modal(
                    pending,
                    &state,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_close_workspace.clone(), |this, pending| {
                this.child(close_workspace_modal(pending, colors, cx))
            })
            .when_some(self.pending_delete_data_row.clone(), |this, menu| {
                this.child(delete_data_row_modal(menu, colors, cx))
            })
            .when_some(
                ai_model_dialog(
                    self.ai_model_dialog.clone(),
                    self.ai_model_dialog_filter_input.clone(),
                    self.ai_model_dialog_filter.clone(),
                    colors,
                    cx,
                ),
                |this, dialog| this.child(dialog),
            )
            .when_some(
                ai_plugin_config_dialog(self.ai_plugin_config_dialog.clone(), colors, cx),
                |this, dialog| this.child(dialog),
            )
            .when_some(self.data_row_viewer.clone(), |this, viewer| {
                this.child(data_row_viewer_modal(
                    viewer,
                    &state,
                    self.row_detail_search_input.clone(),
                    &self.row_detail_search,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_dirty_data_action.clone(), |this, action| {
                this.child(dirty_data_action_modal(action, colors, cx))
            })
            .when_some(state.pending_dirty_tab_close, |this, tab_id| {
                this.child(dirty_tab_close_modal(tab_id, &state, colors, cx))
            })
            .when(
                self.pending_mongo_document_switch.is_some()
                    || self.pending_mongo_document_open.is_some(),
                |this| {
                    this.child(mongo_document_discard_modal(
                        self.focus_handle.clone(),
                        colors,
                        cx,
                    ))
                },
            )
            .when_some(
                self.pending_mongo_document_delete.clone(),
                |this, pending| this.child(mongo_document_delete_modal(&pending, colors, cx)),
            )
            .when_some(self.pending_mongo_import.clone(), |this, form| {
                this.child(mongo_import_modal(form, colors, cx))
            })
            .when_some(pending_apply_preview, |this, preview| {
                this.child(apply_data_changes_modal(
                    preview,
                    pending_apply_submitting,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_query_parameters.clone(), |this, pending| {
                this.child(query_parameter_prompt_modal(
                    pending,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_dangerous_query.clone(), |this, pending| {
                this.child(dangerous_command_modal(
                    DangerousCommandKind::Sql,
                    pending.text,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(
                self.pending_dangerous_redis_command.clone(),
                |this, pending| {
                    this.child(dangerous_command_modal(
                        DangerousCommandKind::Redis,
                        pending.text,
                        self.focus_handle.clone(),
                        colors,
                        cx,
                    ))
                },
            )
            .when_some(self.pending_config_proposal.clone(), |this, proposal| {
                this.child(config_proposal_modal(
                    proposal,
                    &settings_for_config_proposal,
                    self.config_proposal_busy,
                    self.config_delete_confirmed,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            // 「执行 SQL 文件」弹框已迁移到 gpui-component Dialog（window.open_dialog），
            // 由 `Root` 插件层统一渲染，不再在此条件挂载自绘弹层
            .when_some(self.pending_backup_modal.clone(), |this, form| {
                this.child(database_backup_modal(
                    form,
                    self.backup_file_name_input.clone(),
                    self.backup_target_dir_input.clone(),
                    self.backup_note_input.clone(),
                    self.backup_object_search_input.clone(),
                    &self.backup_objects_scroll,
                    &self.backup_tasks,
                    self.backup_pg_client_missing,
                    self.backup_mysql_client_missing,
                    colors,
                    cx,
                ))
            })
            // 恢复数据库弹框：与备份弹框同款自绘模态。
            .when_some(self.restore_modal.clone(), |this, modal| {
                this.child(database_restore_modal(&modal, colors, cx))
            })
            // 结构对比/同步与跨连接数据传输弹框。
            .when_some(compare_transfer_overlay, |this, modal| {
                this.child(modal)
            })
            // 备份 tab：「备份表」查看弹框。
            .when_some(self.backup_tables_modal.clone(), |this, modal| {
                this.child(backup_tables_modal(modal, colors, cx))
            })
            // 备份 tab：「恢复记录」查看弹框。
            .when_some(self.restore_records_modal.clone(), |this, modal| {
                this.child(restore_records_modal(modal, colors, cx))
            })
            // 备份 tab：备注编辑弹框（仅当弹框打开时挂载）。
            .when_some(self.backup_note_modal_path.clone(), |this, _| {
                this.child(backup_note_modal(
                    self.backup_note_edit_input.clone(),
                    self.backup_note_modal_path
                        .as_ref()
                        .and_then(|path| {
                            path.file_name()
                                .map(|name| name.to_string_lossy().to_string())
                        })
                        .unwrap_or_default(),
                    colors,
                    cx,
                ))
            })
            // 备份 tab：删除确认弹框。
            .when_some(self.pending_delete_backup.clone(), |this, path| {
                this.child(backup_delete_confirm_modal(path, colors, cx))
            })
            // Windows：点击自绘关闭按钮后的退出确认弹框。
            .when(self.pending_exit_confirm, |this| {
                this.child(exit_confirm_modal(colors, cx))
            })
            .when_some(update_dialog, |this, dialog| this.child(dialog))
            .when_some(self.backup_log_task, |this, task_id| {
                if let Some(task) = self
                    .backup_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .cloned()
                {
                    this.child(database_backup_log_modal(
                        task,
                        &self.backup_tasks,
                        colors,
                        cx,
                    ))
                } else {
                    this
                }
            })
            .when_some(self.pending_data_export.clone(), |this, form| {
                let export_filter_tab_id = table_data_export_filter_tab_id(form.tab_id);
                let database_label = data_export_object_database_label(&form.object);
                let database_select = self.data_export_object_select(
                    DataExportObjectSelectKey::Database(form.tab_id),
                    vec![database_label.clone()],
                    &database_label,
                    window,
                    cx,
                );
                let table_select = self.data_export_object_select(
                    DataExportObjectSelectKey::Table(form.tab_id),
                    self.table_data_export_table_select_options(&form),
                    &form.object.name,
                    window,
                    cx,
                );
                let custom_filter_rules = self
                    .data_filter_draft_rules
                    .get(&export_filter_tab_id)
                    .cloned()
                    .unwrap_or_else(|| form.custom_filter_rules.clone());
                let custom_sort_rules = self
                    .data_sort_draft_rules
                    .get(&export_filter_tab_id)
                    .cloned()
                    .unwrap_or_else(|| form.custom_sort_rules.clone());
                this.child(table_data_export_modal(
                    form,
                    database_select,
                    table_select,
                    self.data_export_preview.clone(),
                    self.data_export_custom_conditions_open,
                    custom_filter_rules,
                    custom_sort_rules,
                    self.data_filter_popover,
                    self.data_filter_value_input.clone(),
                    self.data_filter_search_input.clone(),
                    self.data_filter_batch_input.clone(),
                    self.data_filter_value_search.clone(),
                    self.data_filter_value_search_loading_until.is_some(),
                    self.data_filter_value_draft.as_ref(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_data_import.clone(), |this, form| {
                let kind = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == form.object.connection_id)
                    .map(|connection| connection.config.kind)
                    .unwrap_or(DatabaseKind::Postgres);
                this.child(table_data_import_modal(form, kind, colors, cx))
            })
            .when_some(self.pending_rename_table.clone(), |this, form| {
                this.child(table_name_form_modal(
                    TableNameForm::Rename(form.clone()),
                    self.rename_table_sql_for_form(&form),
                    self.rename_table_input.clone(),
                    self._rename_table_task.is_some(),
                    self.focus_handle.clone(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_copy_table.clone(), |this, form| {
                this.child(table_name_form_modal(
                    TableNameForm::Copy(form.clone()),
                    self.copy_table_sql_for_form(&form),
                    self.copy_table_input.clone(),
                    self._copy_table_task.is_some(),
                    self.focus_handle.clone(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_routine_ddl.clone(), |this, form| {
                this.child(routine_ddl_modal(form, window, colors, cx))
            })
            .when_some(self.pending_column_choices.clone(), |this, form| {
                this.child(column_choices_modal(
                    form,
                    self.column_choice_value_input.clone(),
                    self.column_choice_label_input.clone(),
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_danger_table_action.clone(), |this, form| {
                // 危险表（删/清空）弹框的选项能力必须在 render 内直接读取连接类型后判定。
                let capabilities = self
                    .controller
                    .state()
                    .connections
                    .iter()
                    .find(|connection| connection.config.id == form.object_path.connection_id)
                    .map(|connection| danger_table_capabilities(connection.config.kind))
                    .unwrap_or_default();
                this.child(danger_table_modal(
                    form.clone(),
                    self.danger_table_sql_for_form(&form),
                    self.danger_table_foreign_key_check_select.clone(),
                    self._danger_table_task.is_some(),
                    capabilities,
                    self.focus_handle.clone(),
                    window,
                    colors,
                    cx,
                ))
            })
            .when_some(self.data_export_log_task, |this, task_id| {
                if let Some(task) = self
                    .data_export_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .cloned()
                {
                    this.child(table_data_export_log_modal(task, colors, cx))
                } else {
                    this
                }
            })
            .when_some(self.display_database_connection, |this, connection_id| {
                this.child(display_database_modal(
                    connection_id,
                    &state,
                    &self.display_database_selection,
                    &self.display_database_search,
                    self.display_database_search_input.clone(),
                    self.display_database_show_system,
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_create_database.clone(), |this, form| {
                let running = self.create_database_running.contains(&form.connection_id);
                this.child(create_database_modal(
                    form,
                    self.create_database_name_input.clone(),
                    self.create_database_charset_select.clone(),
                    self.create_database_collation_select.clone(),
                    self.create_database_owner_input.clone(),
                    self.create_database_template_input.clone(),
                    self.create_database_first_collection_input.clone(),
                    running,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(
                self.pending_create_schema.clone(),
                |this, (_, database_path, _)| {
                    let database_name = database_path
                        .database
                        .clone()
                        .unwrap_or_else(|| database_path.name.clone());
                    this.child(create_schema_modal(
                        database_name,
                        self.create_schema_name_input.clone(),
                        self.create_schema_running,
                        self.focus_handle.clone(),
                        colors,
                        cx,
                    ))
                },
            )
            .when_some(self.mongo_info_dialog.as_ref(), |this, dialog| {
                let name = connection_name(&state, dialog.connection_id);
                this.child(mongo_info_dialog_view(dialog, name, colors, cx))
            })
            .when_some(self.pending_create_collection.clone(), |this, form| {
                this.child(mongo_create_collection_modal(
                    form,
                    self.create_collection_input.clone(),
                    self.create_collection_running,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_rename_collection.clone(), |this, form| {
                this.child(mongo_rename_collection_modal(
                    form,
                    self.rename_collection_input.clone(),
                    self._rename_mongo_collection_task.is_some(),
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_drop_collection.clone(), |this, form| {
                this.child(mongo_drop_collection_modal(
                    form,
                    self._drop_mongo_collection_task.is_some(),
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_mongo_export.clone(), |this, form| {
                this.child(mongo_export_modal(
                    form,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_query_save, |this, tab_id| {
                this.child(query_save_choice_modal(
                    tab_id,
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_connection_query_save, |this, tab_id| {
                this.child(query_save_connection_modal(
                    tab_id,
                    self.query_save_name_input.clone(),
                    self.focus_handle.clone(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.pending_new_query_connection, |this, connection_id| {
                this.child(new_query_scope_modal(
                    connection_id,
                    &state,
                    &self.connecting_connections,
                    colors,
                    cx,
                ))
            })
            .when_some(query_history_detail, |this, entry| {
                this.child(query_history_detail_modal(
                    entry,
                    self.controller.state(),
                    colors,
                    cx,
                ))
            })
            .when_some(self.new_connection_kind, |this, kind| {
                this.child(new_connection_modal(
                    kind,
                    self.new_connection_tab,
                    &self.new_connection_form,
                    &self.new_connection_inputs,
                    self.editing_connection_id.is_some(),
                    self._test_connection_task.is_some(),
                    self.saving_connection,
                    colors,
                    window,
                    cx,
                ))
            })
            .when(self.query_history_open, |this| {
                this.child(query_history_drawer(&state, self, colors, cx))
            })
            .when(self.redis_history_open, |this| {
                this.child(redis_workbench_history_drawer(self, colors, cx))
            })
            .when(self.query_history_quick_open, |this| {
                this.child(query_history_quick_search_modal(&state, self, colors, cx))
            })
            .when(self.quick_open_table_open, |this| {
                this.child(quick_open_table_modal(self, colors, cx))
            })
            // gpui-component 的 Dialog 层不再由应用自绘：`gpui_component::init` 会把
            // `WindowState` 注册为 `Root` 插件，sheet / dialog / notification 层由 `Root`
            // 在应用内容之上统一挂载，重复手动渲染会导致弹框叠加两份。
            // Redis hash 大字段完整值已改为 Hash Data 表格区域内嵌面板，不再走弹框。
            // 任务中心覆盖在全部弹框之上：从状态栏打开后可直接查看/停止后台任务。
            .when(self.task_center.open, |this| {
                this.child(task_center_overlay(
                    &self.task_center,
                    &task_center_rows,
                    task_center_selected.as_ref(),
                    colors,
                    cx,
                ))
            })
            // 性能诊断 HUD：设置开启时在窗口右上角悬浮显示 FPS / 帧耗时 / CPU / GPU / 内存。
            // 挂在根容器（`relative`）最上层，fps_monitor 首次调用会懒建每窗口单例并复用。
            .when(state.settings.performance_diagnostics, |this| {
                this.child(fps_monitor(window, cx))
            })
    }
}

fn render_state_snapshot(state: &AppState) -> AppState {
    AppState {
        ai_conversation: state.ai_conversation.clone(),
        ai_models: state.ai_models.clone(),
        ai_agents: state.ai_agents.clone(),
        ai_plugin_instances: state.ai_plugin_instances.clone(),
        ai_tool_settings: state.ai_tool_settings.clone(),
        ai_execution_policy: state.ai_execution_policy.clone(),
        ai_mcp_server_profiles: state.ai_mcp_server_profiles.clone(),
        ai_mcp_datasources: state.ai_mcp_datasources.clone(),
        ai_audit_rows: state.ai_audit_rows.clone(),
        ai_audit_error: state.ai_audit_error.clone(),
        ai_sessions: state.ai_sessions.clone(),
        ai_sessions_total: state.ai_sessions_total,
        ai_sessions_cursor: state.ai_sessions_cursor.clone(),
        ai_sessions_has_more: state.ai_sessions_has_more,
        ai_sessions_loading_more: state.ai_sessions_loading_more,
        ai_default_model_id: state.ai_default_model_id.clone(),
        ai_jev_status: state.ai_jev_status.clone(),
        ai_runtime_revision: state.ai_runtime_revision,
        connections: state.connections.clone(),
        tabs: state
            .tabs
            .iter()
            .map(|tab| render_tab_snapshot(tab, state.active_tab == Some(tab.id)))
            .collect(),
        active_tab: state.active_tab,
        pending_dirty_tab_close: state.pending_dirty_tab_close,
        settings: state.settings.clone(),
        sidebar_layout: state.sidebar_layout.clone(),
        tasks: state.tasks.clone(),
        query_history: state.query_history.clone(),
        redis_workbench_history: state.redis_workbench_history.clone(),
        next_redis_workbench_history_id: state.next_redis_workbench_history_id,
        last_error: state.last_error.clone(),
    }
}

fn render_tab_snapshot(tab: &TabState, keep_content: bool) -> TabState {
    if keep_content {
        return tab.clone();
    }

    TabState {
        id: tab.id,
        title: localized_tab_title(tab),
        kind: render_tab_kind_snapshot(&tab.kind),
        dirty: tab.dirty,
    }
}

fn render_tab_kind_snapshot(kind: &TabKind) -> TabKind {
    match kind {
        TabKind::AiChat { workspace } => TabKind::AiChat {
            workspace: workspace.clone(),
        },
        TabKind::ObjectList(editor) => TabKind::ObjectList(ObjectListState {
            parent: editor.parent.clone(),
            objects: Vec::new(),
            loading: editor.loading,
            error: editor.error.clone(),
        }),
        TabKind::DataEditor(editor) => TabKind::DataEditor(DataEditorState {
            object: editor.object.clone(),
            page: None,
            original_page: None,
            pagination: editor.pagination,
            load_generation: editor.load_generation,
            changes: None,
            editing_cell: editor.editing_cell,
            cell_detail_panel: CellDetailPanelState::default(),
            table_info: TableInfoState::default(),
            loading: editor.loading,
            submitting: false,
            error: editor.error.clone(),
        }),
        TabKind::QueryEditor(editor) => TabKind::QueryEditor(QueryEditorState {
            connection_id: editor.connection_id,
            database: editor.database.clone(),
            schema: editor.schema.clone(),
            text: String::new(),
            origin: None,
            saved_fingerprint: None,
            running: editor.running,
            results: Vec::new(),
            result_editors: BTreeMap::new(),
            active_result_editor: None,
            summaries: Vec::new(),
            error: editor.error.clone(),
        }),
        TabKind::RedisWorkbench(workbench) => TabKind::RedisWorkbench(RedisWorkbenchState {
            connection_id: workbench.connection_id,
            database: workbench.database,
            text: String::new(),
            running: workbench.running,
            // 快照避免携带完整结果，保持与 QueryEditor 一致的口径。
            executions: Vec::new(),
            error: workbench.error.clone(),
            saved_fingerprint: None,
            next_execution_id: 1,
            collapsed: std::collections::BTreeSet::new(),
            json_views: std::collections::BTreeSet::new(),
        }),
        TabKind::CreateTable(create) => TabKind::CreateTable(create.clone()),
        // 备份列表 tab 状态只有连接+库标识，行数据渲染时现扫磁盘，快照直接克隆。
        TabKind::BackupList(list) => TabKind::BackupList(list.clone()),
        // ER 图 tab 状态只有连接+库+schema 标识，图数据在 desktop 侧缓存，快照直接克隆。
        TabKind::ErDiagram(er) => TabKind::ErDiagram(er.clone()),
        // 集合工作台快照丢掉文档与元数据，只保留定位与查询草稿。
        TabKind::MongoCollection(collection) => {
            let mut snapshot = collection.clone();
            snapshot.page = None;
            snapshot.info = LoadState::NotLoaded;
            snapshot.indexes = LoadState::NotLoaded;
            snapshot.selected_document = None;
            TabKind::MongoCollection(snapshot)
        }
        // Redis CLI 状态本身很轻量，快照直接克隆即可（会话复用依赖 connection_id + database）。
        TabKind::RedisCli(cli) => TabKind::RedisCli(cli.clone()),
        // MongoDB 命令行同理：只记连接 + 库，REPL 会话在桌面端。
        TabKind::MongoCli(cli) => TabKind::MongoCli(cli.clone()),
        // 命令工作台快照丢掉执行记录，只保留定位与草稿。
        TabKind::MongoWorkbench(workbench) => {
            TabKind::MongoWorkbench(MongoWorkbenchState {
                text: String::new(),
                executions: Vec::new(),
                error: workbench.error.clone(),
                saved_fingerprint: None,
                next_execution_id: 1,
                collapsed: std::collections::BTreeSet::new(),
                ..workbench.clone()
            })
        }
        // Redis Pub/Sub 状态很轻量（仅 connection_id + database），快照直接克隆。
        TabKind::RedisPubSub(pubsub) => TabKind::RedisPubSub(pubsub.clone()),

        TabKind::UserAdmin(admin) => TabKind::UserAdmin(UserAdminState {
            connection_id: admin.connection_id,
            sqlserver: admin.sqlserver.clone(),
            oracle: admin.oracle.clone(),
            active_detail_tab: admin.active_detail_tab,
            users: Vec::new(),
            selected_user: admin.selected_user.clone(),
            pending_selected_user: admin.pending_selected_user.clone(),
            creating_user: admin.creating_user,
            grants: Vec::new(),
            grants_loaded_user: admin.grants_loaded_user.clone(),
            member_grants: admin.member_grants.clone(),
            member_grants_loaded_role: admin.member_grants_loaded_role.clone(),
            search: admin.search.clone(),
            loading_users: admin.loading_users,
            loading_grants: admin.loading_grants,
            loading_member_grants: admin.loading_member_grants,
            applying: admin.applying,
            users_error: admin.users_error.clone(),
            grants_error: admin.grants_error.clone(),
            member_grants_error: admin.member_grants_error.clone(),
            apply_error: admin.apply_error.clone(),
            privilege_scope: admin.privilege_scope,
            privilege_database: admin.privilege_database.clone(),
            privilege_table: admin.privilege_table.clone(),
            privilege_role: admin.privilege_role.clone(),
            grant_option: admin.grant_option,
            selected_privileges: admin.selected_privileges.clone(),
            privilege_rows: admin.privilege_rows.clone(),
            base_privilege_rows: admin.base_privilege_rows.clone(),
            next_privilege_row_id: admin.next_privilege_row_id,
            create_user: admin.create_user.clone(),
            create_host: admin.create_host.clone(),
            auth_plugin: admin.auth_plugin.clone(),
            password_expiry_policy: admin.password_expiry_policy.clone(),
            create_password: String::new(),
            new_password: String::new(),
            max_queries_per_hour: admin.max_queries_per_hour.clone(),
            max_updates_per_hour: admin.max_updates_per_hour.clone(),
            max_connections_per_hour: admin.max_connections_per_hour.clone(),
            max_user_connections: admin.max_user_connections.clone(),
            ssl_type: admin.ssl_type.clone(),
            ssl_cipher: admin.ssl_cipher.clone(),
            ssl_issuer: admin.ssl_issuer.clone(),
            ssl_subject: admin.ssl_subject.clone(),
            role_membership_edits: admin.role_membership_edits.clone(),
            member_grant_edits: admin.member_grant_edits.clone(),
            pending_sql: admin.pending_sql.clone(),
            // 弹框意图只在 app 层 dispatch 里用；渲染快照不需要带着它。
            pending_sql_awaits_preview: admin.pending_sql_awaits_preview,
            pending_delete_user: admin.pending_delete_user.clone(),
            pg_grant_kind: admin.pg_grant_kind,
            pg_grant_schema: admin.pg_grant_schema.clone(),
            pg_grant_object: admin.pg_grant_object.clone(),
            pg_grant_signature: admin.pg_grant_signature.clone(),
            pg_object_grants: admin.pg_object_grants.clone(),
            pg_effective_grants: admin.pg_effective_grants.clone(),
            loading_pg_grants: admin.loading_pg_grants,
            pg_grants_error: admin.pg_grants_error.clone(),
            pg_roles: Vec::new(),
            pg_roles_error: admin.pg_roles_error.clone(),
            pg_selected_role: admin.pg_selected_role.clone(),
            pending_selected_pg_role: admin.pending_selected_pg_role.clone(),
            pg_draft: None,
            pg_memberships: Vec::new(),
            pg_memberships_loaded: admin.pg_memberships_loaded,
            pg_membership_edits: Vec::new(),
            pg_grant_database: admin.pg_grant_database.clone(),
            pg_grant_targets: None,
            pg_loading_targets: admin.pg_loading_targets,
            pg_targets_error: admin.pg_targets_error.clone(),
            pg_grant_edits: Vec::new(),
            pg_save_status: admin.pg_save_status,
            pg_plan_error: admin.pg_plan_error.clone(),
            pg_plan_preview: None,
            pg_preview_loading: admin.pg_preview_loading,
            pg_role_filter: admin.pg_role_filter.clone(),
            pg_pending_delete: admin.pg_pending_delete.clone(),
            pg_pending_switch: admin.pg_pending_switch.clone(),
            pg_member_options_supported: admin.pg_member_options_supported,
            pg_loaded_target: admin.pg_loaded_target.clone(),
        }),
        // 服务器面板快照不携带会话 SQL / 锁等待明细，避免进入 AI 上下文或外部导出。
        TabKind::ServerPanel(panel) => {
            let mut snapshot = panel.clone();
            snapshot.activity = None;
            snapshot.error = None;
            TabKind::ServerPanel(snapshot)
        }
        // Redis 服务器快照不携带客户端地址与慢日志命令，避免进入 AI 上下文或外部导出。
        TabKind::RedisServerPanel(panel) => {
            let mut snapshot = panel.clone();
            snapshot.activity = None;
            snapshot.error = None;
            TabKind::RedisServerPanel(snapshot)
        }
        // MongoDB 服务器快照不携带客户端地址与操作明细，避免进入 AI 上下文或外部导出。
        TabKind::MongoServerPanel(panel) => {
            let mut snapshot = panel.clone();
            snapshot.activity = None;
            snapshot.error = None;
            snapshot.samples.clear();
            TabKind::MongoServerPanel(snapshot)
        }
        TabKind::Settings(settings) => TabKind::Settings(settings.clone()),
    }
}

impl Focusable for NavicatMain {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
