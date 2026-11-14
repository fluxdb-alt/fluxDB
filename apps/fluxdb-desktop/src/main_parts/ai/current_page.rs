impl NavicatMain {
    fn ai_context_source_tab(&self) -> Option<&TabState> {
        let state = self.controller.state();
        if let Some(edit) = &self.ai_chat.pending_sql_edit {
            return state.tabs.iter().find(|tab| tab.id.0 == edit.tab_id);
        }
        let active = state.active_tab()?;
        if !matches!(active.kind, TabKind::AiChat { .. }) {
            return Some(active);
        }
        self.ai_chat
            .context_tab
            .and_then(|id| state.tabs.iter().find(|t| t.id == id))
    }

    fn capture_ai_active_context(&self, cx: &App) -> fluxdb_app::UiContextSnapshot {
        self.capture_ai_page_context(cx, false).snapshot
    }

    /// render 只展示轻量描述，发送才固定明确授权的已加载数据快照。
    fn capture_ai_page_context(&self, cx: &App, sending: bool) -> fluxdb_app::CurrentPageContext {
        use fluxdb_app::{CurrentPageContext, PageContextKind, TargetRef};
        let mut page = CurrentPageContext {
            kind: PageContextKind::Other,
            label: "无当前业务页面".into(),
            snapshot: fluxdb_app::capture(
                &fluxdb_app::CapturedTab::NoTab {
                    connection_id: None,
                    database: None,
                    schema: None,
                    last_object: None,
                },
                fluxdb_app::ContextLimits::default(),
            ),
            details: serde_json::json!({"available":false}),
            editor_grant: None,
            data_grant: None,
            mongo_grant: None,
        };
        let Some(tab) = self.ai_context_source_tab() else {
            return page;
        };
        page.label = tab.title.clone();
        page.snapshot.tab_id = Some(tab.id.0);
        if let Some(workspace) = tab.workspace() {
            page.snapshot.target.connection_id = Some(workspace.connection_id.0 as i64);
            page.snapshot.target.database = Some(workspace.database);
        }
        match &tab.kind {
            TabKind::QueryEditor(query) => {
                let target = TargetRef {
                    connection_id: Some(query.connection_id.0 as i64),
                    database: query.database.clone(),
                    schema: query.schema.clone(),
                    ..Default::default()
                };
                let key = QueryPaneKey::new(tab.id, query.active_pane);
                let text = self
                    .query_editors
                    .get(&key)
                    .map(|e| e.read(cx).buffer.snapshot())
                    .unwrap_or_else(|| {
                        fluxdb_editor_core::EditorBuffer::new_from(&query.text).snapshot()
                    });
                let selection = self
                    .query_editors
                    .get(&key)
                    .map(|e| e.read(cx).selection_range())
                    .unwrap_or_else(|| fluxdb_editor_core::Range::new(0, 0));
                page.kind = PageContextKind::SqlEditor;
                page.snapshot =
                    fluxdb_app::capture_editor_snapshot(tab.id.0, target, &text, selection);
                let database_kind = self.query_editor_database_kind(tab.id);
                let output = self
                    .query_output_tabs
                    .get(&key)
                    .copied()
                    .filter(|output| query_output_tab_exists(*output, query, database_kind))
                    .unwrap_or_else(|| {
                        if query_result_entry_count(query) > 0 {
                            QueryOutputTab::Result(0)
                        } else {
                            QueryOutputTab::Summary
                        }
                    });
                let result_index = output
                    .result_index()
                    .and_then(|i| query_result_page_index(query, i));
                let cursor = selection.end.min(text.len());
                let point = text.offset_to_point(cursor);
                let cursor_start = text.clamp_to_char_boundary(cursor.saturating_sub(512));
                let cursor_end =
                    text.clamp_to_char_boundary(cursor.saturating_add(1536).min(text.len()));
                page.details = serde_json::json!({
                    "total_bytes":text.len(),"total_lines":text.line_count(),"running":query.running,
                    "error":query.error.as_ref().map(|e|e.message.chars().take(1024).collect::<String>()),
                    "selection":{"start":selection.start,"end":selection.end},
                    "read_scope":"完整发送时 SQL 快照；选区优先展示但并不缩小工具范围",
                    "active_output":format!("{output:?}"),"active_result_index":result_index,
                    "cursor":{"offset":cursor,"line_number":point.row+1,"byte_column":point.column,
                        "preview":text.text_in_range(fluxdb_editor_core::Range::new(cursor_start,cursor_end)),
                        "scope":"光标附近有界片段，不代表完整语句"},
                    "results":result_index.and_then(|i|query.results.get(i)).into_iter().map(|p|serde_json::json!({
                        "loaded_rows":p.rows.len(),"offset":p.offset,"has_more":p.has_more,
                        "columns":fluxdb_app::page_columns_json(&p.columns)
                    })).collect::<Vec<_>>(),
                    "executions":query.summaries.iter().rev().take(3).map(|s|serde_json::json!({
                        "success":s.success,"message":s.message.chars().take(1024).collect::<String>(),
                        "returned_rows":s.returned_rows,"affected_rows":s.affected_rows,"elapsed_ms":s.elapsed_ms
                    })).collect::<Vec<_>>(),
                });
                if result_index.is_some() {
                    if let Some(table) = self.data_table_states.get(&tab.id) {
                        let delegate = table.read(cx).delegate();
                        let row = delegate
                            .selected_row
                            .or_else(|| delegate.selected_cell.map(|c| c.0))
                            .and_then(|r| delegate.source_row_indexes.get(r).copied());
                        page.details["selected_source_row"] = serde_json::json!(row);
                        page.details["selected_column"] =
                            serde_json::json!(delegate.selected_cell.map(|c| c.1));
                    }
                }
                if !text.is_empty() {
                    page.editor_grant = Some(fluxdb_app::EditorGrant {
                        tab_ref: format!("tab-{}", tab.id.0),
                        snapshot: text,
                        sql_edit: None,
                    });
                }
                if sending && !query.running {
                    let data = result_index.and_then(|selected| {
                        query
                            .result_editors
                            .get(&selected)
                            .and_then(|d| d.page.as_ref())
                            .or_else(|| query.results.get(selected))
                    });
                    if let Some(data) = data {
                        page.data_grant = Some(fluxdb_app::PageDataGrant {
                            reference_id: page.reference_id(),
                            target: page.snapshot.target.clone(),
                            page: std::sync::Arc::new(data.clone()),
                        });
                    }
                }
            }
            TabKind::DataEditor(data) => {
                let redis = matches!(data.object.kind, ObjectKind::RedisDb | ObjectKind::RedisKey);
                page.kind = if redis {
                    PageContextKind::RedisBrowser
                } else if data.table_info.open {
                    PageContextKind::TableStructure
                } else {
                    PageContextKind::TableData
                };
                page.snapshot.target = TargetRef {
                    connection_id: Some(data.object.connection_id.0 as i64),
                    database: data.object.database.clone(),
                    schema: data.object.schema.clone(),
                    object: Some(data.object.name.clone()),
                    redis_db: redis.then(|| {
                        data.object
                            .database
                            .as_deref()
                            .unwrap_or("0")
                            .parse()
                            .unwrap_or(0)
                    }),
                };
                let mut selected = self.data_table_states.get(&tab.id).and_then(|table| {
                    let delegate = table.read(cx).delegate();
                    delegate
                        .selected_row
                        .or_else(|| delegate.selected_cell.map(|c| c.0))
                        .and_then(|r| delegate.source_row_indexes.get(r).copied())
                });
                if redis && self.redis_key_list_mode(tab.id).is_folder() {
                    selected = self
                        .redis_key_list_selected_leaf(tab.id)
                        .as_deref()
                        .and_then(|key| {
                            data.page
                                .as_ref()
                                .and_then(|p| redis_key_page_row_for_key(p, key))
                        });
                }
                page.details = serde_json::json!({
                    "object":data.object.name,
                    "columns":data.page.as_ref().map(|p|fluxdb_app::page_columns_json(&p.columns)),
                    "columns_truncated":data.page.as_ref().is_some_and(|p|p.columns.len()>32),
                    "loading":data.loading,"submitting":data.submitting,"error":data.error.as_ref().map(|e|e.message.chars().take(1024).collect::<String>()),
                    "selected_row":selected,"filter":self.data_filter_texts.get(&tab.id).map(|s|s.chars().take(1024).collect::<String>()),
                    "filter_rules":self.data_filter_rules.get(&tab.id).map(|rules|serde_json::json!({
                        "truncated":rules.len()>16,
                        "items":rules.iter().take(16).map(|r|serde_json::json!({
                            "enabled":r.enabled,"field":r.field,"operator":format!("{:?}",r.operator),
                            "values":r.values.iter().take(16).map(|v|v.chars().take(128).collect::<String>()).collect::<Vec<_>>(),
                                "values_truncated":r.values.len()>16,"grouped":r.group_depth > 0,
                            "advanced_expression":r.advanced_expression.as_ref().map(|v|v.chars().take(1024).collect::<String>())
                        })).collect::<Vec<_>>()
                    })),
                    "sort":self.data_sort_texts.get(&tab.id).map(|s|s.chars().take(1024).collect::<String>()),
                    "sort_rules":self.data_sort_rules.get(&tab.id).map(|rules|serde_json::json!({
                        "truncated":rules.len()>16,
                        "items":rules.iter().take(16).map(|r|serde_json::json!({
                            "enabled":r.enabled,"field":r.field,"ascending":r.ascending
                        })).collect::<Vec<_>>()
                    })),
                    "has_pending_changes":data.changes.is_some(),
                    "loaded_rows":data.page.as_ref().map(|p|p.rows.len()),"offset":data.page.as_ref().map(|p|p.offset),
                    "has_more":data.page.as_ref().map(|p|p.has_more),
                    "table_structure_tab":data.table_info.open.then(||format!("{:?}",data.table_info.active_tab)),
                });
                if data.table_info.open {
                    if let LoadState::Loaded(indexes) = &data.table_info.indexes {
                        page.details["indexes"] = serde_json::json!({
                            "total":indexes.len(),"truncated":indexes.len()>32,
                            "items":indexes.iter().take(32).map(|i|serde_json::json!({
                                "name":i.name,"columns":i.columns,"unique":i.is_unique,"primary":i.is_primary
                            })).collect::<Vec<_>>()
                        });
                    }
                    if let LoadState::Loaded(keys) = &data.table_info.foreign_keys {
                        page.details["foreign_keys"] = serde_json::json!({
                            "total":keys.len(),"truncated":keys.len()>32,
                            "items":keys.iter().take(32).map(|k|serde_json::json!({
                                "name":k.name,"columns":k.columns,"ref_schema":k.ref_schema,
                                "ref_table":k.ref_table,"ref_columns":k.ref_columns
                            })).collect::<Vec<_>>()
                        });
                    }
                    if let LoadState::Loaded(ddl) = &data.table_info.ddl {
                        page.snapshot.excerpt = Some(fluxdb_app::bound_excerpt(
                            ddl,
                            fluxdb_app::ContextLimits::default(),
                        ));
                        page.details["ddl_read_scope"] = serde_json::json!(
                            "已加载的 DDL 预览；详情通过授权的 db.describe_object 查询，不重新授予权限"
                        );
                    }
                }
                if redis {
                    if let Some((loaded, row)) = data
                        .page
                        .as_ref()
                        .and_then(|p| selected.and_then(|r| p.rows.get(r).map(|row| (p, row))))
                    {
                        if let Some(key) = redis_row_text(loaded, row, "键") {
                            page.kind = PageContextKind::RedisKey;
                            page.snapshot.target.object = Some(key.clone());
                            page.details["selected_key"] = serde_json::json!({
                            "key":key,"type":redis_row_text(loaded,row,"类型"),
                            "ttl":redis_row_text(loaded,row,"TTL"),"size":redis_row_text(loaded,row,"大小")});
                            let value = redis_row_text(loaded, row, "值").unwrap_or_default();
                            let preview = fluxdb_app::bound_excerpt(
                                &value,
                                fluxdb_app::ContextLimits::default(),
                            );
                            page.details["value"] = serde_json::json!({"preview":preview.text,"truncated":preview.truncated,
                            "scope":"已加载页面值预览；不是服务器上的完整值"});
                        }
                    }
                }
                if sending && !data.loading && !data.submitting {
                    if let Some(data_page) = &data.page {
                        let mut shared = data_page.clone();
                        if redis {
                            shared.rows = selected
                                .and_then(|r| shared.rows.get(r).cloned())
                                .into_iter()
                                .collect();
                        }
                        page.data_grant = Some(fluxdb_app::PageDataGrant {
                            reference_id: page.reference_id(),
                            target: page.snapshot.target.clone(),
                            page: std::sync::Arc::new(shared),
                        });
                    }
                }
            }
            TabKind::ErDiagram(er) => {
                // ER 页只给定位 + 选中对象的有界结构摘要 + 加载状态与版本：
                // 不发送全库关系、业务行数据、连接凭据，也不因画布隐藏节点就判定关系不存在。
                page.kind = PageContextKind::ErDiagram;
                page.snapshot.target = TargetRef {
                    connection_id: Some(er.connection_id.0 as i64),
                    database: Some(er.database.clone()),
                    schema: er.schema.clone(),
                    object: er
                        .center_table
                        .as_ref()
                        .map(|table| table.display()),
                    ..Default::default()
                };
                let canvas = self.er_canvas.borrow();
                let selected = canvas
                    .er_selected_table
                    .get(&tab.id)
                    .cloned()
                    .flatten();
                let tables = self.er_full_tables.get(&tab.id);
                let selected_table = tables
                    .as_ref()
                    .and_then(|list| {
                        list.iter()
                            .find(|table| selected.as_ref().is_some_and(|name| &table.name == name))
                    });
                let selected_columns: Vec<fluxdb_core::ErColumn> = selected_table
                    .map(|table| table.columns.clone())
                    .unwrap_or_default();
                drop(canvas);
                let relationship = selected_table.as_ref().and_then(|_| {
                    self.er_relationship_panel_selected
                        .get(&tab.id)
                        .cloned()
                        .flatten()
                        .and_then(|id| {
                            self.er_relationships
                                .get(&tab.id)
                                .and_then(|list| list.iter().find(|rel| rel.id == id).cloned())
                        })
                });
                page.details = serde_json::json!({
                    "scope":{"connection_id":er.connection_id.0,"database":er.database,"schema":er.schema,
                        "relationship_scope_key":fluxdb_core::er_relationship_scope_key(
                            er.connection_id,&er.database,er.schema.as_deref())},
                    "center_table":er.center_table.as_ref().map(|table|table.display()),
                    "selected_table":selected_table.as_ref().map(|table| serde_json::json!({
                        "name":table.name,
                        "column_count":table.columns.len(),
                        "load_status":format!("{:?}",table.status),
                        "columns":selected_columns.iter().take(16).map(|column|serde_json::json!({
                            "name":column.name.chars().take(128).collect::<String>(),
                            "type":column.type_name.as_ref().map(|s|s.chars().take(128).collect::<String>()),
                            "nullable":column.nullable,"primary_key":column.primary_key,
                        })).collect::<Vec<_>>(),
                        "columns_truncated":selected_columns.len()>16,
                    })),
                    "selected_relationship":relationship.as_ref().map(|rel| serde_json::json!({
                        "relationship_id":rel.id,"revision":rel.revision,"role":rel.role,
                        "left_entity":rel.left_entity,"right_entity":rel.right_entity,
                        "review":format!("{:?}",rel.review.state),
                        "origin":format!("{:?}",rel.origin),
                        "validity":format!("{:?}",rel.validity.state),
                        "column_pairs":rel.column_pairs.iter().map(|pair|serde_json::json!({
                            "left_column":fluxdb_core::er_column_id_name(&pair.left_column),
                            "right_column":fluxdb_core::er_column_id_name(&pair.right_column),
                        })).collect::<Vec<_>>(),
                        "required_filters":rel.required_filters.iter().map(|filter|serde_json::json!({
                            "side":format!("{:?}",filter.side),
                            "column":fluxdb_core::er_column_id_name(&filter.column_id),
                            "op":format!("{:?}",filter.op),
                        })).collect::<Vec<_>>(),
                        "description":rel.description.as_ref().map(|text|text.chars().take(512).collect::<String>()),
                    })),
                    "load_status":{
                        "table_count":tables.as_ref().map(|list|list.len()),
                        "foreign_keys":self.er_graphs.get(&tab.id).map(|graph|format!("{:?}",graph.relation_status)),
                        "logical_relationships":if self.er_relationship_loading.contains(&tab.id) {"loading"} else {"loaded"},
                        "pending_rebind":self.er_rebind_pending.get(&tab.id).map(|items|items.len()),
                    },
                    "version":self.er_generation.get(&tab.id).copied().unwrap_or(0),
                    "read_scope":"只含定位与选中对象摘要；关系详情用 db.get_relationships 按需读取，\
                                 本页引用不构成额外数据库授权，也不代表画布外没有其它关系",
                });
                page.snapshot.notes = vec!["关系目录与画布可见范围无关；未画出的表也可能有关系".into()];
            }
            TabKind::MongoCollection(collection) => {
                // 集合页只给定位、查询参数与状态：文档值不进首条消息，发送时登记授权，
                // 由 `mongo.read_documents` 按有界只读快照按需读取。
                page.kind = PageContextKind::MongoCollection;
                page.snapshot.target = TargetRef {
                    connection_id: Some(collection.object.connection_id.0 as i64),
                    database: collection.object.database.clone(),
                    schema: collection.object.schema.clone(),
                    object: Some(collection.object.name.clone()),
                    ..Default::default()
                };
                let selected_id = collection
                    .selected_document
                    .and_then(|index| collection.page.as_ref()?.documents.get(index))
                    .map(|document| document.id.clone());
                page.details = serde_json::json!({
                    "collection": collection.object.name,
                    "object_kind": format!("{:?}", collection.object.kind),
                    "read_only": collection.read_only,
                    "section": format!("{:?}", collection.section),
                    "display": format!("{:?}", collection.display),
                    "query": {
                        "filter": collection.applied_query.filter,
                        "projection": collection.applied_query.projection,
                        "sort": collection.applied_query.sort,
                        "limit": collection.applied_query.limit,
                    },
                    "pagination": {
                        "offset": collection.pagination.offset,
                        "limit": collection.pagination.limit,
                    },
                    "loading": collection.loading,
                    "error": collection.error.as_ref().map(|e| e.message.chars().take(1024).collect::<String>()),
                    "loaded_documents": collection.page.as_ref().map(|page| page.documents.len()),
                    "total": collection.page.as_ref().and_then(|page| page.total),
                    "has_more": collection.page.as_ref().is_some_and(|page| page.has_more),
                    "selected_document_id": selected_id,
                    "aggregation": {
                        "pipeline_edited": !collection.aggregation_pipeline.is_empty(),
                        "state": match &collection.aggregation {
                            fluxdb_app::LoadState::NotLoaded => "not_loaded",
                            fluxdb_app::LoadState::Loading => "loading",
                            fluxdb_app::LoadState::Loaded(_) => "loaded",
                            fluxdb_app::LoadState::Failed(_) => "failed",
                        },
                    },
                    "read_scope": "只含定位、查询参数与加载状态；文档值经 mongo.read_documents 按需读取，\
                                   本页引用不构成额外数据库授权",
                });
                page.snapshot.notes = vec![
                    "已加载文档不代表集合全集；过滤条件之外的文档不可见".into(),
                ];
                if sending && !collection.loading {
                    if let Some(documents) = &collection.page {
                        page.mongo_grant = Some(fluxdb_app::MongoDocumentsGrant {
                            reference_id: page.reference_id(),
                            target: page.snapshot.target.clone(),
                            collection: collection.object.name.clone(),
                            documents: std::sync::Arc::new(
                                documents
                                    .documents
                                    .iter()
                                    .map(|document| (document.id.clone(), document.json.clone()))
                                    .collect(),
                            ),
                            filter: collection.applied_query.filter.clone(),
                        });
                    }
                }
            }
            TabKind::RedisWorkbench(workbench) => {
                page.kind = PageContextKind::RedisWorkbench;
                page.snapshot.target = TargetRef {
                    connection_id: Some(workbench.connection_id.0 as i64),
                    database: Some(workbench.database.to_string()),
                    redis_db: Some(workbench.database),
                    ..Default::default()
                };
                page.snapshot.excerpt = Some(fluxdb_app::bound_excerpt(
                    &workbench.text,
                    fluxdb_app::ContextLimits::default(),
                ));
                page.details = serde_json::json!({"running":workbench.running,
                    "error":workbench.error.as_ref().map(|e|&e.message),"execution_count":workbench.executions.len(),
                    "note":"只发送命令草稿与状态，不自动发送历史回复"});
            }
            TabKind::MongoWorkbench(workbench) => {
                // 只发定位与状态：结果区里是查询回来的文档内容，一律不进首条消息，
                // 需要时由 AI 走有界只读工具自己取。
                page.snapshot.target = TargetRef {
                    connection_id: Some(workbench.connection_id.0 as i64),
                    database: Some(workbench.database.clone()),
                    ..Default::default()
                };
                page.details = serde_json::json!({"running":workbench.running,
                    "error":workbench.error.as_ref().map(|e|&e.message),
                    "execution_count":workbench.executions.len(),
                    "note":"只发送定位与状态，不发送语句草稿与结果区内容"});
            }
            TabKind::Settings(_) => {
                page.kind = PageContextKind::Settings;
                let revision =
                    fluxdb_app::settings_fingerprint(&self.controller.state().settings);
                page.snapshot.tab_revision = revision;
                let section = settings_ai_section(self.settings_panel_section);
                page.details = serde_json::json!({
                    "revision": revision,
                    "active_section": settings_ai_section_key(self.settings_panel_section),
                    "draft_dirty": self.settings_editor_draft != self.controller.state().settings,
                    "values": section.map(|section| {
                        fluxdb_app::settings_section_public_json(
                            &self.controller.state().settings,
                            section,
                        )
                    }),
                    "editable_sections": ["editor", "appearance", "data", "system"],
                    "read_scope": "只含当前分区的非敏感设置值；不含客户端路径、快捷键、危险操作清单或任何凭据",
                    "note": "设置修改通过 toolbox.search / toolbox.load 加载 config.setting.update；仅限白名单字段，用户确认提案后才保存"
                });
            }
            _ => {
                page.details = serde_json::json!({"available":false,"note":"当前页面正文未接入 AI，仅提供目标定位；不发送设置表单与凭据"});
                page.details["page_type"] = serde_json::json!(match &tab.kind {
                    TabKind::RedisCli(_) => "redis_cli",
                    TabKind::RedisPubSub(_) => "redis_pubsub",
                    TabKind::CreateTable(_) => "table_design",
                    TabKind::ObjectList(_) => "object_list",
                    TabKind::ErDiagram(_) => "er_diagram",
                    TabKind::UserAdmin(_) => "user_admin",
                    TabKind::BackupList(_) => "backup_list",
                    TabKind::Settings(_) => "settings",
                    _ => "unsupported",
                });
                page.snapshot.notes = vec!["当前页面正文未接入 AI".into()];
            }
        }
        page
    }
}

