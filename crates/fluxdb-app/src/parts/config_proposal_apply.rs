/// 确认后的应用结果；不含凭据。模型编辑仍交给宿主本地表单完成 Keep/Set/Clear。
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigApplyOutcome {
    Applied,
    Settings(Box<fluxdb_core::Settings>),
    ModelEditor {
        id: Option<String>,
        draft: Box<ModelCreateProposal>,
    },
}

fn config_error(code: &str) -> fluxdb_core::Error {
    fluxdb_core::Error::new(fluxdb_core::ErrorKind::Permission, code)
}

impl AppController {
    /// 仅由用户确认按钮在后台派发；不得由 AI 工具直接调用。
    fn apply_confirmed_config(
        &mut self,
        proposal: &ConfigProposal,
    ) -> fluxdb_core::Result<ConfigApplyOutcome> {
        let storage = self.ai_storage()?;
        let now = ai_chat_now();
        if now < proposal.created_at_ms || now.saturating_sub(proposal.created_at_ms) > 600_000 {
            return Err(config_error("config_proposal_expired"));
        }
        // 先登记开始事实；任何失败都不能靠重复点击重放写入。
        // 跨配置/外部文件无法共享事务，中断后保留 started，要求用户核实后重新提案。
        let mut conn = storage.open_ai_connection()?;
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| config_error(&e.to_string()))?;
        let prior = fluxdb_storage::ai_platform::sessions::list_run_events(&tx, &proposal.turn_id)?;
        if prior.iter().any(|event| {
            event.kind == "config_started"
                && serde_json::from_str::<serde_json::Value>(&event.summary_json)
                    .is_ok_and(|value| value["request_id"] == proposal.request_id)
        }) {
            return Err(config_error("config_proposal_already_applied"));
        }
        fluxdb_storage::ai_platform::run_events::append(
            &tx,
            &proposal.turn_id,
            "config_started",
            &serde_json::json!({"request_id":proposal.request_id}).to_string(),
            now,
        )?;
        tx.commit().map_err(|e| config_error(&e.to_string()))?;
        let outcome = self.execute_config_change(proposal);
        let status = match &outcome {
            Ok(ConfigApplyOutcome::ModelEditor { .. }) => "editing",
            Ok(_) => "applied",
            Err(_) => "failed",
        };
        // 只记已知状态，不把模型提示词、Key 或任意外部错误正文写入事件。
        storage.with_ai_connection(|conn| {
            fluxdb_storage::ai_platform::run_events::append(
                conn,
                &proposal.turn_id,
                "config_result",
                &serde_json::json!({"request_id":proposal.request_id,"status":status}).to_string(),
                ai_chat_now(),
            )
            .map(|_| ())
        })?;
        outcome
    }

    fn execute_config_change(
        &mut self,
        proposal: &ConfigProposal,
    ) -> fluxdb_core::Result<ConfigApplyOutcome> {
        let storage = self.ai_storage()?;
        let revision = || {
            i64::try_from(proposal.expected_revision)
                .map_err(|_| config_error("config_proposal_stale"))
        };
        let command = match &proposal.kind {
            ConfigProposalKind::SettingUpdate(prefill) => {
                let settings = storage.load_settings()?;
                if settings_fingerprint(&settings) != proposal.expected_revision {
                    return Err(config_error("config_proposal_stale"));
                }
                let target = settings_after_prefill(&settings, prefill)
                    .map_err(|_| config_error("config_proposal_invalid"))?;
                storage.save_settings(&target)?;
                self.dispatch(AppCommand::SaveSettings(target.clone()));
                return Ok(ConfigApplyOutcome::Settings(Box::new(target)));
            }
            ConfigProposalKind::ModelCreate(draft) => {
                validate_model_proposal(draft)?;
                return Ok(ConfigApplyOutcome::ModelEditor {
                    id: None,
                    draft: Box::new(draft.clone()),
                });
            }
            ConfigProposalKind::ModelUpdate { id, changes } => {
                let current = storage.with_ai_connection(|conn| models::load_model(conn, id))?;
                if current.revision != revision()? {
                    return Err(config_error("config_proposal_stale"));
                }
                let mut draft = model_proposal_from_view(&current);
                patch_model_proposal(&mut draft, changes)?;
                validate_model_proposal(&draft)?;
                return Ok(ConfigApplyOutcome::ModelEditor {
                    id: Some(id.clone()),
                    draft: Box::new(draft),
                });
            }
            ConfigProposalKind::ModelDelete { id } => AppCommand::DeleteAiModel {
                id: id.clone(),
                expected_revision: revision()?,
            },
            ConfigProposalKind::AgentDelete { id } => {
                ensure_other_agent(&storage, proposal, id)?;
                AppCommand::DeleteAiAgent {
                    id: id.clone(),
                    expected_revision: revision()?,
                }
            }
            ConfigProposalKind::ToolSetEnabled { tool_id, enabled } => {
                if !is_known_ai_tool(tool_id) {
                    return Err(config_error("config_proposal_invalid"));
                }
                AppCommand::SetAiToolEnabled {
                    tool_id: tool_id.clone(),
                    enabled: *enabled,
                    expected_revision: revision()?,
                }
            }
            ConfigProposalKind::AgentCreate(draft) => AppCommand::SaveAiAgent(AiAgentSaveInput {
                id: None,
                display_name: draft.display_name.clone(),
                description: draft.description.clone(),
                system_prompt: draft.system_prompt.clone(),
                model_profile_id: draft.model_profile_id.clone(),
                allowed_tools: draft.allowed_tools.clone(),
                enabled: draft.enabled,
                expected_revision: 0,
                compression_trigger_percent: draft.compression_trigger_percent,
                max_iterations: draft.max_iterations,
            }),
            ConfigProposalKind::AgentUpdate { id, changes } => {
                let current = storage.with_ai_connection(|conn| agents::load_agent(conn, id))?;
                if current.revision != revision()? {
                    return Err(config_error("config_proposal_stale"));
                }
                // 不允许提案改写自己的权限；另一个 Agent 的改动仍需完整确认。
                ensure_other_agent(&storage, proposal, id)?;
                let mut input = AiAgentSaveInput {
                    id: Some(id.clone()),
                    display_name: current.display_name,
                    description: current.description,
                    system_prompt: current.system_prompt,
                    model_profile_id: current.default_model_profile_id,
                    allowed_tools: current.allowed_tools,
                    enabled: current.enabled,
                    expected_revision: current.revision,
                    compression_trigger_percent: current.compression_trigger_percent,
                    max_iterations: current.max_iterations,
                };
                patch_agent_proposal(&mut input, changes)?;
                AppCommand::SaveAiAgent(input)
            }
            ConfigProposalKind::SessionArchive { id } => AppCommand::ArchiveAiSession {
                id: id.clone(),
                expected_revision: revision()?,
            },
            ConfigProposalKind::SessionPurge { id } => AppCommand::PurgeAiSession {
                id: id.clone(),
                expected_revision: revision()?,
            },
            ConfigProposalKind::McpServerSetEnabled { id, enabled } => {
                let current = storage
                    .with_ai_connection(|conn| {
                        fluxdb_storage::ai_platform::mcp_server::load_server_profile(conn, id)
                    })?
                    .ok_or_else(|| config_error("config_proposal_missing"))?;
                if current.revision != revision()? {
                    return Err(config_error("config_proposal_stale"));
                }
                AppCommand::SaveAiMcpServerProfile(AiMcpServerProfileSaveInput {
                    id: Some(id.clone()),
                    display_name: current.display_name,
                    enabled: *enabled,
                    expected_revision: Some(revision()?),
                })
            }
            ConfigProposalKind::McpSyncClient {
                client_key,
                registered,
            } => {
                if client_from_key(client_key).is_none() {
                    return Err(config_error("config_proposal_invalid"));
                }
                AppCommand::SetMcpClientRegistration {
                    client_key: client_key.clone(),
                    registered: *registered,
                }
            }
        };
        let event = self.dispatch(command);
        match event {
            AppEvent::AiSettingsError(message) => Err(fluxdb_core::Error::new(
                fluxdb_core::ErrorKind::Permission,
                message,
            )),
            AppEvent::AiModelDeleted { .. }
            | AppEvent::AiAgentDeleted { .. }
            | AppEvent::AiAgentSaved { .. }
            | AppEvent::AiToolEnabled { .. }
            | AppEvent::AiSessionArchived { .. }
            | AppEvent::AiSessionPurged { .. }
            | AppEvent::AiMcpServerProfileSaved { .. }
            | AppEvent::AiMcpClientsSynced { .. }
            | AppEvent::AiMcpClientsLoaded(_) => Ok(ConfigApplyOutcome::Applied),
            _ => Err(config_error("config_proposal_invalid")),
        }
    }
}

