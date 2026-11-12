// AI 管理命令独立职责；从既有 dispatch 结构拆出，公共状态仍通过 AppEvent 表达。
impl AppController {
    fn dispatch_ai_settings(&mut self, command: AppCommand) -> AppEvent {
        match command {
            AppCommand::SaveExecutionPolicy {settings,expected_revision} => {
                let result=self.ai_storage().and_then(|st| {
                    with_ai_storage(&st,|conn| {
                        fluxdb_storage::ai_platform::execution_policy::save(conn,ai_chat_now(),&settings,expected_revision)
                    }).map(|rev| {
                        self.state.ai_execution_policy=settings;
                        self.state.ai_runtime_revision=rev;
                    })
                });
                return match result {Ok(())=>AppEvent::AiSettingsLoaded,Err(e)=>AppEvent::AiSettingsError(e.message)};
            },
            AppCommand::EnsureMcpControl => {
                let result=self.ai_storage().and_then(|st| {
                    with_ai_storage(&st,|conn| {
                        use fluxdb_storage::ai_platform::mcp_server::*;
                        if load_server_profile(conn,crate::DEFAULT_MCP_PROFILE_ID)?.is_none() {
                            save_server_profile(conn,ai_chat_now(),&ServerProfileEdit {
                                id:crate::DEFAULT_MCP_PROFILE_ID.into(),display_name:"本地 MCP".into(),
                                enabled:false,
                                options_json:"{}".into(),
                            },None)?;
                        }
                        Ok(())
                    })?;
                    refresh_ai_settings(&mut self.state,&st)
                });
                return match result {Ok(())=>AppEvent::AiSettingsLoaded,Err(e)=>AppEvent::AiSettingsError(e.message)};
            },
            AppCommand::SaveAiMcpDatasource(input) => {
                let result = self.ai_storage().and_then(|st| {
                    save_ai_mcp_datasource(&mut self.state, &st, input, ai_chat_now())
                });
                return match result {
                    Ok((connection_id, revision)) => {
                        AppEvent::AiMcpDatasourceSaved { connection_id, revision }
                    }
                    Err(e) => AppEvent::AiSettingsError(e.message),
                };
            }
            AppCommand::DeleteAiMcpDatasource { profile_id, connection_id } => {
                let result = self.ai_storage().and_then(|st| {
                    delete_ai_mcp_datasource(&mut self.state, &st, &profile_id, connection_id)
                });
                return match result {
                    Ok(()) => AppEvent::AiMcpDatasourceDeleted { connection_id },
                    Err(e) => AppEvent::AiSettingsError(e.message),
                };
            }
            AppCommand::SaveAiModel(input) => {
                let storage = self.ai_storage();
                let result = storage.and_then(|st| save_ai_model(&mut self.state, &st, &input));
                match result {
                    Ok((id, revision)) => {
                        return AppEvent::AiModelSaved { id, revision };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::DeleteAiModel {
                id,
                expected_revision,
            } => {
                let result = self.ai_storage().and_then(|storage| {
                    with_ai_storage(&storage, |conn| {
                        models::delete_model(conn, &id, expected_revision)
                    })
                });
                match result {
                    Ok(()) => {
                        self.state.ai_models.retain(|model| model.id != id);
                        tracing::info!(model_id = %id, "AI 模型已删除");
                        return AppEvent::AiModelDeleted { id };
                    }
                    Err(error) => {
                        tracing::warn!(model_id = %id, reason = %error.message, "删除 AI 模型失败");
                        return AppEvent::AiSettingsError(error.message);
                    }
                }
            }
            AppCommand::CompleteAiModelDelete { id, result } => match result {
                Ok(()) => {
                    self.state.ai_models.retain(|model| model.id != id);
                    return AppEvent::AiModelDeleted { id };
                }
                Err(message) => return AppEvent::AiSettingsError(message),
            },
            AppCommand::CompleteAiAgentDelete {
                id,
                runtime_revision,
                result,
            } => {
                return match result {
                    Ok(()) => {
                        self.state.ai_agents.retain(|a| a.id != id);
                        self.state.ai_runtime_revision =
                            self.state.ai_runtime_revision.max(runtime_revision);
                        AppEvent::AiAgentDeleted { id }
                    }
                    Err(message) => AppEvent::AiSettingsError(message),
                };
            }
            AppCommand::DeleteAiAgent {
                id,
                expected_revision,
            } => {
                let result = self.ai_storage().and_then(|storage| {
                    with_ai_storage(&storage, |conn| {
                        fluxdb_storage::ai_platform::agents::delete_agent_with_runtime_revision(
                            conn,
                            &id,
                            expected_revision,
                        )
                    })
                });
                match result {
                    Ok(revision) => {
                        self.state.ai_agents.retain(|a| a.id != id);
                        self.state.ai_runtime_revision = revision;
                        return AppEvent::AiAgentDeleted { id };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::SaveAiAgent(input) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let storage = self.ai_storage();
                let result = storage.and_then(|st| save_ai_agent(&mut self.state, &st, input, now));
                match result {
                    Ok(id) => {
                        let revision = self
                            .state
                            .ai_agents
                            .iter()
                            .find(|a| a.id == id)
                            .map(|a| a.revision)
                            .unwrap_or(0);
                        return AppEvent::AiAgentSaved { id, revision };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::RefreshAiSettings => {
                let storage = self.ai_storage();
                let result = storage.and_then(|st| refresh_ai_settings(&mut self.state, &st));
                match result {
                    Ok(()) => return AppEvent::AiSettingsLoaded,
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::SaveAiMcpServerProfile(input) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let storage = self.ai_storage();
                let result =
                    storage.and_then(|st| save_ai_mcp_server_profile(&mut self.state, &st, input, now));
                match result {
                    Ok((id, revision)) => {
                        // 开关变化立即作用到应用内端点：开启则监听，关闭则停止并释放端口。
                        self.sync_mcp_http_endpoint();
                        return AppEvent::AiMcpServerProfileSaved { id, revision };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::LoadMoreAiSessions => {
                let storage = self.ai_storage();
                let result = storage.and_then(|st| load_more_ai_sessions(&mut self.state, &st));
                match result {
                    Ok(appended) => {
                        return AppEvent::AiSessionsAppended {
                            appended,
                            has_more: self.state.ai_sessions_has_more,
                        };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::RenameAiSession {
                id,
                title,
                expected_revision,
            } => {
                let title = title.trim().to_string();
                if title.is_empty() {
                    return AppEvent::AiSettingsError("会话名称不能为空".into());
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let result = self.ai_storage().and_then(|storage| {
                    with_ai_storage(&storage, |conn| {
                        fluxdb_storage::ai_platform::sessions::rename_session(
                            conn,
                            now,
                            &id,
                            &title,
                            expected_revision,
                        )
                    })
                });
                match result {
                    Ok(revision) => {
                        if let Some(item) = self.state.ai_sessions.iter_mut().find(|s| s.id == id) {
                            item.title = title.clone();
                            item.revision = revision;
                        }
                        return AppEvent::AiSessionRenamed {
                            id,
                            title,
                            revision,
                        };
                    }
                    Err(error) => return AppEvent::AiSettingsError(error.message),
                }
            }
            AppCommand::ArchiveAiSession {
                id,
                expected_revision,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let result = self.ai_storage().and_then(|storage| {
                    with_ai_storage(&storage, |conn| {
                        fluxdb_storage::ai_platform::sessions::archive_session(
                            conn,
                            now,
                            &id,
                            expected_revision,
                        )
                    })
                });
                match result {
                    Ok(revision) => {
                        if let Some(item) = self.state.ai_sessions.iter_mut().find(|s| s.id == id) {
                            item.status = "archived".into();
                            item.revision = revision;
                        }
                        return AppEvent::AiSessionArchived { id, revision };
                    }
                    Err(error) => return AppEvent::AiSettingsError(error.message),
                }
            }
            AppCommand::RestoreAiSession {
                id,
                expected_revision,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let result = self.ai_storage().and_then(|storage| {
                    with_ai_storage(&storage, |conn| {
                        fluxdb_storage::ai_platform::sessions::restore_session(
                            conn,
                            now,
                            &id,
                            expected_revision,
                        )
                    })
                });
                match result {
                    Ok(revision) => {
                        if let Some(item) = self.state.ai_sessions.iter_mut().find(|s| s.id == id) {
                            item.status = "active".into();
                            item.revision = revision;
                        }
                        return AppEvent::AiSessionRestored { id, revision };
                    }
                    Err(error) => return AppEvent::AiSettingsError(error.message),
                }
            }
            AppCommand::PurgeAiSession {
                id,
                expected_revision,
            } => {
                let result = self.ai_storage().and_then(|storage| {
                    // CAS 复核：归档后的 revision 必须与调用方一致，避免误删并发变化的会话。
                    let current = with_ai_storage(&storage, |conn| {
                        fluxdb_storage::ai_platform::sessions::load_session(conn, &id)
                    })?;
                    if current.revision != expected_revision {
                        return Err(fluxdb_core::Error::new(
                            fluxdb_core::ErrorKind::Internal,
                            "会话已被其他窗口修改，请重新加载",
                        ));
                    }
                    let plan = with_ai_storage(&storage, |conn| {
                        fluxdb_storage::ai_platform::sessions::purge_archived_session(conn, &id)
                    })?;
                    // 数据库删除已提交，文件清理失败不回滚；仅记录告警。
                    purge_session_files(&storage, &plan);
                    Ok(())
                });
                match result {
                    Ok(()) => {
                        self.state.ai_sessions.retain(|session| session.id != id);
                        self.state.ai_sessions_total = (self.state.ai_sessions_total - 1).max(0);
                        return AppEvent::AiSessionPurged { id };
                    }
                    Err(error) => return AppEvent::AiSettingsError(error.message),
                }
            }
            AppCommand::CompleteAiSessionPurge { id, result } => {
                return match result {
                    Ok(()) => {
                        self.state.ai_sessions.retain(|session| session.id != id);
                        AppEvent::AiSessionPurged { id }
                    }
                    Err(message) => AppEvent::AiSettingsError(message),
                };
            }
            AppCommand::CompleteAiSessionRename {
                id,
                title,
                revision,
                result,
            } => {
                return match result {
                    Ok(()) => {
                        if let Some(item) = self.state.ai_sessions.iter_mut().find(|s| s.id == id) {
                            item.title = title.clone();
                            item.revision = revision;
                        }
                        AppEvent::AiSessionRenamed {
                            id,
                            title,
                            revision,
                        }
                    }
                    Err(message) => AppEvent::AiSettingsError(message),
                };
            }
            AppCommand::CompleteAiSessionStatus {
                id,
                archived,
                revision,
                result,
            } => {
                return match result {
                    Ok(()) => {
                        if let Some(item) = self.state.ai_sessions.iter_mut().find(|s| s.id == id) {
                            item.status = if archived { "archived" } else { "active" }.into();
                            item.revision = revision;
                        }
                        if archived {
                            AppEvent::AiSessionArchived { id, revision }
                        } else {
                            AppEvent::AiSessionRestored { id, revision }
                        }
                    }
                    Err(message) => AppEvent::AiSettingsError(message),
                };
            }
            AppCommand::SetAiModelEnabled {
                id,
                enabled,
                expected_revision,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let storage = self.ai_storage();
                let result = storage.and_then(|st| {
                    set_ai_model_enabled(&mut self.state, &st, &id, enabled, expected_revision, now)
                });
                match result {
                    Ok(()) => {
                        return AppEvent::AiModelEnabled {
                            id,
                            enabled,
                            revision: expected_revision + 1,
                        };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::SetAiPluginInstanceEnabled {
                id,
                enabled,
                expected_revision,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let storage = self.ai_storage();
                let result = storage.and_then(|st| {
                    set_ai_plugin_enabled(
                        &mut self.state,
                        &st,
                        &id,
                        enabled,
                        expected_revision,
                        now,
                    )
                });
                match result {
                    Ok(()) => {
                        return AppEvent::AiPluginInstanceEnabled {
                            id,
                            enabled,
                            revision: expected_revision + 1,
                        };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::SaveAiPluginConfig {
                instance_id,
                selections,
                expected_revision,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let storage = self.ai_storage();
                let result = storage.and_then(|st| {
                    save_ai_plugin_config(
                        &mut self.state,
                        &st,
                        &instance_id,
                        &selections,
                        expected_revision,
                        now,
                    )
                });
                match result {
                    Ok(revision) => {
                        return AppEvent::AiPluginConfigSaved {
                            instance_id,
                            revision,
                        };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::SetAiToolEnabled {
                tool_id,
                enabled,
                expected_revision,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let storage = self.ai_storage();
                let result = storage.and_then(|st| {
                    set_ai_tool_enabled(
                        &mut self.state,
                        &st,
                        &tool_id,
                        enabled,
                        expected_revision,
                        now,
                    )
                });
                match result {
                    Ok(_) => {
                        tracing::info!(
                            tool_id = %tool_id,
                            enabled,
                            "标准工具全局启停已更新"
                        );
                        return AppEvent::AiToolEnabled {
                            tool_id,
                            enabled,
                            revision: expected_revision + 1,
                        };
                    }
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::SetAiDefaultModel {
                model_id,
                expected_revision,
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let storage = self.ai_storage();
                let result = storage.and_then(|st| {
                    set_ai_default_model(&mut self.state, &st, model_id, expected_revision, now)
                });
                match result {
                    Ok(revision) => return AppEvent::AiDefaultModelsSaved { revision },
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::TestAiModel { id } => {
                let storage = self.ai_storage();
                let result = storage.and_then(|st| test_ai_model(&self.state, &st, &id));
                match result {
                    Ok((id, status)) => return AppEvent::AiModelTested { id, status },
                    Err(e) => return AppEvent::AiSettingsError(e.message),
                }
            }
            AppCommand::FetchAiModels {
                provider,
                endpoint,
                endpoint_is_full_url,
                api_key,
            } => {
                if endpoint_is_full_url {
                    return AppEvent::AiModelsFetchFailed {
                        message: "完整 URL 模式不自动获取模型列表，请手工填写模型 ID".into(),
                    };
                }
                // 纯网络读取：不触碰 storage，api_key 仅用于本次请求头。
                match fetch_ai_model_capabilities(&provider, endpoint.as_deref(), &api_key) {
                    Ok(capabilities) => {
                        return AppEvent::AiModelsFetched {
                            models: capabilities.iter().map(|m| m.id.clone()).collect(),
                            capabilities,
                        };
                    }
                    Err(e) => return AppEvent::AiModelsFetchFailed { message: e.message },
                }
            }
            _ => unreachable!("仅 AI 管理命令进入此分发器"),
        }
    }
}