fn settings_ai_section(
    section: SettingsPanelSection,
) -> Option<fluxdb_app::SettingsSection> {
    match section {
        SettingsPanelSection::Editor => Some(fluxdb_app::SettingsSection::Editor),
        SettingsPanelSection::Appearance => Some(fluxdb_app::SettingsSection::Appearance),
        SettingsPanelSection::Data => Some(fluxdb_app::SettingsSection::Data),
        SettingsPanelSection::System => Some(fluxdb_app::SettingsSection::System),
        _ => None,
    }
}

fn settings_ai_section_key(section: SettingsPanelSection) -> &'static str {
    match section {
        SettingsPanelSection::Editor => "editor",
        SettingsPanelSection::Shortcuts => "shortcuts",
        SettingsPanelSection::Appearance => "appearance",
        SettingsPanelSection::System => "system",
        SettingsPanelSection::Data => "data",
        SettingsPanelSection::ConnectionSecurity => "connection_security",
        SettingsPanelSection::DatabaseSupport => "database_support",
        SettingsPanelSection::AiModels => "ai_models",
        SettingsPanelSection::AiToolsMcp => "ai_tools_mcp",
        SettingsPanelSection::AiAgents => "ai_agents",
        SettingsPanelSection::AiSessions => "ai_sessions",
        SettingsPanelSection::About => "about",
    }
}