fn ensure_other_agent(
    storage: &fluxdb_storage::FileStorage,
    proposal: &ConfigProposal,
    id: &str,
) -> fluxdb_core::Result<()> {
    let session = storage.with_ai_connection(|conn| {
        let turn = fluxdb_storage::ai_platform::sessions::load_turn(conn, &proposal.turn_id)?;
        fluxdb_storage::ai_platform::sessions::load_session(conn, &turn.session_id)
    })?;
    if session.agent_id == id {
        return Err(config_error("config_proposal_self_agent"));
    }
    Ok(())
}

fn model_proposal_from_view(model: &models::ModelProfileView) -> ModelCreateProposal {
    ModelCreateProposal {
        display_name: model.display_name.clone(),
        provider: model.provider.clone(),
        model: model.model.clone(),
        endpoint: model.endpoint.clone(),
        endpoint_is_full_url: model.endpoint_is_full_url,
        enabled: model.enabled,
        egress: model.egress == models::EgressPermission::Allow,
        context_window_tokens: model.context_window_tokens,
        max_output_tokens: model.max_output_tokens,
        input_price_micros: model.input_price_micros_per_million,
        output_price_micros: model.output_price_micros_per_million,
        cached_input_price_micros: model.cached_input_price_micros_per_million,
    }
}

