// MCP Server 管理页动作：加载本地设置、保存 Profile、维护数据源与全局写入策略。

impl NavicatMain {
    /// 确保默认 Profile 已存在（首次进入页签时补建），返回刷新后的当前配置。
    fn ensure_mcp_profile(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<fluxdb_app::AiMcpServerProfileItem> {
        match self.controller.dispatch(AppCommand::EnsureMcpControl) {
            AppEvent::AiSettingsError(message) => {
                self.show_message(&message, AppMessageKind::Error, cx);
                return None;
            }
            _ => {}
        }
        let state = self.controller.state();
        state
            .ai_mcp_server_profiles
            .iter()
            .find(|profile| profile.id == fluxdb_app::DEFAULT_MCP_PROFILE_ID)
            .cloned()
    }

    /// 保存 MCP HTTP 端口：写入全局 Settings（config.toml），端点按新端口重启。
    ///
    /// 端口非法时只提示、不保存，避免把端点切到不可用端口。
    fn save_mcp_http_port(&mut self, cx: &mut Context<Self>) {
        let raw = self.mcp_port_input.read(cx).value().trim().to_string();
        let Ok(port) = raw.parse::<u16>() else {
            self.show_message(tr("app.ai.mcp.port_invalid").as_ref(), AppMessageKind::Error, cx);
            return;
        };
        if port == 0 {
            self.show_message(tr("app.ai.mcp.port_invalid").as_ref(), AppMessageKind::Error, cx);
            return;
        }
        let mut settings = self.controller.state().settings.clone();
        if settings.mcp_http_port == port {
            return;
        }
        settings.mcp_http_port = port;
        // 控制器侧更新 state 并重启端点；随后立即持久化到 config.toml。
        let _ = self.controller.dispatch(AppCommand::SaveSettings(settings));
        let _ = self.storage.save_settings(&self.controller.state().settings);
        self.show_message(tr("app.ai.mcp.port_saved").as_ref(), AppMessageKind::Success, cx);
        cx.notify();
    }

    /// 切换默认 Profile 的对外服务开关：名称与 revision 沿用已存值，只提交 enabled。
    fn set_mcp_profile_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(profile) = self.ensure_mcp_profile(cx) else {
            return;
        };
        let input = fluxdb_app::AiMcpServerProfileSaveInput {
            id: Some(profile.id),
            display_name: profile.display_name,
            enabled,
            expected_revision: Some(profile.revision),
        };
        match self
            .controller
            .dispatch(AppCommand::SaveAiMcpServerProfile(input))
        {
            AppEvent::AiMcpServerProfileSaved { .. } => {}
            AppEvent::AiSettingsError(message) => {
                self.show_message(&message, AppMessageKind::Error, cx);
                // CAS 冲突时重新加载，让 UI 与后端回到同一事实。
                let _ = self.controller.dispatch(AppCommand::RefreshAiSettings);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 保存全局默认写入策略（复用运行设置 revision 做 CAS）。
    fn save_mcp_global_policy(&mut self, value: fluxdb_core::McpWritePolicy, cx: &mut Context<Self>) {
        let mut settings = self.controller.state().ai_execution_policy.clone();
        settings.mcp_default = value;
        let expected_revision = self.controller.state().ai_runtime_revision;
        match self.controller.dispatch(AppCommand::SaveExecutionPolicy {
            settings,
            expected_revision,
        }) {
            AppEvent::AiSettingsError(message) => {
                self.show_message(&message, AppMessageKind::Error, cx)
            }
            _ => self.show_message(tr("app.ai.mcp.global_policy_saved").as_ref(), AppMessageKind::Success, cx),
        }
        cx.notify();
    }

    /// 就地保存某数据源的写入策略（读回当前 revision 做对象级 CAS）。
    fn save_mcp_datasource_policy(
        &mut self,
        profile_id: &str,
        connection_id: i64,
        write_policy: String,
        expected_revision: i64,
        cx: &mut Context<Self>,
    ) {
        let input = fluxdb_app::AiMcpDatasourceSaveInput {
            profile_id: profile_id.to_string(),
            connection_id,
            write_policy,
            expected_revision: Some(expected_revision),
        };
        match self
            .controller
            .dispatch(AppCommand::SaveAiMcpDatasource(input))
        {
            AppEvent::AiMcpDatasourceSaved { .. } => {
                self.show_message(
                    tr("app.ai.mcp.policy_updated"),
                    AppMessageKind::Success,
                    cx,
                );
            }
            AppEvent::AiSettingsError(message) => {
                self.show_message(&message, AppMessageKind::Error, cx);
                // CAS 冲突时重新加载，让 UI 与后端回到同一事实。
                let _ = self.controller.dispatch(AppCommand::RefreshAiSettings);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 从对外服务移除数据源。
    fn remove_mcp_datasource(
        &mut self,
        profile_id: &str,
        connection_id: i64,
        cx: &mut Context<Self>,
    ) {
        match self.controller.dispatch(AppCommand::DeleteAiMcpDatasource {
            profile_id: profile_id.to_string(),
            connection_id,
        }) {
            AppEvent::AiMcpDatasourceDeleted { .. } => {
                self.show_message(
                    tr("app.ai.mcp.datasource_removed"),
                    AppMessageKind::Success,
                    cx,
                );
            }
            AppEvent::AiSettingsError(message) => {
                self.show_message(&message, AppMessageKind::Error, cx)
            }
            _ => {}
        }
        cx.notify();
    }
}
