impl AppController {
    pub fn dispatch(&mut self, command: AppCommand) -> AppEvent {
        if should_clear_completion_cache(&command) {
            self.clear_completion_cache();
        }
        // 对象刷新保留旧候选，仅标记相关索引失效，避免主线程释放并重建整个索引。
        match &command {
            AppCommand::RefreshObject(Some(object)) => self.invalidate_completion_metadata(
                object.connection_id,
                object.database.as_deref(),
                object.schema.as_deref(),
                None,
            ),
            AppCommand::RefreshObject(None) | AppCommand::RefreshConnectionTree => {
                let scopes = self
                    .completion_index
                    .lock()
                    .ok()
                    .map(|index| index.metas.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                for scope in scopes {
                    self.invalidate_completion_metadata(
                        scope.connection_id,
                        scope.database.as_deref(),
                        scope.schema.as_deref(),
                        None,
                    );
                }
            }
            _ => {}
        }

        match command {
            command @ (AppCommand::BrowseAiTableObjects { .. }
            | AppCommand::FinishAiTableObjects { .. }
            | AppCommand::LoadAiTableColumns(_)
            | AppCommand::RememberAiTableObject(_)
            | AppCommand::BeginAiTableQuery { .. }
            | AppCommand::LoadAiTablePage { .. }) => self.dispatch_ai_table_page(command),
            AppCommand::AiConversation(command) => return self.dispatch_ai_conversation(command),
            command @ (AppCommand::OpenMongoCli { .. }
            | AppCommand::LoadMongoFieldSample { .. }
            | AppCommand::OpenMongoCollection { .. }
            | AppCommand::BeginMongoLoad(_)
            | AppCommand::LoadMongoDocuments { .. }
            | AppCommand::FinishMongoDocuments { .. }
            | AppCommand::LoadMongoCollectionInfo(_)
            | AppCommand::FinishMongoCollectionInfo { .. }
            | AppCommand::LoadMongoIndexes(_)
            | AppCommand::FinishMongoIndexes { .. }
            | AppCommand::LoadMongoServerInfo(_)
            | AppCommand::CreateMongoCollection { .. }
            | AppCommand::DropMongoCollection { .. }
            | AppCommand::RenameMongoCollection { .. }
            | AppCommand::MongoRunCommand { .. }
            | AppCommand::SelectMongoSection { .. }
            | AppCommand::SelectMongoDisplayMode { .. }
            | AppCommand::SelectMongoDocument { .. }
            | AppCommand::SaveMongoDocument { .. }
            | AppCommand::DeleteMongoDocument { .. }
            | AppCommand::InsertMongoDocument { .. }
            | AppCommand::SetMongoOffset { .. }
            | AppCommand::UpdateMongoAggregation { .. }
            | AppCommand::BeginMongoAggregation(_)
            | AppCommand::RunMongoAggregation { .. }
            | AppCommand::FinishMongoAggregation { .. }) => self.dispatch_mongo_command(command),
            command @ (AppCommand::PrepareBackup { .. }
            | AppCommand::RunBackup(_)
            | AppCommand::PrepareRestore(_)
            | AppCommand::ProbeRestore(_)
            | AppCommand::RunRestore { .. }
            | AppCommand::ImportTableData { .. }
            | AppCommand::ImportMongoDocuments { .. }) => {
                self.dispatch_database_task(command, &AtomicBool::new(false), &mut |_| {})
            }
            AppCommand::LoadConnections
            | AppCommand::ReplaceConnections(..)
            | AppCommand::ReplaceSidebarLayout(..)
            | AppCommand::CreateConnection(..)
            | AppCommand::UpdateConnection(..)
            | AppCommand::CreateConnectionGroup(..)
            | AppCommand::RenameConnectionGroup { .. }
            | AppCommand::ToggleConnectionGroup(..)
            | AppCommand::DeleteConnectionGroup(..)
            | AppCommand::MoveConnectionToGroup { .. }
            | AppCommand::MoveConnectionToTopLevel(..)
            | AppCommand::TestConnection(..)
            | AppCommand::DiscoverRedisConnection { .. }
            | AppCommand::DiscoverMongoConnection { .. }
            | AppCommand::OpenConnection(..)
            | AppCommand::ToggleConnectionExpanded(..)
            | AppCommand::DisconnectConnection(..)
            | AppCommand::DisconnectDatabase { .. }
            | AppCommand::DeleteConnection(..)
            | AppCommand::CreateDatabase(..)
            | AppCommand::CreateSchema { .. }
            | AppCommand::DeleteDatabase { .. }
            | AppCommand::LoadObjectChildren(..)
            | AppCommand::RefreshObject(..)
            | AppCommand::RefreshConnectionTree => self.dispatch_connection_command(command),
            AppCommand::LoadPgRoles(..)
            | AppCommand::CreatePgRole { .. }
            | AppCommand::AlterPgRolePassword { .. }
            | AppCommand::RenamePgRole { .. }
            | AppCommand::DropPgRole { .. }
            | AppCommand::SelectPgRole { .. }
            | AppCommand::SetPgRoleSwitchPending { .. }
            | AppCommand::PgCancelSwitchRole(..)
            | AppCommand::DiscardPgDraftAndSelect { .. }
            | AppCommand::PgBeginCreateRole(..)
            | AppCommand::PgCancelDraft(..)
            | AppCommand::DiscardPgDraftEdits(..)
            | AppCommand::SetPgDraftName { .. }
            | AppCommand::SetPgDraftCanLogin { .. }
            | AppCommand::SetPgDraftAttr { .. }
            | AppCommand::SetPgDraftConnectionLimit { .. }
            | AppCommand::SetPgDraftValidUntil { .. }
            | AppCommand::SetPgDraftPasswordOp { .. }
            | AppCommand::SetPgDraftPassword { .. }
            | AppCommand::StartUserAdminPgRolesLoad(..)
            | AppCommand::LoadUserAdminPgRoles(..)
            | AppCommand::FinishUserAdminPgRolesLoad { .. }
            | AppCommand::StartPgMembershipsLoad(..)
            | AppCommand::LoadPgMemberships(..)
            | AppCommand::FinishPgMembershipsLoad { .. }
            | AppCommand::PgMembershipGrant { .. }
            | AppCommand::PgMembershipRevoke { .. }
            | AppCommand::PgMembershipRemoveEdit { .. }
            | AppCommand::SetPgGrantDatabase { .. }
            | AppCommand::StartPgGrantTargetsLoad(..)
            | AppCommand::LoadPgGrantTargets { .. }
            | AppCommand::FinishPgGrantTargetsLoad { .. }
            | AppCommand::PgToggleGrant { .. }
            | AppCommand::PgRemoveGrantEdit { .. }
            | AppCommand::StartPgPlanPreview(..)
            | AppCommand::LoadPgPlanPreview(..)
            | AppCommand::FinishPgPlanPreview { .. }
            | AppCommand::StartPgPlanApply(..)
            | AppCommand::ApplyPgRolePlan(..)
            | AppCommand::FinishPgRolePlanApply { .. }
            | AppCommand::PgBeginDeleteRole(..)
            | AppCommand::PgCancelDeleteRole(..)
            | AppCommand::SetPgRoleFilter { .. }
            | AppCommand::SetUserAdminPgGrantTarget { .. }
            | AppCommand::LoadUserAdminPgObjectGrants(..)
            | AppCommand::StartUserAdminPgObjectGrantsLoad(..)
            | AppCommand::FinishUserAdminPgObjectGrantsLoad { .. } => self.dispatch_pg_user_admin_command(command),
            AppCommand::OpenObjectList(..)
            | AppCommand::OpenDataEditor(..)
            | AppCommand::OpenBackupList(..)
            | AppCommand::OpenErDiagram(..)
            | AppCommand::OpenQueryEditor(..)
            | AppCommand::OpenUserAdmin(..)
            | AppCommand::OpenSettings
            | AppCommand::OpenAiChatTab
            | AppCommand::OpenQueryEditorInDatabase { .. }
            | AppCommand::OpenRedisWorkbench { .. }
            | AppCommand::OpenRedisCli { .. }
            | AppCommand::OpenRedisPubSub { .. }
            | AppCommand::CloseTab(..)
            | AppCommand::CloseTabs(..)
            | AppCommand::ConfirmCloseDirtyTab(..)
            | AppCommand::CancelCloseDirtyTab(..)
            | AppCommand::ActivateTab(..)
            | AppCommand::DeactivateTab
            | AppCommand::PinTab(..)
            | AppCommand::SetTabDirty { .. } => self.dispatch_tab_command(command),
            AppCommand::LoadErRelationships { scope_key } => {
                let service =
                    self.er_model_service(scope_key.clone())
                        .ok_or_else(|| UserFacingError {
                            title: "ER 关系加载失败".into(),
                            message: "ER 存储尚未初始化".into(),
                            detail: None,
                            retryable: true,
                        });
                match service.and_then(|service| {
                    service.list().map_err(|error| UserFacingError {
                        title: "ER 关系加载失败".into(),
                        message: error.to_string(),
                        detail: None,
                        retryable: true,
                    })
                }) {
                    Ok(relationships) => AppEvent::ErRelationshipsLoaded {
                        scope_key,
                        relationships,
                    },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::CreateErRelationship {
                scope_key,
                relationship,
            } => {
                let result = self
                    .er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError {
                        title: "ER 关系创建失败".into(),
                        message: "ER 存储尚未初始化".into(),
                        detail: None,
                        retryable: true,
                    })
                    .and_then(|service| {
                        service
                            .create(relationship)
                            .map_err(|error| UserFacingError {
                                title: "ER 关系创建失败".into(),
                                message: error.to_string(),
                                detail: None,
                                retryable: false,
                            })
                    });
                match result {
                    Ok(relationship) => AppEvent::ErRelationshipChanged {
                        scope_key,
                        relationship,
                    },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::UpdateErRelationship {
                scope_key,
                relationship,
                expected_revision,
            } => {
                let id = relationship.id.clone();
                let result = self
                    .er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError {
                        title: "ER 关系更新失败".into(),
                        message: "ER 存储尚未初始化".into(),
                        detail: None,
                        retryable: true,
                    })
                    .and_then(|service| {
                        service
                            .update(&id, expected_revision, move |current| {
                                *current = relationship;
                                Ok(())
                            })
                            .map_err(|error| UserFacingError {
                                title: "ER 关系更新失败".into(),
                                message: error.to_string(),
                                detail: None,
                                retryable: false,
                            })
                    });
                match result {
                    Ok(relationship) => AppEvent::ErRelationshipChanged {
                        scope_key,
                        relationship,
                    },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::RebindErRelationship {
                scope_key,
                relationship,
                expected_revision,
            } => {
                let id = relationship.id.clone();
                let result = self
                    .er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError {
                        title: "ER 自动重绑失败".into(),
                        message: "ER 存储尚未初始化".into(),
                        detail: None,
                        retryable: true,
                    })
                    .and_then(|service| {
                        service
                            .rebind_identity(&id, expected_revision, relationship)
                            .map_err(|error| UserFacingError {
                                title: "ER 自动重绑失败".into(),
                                message: error.to_string(),
                                detail: None,
                                retryable: false,
                            })
                    });
                match result {
                    Ok(relationship) => AppEvent::ErRelationshipChanged {
                        scope_key,
                        relationship,
                    },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::ConfirmErRelationship {
                scope_key,
                id,
                expected_revision,
                by,
            } => {
                let result = self
                    .er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError {
                        title: "ER 关系确认失败".into(),
                        message: "ER 存储尚未初始化".into(),
                        detail: None,
                        retryable: true,
                    })
                    .and_then(|service| {
                        service
                            .confirm(&id, expected_revision, &by)
                            .map_err(|error| UserFacingError {
                                title: "ER 关系确认失败".into(),
                                message: error.to_string(),
                                detail: None,
                                retryable: false,
                            })
                    });
                match result {
                    Ok(relationship) => AppEvent::ErRelationshipChanged {
                        scope_key,
                        relationship,
                    },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::RejectErRelationship {
                scope_key,
                id,
                expected_revision,
            } => {
                let result = self
                    .er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError {
                        title: "ER 关系拒绝失败".into(),
                        message: "ER 存储尚未初始化".into(),
                        detail: None,
                        retryable: true,
                    })
                    .and_then(|service| {
                        service
                            .reject(&id, expected_revision)
                            .map_err(|error| UserFacingError {
                                title: "ER 关系拒绝失败".into(),
                                message: error.to_string(),
                                detail: None,
                                retryable: false,
                            })
                    });
                match result {
                    Ok(relationship) => AppEvent::ErRelationshipChanged {
                        scope_key,
                        relationship,
                    },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::AdoptErRelationshipProposals {
                scope_key,
                connection_id,
                database,
                schema,
                proposals,
                message_part_id,
                batch_id,
            } => {
                let result = (|| {
                    let expected_scope = fluxdb_core::er_relationship_scope_key(
                        connection_id,
                        &database,
                        schema.as_deref(),
                    );
                    if expected_scope != scope_key {
                        return Err(UserFacingError {
                            title: "关系建议采纳失败".into(),
                            message: "建议作用域与当前数据库不一致".into(),
                            detail: None,
                            retryable: false,
                        });
                    }
                    let config = self
                        .state
                        .connections
                        .iter()
                        .find(|connection| connection.config.id == connection_id)
                        .map(|connection| connection.config.clone())
                        .ok_or_else(|| UserFacingError {
                            title: "关系建议采纳失败".into(),
                            message: "连接已不存在，无法核实建议作用域".into(),
                            detail: None,
                            retryable: false,
                        })?;
                    let source = self
                        .er_relationship_source()
                        .ok_or_else(|| UserFacingError {
                            title: "关系建议采纳失败".into(),
                            message: "ER 存储尚未初始化".into(),
                            detail: None,
                            retryable: true,
                        })?;
                    er_validate_proposals_for_adoption(
                        &config,
                        &scope_key,
                        source.as_ref(),
                        &proposals,
                    )
                    .map_err(|error| UserFacingError {
                        title: "关系建议采纳失败".into(),
                        message: error.message,
                        detail: Some(error.code.into()),
                        retryable: false,
                    })?;
                    let service = self.er_model_service(scope_key.clone()).ok_or_else(|| {
                        UserFacingError {
                            title: "关系建议采纳失败".into(),
                            message: "ER 存储尚未初始化".into(),
                            detail: None,
                            retryable: true,
                        }
                    })?;
                    let adopted = service
                        .create_confirmed_batch(proposals.clone(), "user")
                        .map_err(|error| UserFacingError {
                            title: "关系建议采纳失败".into(),
                            message: error.to_string(),
                            detail: None,
                            retryable: false,
                        })?;
                    // 批次卡是用户唯一能看到“还剩下什么”的地方；采纳后同步更新，
                    // 避免再次打开仍把已写入项当作待审阅。
                    if let Ok(storage) = self.ai_storage() {
                        let adopted_ids = proposals
                            .iter()
                            .map(|relationship| relationship.id.clone())
                            .collect::<BTreeSet<_>>();
                        let now = ai_chat_now();
                        let _ = with_ai_storage(&storage, |conn| {
                            let part = fluxdb_storage::ai_platform::sessions::load_part(
                                conn,
                                &message_part_id,
                            )?;
                            let mut payload: serde_json::Value =
                                serde_json::from_str(&part.payload_json).map_err(|_| {
                                    Error::new(
                                        ErrorKind::Internal,
                                        "关系建议卡片内容损坏，无法记录采纳结果",
                                    )
                                })?;
                            if payload.get("batch_id").and_then(serde_json::Value::as_str)
                                != Some(batch_id.as_str())
                            {
                                return Err(Error::new(
                                    ErrorKind::Internal,
                                    "关系建议批次已变化，未更新卡片状态",
                                ));
                            }
                            let mut ready = 0usize;
                            if let Some(items) = payload
                                .get_mut("items")
                                .and_then(serde_json::Value::as_array_mut)
                            {
                                for item in items {
                                    let item_id = item
                                        .get("item_id")
                                        .and_then(serde_json::Value::as_str)
                                        .map(str::to_string);
                                    if item_id.as_ref().is_some_and(|id| adopted_ids.contains(id)) {
                                        item["status"] = serde_json::json!("adopted");
                                        item["relationship"] = serde_json::Value::Null;
                                    }
                                    if item.get("status").and_then(serde_json::Value::as_str)
                                        == Some("ready")
                                    {
                                        ready += 1;
                                    }
                                }
                            }
                            payload["counts"]["ready"] = serde_json::json!(ready);
                            let previously_adopted = payload
                                .get("review")
                                .and_then(|review| review.get("adopted"))
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0)
                                as usize;
                            payload["review"] = serde_json::json!({
                                "status": if ready == 0 { "complete" } else { "partial" },
                                "adopted": previously_adopted + adopted.len(),
                            });
                            fluxdb_storage::ai_platform::sessions::update_part(
                                conn,
                                now,
                                &message_part_id,
                                fluxdb_storage::ai_platform::sessions::PartStatus::Complete,
                                &payload.to_string(),
                                part.revision,
                            )?;
                            Ok(())
                        });
                    }
                    Ok(AppEvent::ErRelationshipProposalsAdopted {
                        scope_key,
                        relationships: adopted,
                    })
                })();
                match result {
                    Ok(event) => event,
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::DeleteErRelationship {
                scope_key,
                id,
                expected_revision,
            } => {
                let result = self
                    .er_model_service(scope_key.clone())
                    .ok_or_else(|| UserFacingError {
                        title: "ER 关系删除失败".into(),
                        message: "ER 存储尚未初始化".into(),
                        detail: None,
                        retryable: true,
                    })
                    .and_then(|service| {
                        service
                            .delete(&id, expected_revision)
                            .map_err(|error| UserFacingError {
                                title: "ER 关系删除失败".into(),
                                message: error.to_string(),
                                detail: None,
                                retryable: false,
                            })
                    });
                match result {
                    Ok(()) => AppEvent::ErRelationshipDeleted { scope_key, id },
                    Err(error) => AppEvent::Failed(error),
                }
            }
            AppCommand::LoadDataPage(tab_id) => {
                let generation = match self.find_data_editor_mut(tab_id) {
                    Some(editor) => editor.begin_load(),
                    None => {
                        return self
                            .fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                    }
                };
                self.load_data_page_command(tab_id, generation, Vec::new(), Vec::new(), None)
            }
            AppCommand::BeginDataPageLoad(tab_id) => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                let generation = editor.begin_load();
                AppEvent::DataPageLoadStarted { tab_id, generation }
            }
            AppCommand::SetDataPagePagination {
                tab_id,
                offset,
                limit,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::DataEditor(editor) = &mut tab.kind
                {
                    editor.pagination = Pagination::new(offset, limit);
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"))
                }
            }
            AppCommand::LoadDataPageWithSort {
                tab_id,
                generation,
                sort,
                filters,
                expression,
            } => self.load_data_page_command(tab_id, generation, sort, filters, expression),
            AppCommand::LoadRedisKey { tab_id, key } => self.load_redis_key_command(tab_id, key),
            AppCommand::LoadRedisKeyMetadata { tab_id, keys } => {
                self.load_redis_key_metadata_command(tab_id, keys)
            }
            AppCommand::LoadRedisOverview(connection_id) => {
                self.load_redis_overview_command(connection_id)
            }
            AppCommand::LoadRedisServerVersion(connection_id) => {
                self.load_redis_server_version_command(connection_id)
            }
            AppCommand::FinishRedisKeyRefresh {
                tab_id,
                key,
                result,
            } => self.finish_redis_key_refresh_command(tab_id, key, result),
            AppCommand::ApplyRedisKeyValue {
                tab_id,
                key,
                new_key,
                ttl,
                value,
            } => self.apply_redis_key_value_command(tab_id, key, new_key, ttl, value),
            AppCommand::AddRedisStreamEntry {
                tab_id,
                key,
                id,
                fields,
                maxlen,
            } => self.apply_redis_stream_entry_add_command(tab_id, key, id, fields, maxlen),
            AppCommand::DeleteRedisStreamEntry {
                tab_id,
                key,
                entry_id,
            } => self.apply_redis_stream_entry_delete_command(tab_id, key, entry_id),
            AppCommand::DeleteRedisSetMember {
                tab_id,
                key,
                member,
            } => self.apply_redis_set_member_delete_command(tab_id, key, member),
            AppCommand::AddRedisSetMember {
                tab_id,
                key,
                member,
            } => self.apply_redis_set_member_add_command(tab_id, key, member),
            AppCommand::LoadRedisSetMembers {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_set_members_command(tab_id, key, query, cursor),
            AppCommand::LoadRedisHashFields {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_hash_fields_command(tab_id, key, query, cursor),
            AppCommand::SetRedisHashField {
                tab_id,
                key,
                field,
                value,
                ttl,
            } => self.apply_redis_hash_field_set_command(tab_id, key, field, value, ttl),
            AppCommand::SetRedisHashFieldRaw {
                tab_id,
                key,
                field,
                value,
                ttl,
            } => self.apply_redis_hash_field_set_raw_command(tab_id, key, field, value, ttl),
            AppCommand::SetRedisHashFieldTtl {
                tab_id,
                key,
                field,
                ttl,
            } => self.apply_redis_hash_field_ttl_command(tab_id, key, field, ttl),
            AppCommand::LoadRedisHashFieldFull { tab_id, key, field } => {
                self.load_redis_hash_field_full_command(tab_id, key, field)
            }
            AppCommand::LoadRedisStringValue { tab_id, key, full } => {
                self.load_redis_string_value_command(tab_id, key, full)
            }
            AppCommand::DownloadRedisStringValue { tab_id, key } => {
                self.download_redis_string_value_command(tab_id, key)
            }
            AppCommand::RenameRedisHashField {
                tab_id,
                key,
                old_field,
                new_field,
                value,
            } => {
                self.apply_redis_hash_field_rename_command(tab_id, key, old_field, new_field, value)
            }
            AppCommand::DeleteRedisHashField { tab_id, key, field } => {
                self.apply_redis_hash_field_delete_command(tab_id, key, field)
            }
            AppCommand::LoadRedisZSetMembers {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_zset_members_command(tab_id, key, query, cursor),
            AppCommand::AddRedisZSetMember {
                tab_id,
                key,
                member,
                score,
            } => self.apply_redis_zset_member_add_command(tab_id, key, member, score),
            AppCommand::UpdateRedisZSetScore {
                tab_id,
                key,
                member,
                score,
            } => self.apply_redis_zset_score_update_command(tab_id, key, member, score),
            AppCommand::DeleteRedisZSetMember {
                tab_id,
                key,
                member,
            } => self.apply_redis_zset_member_delete_command(tab_id, key, member),
            AppCommand::LoadRedisListItems {
                tab_id,
                key,
                query,
                cursor,
            } => self.load_redis_list_items_command(tab_id, key, query, cursor),
            AppCommand::LoadRedisStreamEntries {
                tab_id,
                key,
                since_ms,
                until_ms,
                cursor,
            } => self.load_redis_stream_entries_command(tab_id, key, since_ms, until_ms, cursor),
            AppCommand::LoadRedisStreamGroups { tab_id, key } => {
                self.load_redis_stream_groups_command(tab_id, key)
            }
            AppCommand::PushRedisListItems {
                tab_id,
                key,
                items,
                head,
            } => self.apply_redis_list_items_push_command(tab_id, key, items, head),
            AppCommand::CreateRedisKey { tab_id, request } => {
                self.apply_redis_create_key_command(tab_id, request)
            }
            AppCommand::SetRedisListItem {
                tab_id,
                key,
                index,
                expected_old,
                value,
            } => self.apply_redis_list_item_set_command(tab_id, key, index, expected_old, value),
            AppCommand::DeleteRedisListItem {
                tab_id,
                key,
                index,
                expected_old,
            } => self.apply_redis_list_item_delete_command(tab_id, key, index, expected_old),
            AppCommand::PopRedisListItems {
                tab_id,
                key,
                head,
                count,
            } => self.apply_redis_list_items_pop_command(tab_id, key, head, count),
            AppCommand::FinishDataPageLoad {
                tab_id,
                generation,
                result,
            } => match result {
                Ok(page) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        if !editor.accepts_load(generation) {
                            return AppEvent::DataPageLoadDiscarded { tab_id, generation };
                        }
                        editor.page = Some(page.clone());
                        editor.original_page = Some(page.clone());
                        editor.changes = None;
                        editor.editing_cell = None;
                        editor.cell_detail_panel = CellDetailPanelState::default();
                        editor.loading = false;
                        editor.error = None;
                        tab.dirty = false;
                        AppEvent::DataLoaded(tab_id, page)
                    } else {
                        self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"))
                    }
                }
                Err(user_error) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::DataEditor(editor) = &mut tab.kind
                    {
                        if !editor.accepts_load(generation) {
                            return AppEvent::DataPageLoadDiscarded { tab_id, generation };
                        }
                        editor.loading = false;
                        editor.error = Some(user_error.clone());
                    }
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            AppCommand::ToggleTableInfo { tab_id, tab } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                if editor.table_info.open && editor.table_info.active_tab == tab {
                    editor.table_info.open = false;
                } else {
                    editor.table_info.open = true;
                    editor.table_info.active_tab = tab;
                    mark_table_info_loading(&mut editor.table_info, tab);
                }
                AppEvent::TableInfoChanged(tab_id, tab)
            }
            AppCommand::CloseTableInfo(tab_id) => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                let active_tab = editor.table_info.active_tab;
                editor.table_info.open = false;
                AppEvent::TableInfoChanged(tab_id, active_tab)
            }
            AppCommand::SelectTableInfoTab { tab_id, tab } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.open = true;
                editor.table_info.active_tab = tab;
                mark_table_info_loading(&mut editor.table_info, tab);
                AppEvent::TableInfoChanged(tab_id, tab)
            }
            AppCommand::LoadTableInfo { tab_id, tab } => {
                let Some(object) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::DataEditor(editor) => Some(editor.object.clone()),
                    _ => None,
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };

                match self.load_table_info(&object, tab) {
                    Ok(result) => AppEvent::TableInfoLoaded(tab_id, result.tab(), result),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::FinishTableInfoLoad {
                tab_id,
                tab,
                result,
            } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                match result {
                    Ok(result) => {
                        apply_table_info_result(&mut editor.table_info, result);
                        AppEvent::TableInfoChanged(tab_id, tab)
                    }
                    Err(error) => {
                        set_table_info_failed(&mut editor.table_info, tab, error.clone());
                        self.state.last_error = Some(error);
                        AppEvent::TableInfoChanged(tab_id, tab)
                    }
                }
            }
            AppCommand::SetTableInfoSearch { tab_id, search } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.search = search;
                AppEvent::TableInfoChanged(tab_id, editor.table_info.active_tab)
            }
            AppCommand::SetTableInfoWidth { tab_id, width } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.width = width.clamp(260., 800.);
                AppEvent::TableInfoChanged(tab_id, editor.table_info.active_tab)
            }
            AppCommand::ToggleDdlWrap(tab_id) => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.ddl_wrap = !editor.table_info.ddl_wrap;
                AppEvent::TableInfoChanged(tab_id, TableInfoTab::Ddl)
            }
            AppCommand::HighlightDataColumn { tab_id, column } => {
                let Some(editor) = self.find_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据编辑器标签页不存在"));
                };
                editor.table_info.highlighted_column = Some(column);
                AppEvent::TableInfoChanged(tab_id, TableInfoTab::Columns)
            }
            AppCommand::EditDataCell {
                tab_id,
                row,
                column,
                value,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    match edit_data_cell(editor, row, column, value) {
                        Ok(()) => {
                            tab.dirty = editor
                                .changes
                                .as_ref()
                                .is_some_and(|changes| !changes.is_empty());
                            AppEvent::TabActivated(tab_id)
                        }
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::LoadCellText {
                tab_id,
                row,
                column,
            } => match self.load_cell_text(tab_id, row, column) {
                Ok(text) => AppEvent::CellTextLoaded {
                    tab_id,
                    row,
                    column,
                    text,
                },
                Err(error) => self.fail(error),
            },
            AppCommand::FinishCellTextLoad {
                tab_id,
                row,
                column,
                result,
            } => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                let active = editor.cell_detail_panel.active_cell;
                match result {
                    Ok(text) => {
                        // 只写回仍处于同一单元格的面板：后台期间用户可能已经切走。
                        if active == Some(CellPosition { row, column }) {
                            editor.cell_detail_panel.full_value = Some(text.clone());
                            editor.cell_detail_panel.edit_value = text;
                            // 用户是在编辑动作里触发的加载：读完后直接进编辑态。
                            if editor.cell_detail_panel.pending_edit {
                                editor.cell_detail_panel.pending_edit = false;
                                editor.cell_detail_panel.mode = CellDetailMode::Edit;
                            }
                        }
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(user_error) => {
                        self.state.last_error = Some(user_error.clone());
                        AppEvent::Failed(user_error)
                    }
                }
            }
            AppCommand::LoadCellContentChunk {
                        tab_id,
                        row,
                        column,
                content_id,
                kind,
                offset,
            } => match self.load_cell_content_chunk(&content_id, kind, offset) {
                Ok(chunk) => AppEvent::CellContentChunkLoaded {
                    tab_id,
                    row,
                    column,
                    chunk,
                    },
                    Err(error) => self.fail(error),
            },
            AppCommand::SaveCellContentToFile {
                tab_id,
                content_id,
                path,
            } => match self.save_cell_content_to_file(&content_id, &path) {
                Ok(bytes) => AppEvent::CellContentSaved { tab_id, bytes },
                Err(error) => {
                    let user_error = UserFacingError::from(error);
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            AppCommand::DownloadBinaryCell {
                tab_id,
                row,
                column,
            } => match self.download_binary_cell(tab_id, row, column) {
                Ok(bytes) => AppEvent::BinaryCellDownloaded {
                    tab_id,
                    row,
                    column,
                    bytes,
                },
                Err(error) => self.fail(error),
            },
            AppCommand::UpdateBinaryCell {
                tab_id,
                row,
                column,
                payload,
            } => self.update_binary_cell_command(tab_id, row, column, payload),
            AppCommand::SetBinaryCellNull {
                tab_id,
                row,
                column,
            } => self.update_binary_cell_command(tab_id, row, column, UploadPayload::SetNull),
            AppCommand::ReplaceBinaryCellFromFile {
                tab_id,
                row,
                column,
                path,
            } => self.update_binary_cell_command(
                tab_id,
                row,
                column,
                UploadPayload::FilePath(path),
            ),
            AppCommand::OpenCellDetail {
                tab_id,
                row,
                column,
            } => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match current_cell_value(editor, row, column) {
                    Ok(value) => {
                        let needs_full_text = matches!(value, CellValue::TruncatedText { .. });
                        editor.cell_detail_panel = CellDetailPanelState {
                            open: true,
                            active_cell: Some(CellPosition { row, column }),
                            mode: CellDetailMode::View,
                            edit_value: cell_value_edit_text(&value),
                            full_value: None,
                            pending_edit: false,
                        };
                        if needs_full_text {
                            AppEvent::CellTextLoadRequested {
                                tab_id,
                                row,
                                column,
                            }
                        } else {
                        AppEvent::TabActivated(tab_id)
                    }
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::CloseCellDetail(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                editor.cell_detail_panel = CellDetailPanelState::default();
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::StartCellDetailEdit(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match active_detail_cell_value(editor) {
                    // 二进制一律不进编辑态：编辑框只可能拿到 64 字节 hex 预览，
                    // 保存会用预览覆盖整值。改内容请走「上传替换」，清空请走「设为 NULL」。
                    Ok(CellValue::Bytes(_) | CellValue::BinarySummary(_)) => {
                        self.fail(Error::new(
                            ErrorKind::Unsupported,
                            "二进制字段不支持就地编辑，请使用上传替换或设为 NULL",
                        ))
                    }
                    // 超大字段（含等着提交的流式上传占位值）不进编辑态：正文在内容
                    // 存储里，编辑框只可能拿到预览。改内容走「上传替换」，清空走「设为 NULL」。
                    Ok(CellValue::LargeValue(_)) => self.fail(Error::new(
                        ErrorKind::Unsupported,
                        "超大字段不支持就地编辑；请使用分段查看或上传替换",
                    )),
                    // 预览不是完整值：先按需加载完整文本，读到之前不进入编辑态。
                    Ok(CellValue::TruncatedText { .. }) => {
                        let active_cell = editor.cell_detail_panel.active_cell;
                        match (
                            editor.cell_detail_panel.full_value.clone(),
                            active_cell,
                        ) {
                            (Some(text), _) => {
                                editor.cell_detail_panel.mode = CellDetailMode::Edit;
                                editor.cell_detail_panel.edit_value = text;
                                AppEvent::TabActivated(tab_id)
                            }
                            (None, Some(cell)) => {
                                editor.cell_detail_panel.pending_edit = true;
                                AppEvent::CellTextLoadRequested {
                                    tab_id,
                                    row: cell.row,
                                    column: cell.column,
                                }
                            }
                            (None, None) => self.fail(Error::new(
                                ErrorKind::Internal,
                                "未选择单元格",
                            )),
                        }
                    }
                    Ok(value) => {
                        editor.cell_detail_panel.mode = CellDetailMode::Edit;
                        editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::UpdateCellDetailEditValue { tab_id, value } => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                if editor.cell_detail_panel.open {
                    editor.cell_detail_panel.edit_value = value;
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::CancelCellDetailEdit(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match active_detail_cell_value(editor) {
                    Ok(value) => {
                        editor.cell_detail_panel.mode = CellDetailMode::View;
                        editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::SaveCellDetailEdit(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    match save_cell_detail_edit(editor) {
                        Ok(()) => {
                            tab.dirty = editor
                                .changes
                                .as_ref()
                                .is_some_and(|changes| !changes.is_empty());
                            AppEvent::TabActivated(tab_id)
                        }
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::SetCellDetailNull(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    match set_cell_detail_value(editor, CellValue::Null) {
                        Ok(()) => {
                            tab.dirty = editor
                                .changes
                                .as_ref()
                                .is_some_and(|changes| !changes.is_empty());
                            AppEvent::TabActivated(tab_id)
                        }
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::RestoreCellDetailOriginalValue(tab_id) => {
                let Some(editor) = self.find_editable_data_editor_mut(tab_id) else {
                    return self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"));
                };
                match active_original_cell_value(editor) {
                    Ok(value) => {
                        // 原始值可能只是预览：有完整值就用完整值，别把编辑框降级成片段。
                        editor.cell_detail_panel.edit_value = match &value {
                            CellValue::TruncatedText { .. } => editor
                                .cell_detail_panel
                                .full_value
                                .clone()
                                .unwrap_or_else(|| cell_value_edit_text(&value)),
                            _ => cell_value_edit_text(&value),
                        };
                        AppEvent::TabActivated(tab_id)
                    }
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::InsertDataRow {
                tab_id,
                result_index,
                after_row,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id) {
                    match apply_data_editor_edit(tab, result_index, |editor| {
                        insert_data_row(editor, after_row)
                    }) {
                        Ok(()) => AppEvent::TabActivated(tab_id),
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::CloneDataRow {
                tab_id,
                result_index,
                row,
                after_row,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id) {
                    match apply_data_editor_edit(tab, result_index, |editor| {
                        clone_data_row(editor, row, after_row)
                    }) {
                        Ok(()) => AppEvent::TabActivated(tab_id),
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::DeleteDataRow {
                tab_id,
                result_index,
                row,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id) {
                    match apply_data_editor_edit(tab, result_index, |editor| {
                        delete_data_row(editor, row)
                    }) {
                        Ok(()) => AppEvent::TabActivated(tab_id),
                        Err(error) => self.fail(error),
                    }
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::DiscardDataChanges(tab_id) => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let Some(editor) = editable_data_editor_mut(&mut tab.kind)
                {
                    editor.page = editor.original_page.clone();
                    editor.changes = None;
                    editor.editing_cell = None;
                    if editor.cell_detail_panel.open {
                        if let Ok(value) = active_detail_cell_value(editor) {
                            editor.cell_detail_panel.mode = CellDetailMode::View;
                            editor.cell_detail_panel.edit_value = cell_value_edit_text(&value);
                        }
                    }
                    editor.error = None;
                    if let TabKind::QueryEditor(query_editor) = &mut tab.kind
                        && let Some(page_index) = query_editor.active_result_editor
                        && let Some(page) = query_editor
                            .result_editors
                            .get(&page_index)
                            .and_then(|result| result.page.clone())
                        && let Some(result) = query_editor.results.get_mut(page_index)
                    {
                        *result = page;
                    }
                    tab.dirty = false;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "数据结果不存在"))
                }
            }
            AppCommand::StartApplyDataChanges(tab_id) => {
                self.start_apply_data_changes_command(tab_id)
            }
            AppCommand::RunApplyDataChanges {
                tab_id,
                sort,
                filters,
            } => {
                let result = self.run_apply_data_changes_command(tab_id, sort, filters);
                self.finish_apply_data_changes_command(tab_id, result)
            }
            AppCommand::FinishApplyDataChanges { tab_id, result } => {
                self.finish_apply_data_changes_command(tab_id, result)
            }
            AppCommand::UpdateQueryText { tab_id, text } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.text = text;
                    editor.error = None;
                    tab.dirty = editor.has_unsaved_sql();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::UpdateRedisWorkbenchText { tab_id, text } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::RedisWorkbench(workbench) = &mut tab.kind
                {
                    workbench.text = text;
                    workbench.error = None;
                    tab.dirty = workbench.has_unsaved_text();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ))
                }
            }
            AppCommand::BeginRedisWorkbenchExecution {
                tab_id,
                execution_id,
            } => {
                // 准备步只做状态变更：真实 RESP 往返由 UI 派发到后台线程跑
                // `RunRedisWorkbench`，结果再经 `FinishRedisWorkbenchExecution` 落回。
                // 重跑要按 id 定位记录，记录已被删除时明确失败，不进入运行态。
                if execution_id.is_some()
                    && self.redis_workbench_text(tab_id, execution_id).is_none()
                {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 记录不存在",
                    ));
                }
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                let TabKind::RedisWorkbench(workbench) = &mut tab.kind else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                workbench.running = true;
                workbench.error = None;
                // 执行顶部草稿才清空输入框；重跑结果区记录不打扰当前草稿。
                if execution_id.is_none() {
                    workbench.text.clear();
                    workbench.saved_fingerprint = Some(QueryFingerprint::for_text(""));
                    tab.dirty = workbench.has_unsaved_text();
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::RunRedisWorkbench {
                tab_id,
                execution_id,
            } => {
                // 只执行、不写标签页状态：本命令跑在后台线程的控制器副本上，
                // 状态回写统一交给主线程的 FinishRedisWorkbenchExecution。
                let Some((target, text)) = self.redis_workbench_run_scope(tab_id, execution_id)
                else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                // 执行单元 = 单条命令：把输入文本交给批量执行入口，得到
                // 「每条命令一个 `CommandWorkbenchExecution`」的列表，逐条追加到
                // 结果区，各自独立占一张卡片（独立 Run / Delete / 时间 / 耗时）。
                let request = CommandWorkbenchRequest {
                    target,
                    text: text.clone(),
                    run_mode: CommandRunMode::Text,
                    results_mode: CommandResultsMode::Default,
                    batch_size: 0,
                    continue_on_error: true,
                    source: CommandExecutionSource::Workbench,
                };
                // 失败也用同一事件回传：入历史要带实际执行的文本，而不是执行期间的草稿。
                let result = self
                    .execute_command_workbench_commands(&request)
                    .map_err(UserFacingError::from);
                AppEvent::RedisWorkbenchCommandsRan {
                    tab_id,
                    text,
                    result,
                }
            }
            AppCommand::FinishRedisWorkbenchExecution {
                tab_id,
                text,
                result,
            } => match result {
                Ok(executions) => {
                    let mut history_entries = Vec::new();
                    let Some(tab) = self.find_tab_mut(tab_id) else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    };
                    let TabKind::RedisWorkbench(workbench) = &mut tab.kind else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    };
                    workbench.running = false;
                    workbench.error = None;
                    for mut execution in executions {
                        // App 层为 connector 产出的每条 execution 分配自增 id，
                        // 供结果区的单条 Run / Delete 精确定位。
                        execution.id = workbench.next_execution_id;
                        workbench.next_execution_id += 1;
                        history_entries.push(execution.clone());
                        workbench.executions.push(execution);
                    }
                    // 顶部输入框可能已被 Run 清空；脏标记基于「未执行改动」实时更新。
                    tab.dirty = workbench.has_unsaved_text();
                    // 每条命令进入历史体系（持久化由桌面层在 state 变化时落盘）。
                    for execution in &history_entries {
                        self.record_redis_workbench_history_execution(execution);
                    }
                    // 结果区由 state 驱动渲染：返回 TabActivated 触发该标签页 UI 重绘，
                    // 桌面端 dispatch 包装会统一 cx.notify()。
                    AppEvent::TabActivated(tab_id)
                }
                Err(user_error) => {
                    let Some(tab) = self.find_tab_mut(tab_id) else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "Redis Workbench 标签页不存在",
                        ));
                    };
                    let (connection_id, database) = match &mut tab.kind {
                        TabKind::RedisWorkbench(workbench) => {
                            workbench.running = false;
                            workbench.error = Some(user_error.clone());
                            (workbench.connection_id, workbench.database)
                        }
                        _ => {
                            return self.fail(Error::new(
                                ErrorKind::Internal,
                                "Redis Workbench 标签页不存在",
                            ));
                        }
                    };
                    self.record_redis_workbench_history_failure(
                        connection_id,
                        database,
                        &text,
                        &user_error.message,
                    );
                    self.state.last_error = Some(user_error.clone());
                    AppEvent::Failed(user_error)
                }
            },
            AppCommand::DeleteRedisWorkbenchRecord {
                tab_id,
                execution_id,
            } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    // 只删除指定 id 的一条记录，不影响其他记录；同时清理其折叠态与 JSON 视图标记。
                    workbench
                        .executions
                        .retain(|execution| execution.id != execution_id);
                    workbench.collapsed.remove(&execution_id);
                    workbench
                        .json_views
                        .retain(|(old_id, _)| *old_id != execution_id);
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ClearRedisWorkbenchResults(tab_id) => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    workbench.executions.clear();
                    workbench.error = None;
                    // 记录清空后折叠态与 JSON 视图标记一并清空，避免残留已删除记录的标记。
                    workbench.collapsed.clear();
                    workbench.json_views.clear();
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ToggleRedisWorkbenchRecordCollapse {
                tab_id,
                execution_id,
            } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    // 折叠/展开切换：下落（指记录仍在）、否则补回。
                    if !workbench.collapsed.insert(execution_id) {
                        workbench.collapsed.remove(&execution_id);
                    }
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::ToggleRedisWorkbenchJsonView {
                tab_id,
                execution_id,
                command_index,
            } => {
                let Some(tab) = self.find_tab_mut(tab_id) else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                };
                if let TabKind::RedisWorkbench(workbench) = &mut tab.kind {
                    // Text / JSON 切换：下落（指当前在 JSON 视图）、否则补回，按执行项维度记忆。
                    if !workbench.json_views.insert((execution_id, command_index)) {
                        workbench.json_views.remove(&(execution_id, command_index));
                    }
                } else {
                    return self.fail(Error::new(
                        ErrorKind::Internal,
                        "Redis Workbench 标签页不存在",
                    ));
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::DeleteRedisWorkbenchHistory { scope, id } => {
                // 按 scope + id 删除单条历史；删除是状态变化，无需特定标签页事件。
                self.delete_history(&scope, id);
                AppEvent::RedisWorkbenchHistoryChanged
            }
            AppCommand::ClearRedisWorkbenchHistory { scope } => {
                // 清空某 scope 下的全部历史。
                self.clear_history(&scope);
                AppEvent::RedisWorkbenchHistoryChanged
            }
            AppCommand::RequestQueryCompletions {
                tab_id,
                request_seq,
                cursor,
                explicit,
            } => {
                let Some(editor) = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::QueryEditor(editor) => Some(editor),
                    _ => None,
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };
                match self.query_completions(editor, cursor, explicit) {
                    Ok(result) => AppEvent::QueryCompletionsLoaded(tab_id, request_seq, result),
                    Err(error) => self.fail(error),
                }
            }
            AppCommand::WarmCompletionIndex {
                connection_id,
                database,
            } => match self.warm_completion_index(connection_id, database.as_deref(), None) {
                Ok(()) => AppEvent::CompletionIndexWarmed(connection_id, database),
                Err(error) => self.fail(error),
            },
            AppCommand::FormatQuerySql(tab_id) => {
                let Some((connection_id, text)) = self.find_tab(tab_id).and_then(|tab| {
                    let TabKind::QueryEditor(editor) = &tab.kind else {
                        return None;
                    };
                    Some((editor.connection_id, editor.text.clone()))
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                let kind = self
                    .connection_kind(connection_id)
                    .unwrap_or(DatabaseKind::MySql);
                let formatted = format_sql_text_for_dialect(&text, kind);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.text = formatted;
                    editor.error = None;
                    tab.dirty = editor.has_unsaved_sql();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::CompressQuerySql(tab_id) => {
                let Some(text) = self.find_tab(tab_id).and_then(|tab| {
                    let TabKind::QueryEditor(editor) = &tab.kind else {
                        return None;
                    };
                    Some(editor.text.clone())
                }) else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                let compressed = compress_sql_text(&text);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.text = compressed;
                    editor.error = None;
                    tab.dirty = editor.has_unsaved_sql();
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::MarkQuerySaved {
                tab_id,
                title,
                origin,
            } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    tab.title = title;
                    editor.origin = Some(origin);
                    editor.saved_fingerprint = Some(QueryFingerprint::for_text(&editor.text));
                    tab.dirty = false;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::StartQueryExecution(tab_id) => {
                // 每次执行前登记一个全新的取消标志：既重置上一次「停止」的残留置位，
                // 也让后台执行线程能拿到与「停止」按钮同一个标志。
                self.register_query_cancel_flag(tab_id);
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    editor.running = true;
                    editor.error = None;
                    AppEvent::TabActivated(tab_id)
                } else {
                    self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"))
                }
            }
            AppCommand::CancelQueryExecution(tab_id) => {
                // 没有登记标志（该标签当前没有在执行的查询）时不报错，只如实记录，避免
                // 「停止」按钮在竞态窗口内点两次就弹错误。
                let had_running_query = self.request_query_cancel(tab_id);
                tracing::info!(
                    target: "fluxdb_app",
                    tab_id = tab_id.0,
                    had_running_query,
                    "已请求取消查询执行"
                );
                AppEvent::QueryCancelRequested(tab_id)
            }
            AppCommand::ExecuteQuery(tab_id) => {
                let options = self.default_query_execution_options();
                let request = self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                    TabKind::QueryEditor(editor) => Some(QueryRequest {
                        connection_id: editor.connection_id,
                        database: editor.database.clone(),
                        // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                        session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                        schema: editor.schema.clone(),
                        text: sql_text_for_execution(
                            &editor.text,
                            options.page_size,
                            self.connection_kind(editor.connection_id)
                                .unwrap_or(DatabaseKind::MySql),
                        ),
                        mode: fluxdb_core::QueryMode::All,
                        options,
                    }),
                    _ => None,
                });

                let Some(request) = request else {
                    return self.fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                };

                let cancel_flag = self.query_cancel_flag(tab_id);
                let result = self
                    .execute_query_with_cancel(&request, &|| query_cancel_requested(&cancel_flag));
                // 取消标志只属于本次执行。执行器已经返回后立即回收，不能等 UI 的异步
                // Finish 命令，否则直接调用 App 层的入口会遗留已置位标志。
                self.clear_query_cancel_flag(tab_id);
                match result {
                    Ok(execution) => {
                        let result_editors = query_result_editors(self, &request, &execution);
                        let active_result_editor = result_editors.keys().next().copied();
                        if let Some(tab) = self.find_tab_mut(tab_id)
                            && let TabKind::QueryEditor(editor) = &mut tab.kind
                        {
                            editor.running = false;
                            editor.results = execution.results.clone();
                            editor.result_editors = result_editors;
                            editor.active_result_editor = active_result_editor;
                            editor.summaries = execution.summaries.clone();
                            editor.error = None;
                            tab.dirty = false;
                        }
                        self.record_query_execution_history(&request, &execution);
                        AppEvent::QueryFinished(tab_id, execution)
                    }
                    Err(error) => {
                        self.record_failed_query_history(&request);
                        let user_error = UserFacingError::from(error);
                        if let Some(tab) = self.find_tab_mut(tab_id)
                            && let TabKind::QueryEditor(editor) = &mut tab.kind
                        {
                            editor.running = false;
                            editor.summaries = Vec::new();
                            editor.result_editors.clear();
                            editor.active_result_editor = None;
                            editor.error = Some(user_error.clone());
                        }
                        self.state.last_error = Some(user_error.clone());
                        AppEvent::Failed(user_error)
                    }
                }
            }
            AppCommand::ExecuteQueryText { tab_id, text } => {
                let options = self.default_query_execution_options();
                self.execute_query_text_with_options(tab_id, text, options)
            }
            AppCommand::ExecuteQueryTextWithOptions {
                tab_id,
                text,
                options,
            } => self.execute_query_text_with_options(tab_id, text, options),
            AppCommand::FinishQueryExecution { tab_id, result } => match result {
                Ok(execution) => {
                    self.clear_query_cancel_flag(tab_id);
                    let history_request = self.find_tab(tab_id).and_then(|tab| {
                        let TabKind::QueryEditor(editor) = &tab.kind else {
                            return None;
                        };
                        Some(QueryRequest {
                            connection_id: editor.connection_id,
                            database: editor.database.clone(),
                            // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                            session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                            schema: editor.schema.clone(),
                            text: editor.text.clone(),
                            mode: fluxdb_core::QueryMode::All,
                            options: QueryExecutionOptions::default(),
                        })
                    });
                    let result_editor = history_request
                        .as_ref()
                        .map(|request| query_result_editors(self, request, &execution))
                        .unwrap_or_default();
                    let active_result_editor = result_editor.keys().next().copied();
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        editor.running = false;
                        editor.results = execution.results.clone();
                        editor.result_editors = result_editor;
                        editor.active_result_editor = active_result_editor;
                        editor.summaries = execution.summaries.clone();
                        editor.error = None;
                        tab.dirty = false;
                    } else {
                        return self
                            .fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                    }
                    if let Some(request) = history_request {
                        self.record_query_execution_history(&request, &execution);
                    }
                    AppEvent::QueryFinished(tab_id, execution)
                }
                Err(error) => {
                    self.clear_query_cancel_flag(tab_id);
                    let history_request = self.find_tab(tab_id).and_then(|tab| {
                        let TabKind::QueryEditor(editor) = &tab.kind else {
                            return None;
                        };
                        Some(QueryRequest {
                            connection_id: editor.connection_id,
                            database: editor.database.clone(),
                            // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                            session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                            schema: editor.schema.clone(),
                            text: sql_text_for_execution(
                                &editor.text,
                                Pagination::DEFAULT_LIMIT,
                                self.connection_kind(editor.connection_id)
                                    .unwrap_or(DatabaseKind::MySql),
                            ),
                            mode: fluxdb_core::QueryMode::All,
                            options: QueryExecutionOptions::default(),
                        })
                    });
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        editor.running = false;
                        editor.summaries = Vec::new();
                        editor.result_editors.clear();
                        editor.active_result_editor = None;
                        editor.error = Some(error.clone());
                    }
                    if let Some(request) = history_request {
                        self.record_failed_query_history(&request);
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::FinishQueryResultPageRefresh {
                tab_id,
                result_index,
                page_index,
                result,
            } => match result {
                Ok(execution) => {
                    let Some(page) = execution.results.first().cloned() else {
                        return self
                            .fail(Error::new(ErrorKind::Internal, "刷新结果页没有返回结果表"));
                    };
                    let Some(summary) = first_successful_result_summary(&execution).cloned() else {
                        return self.fail(Error::new(
                            ErrorKind::Internal,
                            "刷新结果页没有返回结果摘要",
                        ));
                    };
                    let Some((connection_id, database, schema)) =
                        self.find_tab(tab_id).and_then(|tab| match &tab.kind {
                            TabKind::QueryEditor(editor) => Some((
                                editor.connection_id,
                                editor.database.clone(),
                                editor.schema.clone(),
                            )),
                            _ => None,
                        })
                    else {
                        return self
                            .fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                    };
                    let request = QueryRequest {
                        connection_id,
                        database,
                        schema,
                        // 查询编辑器标签 = 一个独占 PG 会话（设计 §3.3）：事务/临时表/SET 跨多次执行保持。
                        session_id: Some(fluxdb_core::QuerySessionId(tab_id.0)),
                        text: summary.sql.clone(),
                        mode: fluxdb_core::QueryMode::Selection,
                        options: QueryExecutionOptions::default(),
                    };
                    let mut refreshed_result_editor =
                        query_result_editors(self, &request, &execution).remove(&0);
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        if page_index >= editor.results.len() {
                            return self.fail(Error::new(ErrorKind::Internal, "结果页不存在"));
                        }
                        editor.running = false;
                        editor.results[page_index] = page.clone();
                        if let Some(result_editor) = refreshed_result_editor.as_mut() {
                            result_editor.cell_detail_panel = editor
                                .result_editors
                                .get(&page_index)
                                .map(|editor| editor.cell_detail_panel.clone())
                                .unwrap_or_default();
                        }
                        if let Some(result_editor) = refreshed_result_editor {
                            editor.result_editors.insert(page_index, result_editor);
                            editor.active_result_editor = Some(page_index);
                        } else {
                            editor.result_editors.remove(&page_index);
                            if editor.active_result_editor == Some(page_index) {
                                editor.active_result_editor =
                                    editor.result_editors.keys().next().copied();
                            }
                        }
                        if let Some(summary_index) =
                            query_result_summary_index(&editor.summaries, result_index)
                        {
                            editor.summaries[summary_index] = summary;
                        }
                        editor.error = None;
                        tab.dirty = false;
                    } else {
                        return self
                            .fail(Error::new(ErrorKind::Internal, "查询编辑器标签页不存在"));
                    }
                    AppEvent::QueryResultPageRefreshed {
                        tab_id,
                        result_index,
                        page_index,
                        page,
                    }
                }
                Err(error) => {
                    if let Some(tab) = self.find_tab_mut(tab_id)
                        && let TabKind::QueryEditor(editor) = &mut tab.kind
                    {
                        editor.running = false;
                        editor.error = Some(error.clone());
                    }
                    self.state.last_error = Some(error.clone());
                    AppEvent::Failed(error)
                }
            },
            AppCommand::ActivateQueryResultEditor { tab_id, page_index } => {
                if let Some(tab) = self.find_tab_mut(tab_id)
                    && let TabKind::QueryEditor(editor) = &mut tab.kind
                {
                    set_active_query_result_editor(editor, page_index);
                }
                AppEvent::TabActivated(tab_id)
            }
            AppCommand::LoadUserAdminUsers(..)
            | AppCommand::StartUserAdminUsersLoad(..)
            | AppCommand::FinishUserAdminUsersLoad { .. }
            | AppCommand::LoadUserAdminGrants { .. }
            | AppCommand::LoadUserAdminMemberGrants { .. }
            | AppCommand::StartUserAdminGrantsLoad { .. }
            | AppCommand::FinishUserAdminGrantsLoad { .. }
            | AppCommand::StartUserAdminMemberGrantsLoad { .. }
            | AppCommand::FinishUserAdminMemberGrantsLoad { .. }
            | AppCommand::SelectUserAdminUser { .. }
            | AppCommand::BeginUserAdminCreateUser(..)
            | AppCommand::EndUserAdminCreateUser(..)
            | AppCommand::DiscardUserAdminEdits(..)
            | AppCommand::BeginUserAdminDeleteUser(..)
            | AppCommand::CancelUserAdminDeleteUser(..)
            | AppCommand::SelectUserAdminDetailTab { .. }
            | AppCommand::SetUserAdminSearch { .. }
            | AppCommand::SetUserAdminPrivilegeDatabase { .. }
            | AppCommand::SetUserAdminPrivilegeTable { .. }
            | AppCommand::ToggleUserAdminPrivilege { .. }
            | AppCommand::SetUserAdminGrantOption { .. }
            | AppCommand::AddUserAdminPrivilegeRow { .. }
            | AppCommand::SetUserAdminPrivilegeRowDatabase { .. }
            | AppCommand::ToggleUserAdminPrivilegeRowPrivilege { .. }
            | AppCommand::SetUserAdminPrivilegeRowGrantOption { .. }
            | AppCommand::SetUserAdminCreateUser { .. }
            | AppCommand::SetUserAdminCreateHost { .. }
            | AppCommand::SetUserAdminAuthPlugin { .. }
            | AppCommand::SetUserAdminPasswordExpiryPolicy { .. }
            | AppCommand::SetUserAdminCreatePassword { .. }
            | AppCommand::SetUserAdminNewPassword { .. }
            | AppCommand::SetUserAdminMaxQueriesPerHour { .. }
            | AppCommand::SetUserAdminMaxUpdatesPerHour { .. }
            | AppCommand::SetUserAdminMaxConnectionsPerHour { .. }
            | AppCommand::SetUserAdminMaxUserConnections { .. }
            | AppCommand::SetUserAdminSslType { .. }
            | AppCommand::SetUserAdminSslCipher { .. }
            | AppCommand::SetUserAdminSslIssuer { .. }
            | AppCommand::SetUserAdminSslSubject { .. }
            | AppCommand::SetUserAdminRoleMembershipGranted { .. }
            | AppCommand::SetUserAdminRoleMembershipDefault { .. }
            | AppCommand::SetUserAdminRoleMemberGranted { .. }
            | AppCommand::PreviewUserAdminSql { .. }
            | AppCommand::BeginUserAdminSqlConfirm(..)
            | AppCommand::ClearUserAdminPendingSql(..)
            | AppCommand::StartUserAdminSqlApply(..)
            | AppCommand::ApplyUserAdminSql { .. }
            | AppCommand::FinishUserAdminSqlApply { .. } => self.dispatch_user_admin_command(command),
            AppCommand::LoadSqlServerSecurityDatabases(..)
            | AppCommand::FinishSqlServerSecurityDatabases { .. }
            | AppCommand::SelectSqlServerSecurityDatabase { .. }
            | AppCommand::StartSqlServerSecurityLoad(..)
            | AppCommand::LoadSqlServerSecurity(..)
            | AppCommand::FinishSqlServerSecurityLoad { .. }
            | AppCommand::StartSqlServerPermissionsLoad { .. }
            | AppCommand::LoadSqlServerPermissions { .. }
            | AppCommand::FinishSqlServerPermissionsLoad { .. }
            | AppCommand::SelectSqlServerPrincipal { .. }
            | AppCommand::SetSqlServerSecurityFilter { .. }
            | AppCommand::OpenSqlServerSecurityForm { .. }
            | AppCommand::CloseSqlServerPermissionComposer(..)
            | AppCommand::SetSqlServerSecurityFormField { .. }
            | AppCommand::SetSqlServerSecurityFormFlag { .. }
            | AppCommand::DiscardSqlServerSecurityChanges(..)
            | AppCommand::SubmitSqlServerSecurityForm(..)
            | AppCommand::ToggleSqlServerRoleMembership { .. }
            | AppCommand::SetSqlServerSelectedLoginEnabled { .. }
            | AppCommand::RevokeSqlServerPermission { .. }
            | AppCommand::StartSqlServerSecurityPreview(..)
            | AppCommand::LoadSqlServerSecurityPreview(..)
            | AppCommand::FinishSqlServerSecurityPreview { .. }
            | AppCommand::StartSqlServerSecurityApply(..)
            | AppCommand::ApplySqlServerSecurityPlan(..)
            | AppCommand::FinishSqlServerSecurityApply { .. }
            | AppCommand::BeginDeleteSqlServerPrincipal(..)
            | AppCommand::ConfirmDeleteSqlServerPrincipal(..)
            | AppCommand::CancelDeleteSqlServerPrincipal(..) => self.dispatch_sqlserver_user_admin_command(command),
            AppCommand::StartOracleUsersLoad(..)
            | AppCommand::LoadOracleUsers(..)
            | AppCommand::FinishOracleUsersLoad { .. }
            | AppCommand::SelectOracleUser { .. }
            | AppCommand::SetOracleGrantDraftField { .. }
            | AppCommand::SetOracleGrantDraftOption { .. }
            | AppCommand::QueueOracleGrantDraft { .. }
            | AppCommand::RemoveOraclePendingGrant { .. }
            | AppCommand::RevokeOracleRole { .. }
            | AppCommand::RevokeOracleSystemPrivilege { .. }
            | AppCommand::RevokeOracleObjectPrivilege { .. }
            | AppCommand::BeginOracleCreateUser(..)
            | AppCommand::CancelOracleCreateUser(..)
            | AppCommand::SetOracleCreateDraftField { .. }
            | AppCommand::SetOracleCreateDraftFlag { .. }
            | AppCommand::SetOracleUserPassword { .. }
            | AppCommand::SetOracleUserLocked { .. }
            | AppCommand::DiscardOracleUserChanges(..)
            | AppCommand::PreviewOracleApply(..)
            | AppCommand::StartOracleApply(..)
            | AppCommand::ApplyOraclePlan(..)
            | AppCommand::FinishOracleApply { .. }
            | AppCommand::BeginDeleteOracleUser(..)
            | AppCommand::CancelDeleteOracleUser(..)
            | AppCommand::SetOracleDeleteCascade { .. }
            | AppCommand::ConfirmDeleteOracleUser(..) => self.dispatch_oracle_user_admin_command(command),
            AppCommand::ReplaceQueryHistory(history) => {
                self.state.query_history = history;
                AppEvent::SettingsSaved
            }
            command @ (AppCommand::SaveAiModel(_)
            | AppCommand::DeleteAiModel { .. }
            | AppCommand::CompleteAiModelDelete { .. }
            | AppCommand::DeleteAiAgent { .. }
            | AppCommand::CompleteAiAgentDelete { .. }
            | AppCommand::SaveAiAgent(_)
            | AppCommand::SaveAiMcpServerProfile(_)
            | AppCommand::SaveExecutionPolicy { .. }
            | AppCommand::EnsureMcpControl
            | AppCommand::SaveAiMcpDatasource(_)
            | AppCommand::DeleteAiMcpDatasource { .. }
            | AppCommand::RefreshAiSettings
            | AppCommand::LoadMoreAiSessions
            | AppCommand::RenameAiSession { .. }
            | AppCommand::ArchiveAiSession { .. }
            | AppCommand::RestoreAiSession { .. }
            | AppCommand::PurgeAiSession { .. }
            | AppCommand::CompleteAiSessionPurge { .. }
            | AppCommand::CompleteAiSessionRename { .. }
            | AppCommand::CompleteAiSessionStatus { .. }
            | AppCommand::SetAiModelEnabled { .. }
            | AppCommand::SetAiPluginInstanceEnabled { .. }
            | AppCommand::SaveAiPluginConfig { .. }
            | AppCommand::SetAiToolEnabled { .. }
            | AppCommand::SetAiDefaultModel { .. }
            | AppCommand::TestAiModel { .. }
            | AppCommand::FetchAiModels { .. }) => self.dispatch_ai_settings(command),
            command @ (AppCommand::SyncMcpClients { .. }
            | AppCommand::RefreshMcpClients { .. }
            | AppCommand::SetMcpClientRegistration { .. }) => self.dispatch_mcp_clients(command),
            AppCommand::ApplyConfirmedConfig {
                proposal,
                binary_path,
            } => match self.apply_confirmed_config(&proposal, &binary_path) {
                Ok(outcome) => AppEvent::ConfigProposalApplied(outcome),
                Err(error) => AppEvent::AiSettingsError(error.message),
            },
            AppCommand::SaveSettings(settings) => {
                self.state.settings = settings;
                AppEvent::SettingsSaved
            }
            AppCommand::ClearCompletionIndexCache => {
                self.clear_completion_cache();
                if let Some(storage) = &self.completion_index_storage {
                    let _ = storage.clear_completion_indexes();
                }
                AppEvent::SettingsSaved
            }
            // 建表/设计表/表操作命令经域路由转发至 dispatch_table_command，
            // 避免在此巨型 match 中持续膨胀（未在此处显式匹配的其余命令兜底转发）。
            _ => self.dispatch_table_command(command),
        }
    }
}

impl AppController {
    /// 释放与这些标签页绑定的独占查询会话（关闭标签时调用）。
    ///
    /// 标签的查询会话 id 就是 `tab_id`（见各 `QueryRequest` 构造点）；标签关闭后连接由
    /// 服务端回收，未提交事务随之回滚——不这样做，连接会一直挂到空闲 TTL 才消失。
    fn release_tab_query_sessions(&mut self, tab_ids: &[TabId]) {
        // 关标签先置位取消标志（标签关了就不该再让服务端跑下去），再从表里移除；
        // 仍在收尾的执行线程持的是自己的 `Arc`，置位照常生效。
        if let Ok(mut flags) = self.query_cancel_flags.lock() {
            for tab_id in tab_ids {
                if let Some(flag) = flags.remove(tab_id) {
                    flag.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        // 标签关闭后它结果页里外置的大字段内容没有消费者：按归属整批释放。
        // 查询结果按标签归属，表浏览按「连接 + 表」归属（同表翻页共用一份）。
        if let Some(store) = fluxdb_core::cell_content_store() {
            for tab in &self.state.tabs {
                if !tab_ids.contains(&tab.id) {
                    continue;
                }
                match &tab.kind {
                    TabKind::QueryEditor(editor) => store.release_owner(
                        &fluxdb_core::result_content_owner(
                            Some(fluxdb_core::QuerySessionId(tab.id.0)),
                            editor.connection_id,
                        ),
                    ),
                    // 同一张表可能同时开着多个标签，共用一个归属：还有别的标签在用就不释放。
                    TabKind::DataEditor(editor)
                        if !self.state.tabs.iter().any(|other| {
                            !tab_ids.contains(&other.id)
                                && matches!(&other.kind,
                                    TabKind::DataEditor(other_editor)
                                        if other_editor.object == editor.object)
                        }) =>
                    {
                        store.release_owner(&fluxdb_core::table_content_owner(&editor.object))
                    }
                    _ => {}
                }
            }
        }
        for tab in &self.state.tabs {
            if !tab_ids.contains(&tab.id) {
                continue;
            }
            if let TabKind::QueryEditor(editor) = &tab.kind {
                fluxdb_connectors::pg_close_query_session(
                    editor.connection_id,
                    fluxdb_core::QuerySessionId(tab.id.0),
                );
                fluxdb_connectors::sqlserver_close_query_session(
                    editor.connection_id,
                    fluxdb_core::QuerySessionId(tab.id.0),
                );
                settle_closed_query_history(
                    &mut self.state.query_history,
                    editor.connection_id,
                    Some(fluxdb_core::QuerySessionId(tab.id.0)),
                );
            }
        }
    }

    fn default_query_execution_options(&self) -> QueryExecutionOptions {
        QueryExecutionOptions {
            // 直接透传 page_size：0 表示「不限制」，不能经 Pagination::new 的
            // clamp(1, MAX) 被改成 1。
            page_size: self.state.settings.page_size,
            ..QueryExecutionOptions::default()
        }
    }

    /// 解析本次 Redis Workbench 要执行的命令文本：给了 `execution_id` 取结果区那条记录，
    /// 否则取顶部草稿；标签页不存在或不是 Workbench 时返回 None。
    fn redis_workbench_text(&self, tab_id: TabId, execution_id: Option<u64>) -> Option<String> {
        let tab = self.find_tab(tab_id)?;
        let TabKind::RedisWorkbench(workbench) = &tab.kind else {
            return None;
        };
        match execution_id {
            Some(execution_id) => workbench
                .executions
                .iter()
                .find(|execution| execution.id == execution_id)
                .map(|execution| execution.text.clone()),
            None => Some(workbench.text.clone()),
        }
    }

    /// 解析一次执行的目标作用域与命令文本。
    ///
    /// 文本由本函数按 `execution_id` 解析、不接 UI 传值：`RunRedisWorkbench` 跑在后台线程的
    /// 控制器副本上，而 UI 在派发前已把草稿清空，只有副本自己知道本次该执行什么。
    fn redis_workbench_run_scope(
        &self,
        tab_id: TabId,
        execution_id: Option<u64>,
    ) -> Option<(CommandExecutionTarget, String)> {
        let tab = self.find_tab(tab_id)?;
        let target = match &tab.kind {
            TabKind::RedisWorkbench(workbench) => CommandExecutionTarget::Redis {
                connection_id: workbench.connection_id,
                database: workbench.database,
            },
            _ => return None,
        };
        let text = self.redis_workbench_text(tab_id, execution_id)?;
        Some((target, text))
    }

    fn apply_create_table(&self, tab_id: TabId) -> fluxdb_core::Result<()> {
        let create = self
            .find_tab(tab_id)
            .and_then(|tab| match &tab.kind {
                TabKind::CreateTable(create) => Some(create.clone()),
                _ => None,
            })
            .ok_or_else(|| Error::new(ErrorKind::Internal, "新建表标签页不存在"))?;
        if let Some(message) = create.validation_error() {
            return Err(Error::new(ErrorKind::Query, message));
        }
        // 达梦不支持 `BEGIN; … COMMIT;` 这种 SQL 事务包装；设计表按批次直接执行，
        // 保存前仍做外部 DDL 一致性检查，避免用旧快照覆盖别人的改动。
        if create.database_kind == DatabaseKind::Dameng && create.is_design() {
            self.ensure_design_not_stale(&create)?;
            let sql = create
                .sql_preview()
                .map_err(|message| Error::new(ErrorKind::Query, message))?;
            let request = QueryRequest {
                connection_id: create.connection_id,
                database: create.database.clone(),
                session_id: None,
                schema: create
                    .schema
                    .trim()
                    .is_empty()
                    .then_some(None)
                    .unwrap_or_else(|| Some(create.schema.trim().to_string())),
                text: sql,
                mode: fluxdb_core::QueryMode::All,
                options: QueryExecutionOptions {
                    continue_on_error: false,
                    split_statements: true,
                    ..QueryExecutionOptions::default()
                },
            };
            let execution = self.execute_query(&request)?;
            if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                return Err(Error::new(ErrorKind::Query, summary.message.clone()));
            }
            self.mark_query_history_completion_dirty(&request, &request.text);
            return Ok(());
        }
        // PG 的 CREATE TABLE / ALTER 计划是同批事务性语句：走单批路径（与设计模式一致），
        // 不套用 MySQL/SQLite 的「建表 SQL + 单独触发器」拆分（那会丢掉 PG 触发器）。
        if create.is_design()
            || matches!(
                create.database_kind,
                DatabaseKind::Postgres | DatabaseKind::SqlServer
            )
        {
            if matches!(
                create.database_kind,
                DatabaseKind::Postgres | DatabaseKind::SqlServer
            ) {
                self.ensure_design_not_stale(&create)?;
            }
            let sql = create
                .sql_preview()
                .map_err(|message| Error::new(ErrorKind::Query, message))?;
            return self.apply_postgres_or_design_sql(&create, &sql);
        }
        let mut base_create = create.clone();
        base_create.triggers.clear();
        let base_sql = base_create
            .sql_preview()
            .map_err(|message| Error::new(ErrorKind::Query, message))?;
        let mut statements = vec![(base_sql, true)];
        let trigger_statements = match create.database_kind {
            DatabaseKind::MySql | DatabaseKind::TiDb => {
                create_table_mysql_trigger_statements(&create)
            }
            DatabaseKind::Sqlite => create_table_sqlite_trigger_statements(&create),
            _ => Ok(Vec::new()),
        }
        .map_err(|message| Error::new(ErrorKind::Query, message))?;
        statements.extend(trigger_statements.into_iter().map(|sql| (sql, false)));

        for (sql, split_statements) in statements {
            let request = QueryRequest {
                connection_id: create.connection_id,
                database: create.database.clone(),
                session_id: None,
                // 建表向导的 schema 作用域随状态下传（PG 显式 schema 时与会话 search_path 对齐）。
                schema: create
                    .schema
                    .trim()
                    .is_empty()
                    .then_some(None)
                    .unwrap_or_else(|| Some(create.schema.trim().to_string())),
                text: sql,
                mode: fluxdb_core::QueryMode::All,
                options: QueryExecutionOptions {
                    continue_on_error: false,
                    split_statements,
                    ..QueryExecutionOptions::default()
                },
            };
            let execution = self.execute_query(&request)?;
            if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
                return Err(Error::new(ErrorKind::Query, summary.message.clone()));
            }
            self.mark_query_history_completion_dirty(&request, &request.text);
        }
        Ok(())
    }

    /// 执行 PG 建表/设计 SQL：同一事务内提交，失败整体回滚（§9.2）。
    ///
    /// 生成的语句都是事务性 DDL（不含 CREATE INDEX CONCURRENTLY 等），因此统一包裹
    /// `BEGIN … COMMIT`；任一条失败时不执行 COMMIT，连接释放即回滚，不会留下半套结构。
    fn apply_postgres_or_design_sql(
        &self,
        create: &CreateTableState,
        sql: &str,
    ) -> fluxdb_core::Result<()> {
        let text = format!(
            "BEGIN;
{sql}
COMMIT;"
        );
        let request = QueryRequest {
            connection_id: create.connection_id,
            database: create.database.clone(),
            session_id: None,
            schema: create
                .schema
                .trim()
                .is_empty()
                .then_some(None)
                .unwrap_or_else(|| Some(create.schema.trim().to_string())),
            text,
            mode: fluxdb_core::QueryMode::All,
            options: QueryExecutionOptions {
                continue_on_error: false,
                split_statements: true,
                ..QueryExecutionOptions::default()
            },
        };
        let execution = self.execute_query(&request)?;
        if let Some(summary) = execution.summaries.iter().find(|summary| !summary.success) {
            return Err(Error::new(ErrorKind::Query, summary.message.clone()));
        }
        self.mark_query_history_completion_dirty(&request, &request.text);
        Ok(())
    }

    /// 外部 DDL 保护：保存前重查表结构，与打开设计器时的 DDL 不一致就拒绝应用（§9.2）。
    ///
    /// 不拿过期快照覆盖别人的改动；用户刷新后重新预览即可继续。
    fn ensure_design_not_stale(&self, create: &CreateTableState) -> fluxdb_core::Result<()> {
        let CreateTableMode::Design {
            object,
            original_ddl: Some(original_ddl),
            ..
        } = &create.mode
        else {
            return Ok(());
        };
        let config = self
            .connection_config(create.connection_id)
            .ok_or_else(|| Error::new(ErrorKind::Connection, "连接不存在"))?;
        let current = table_ddl_for_connection(config, object)?;
        // 打开设计器时基线经 format_sql_text_for_dialect 规整（load_table_info_for_connection 的 Ddl 路径）。
        // 校验侧必须用同一步规整：否则原始 DDL 与规整 DDL 对同一表逐字不等，会把「自己保存的改动」误判为外部变化。
        let current = format_sql_text_for_dialect(&current, config.kind);
        if current.trim() != original_ddl.trim() {
            tracing::warn!(
                target: "gdb_create_table",
                connection_id = ?create.connection_id,
                table = %object.name,
                "表结构已在外部变化，拒绝应用过期设计"
            );
            return Err(Error::new(
                ErrorKind::Query,
                "表结构已在外部变化，请重新打开设计器并确认预览后再保存",
            ));
        }
        Ok(())
    }
}

/// 查询取消判定：标志未登记（非编辑器执行/后台任务）视为不可取消，与旧行为一致。
fn query_cancel_requested(flag: &Option<Arc<std::sync::atomic::AtomicBool>>) -> bool {
    flag.as_ref()
        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
}

fn query_result_summary_has_result_tab(summary: &QueryExecutionSummary) -> bool {
    summary.kind == fluxdb_core::QueryStatementKind::ResultSet || !summary.success
}

fn query_result_summary_index(
    summaries: &[QueryExecutionSummary],
    result_index: usize,
) -> Option<usize> {
    let mut entry_index = 0usize;
    for (summary_index, summary) in summaries.iter().enumerate() {
        if !query_result_summary_has_result_tab(summary) {
            continue;
        }
        if entry_index == result_index {
            return Some(summary_index);
        }
        entry_index += 1;
    }
    None
}

fn first_successful_result_summary(
    execution: &QueryExecutionResult,
) -> Option<&QueryExecutionSummary> {
    execution.summaries.iter().find(|summary| {
        summary.kind == fluxdb_core::QueryStatementKind::ResultSet && summary.success
    })
}