fn validate_model_proposal(draft: &ModelCreateProposal) -> fluxdb_core::Result<()> {
    if !ModelProvider::ALL
        .iter()
        .any(|p| p.stored_id() == draft.provider)
        || draft.display_name.trim().is_empty()
        || draft.model.trim().is_empty()
        || draft.context_window_tokens.is_some_and(|n| n <= 0)
        || draft.max_output_tokens.is_some_and(|n| n <= 0)
        || draft
            .context_window_tokens
            .zip(draft.max_output_tokens)
            .is_some_and(|(c, o)| o > c)
    {
        return Err(config_error("config_proposal_invalid"));
    }
    if let Some(endpoint) = &draft.endpoint {
        ai_endpoint_url(endpoint)?;
        // 带凭据的 URL 不得进入普通配置或确认卡。
        let url = url::Url::parse(endpoint).map_err(|_| config_error("config_proposal_invalid"))?;
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
            return Err(config_error("config_proposal_invalid"));
        }
    } else if draft.endpoint_is_full_url {
        return Err(config_error("config_proposal_invalid"));
    }
    Ok(())
}

fn patch_model_proposal(
    draft: &mut ModelCreateProposal,
    changes: &serde_json::Map<String, serde_json::Value>,
) -> fluxdb_core::Result<()> {
    for (key, value) in changes {
        match key.as_str() {
            "display_name" => draft.display_name = config_string(value)?,
            "provider" => draft.provider = config_string(value)?,
            "model" => draft.model = config_string(value)?,
            "endpoint" => draft.endpoint = Some(config_string(value)?),
            "endpoint_is_full_url" => draft.endpoint_is_full_url = config_bool(value)?,
            "enabled" => draft.enabled = config_bool(value)?,
            "egress" => draft.egress = config_bool(value)?,
            "context_window_tokens" => draft.context_window_tokens = Some(config_integer(value)?),
            "max_output_tokens" => draft.max_output_tokens = Some(config_integer(value)?),
            "input_price_micros" => draft.input_price_micros = Some(config_integer(value)?),
            "output_price_micros" => draft.output_price_micros = Some(config_integer(value)?),
            "cached_input_price_micros" => {
                draft.cached_input_price_micros = Some(config_integer(value)?)
            }
            _ => return Err(config_error("config_proposal_invalid")),
        }
    }
    Ok(())
}

fn patch_agent_proposal(
    draft: &mut AiAgentSaveInput,
    changes: &serde_json::Map<String, serde_json::Value>,
) -> fluxdb_core::Result<()> {
    for (key, value) in changes {
        match key.as_str() {
            "display_name" => draft.display_name = config_string(value)?,
            "description" => draft.description = config_string(value)?,
            "system_prompt" => draft.system_prompt = config_string(value)?,
            "model_profile_id" => draft.model_profile_id = Some(config_string(value)?),
            "enabled" => draft.enabled = config_bool(value)?,
            "compression_trigger_percent" => {
                draft.compression_trigger_percent = u8::try_from(config_integer(value)?)
                    .map_err(|_| config_error("config_proposal_invalid"))?;
            }
            "max_iterations" => draft.max_iterations = config_integer(value)?,
            "allowed_tools" => {
                draft.allowed_tools = value
                    .as_array()
                    .ok_or_else(|| config_error("config_proposal_invalid"))?
                    .iter()
                    .map(config_string)
                    .collect::<fluxdb_core::Result<Vec<_>>>()?;
                if draft.allowed_tools.iter().any(|id| !is_known_ai_tool(id)) {
                    return Err(config_error("config_proposal_invalid"));
                }
            }
            _ => return Err(config_error("config_proposal_invalid")),
        }
    }
    Ok(())
}

fn config_string(value: &serde_json::Value) -> fluxdb_core::Result<String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| config_error("config_proposal_invalid"))
}
fn config_bool(value: &serde_json::Value) -> fluxdb_core::Result<bool> {
    value
        .as_bool()
        .ok_or_else(|| config_error("config_proposal_invalid"))
}
fn config_integer(value: &serde_json::Value) -> fluxdb_core::Result<i64> {
    value
        .as_i64()
        .ok_or_else(|| config_error("config_proposal_invalid"))
}

#[cfg(test)]
mod config_proposal_apply_tests {
    use super::*;

    fn isolated_profile() -> (AppController, fluxdb_storage::FileStorage) {
        let storage = fluxdb_storage::FileStorage::new(
            std::env::temp_dir().join(format!("fluxdb-config-{}", uuid::Uuid::new_v4())),
        );
        storage
            .save_settings(&Settings::default())
            .expect("initial settings");
        storage
            .with_ai_connection(|conn| {
                fluxdb_storage::ai_platform::sessions::create_session(
                    conn,
                    ai_chat_now(),
                    "config-session",
                    agents::BUILTIN_AGENT_ID,
                    "Configuration test",
                )?;
                fluxdb_storage::ai_platform::sessions::start_turn(
                    conn,
                    ai_chat_now(),
                    "config-session",
                    "config-turn",
                    "{}",
                    "{}",
                )?;
                Ok(())
            })
            .expect("session");
        let mut controller = AppController::new();
        controller.set_ai_storage(storage.clone());
        (controller, storage)
    }

    fn proposal(kind: ConfigProposalKind, revision: u64) -> ConfigProposal {
        ConfigProposal {
            request_id: uuid::Uuid::new_v4().to_string(),
            turn_id: "config-turn".into(),
            expected_revision: revision,
            created_at_ms: ai_chat_now(),
            kind,
        }
    }

    #[test]
    fn confirmed_tool_change_is_stored_audited_and_not_replayed() {
        let (mut controller, storage) = isolated_profile();
        let agent_id = save_ai_agent(
            &mut controller.state,
            &storage,
            AiAgentSaveInput {
                display_name: "Restricted".into(),
                allowed_tools: vec!["toolbox.inspect".into()],
                ..Default::default()
            },
            ai_chat_now(),
        )
        .expect("restricted agent");
        let proposal = proposal(
            ConfigProposalKind::ToolSetEnabled {
                tool_id: "toolbox.inspect".into(),
                enabled: false,
            },
            0,
        );
        assert!(
            storage
                .with_ai_connection(|conn| tool_settings::load_tool_setting(conn, "toolbox.inspect"))
                .expect("before")
                .is_none()
        );
        let result = controller
            .apply_confirmed_config(&proposal)
            .expect("apply");
        assert_eq!(result, ConfigApplyOutcome::Applied);
        let saved = storage
            .with_ai_connection(|conn| tool_settings::load_tool_setting(conn, "toolbox.inspect"))
            .expect("stored")
            .expect("setting");
        assert!(!saved.enabled);
        assert_eq!(saved.revision, 1);
        let restricted = storage
            .with_ai_connection(|conn| agents::load_agent(conn, &agent_id))
            .expect("restricted agent remains restricted");
        assert_eq!(restricted.allowed_tools, ["toolbox.inspect"]);
        assert_eq!(restricted.revision, 1);
        assert_eq!(
            controller
                .apply_confirmed_config(&proposal)
                .expect_err("replay")
                .message,
            "config_proposal_already_applied"
        );
        let events = storage
            .with_ai_connection(|conn| {
                fluxdb_storage::ai_platform::sessions::list_run_events(conn, "config-turn")
            })
            .expect("events");
        assert!(
            events
                .iter()
                .any(|e| e.kind == "config_result" && e.summary_json.contains("applied"))
        );
        std::fs::remove_dir_all(storage.root_dir()).expect("remove isolated profile");
    }

    #[test]
    fn invalid_and_stale_changes_do_not_report_success_or_modify_settings() {
        let (mut controller, storage) = isolated_profile();
        let original = storage.load_settings().expect("settings");
        let prefill = parse_settings_prefill(&serde_json::json!({
            "section":"editor", "fields":{"editor_font_size":18}
        }))
        .expect("prefill");
        let stale = proposal(ConfigProposalKind::SettingUpdate(prefill), 0);
        assert_eq!(
            controller
                .apply_confirmed_config(&stale)
                .expect_err("stale")
                .message,
            "config_proposal_stale"
        );
        assert_eq!(storage.load_settings().expect("unchanged"), original);
        let unknown = proposal(
            ConfigProposalKind::ToolSetEnabled {
                tool_id: "not.a.tool".into(),
                enabled: false,
            },
            0,
        );
        assert_eq!(
            controller
                .apply_confirmed_config(&unknown)
                .expect_err("unknown")
                .message,
            "config_proposal_invalid"
        );
        assert!(
            storage
                .with_ai_connection(|conn| tool_settings::load_tool_setting(conn, "not.a.tool"))
                .expect("unknown absent")
                .is_none()
        );
        std::fs::remove_dir_all(storage.root_dir()).expect("remove isolated profile");
    }

    #[test]
    fn agent_patch_and_soft_delete_preserve_unspecified_values_and_session_history() {
        let (mut controller, storage) = isolated_profile();
        let id = save_ai_agent(
            &mut controller.state,
            &storage,
            AiAgentSaveInput {
                display_name: "Original".into(),
                description: "Keep description".into(),
                system_prompt: "Keep this prompt".into(),
                allowed_tools: vec!["toolbox.inspect".into()],
                enabled: false,
                ..Default::default()
            },
            ai_chat_now(),
        )
        .expect("agent");
        let update = proposal(
            ConfigProposalKind::AgentUpdate {
                id: id.clone(),
                changes: serde_json::json!({"display_name":"Renamed"})
                    .as_object()
                    .expect("map")
                    .clone(),
            },
            1,
        );
        controller
            .apply_confirmed_config(&update)
            .expect("update");
        let agent = storage
            .with_ai_connection(|conn| agents::load_agent(conn, &id))
            .expect("agent");
        assert_eq!(agent.display_name, "Renamed");
        assert_eq!(agent.description, "Keep description");
        assert_eq!(agent.system_prompt, "Keep this prompt");
        assert_eq!(agent.allowed_tools, ["toolbox.inspect"]);
        assert!(!agent.enabled);
        storage
            .with_ai_connection(|conn| {
                fluxdb_storage::ai_platform::sessions::create_session(
                    conn,
                    ai_chat_now(),
                    "referencing",
                    &id,
                    "Referenced",
                )
            })
            .expect("reference");
        let delete = proposal(ConfigProposalKind::AgentDelete { id: id.clone() }, 2);
        controller
            .apply_confirmed_config(&delete)
            .expect("soft delete");
        assert!(
            !storage
                .with_ai_connection(agents::list_agents)
                .expect("agents")
                .iter()
                .any(|agent| agent.id == id)
        );
        let session = storage
            .with_ai_connection(|conn| {
                fluxdb_storage::ai_platform::sessions::load_session(conn, "referencing")
            })
            .expect("preserved session");
        assert_eq!(session.agent_id, id);
        std::fs::remove_dir_all(storage.root_dir()).expect("remove isolated profile");
    }

    #[test]
    fn model_patch_opens_local_editor_without_writing_or_clearing_credential() {
        let (mut controller, storage) = isolated_profile();
        let event = controller.dispatch(AppCommand::SaveAiModel(AiModelSaveInput {
            display_name: "Original model".into(),
            provider: "openai".into(),
            model: "fixture-model".into(),
            endpoint: Some("https://models.invalid/v1".into()),
            api_key: Some("test-only-key".into()),
            context_window_tokens: Some(8192),
            max_output_tokens: Some(1024),
            ..Default::default()
        }));
        let AppEvent::AiModelSaved { id, revision } = event else {
            panic!("save model failed");
        };
        let update = proposal(
            ConfigProposalKind::ModelUpdate {
                id: id.clone(),
                changes: serde_json::json!({"display_name":"Renamed model"})
                    .as_object()
                    .expect("map")
                    .clone(),
            },
            revision as u64,
        );
        let outcome = controller
            .apply_confirmed_config(&update)
            .expect("prepare");
        let ConfigApplyOutcome::ModelEditor {
            id: editor_id,
            draft,
        } = outcome
        else {
            panic!("editor");
        };
        assert_eq!(editor_id.as_deref(), Some(id.as_str()));
        assert_eq!(draft.display_name, "Renamed model");
        assert_eq!(draft.context_window_tokens, Some(8192));
        assert_eq!(draft.max_output_tokens, Some(1024));
        let current = storage
            .with_ai_connection(|conn| models::load_model(conn, &id))
            .expect("original");
        assert_eq!(current.display_name, "Original model");
        assert_eq!(current.revision, revision);
        assert_eq!(
            current.credential_status,
            fluxdb_storage::ai_platform::model_secret::CredentialStatus::Configured
        );
        std::fs::remove_dir_all(storage.root_dir()).expect("remove isolated profile");
    }
}
