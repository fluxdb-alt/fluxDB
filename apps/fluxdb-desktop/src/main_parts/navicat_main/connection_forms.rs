impl NavicatMain {
    fn confirm_delete_connection(&mut self, cx: &mut Context<Self>) {
        let Some(connection_id) = self.pending_delete_connection.take() else {
            return;
        };
        // 删除前保留连接 ID，提交成功后仅清理该连接的加密凭据。
        let deleted_config = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| connection.config.clone());

        let previous = self.controller.clone();
        self.dispatch(AppCommand::DeleteConnection(connection_id), cx);
        if let Err(error) = self
            .storage
            .save_connections(&self.controller.connection_configs())
        {
            self.controller = previous;
            self.show_message(
                tr("app.connection.error.delete_failed").replace("%{error}", &error.to_string()),
                AppMessageKind::Error,
                cx,
            );
            cx.notify();
            return;
        }
        if let Some(config) = deleted_config {
            self.storage.delete_connection_secrets(&config);
        }
        self.connecting_connections.remove(&connection_id);
        self._connection_tasks.remove(&connection_id.0);
        self.loaded_database_children
            .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
        self.loading_databases
            .retain(|key| !key.starts_with(&format!("{}:", connection_id.0)));
        self.expanded_databases
            .retain(|key, _| !key.starts_with(&format!("{}:", connection_id.0)));
        self.expanded_object_groups
            .retain(|key, _| !key.starts_with(&format!("{}:", connection_id.0)));
        cx.notify();
    }

    fn create_connection_from_form(&mut self, cx: &mut Context<Self>) {
        self.submit_new_connection(true, cx);
    }

    /// 「仅保存」：只校验并落库，不测试、不自动打开连接。
    fn save_connection_without_test(&mut self, cx: &mut Context<Self>) {
        self.submit_new_connection(false, cx);
    }

    fn submit_new_connection(&mut self, open_after_save: bool, cx: &mut Context<Self>) {
        let kind = self.new_connection_kind.unwrap_or(DatabaseKind::MySql);
        self.saving_connection = true;
        if let Err(message) = self.validate_new_connection(kind) {
            self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(message));
            self.saving_connection = false;
            cx.notify();
            return;
        }
        let draft = self.new_connection_draft(kind);
        if let Some(connection_id) = self.editing_connection_id {
            let existing = self
                .controller
                .state()
                .connections
                .iter()
                .find(|connection| connection.config.id == connection_id)
                .map(|connection| connection.config.clone());
            let mut config = draft.into_config(connection_id);
            if let Some(existing) = existing {
                let new_options = config.options;
                config.options = existing.options;
                config.options.extend(new_options);
            }
            let previous = self.controller.clone();
            let event = self
                .controller
                .dispatch(AppCommand::UpdateConnection(config.clone()));
            if let Err(error) = self
                .storage
                .save_connections(&self.controller.connection_configs())
            {
                self.controller = previous;
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                    tr("app.connection.error.save_failed").replace("%{error}", &error.to_string()),
                ));
                self.show_message(
                    tr("app.connection.error.save_failed").replace("%{error}", &error.to_string()),
                    AppMessageKind::Error,
                    cx,
                );
                self.saving_connection = false;
                cx.notify();
                return;
            }
            self.new_connection_kind = None;
            self.new_connection_password_visible = false;
            self.editing_connection_id = None;
            if open_after_save && matches!(event, fluxdb_app::AppEvent::ConnectionUpdated(_)) {
                self.open_connection_from_sidebar(config.id, cx);
            }
            self.saving_connection = false;
            cx.notify();
            return;
        }

        // 仅保存：跳过建连测试，直接落库；不自动打开连接。
        if !open_after_save {
            self.persist_new_connection(draft, open_after_save, cx);
            return;
        }

        let test_config = draft.clone().into_config(fluxdb_core::ConnectionId(0));
        // 建连测试是真实网络 I/O：交给后台线程跑，既不让弹框卡住主线程，也让驱动里可能的
        // panic 不再落进 GPUI 主线程的 C 回调栈帧（该栈不能 unwind，会 abort 整个进程）。
        self.new_connection_form.test_status = Some(ConnectionTestStatus::Pending(
            tr("app.connection.status.testing").to_string(),
        ));
        let mut controller = self.controller.clone();
        self._test_connection_task = Some(cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::TestConnection(test_config))
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this._test_connection_task = None;
                    // 测试期间弹框已取消：丢弃迟到结果，不再继续保存。
                    if !this.saving_connection {
                        return;
                    }
                    this.finish_new_connection_creation(draft, event, cx);
                });
            });
        }));
        cx.notify();
    }

    /// 「保存并连接」的测试完成回调：测试通过才落库并打开连接，失败则把错误留在弹框里。
    fn finish_new_connection_creation(
        &mut self,
        draft: ConnectionDraft,
        event: fluxdb_app::AppEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            fluxdb_app::AppEvent::ConnectionTested(_, Ok(())) => {}
            fluxdb_app::AppEvent::ConnectionTested(_, Err(error)) => {
                self.new_connection_form.test_status =
                    Some(ConnectionTestStatus::Error(error.to_string()));
                self.saving_connection = false;
                cx.notify();
                return;
            }
            fluxdb_app::AppEvent::Failed(error) => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(format!(
                    "{}：{}",
                    error.title, error.message
                )));
                self.saving_connection = false;
                cx.notify();
                return;
            }
            _ => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                    tr("app.connection.status.failed_unsaved").to_string(),
                ));
                self.saving_connection = false;
                cx.notify();
                return;
            }
        }
        self.persist_new_connection(draft, true, cx);
    }

    /// 新建连接的落库收尾：保存成功才关闭弹框；`open_after_save` 区分「保存并连接」与「仅保存」。
    fn persist_new_connection(&mut self, draft: ConnectionDraft, open_after_save: bool, cx: &mut Context<Self>) {
        let previous = self.controller.clone();
        let event = self
            .controller
            .dispatch(AppCommand::CreateConnection(draft));
        if let Err(error) = self
            .storage
            .save_connections(&self.controller.connection_configs())
        {
            self.controller = previous;
            self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                tr("app.connection.error.save_failed").replace("%{error}", &error.to_string()),
            ));
            self.show_message(
                tr("app.connection.error.save_failed").replace("%{error}", &error.to_string()),
                AppMessageKind::Error,
                cx,
            );
            self.saving_connection = false;
            cx.notify();
            return;
        }
        self.new_connection_kind = None;
        self.new_connection_password_visible = false;
        if let fluxdb_app::AppEvent::ConnectionCreated(config) = event {
            if let Some(group_id) = self.new_connection_target_group.take() {
                let _ = self.controller.dispatch(AppCommand::MoveConnectionToGroup {
                    connection_id: config.id,
                    group_id,
                });
                if let Some(group) = self
                    .controller
                    .state()
                    .sidebar_layout
                    .groups
                    .iter()
                    .find(|group| group.id == group_id && group.collapsed)
                {
                    let _ = self
                        .controller
                        .dispatch(AppCommand::ToggleConnectionGroup(group.id));
                }
            }
            self.persist_sidebar_layout();
            if open_after_save {
                self.open_connection_from_sidebar(config.id, cx);
            }
        }
        self.saving_connection = false;
        cx.notify();
    }

    fn show_edit_connection(
        &mut self,
        connection_id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self
            .controller
            .state()
            .connections
            .iter()
            .find(|connection| connection.config.id == connection_id)
            .map(|connection| connection.config.clone())
        else {
            return;
        };

        self.connection_context_menu = None;
        self.group_context_menu = None;
        self.editing_connection_id = Some(connection_id);
        self.new_connection_target_group = None;
        self.new_connection_kind = Some(config.kind);
        self.new_connection_tab = NewConnectionTab::Connection;
        self.new_connection_password_visible = false;
        self.new_connection_form = NewConnectionForm::from_config(&config);
        self.new_connection_inputs
            .sync_from_form(&self.new_connection_form, window, cx);
        self.sync_new_connection_selects(window, cx);
        self.new_connection_inputs
            .refresh_placeholders(Some(config.kind), window, cx);
        self.new_connection_inputs.password.update(cx, |input, cx| {
            input.set_masked(true, window, cx);
        });
        self.new_connection_inputs
            .focus_field(first_connection_field(config.kind), window, cx);
        cx.notify();
    }

    fn show_new_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kind = DatabaseKind::MySql;
        let from_group_menu = self.group_context_menu.is_some();
        self.connection_context_menu = None;
        self.tab_context_menu = None;
        self.tab_switcher = None;
        self.group_context_menu = None;
        if !from_group_menu {
            self.new_connection_target_group = None;
        }
        self.new_connection_kind = Some(kind);
        self.editing_connection_id = None;
        self.new_connection_tab = NewConnectionTab::Connection;
        self.new_connection_password_visible = false;
        self.new_connection_form = NewConnectionForm::for_kind(kind, self.next_connection_index());
        self.new_connection_inputs
            .sync_from_form(&self.new_connection_form, window, cx);
        self.sync_new_connection_selects(window, cx);
        self.new_connection_inputs
            .refresh_placeholders(Some(kind), window, cx);
        self.new_connection_inputs.password.update(cx, |input, cx| {
            input.set_masked(true, window, cx);
        });
        self.new_connection_inputs
            .focus_field(first_connection_field(kind), window, cx);
        cx.notify();
    }

    fn set_new_connection_kind(
        &mut self,
        kind: DatabaseKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_connection_kind = Some(kind);
        self.new_connection_password_visible = false;
        self.new_connection_form = NewConnectionForm::for_kind(kind, self.next_connection_index());
        self.new_connection_inputs
            .sync_from_form(&self.new_connection_form, window, cx);
        self.sync_new_connection_selects(window, cx);
        self.new_connection_inputs
            .refresh_placeholders(Some(kind), window, cx);
        self.new_connection_inputs.password.update(cx, |input, cx| {
            input.set_masked(true, window, cx);
        });
        self.new_connection_inputs
            .focus_field(first_connection_field(kind), window, cx);
        cx.notify();
    }

    /// 打开新建/编辑连接弹框时，把表单里的 SSH 认证方式同步到对应 Select 光标，
    /// 保证下拉与表单字段一致（SelectState 独立维护光标）。
    fn sync_new_connection_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 下拉值已与表单字段一致，不再经过展示文案转换。
        let ssh_auth = self.new_connection_form.ssh_auth.clone();
        self.new_connection_inputs
            .ssh_auth_select
            .update(cx, |select, cx| {
                select.set_selected_value(&ssh_auth, window, cx);
            });
        // SSL 模式下拉回填：选项值与表单字段值相同，直接同步光标。
        let pg_ssl_mode = self.new_connection_form.pg_tls_ssl_mode.clone();
        self.new_connection_inputs
            .pg_ssl_mode_select
            .update(cx, |select, cx| {
                select.set_selected_value(&pg_ssl_mode, window, cx);
            });
        // MySQL SSL 模式：选项集随隧道/代理状态变化，同步时同时收敛档位。
        self.sync_mysql_ssl_mode_options(window, cx);
        // SSH 主机密钥策略：编辑旧连接时必须回填，否则默认 strict 会改变行为。
        let host_key_policy = self.new_connection_form.ssh_host_key_policy.clone();
        self.new_connection_inputs
            .ssh_host_key_policy_select
            .update(cx, |select, cx| {
                select.set_selected_value(&host_key_policy, window, cx);
            });
        // 代理类型由下拉独立维护光标，编辑已有连接时必须按档案回填。
        let mysql_proxy_type = self.new_connection_form.mysql_proxy_type.clone();
        self.new_connection_inputs
            .mysql_proxy_type_select
            .update(cx, |select, cx| {
                select.set_selected_value(&mysql_proxy_type, window, cx);
            });
        // Oracle 寻址方式与登录身份同样是独立光标：编辑 SID / SYSDBA 连接时
        // 必须回填，否则下拉显示默认项，用户一碰就写错档案。
        let oracle_connect_kind = self.new_connection_form.oracle_connect_kind.clone();
        self.new_connection_inputs
            .oracle_connect_kind_select
            .update(cx, |select, cx| {
                select.set_selected_value(&oracle_connect_kind, window, cx);
            });
        let oracle_privilege = self.new_connection_form.oracle_privilege.clone();
        self.new_connection_inputs
            .oracle_privilege_select
            .update(cx, |select, cx| {
                select.set_selected_value(&oracle_privilege, window, cx);
            });
        // SQL Server 加密模式同样由下拉独立维护光标，编辑连接时必须从档案回填。
        let sqlserver_encryption = self.new_connection_form.sqlserver_encryption.clone();
        self.new_connection_inputs
            .sqlserver_encryption_select
            .update(cx, |select, cx| {
                select.set_selected_value(&sqlserver_encryption, window, cx);
            });
        // MongoDB 认证机制：空串代表「默认（驱动协商）」，需要显式同步回第一项。
        let mongo_auth_mechanism = self.new_connection_form.mongo_auth_mechanism.clone();
        self.new_connection_inputs
            .mongo_auth_mechanism_select
            .update(cx, |select, cx| {
                select.set_selected_value(&mongo_auth_mechanism, window, cx);
            });
    }

    fn cancel_new_connection(&mut self, cx: &mut Context<Self>) {
        // 释放建连测试任务槽并复位 loading：否则下次打开弹框时按钮仍是禁用态，
        // 迟到的测试结果也会被保存流程丢弃。
        self._test_connection_task = None;
        self.saving_connection = false;
        self.new_connection_kind = None;
        self.new_connection_password_visible = false;
        self.editing_connection_id = None;
        self.new_connection_target_group = None;
        cx.notify();
    }

    fn toggle_new_connection_password_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_connection_password_visible = !self.new_connection_password_visible;
        let visible = self.new_connection_password_visible;
        self.new_connection_inputs.password.update(cx, |input, cx| {
            input.set_masked(!visible, window, cx);
        });
        cx.notify();
    }

    fn set_new_connection_tab(&mut self, tab: NewConnectionTab, cx: &mut Context<Self>) {
        self.new_connection_tab = tab;
        cx.notify();
    }

    fn set_new_connection_color(&mut self, color: &'static str, cx: &mut Context<Self>) {
        self.new_connection_form.color = color.to_string();
        self.new_connection_form.test_status = None;
        cx.notify();
    }

    fn next_connection_index(&self) -> usize {
        self.controller.state().connections.len() + 1
    }

    fn set_connection_field_value(
        &mut self,
        field: ConnectionField,
        value: String,
        cx: &mut Context<Self>,
    ) {
        let form = &mut self.new_connection_form;
        form.test_status = None;
        match field {
            ConnectionField::Name => form.name = value,
            ConnectionField::Host => form.host = value,
            ConnectionField::Port => form.port = value,
            ConnectionField::Username => form.username = value,
            ConnectionField::Password => form.password = value,
            ConnectionField::Database | ConnectionField::MongoDefaultDb => form.database = value,
            ConnectionField::DamengSchema => form.dameng_schema = value,
            ConnectionField::DamengConnectTimeout => form.dameng_connect_timeout_secs = value,
            ConnectionField::DamengQueryTimeout => form.dameng_query_timeout_secs = value,
            ConnectionField::DamengApplicationName => form.dameng_application_name = value,
            ConnectionField::UrlParams => form.url_params = value,
            ConnectionField::SqlitePath => form.sqlite_path = value,
            ConnectionField::MongoAuthDb => form.mongo_auth_db = value,
            ConnectionField::TlsCa => form.tls_ca = value,
            ConnectionField::TlsClientCert => form.tls_client_cert = value,
            ConnectionField::TlsClientKey => form.tls_client_key = value,
            ConnectionField::TlsClientKeyPassphrase => form.tls_client_key_passphrase = value,
            ConnectionField::TlsSni => form.tls_sni = value,
            ConnectionField::SshHost => form.ssh_host = value,
            ConnectionField::SshPort => form.ssh_port = value,
            ConnectionField::SshUsername => form.ssh_username = value,
            ConnectionField::SshPassword => form.ssh_password = value,
            ConnectionField::SshPrivateKey => form.ssh_private_key = value,
            ConnectionField::SshPassphrase => form.ssh_passphrase = value,
            // —— PostgreSQL 专用 ——
            ConnectionField::PgDefaultSchema => form.pg_default_schema = value,
            ConnectionField::PgApplicationName => form.pg_application_name = value,
            ConnectionField::PgConnectTimeoutSecs => form.pg_connect_timeout_secs = value,
            ConnectionField::PgQueryTimeoutSecs => form.pg_query_timeout_secs = value,
            ConnectionField::PgTimezone => form.pg_timezone = value,
            // —— MySQL / TiDB 专用 ——
            ConnectionField::MysqlCharset => form.mysql_charset = value,
            ConnectionField::MysqlCollation => form.mysql_collation = value,
            ConnectionField::MysqlSocketPath => form.mysql_socket_path = value,
            ConnectionField::MysqlTimezone => form.mysql_timezone = value,
            ConnectionField::MysqlInitSql => form.mysql_init_sql = value,
            ConnectionField::MysqlSshConnectTimeout => form.mysql_ssh_connect_timeout_secs = value,
            ConnectionField::MysqlSshKeepalive => form.mysql_ssh_keepalive_secs = value,
            ConnectionField::MysqlProxyHost => form.mysql_proxy_host = value,
            ConnectionField::MysqlProxyPort => form.mysql_proxy_port = value,
            ConnectionField::MysqlProxyUsername => form.mysql_proxy_username = value,
            ConnectionField::MysqlProxyPassword => form.mysql_proxy_password = value,
            ConnectionField::MysqlConnectTimeout => form.mysql_connect_timeout_secs = value,
            ConnectionField::MysqlQueryTimeout => form.mysql_query_timeout_secs = value,
            ConnectionField::SqlServerSchema => form.sqlserver_schema = value,
            ConnectionField::SqlServerInstance => form.sqlserver_instance = value,
            ConnectionField::SqlServerConnectTimeout => form.sqlserver_connect_timeout_secs = value,
            ConnectionField::SqlServerQueryTimeout => form.sqlserver_query_timeout_secs = value,
            ConnectionField::SqlServerApplicationName => form.sqlserver_application_name = value,
            // —— Oracle 专用 ——
            ConnectionField::OracleServiceName => form.oracle_service_name = value,
            ConnectionField::OracleSid => form.oracle_sid = value,
            ConnectionField::OracleSchema => form.oracle_schema = value,
            ConnectionField::OracleConnectTimeout => form.oracle_connect_timeout_secs = value,
            ConnectionField::OracleQueryTimeout => form.oracle_query_timeout_secs = value,
            ConnectionField::OracleApplicationName => form.oracle_application_name = value,
            ConnectionField::OracleWalletLocation => form.oracle_wallet_location = value,
            ConnectionField::OracleWalletPassword => form.oracle_wallet_password = value,
            ConnectionField::SentinelMasterName => form.sentinel_master_name = value,
            ConnectionField::SentinelEndpoints => form.sentinel_endpoints = value,
            ConnectionField::ClusterStartNodes => form.cluster_start_nodes = value,
            ConnectionField::AiKeyPrefixes => form.ai_key_prefixes = value,
            ConnectionField::DiscoveryUri => form.discovery_uri = value,
        }
        cx.notify();
    }

    /// 设置布尔开关字段的编辑值（TLS/SSH 启用、TLS 校验、Cluster 只读等）。
    fn set_connection_toggle_field(
        &mut self,
        field: ConnectionToggleField,
        value: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = &mut self.new_connection_form;
        form.test_status = None;
        match field {
            ConnectionToggleField::TlsEnabled => form.tls_enabled = value,
            ConnectionToggleField::TlsVerify => form.tls_verify = value,
            ConnectionToggleField::SshEnabled => form.ssh_enabled = value,
            ConnectionToggleField::ClusterAllowReadonly => form.cluster_allow_readonly = value,
            ConnectionToggleField::ForceStandalone => form.force_standalone = value,
            ConnectionToggleField::MysqlProxyEnabled => form.mysql_proxy_enabled = value,
            ConnectionToggleField::MysqlTcpKeepalive => form.mysql_tcp_keepalive = value,
            ConnectionToggleField::PgTcpKeepalive => form.pg_tcp_keepalive = value,
            ConnectionToggleField::PgShowOtherDatabases => form.pg_show_other_databases = value,
            ConnectionToggleField::PgShowSystemSchemas => form.pg_show_system_schemas = value,
            ConnectionToggleField::MongoSrv => form.mongo_srv = value,
            ConnectionToggleField::ReadOnly => form.read_only = value,
        }
        // 隧道/代理会把拨号目标改写成 127.0.0.1，verify_identity 的证书主机名校验
        // 不可能通过；此时从下拉里移除该档位，已选中的回落到 verify_ca。
        if matches!(
            field,
            ConnectionToggleField::SshEnabled | ConnectionToggleField::MysqlProxyEnabled
        ) {
            self.sync_mysql_ssl_mode_options(window, cx);
        }
        cx.notify();
    }

    /// 按当前传输层状态收敛 MySQL SSL 模式下拉：
    /// 直连可选五档；走 SSH 隧道或代理时隐藏 `verify_identity`（无法校验主机名）。
    fn sync_mysql_ssl_mode_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let form = &mut self.new_connection_form;
        let tunneled = form.ssh_enabled || form.mysql_proxy_enabled;
        if tunneled && form.mysql_tls_ssl_mode == "verify_identity" {
            form.mysql_tls_ssl_mode = "verify_ca".to_string();
        }
        let mode = form.mysql_tls_ssl_mode.clone();
        let mut modes = vec![
            "disabled".to_string(),
            "preferred".to_string(),
            "required".to_string(),
            "verify_ca".to_string(),
        ];
        if !tunneled {
            modes.push("verify_identity".to_string());
        }
        self.new_connection_inputs
            .mysql_ssl_mode_select
            .update(cx, |select, cx| {
                select.set_items(SearchableVec::new(modes), window, cx);
                select.set_selected_value(&mode, window, cx);
            });
    }

    fn choose_sqlite_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("app.connection.picker.sqlite").into()),
        });
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = receiver.await;
            cx.update(|window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let value = path.display().to_string();
                            this.new_connection_inputs
                                .sqlite_path
                                .update(cx, |input, cx| {
                                    input.set_value(value.clone(), window, cx);
                                });
                            this.set_connection_field_value(ConnectionField::SqlitePath, value, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        this.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                            tr("app.connection.error.sqlite_picker")
                                .replace("%{error}", &error.to_string()),
                        ));
                        cx.notify();
                    }
                    Err(error) => {
                        this.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                            tr("app.connection.error.sqlite_picker")
                                .replace("%{error}", &error.to_string()),
                        ));
                        cx.notify();
                    }
                });
            })
            .ok();
        }));
    }

    /// 通用「选择文件」：把选中的路径回填到指定 TLS/SSH 证书/密钥字段。
    fn choose_connection_file(
        &mut self,
        field: ConnectionField,
        prompt: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(prompt.into().into()),
        });
        self._file_picker_task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = receiver.await;
            cx.update(|window, cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            let value = path.display().to_string();
                            this.new_connection_inputs
                                .for_field(field)
                                .update(cx, |input, cx| {
                                    input.set_value(value.clone(), window, cx);
                                });
                            this.set_connection_field_value(field, value, cx);
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        this.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                            tr("app.connection.error.file_picker")
                                .replace("%{error}", &error.to_string()),
                        ));
                        cx.notify();
                    }
                    Err(error) => {
                        this.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                            tr("app.connection.error.file_picker")
                                .replace("%{error}", &error.to_string()),
                        ));
                        cx.notify();
                    }
                });
            })
            .ok();
        }));
    }

    fn test_new_connection(&mut self, cx: &mut Context<Self>) {
        // 已有测试在跑时忽略重复点击，避免叠加并发测试任务。
        if self._test_connection_task.is_some() {
            return;
        }
        let Some(kind) = self.new_connection_kind else {
            return;
        };
        match self.validate_new_connection(kind) {
            Ok(()) => {
                let config = self
                    .new_connection_draft(kind)
                    .into_config(fluxdb_core::ConnectionId(0));
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Pending(
                    tr("app.connection.status.testing").to_string(),
                ));
                let mut controller = self.controller.clone();
                self._test_connection_task = Some(cx.spawn(async move |view, cx| {
                    let event = cx
                        .background_spawn(async move {
                            controller.dispatch(AppCommand::TestConnection(config))
                        })
                        .await;
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.new_connection_form.test_status = Some(match event {
                                fluxdb_app::AppEvent::ConnectionTested(_, Ok(())) => {
                                    ConnectionTestStatus::Success(
                                        tr("app.connection.status.test_success").to_string(),
                                    )
                                }
                                fluxdb_app::AppEvent::ConnectionTested(_, Err(error)) => {
                                    ConnectionTestStatus::Error(error.to_string())
                                }
                                fluxdb_app::AppEvent::Failed(error) => ConnectionTestStatus::Error(
                                    format!("{}：{}", error.title, error.message),
                                ),
                                _ => ConnectionTestStatus::Error(
                                    tr("app.connection.status.test_failed").to_string(),
                                ),
                            });
                            this._test_connection_task = None;
                            cx.notify();
                        });
                    });
                }));
            }
            Err(message) => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(message));
            }
        }
        cx.notify();
    }

    fn validate_new_connection(&self, kind: DatabaseKind) -> Result<(), String> {
        let form = &self.new_connection_form;
        if form.name.trim().is_empty() {
            return Err(tr("app.connection.error.name_required").to_string());
        }
        match kind {
            DatabaseKind::Sqlite => {
                if form.sqlite_path.trim().is_empty() {
                    return Err(tr("app.connection.error.sqlite_path_required").to_string());
                }
            }
            // Redis 走结构化档案校验（Sentinel / Cluster 不强制基础主机）。
            DatabaseKind::Redis => {
                let profile = form.build_redis_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
            }
            // MySQL / TiDB 走结构化档案校验（含 SSH/代理/超时）。
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
                let profile = form.build_mysql_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
            }
            // PostgreSQL 同样走结构化档案校验（TLS 模式与证书、SSH/代理、超时）。
            // KingbaseES 复用同一份 PG 连接档案。
            DatabaseKind::Postgres | DatabaseKind::Kingbase => {
                let profile = form.build_postgres_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
            }
            DatabaseKind::Dameng => {
                let profile = form.build_dameng_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
                let connect_timeout = form
                    .dameng_connect_timeout_secs
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| tr("app.connection.dameng.connect_timeout_invalid").to_string())?;
                let query_timeout = form
                    .dameng_query_timeout_secs
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| tr("app.connection.dameng.query_timeout_invalid").to_string())?;
                if connect_timeout == 0 || connect_timeout > 86400 || query_timeout > 86400 {
                    return Err(tr("app.connection.dameng.timeout_out_of_range").to_string());
                }
            }
            // MongoDB 走结构化档案校验（种子列表 / SRV 单主机 / 用户名必须带密码）。
            DatabaseKind::MongoDb => {
                // 种子列表里显式写了端口就必须合法，不静默回落到 27017。
                for segment in form.host.split(',') {
                    if let Some((_, port)) = segment.trim().rsplit_once(':') {
                        let port = port.trim();
                        if !port.is_empty() {
                            parse_port(port)?;
                        }
                    }
                }
                let profile = form.build_mongo_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
            }
            DatabaseKind::SqlServer => {
                let profile = form.build_sqlserver_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
                let connect_timeout = form
                    .sqlserver_connect_timeout_secs
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| {
                        tr("app.connection.sqlserver.connect_timeout_invalid").to_string()
                    })?;
                let query_timeout = form
                    .sqlserver_query_timeout_secs
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| {
                        tr("app.connection.sqlserver.query_timeout_invalid").to_string()
                    })?;
                if connect_timeout == 0 || connect_timeout > 86400 || query_timeout > 86400 {
                    return Err(tr("app.connection.sqlserver.timeout_out_of_range").to_string());
                }
            }
            // Oracle：Service Name 与 SID 互斥，超时范围与 SQL Server 一致。
            DatabaseKind::Oracle => {
                let profile = form.build_oracle_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
                let connect_timeout = form
                    .oracle_connect_timeout_secs
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| tr("app.connection.oracle.connect_timeout_invalid").to_string())?;
                let query_timeout = form
                    .oracle_query_timeout_secs
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| tr("app.connection.oracle.query_timeout_invalid").to_string())?;
                if connect_timeout == 0 || connect_timeout > 86400 || query_timeout > 86400 {
                    return Err(tr("app.connection.oracle.timeout_out_of_range").to_string());
                }
            }
        }
        Ok(())
    }

    fn new_connection_draft(&self, kind: DatabaseKind) -> ConnectionDraft {
        let form = &self.new_connection_form;
        let mut options = std::collections::BTreeMap::new();
        if !form.username.trim().is_empty() {
            options.insert("username".to_string(), form.username.trim().to_string());
        }
        if !form.password.is_empty() {
            options.insert("password".to_string(), form.password.clone());
        }
        options.insert(CONNECTION_COLOR_OPTION.to_string(), form.color.clone());
        if form.read_only {
            options.insert("read_only".to_string(), "true".to_string());
        }
        if kind == DatabaseKind::Redis {
            // AI Key 授权前缀是 AI 工具的范围配置，不是连接协议参数：
            // 单独成键，不再经过自由文本参数框与白名单。
            if !form.ai_key_prefixes.trim().is_empty() {
                options.insert(
                    "ai_key_prefixes".to_string(),
                    form.ai_key_prefixes.trim().to_string(),
                );
            }
        } else if !form.url_params.trim().is_empty() {
            options.insert("url_params".to_string(), form.url_params.trim().to_string());
        }
        if form.mongo_srv {
            options.insert("srv".to_string(), "true".to_string());
        }
        if !form.mongo_auth_db.trim().is_empty() {
            options.insert("auth_db".to_string(), form.mongo_auth_db.trim().to_string());
        }
        if !form.mongo_auth_mechanism.trim().is_empty() {
            options.insert(
                "auth_mechanism".to_string(),
                form.mongo_auth_mechanism.trim().to_string(),
            );
        }

        let endpoint = match kind {
            DatabaseKind::MySql
            | DatabaseKind::TiDb
            | DatabaseKind::MariaDb
            | DatabaseKind::Redis
            | DatabaseKind::Postgres
            | DatabaseKind::Kingbase
            | DatabaseKind::Dameng
            | DatabaseKind::SqlServer => Endpoint::Tcp {
                host: form.host.trim().to_string(),
                port: form
                    .port
                    .parse()
                    .unwrap_or_else(|_| database_default_port_u16(kind)),
                database: non_empty_option(&form.database),
            },
            // Oracle 没有「数据库名」：endpoint 里记当前生效的 Service Name 或 SID，
            // 供对象树与补全复用；具体是哪一种由档案里的 connect_kind 区分。
            DatabaseKind::Oracle => Endpoint::Tcp {
                host: form.host.trim().to_string(),
                port: form
                    .port
                    .parse()
                    .unwrap_or_else(|_| database_default_port_u16(kind)),
                database: non_empty_option(form.oracle_target_name()),
            },
            // SQLite 的只读直接落到驱动打开标志，服务端层面真正拒绝写入。
            DatabaseKind::Sqlite => Endpoint::SqliteFile {
                path: form.sqlite_path.trim().into(),
                read_only: form.read_only,
            },
            DatabaseKind::MongoDb => Endpoint::Tcp {
                host: form.host.trim().to_string(),
                port: form
                    .port
                    .parse()
                    .unwrap_or_else(|_| database_default_port_u16(kind)),
                database: non_empty_option(&form.database),
            },
        };

        // 各数据库的结构化档案由表单实现填充，新增数据库不再改这里。
        let mut draft = ConnectionDraft {
            name: form.name.trim().to_string(),
            kind,
            endpoint,
            options,
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        };
        connection_form_spec(kind).build_into(form, &mut draft);
        draft
    }

    /// Redis 连接串导入：把 URI 交给 app 层解析。
    /// 解析结果（`AppEvent::RedisConnectionDiscovered`）异步回流后回填表单（见下）。
    fn import_redis_connection_string(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.new_connection_kind else {
            return;
        };
        if kind != DatabaseKind::Redis {
            return;
        }
        let connection_string = self.new_connection_form.discovery_uri.clone();
        if connection_string.trim().is_empty() {
            self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                tr("app.connection.error.redis_uri_required").to_string(),
            ));
            cx.notify();
            return;
        }
        self.new_connection_form.test_status = Some(ConnectionTestStatus::Pending(
            tr("app.connection.status.parsing_uri").to_string(),
        ));
        let mut controller = self.controller.clone();
        self._redis_discover_task = Some(cx.spawn(async move |view, cx| {
            let event = cx
                .background_spawn(async move {
                    controller.dispatch(AppCommand::DiscoverRedisConnection { connection_string })
                })
                .await;
            let _ = cx.update(|cx| {
                let Some(view) = view.upgrade() else {
                    return;
                };
                view.update(cx, |this, cx| {
                    match event {
                        fluxdb_app::AppEvent::RedisConnectionDiscovered(draft) => {
                            this.apply_redis_discovered_draft(&draft, cx);
                            this.new_connection_form.test_status =
                                Some(ConnectionTestStatus::Success(
                                    tr("app.connection.toast.redis_imported").to_string(),
                                ));
                        }
                        fluxdb_app::AppEvent::Failed(error) => {
                            this.new_connection_form.test_status =
                                Some(ConnectionTestStatus::Error(format!(
                                    "{}：{}",
                                    error.title, error.message
                                )));
                        }
                        _ => {
                            this.new_connection_form.test_status =
                                Some(ConnectionTestStatus::Error(
                                    tr("app.connection.error.redis_uri_parse").to_string(),
                                ));
                        }
                    }
                    this._redis_discover_task = None;
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }

    /// MongoDB 连接串导入：`mongodb://` / `mongodb+srv://` 都由 app 层统一解析，
    /// 结果以 `AppEvent::MongoConnectionDiscovered` 回流后回填表单。
    ///
    /// 密码只落在表单值与档案的 `SecretRef::inline` 上，不写 `options`、不进日志。
    fn import_mongo_connection_string(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.new_connection_kind, Some(DatabaseKind::MongoDb)) {
            return;
        }
        let connection_string = self.new_connection_form.discovery_uri.clone();
        if connection_string.trim().is_empty() {
            self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                tr("app.connection.error.mongo_uri_required").to_string(),
            ));
            cx.notify();
            return;
        }
        self.new_connection_form.test_status = Some(ConnectionTestStatus::Pending(
            tr("app.connection.status.parsing_uri").to_string(),
        ));
        // 解析是纯本地计算（SRV 的真正解析在首次拨号时由驱动完成）。
        let event = self
            .controller
            .dispatch(AppCommand::DiscoverMongoConnection { connection_string });
        match event {
            fluxdb_app::AppEvent::MongoConnectionDiscovered(draft) => {
                self.apply_mongo_discovered_draft(&draft);
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Success(
                    tr("app.connection.toast.mongo_imported").to_string(),
                ));
            }
            fluxdb_app::AppEvent::Failed(error) => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(format!(
                    "{}：{}",
                    error.title, error.message
                )));
            }
            _ => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                    tr("app.connection.error.mongo_uri_parse").to_string(),
                ));
            }
        }
        cx.notify();
    }

    /// 把 MongoDB 连接串解析出的草稿回填进表单；已有连接名保留。
    fn apply_mongo_discovered_draft(&mut self, draft: &ConnectionDraft) {
        let Some(profile) = draft.mongo_profile.as_ref() else {
            return;
        };
        let form = &mut self.new_connection_form;
        form.apply_mongo_profile(profile);
        if form.name.trim().is_empty() {
            form.name = draft.name.clone();
        }
        // 回填后输入框需要重新同步（输入框是独立实体）。
        self.redis_discovery_pending_sync = true;
    }

    /// 把连接串解析出的草稿（携带 `redis_profile`）回填进新建/编辑表单。
    ///
    /// 注意：异步回调里拿不到 `Window`，此处只更新表单数据 + 打「待同步输入框」标记；
    /// 真正的输入框实体同步放在 `NavicatMain::render`（有 `Window`）里消费该标记。
    fn apply_redis_discovered_draft(&mut self, draft: &ConnectionDraft, _cx: &mut Context<Self>) {
        if let Some(profile) = draft.redis_profile.as_ref() {
            let form = &mut self.new_connection_form;
            form.apply_profile(profile);
            // 连接名缺省用解析出的名称；已有填写的名字保留。
            if form.name.trim().is_empty() {
                form.name = draft.name.clone();
            }
        }
        self.redis_discovery_pending_sync = true;
    }

    /// PostgreSQL 连接串导入：解析 FAQ 白名单参数并回填全部 PG 表单字段。
    ///
    /// 与通用 SQL 导入的区别：`sslmode` / 证书路径 / `application_name` /
    /// `connect_timeout` / `options` 都会真正落到档案，不再静默丢弃。
    fn import_postgres_connection_string(&mut self, cx: &mut Context<Self>) {
        let raw = self.new_connection_form.discovery_uri.trim().to_string();
        let profile = match fluxdb_core::PostgresConnectionProfile::from_uri(&raw) {
            Ok(profile) => profile,
            Err(message) => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(message));
                cx.notify();
                return;
            }
        };
        let ssl_mode = match profile.tls.ssl_mode {
            fluxdb_core::PostgresSslMode::Disabled => "disable",
            fluxdb_core::PostgresSslMode::Prefer => "prefer",
            fluxdb_core::PostgresSslMode::Require => "require",
            fluxdb_core::PostgresSslMode::VerifyCa => "verify-ca",
            fluxdb_core::PostgresSslMode::VerifyFull => "verify-full",
        };
        let default_port = database_default_port(DatabaseKind::Postgres);
        let form = &mut self.new_connection_form;
        form.host = profile.basic.host;
        form.port = if profile.basic.port == 0 {
            default_port.to_string()
        } else {
            profile.basic.port.to_string()
        };
        form.username = profile.basic.username;
        form.password = profile
            .basic
            .password
            .value()
            .unwrap_or_default()
            .to_string();
        form.database = profile.basic.maintenance_database;
        form.tls_enabled = profile.tls.enabled;
        form.pg_tls_ssl_mode = ssl_mode.to_string();
        form.tls_ca = profile.tls.ca.key;
        form.tls_client_cert = profile.tls.client_cert.key;
        form.tls_client_key = profile.tls.client_key.key;
        form.tls_sni = profile.tls.server_name;
        form.pg_default_schema = profile.scope.default_schema;
        form.pg_application_name = profile.advanced.application_name;
        form.pg_connect_timeout_secs = profile.advanced.connect_timeout_secs.to_string();
        form.pg_query_timeout_secs = profile.advanced.query_timeout_secs.to_string();
        form.pg_timezone = profile.advanced.timezone;
        if form.name.trim().is_empty() {
            form.name = format!("{} - {}", database_kind_label(DatabaseKind::Postgres), form.host);
        }
        form.test_status = Some(ConnectionTestStatus::Success(
            tr("app.connection.toast.uri_imported").to_string(),
        ));
        // 表单值已更新，但输入框是独立实体，等下一次渲染统一回填。
        self.redis_discovery_pending_sync = true;
        cx.notify();
    }

    /// SQL 类型连接串导入：`mysql://user:pass@host:3306/db?k=v` 形式回填表单。
    /// 参数只在 MySQL 系列保留——其它驱动没有通用 DSN 参数入口，留空避免假配置。
    fn import_sql_connection_string(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.new_connection_kind else {
            return;
        };
        let raw = self.new_connection_form.discovery_uri.trim().to_string();
        let parsed = match parse_sql_connection_uri(&raw) {
            Ok(parsed) => parsed,
            Err(message) => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(message));
                cx.notify();
                return;
            }
        };
        let default_port = database_default_port(kind);
        let form = &mut self.new_connection_form;
        form.host = parsed.host;
        form.port = if parsed.port.is_empty() {
            default_port.to_string()
        } else {
            parsed.port
        };
        form.username = parsed.username;
        form.password = parsed.password;
        match kind {
            DatabaseKind::Oracle => form.oracle_service_name = parsed.database,
            DatabaseKind::Sqlite => {}
            _ => form.database = parsed.database,
        }
        if matches!(
            kind,
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb
        ) {
            form.url_params = parsed.params;
        }
        if form.name.trim().is_empty() {
            form.name = format!("{} - {}", database_kind_label(kind), form.host);
        }
        form.test_status = Some(ConnectionTestStatus::Success(
            tr("app.connection.toast.uri_imported").to_string(),
        ));
        // 表单值已更新，但输入框是独立实体，等下一次渲染统一回填。
        self.redis_discovery_pending_sync = true;
        cx.notify();
    }
}

/// SQL 连接串的解析结果（字段已做百分号解码）。
struct ParsedSqlConnectionUri {
    host: String,
    port: String,
    username: String,
    password: String,
    database: String,
    params: String,
}

/// 解析 `scheme://user:pass@host:port/database?k=v`。
///
/// 只做语法拆分，不判断 scheme 与当前数据库类型是否匹配；密码里的 `@` 用最后一个
/// `@` 切分，避免被当成主机分隔符。
fn parse_sql_connection_uri(raw: &str) -> Result<ParsedSqlConnectionUri, String> {
    let invalid = || tr("app.connection.error.uri_parse").to_string();
    let (_, rest) = raw.split_once("://").ok_or_else(invalid)?;
    let (rest, params) = match rest.split_once('?') {
        Some((rest, params)) => (rest, params),
        None => (rest, ""),
    };
    let (authority, database) = match rest.split_once('/') {
        Some((authority, database)) => (authority, database),
        None => (rest, ""),
    };
    let (userinfo, host_port) = match authority.rsplit_once('@') {
        Some((userinfo, host_port)) => (userinfo, host_port),
        None => ("", authority),
    };
    let (username, password) = match userinfo.split_once(':') {
        Some((username, password)) => (username, password),
        None => (userinfo, ""),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => (host, port),
        _ => (host_port, ""),
    };
    if host.trim().is_empty() {
        return Err(invalid());
    }
    Ok(ParsedSqlConnectionUri {
        host: host.to_string(),
        port: port.to_string(),
        username: percent_decode_uri_component(username),
        password: percent_decode_uri_component(password),
        database: percent_decode_uri_component(database),
        params: params.to_string(),
    })
}

/// 连接串里的用户名/密码/库名可能带 `%40` 等转义，回填前先解码。
fn percent_decode_uri_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&input[index + 1..index + 3], 16)
        {
            out.push(value);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod sql_connection_uri_tests {
    use super::{parse_sql_connection_uri, percent_decode_uri_component};

    #[test]
    fn parses_mysql_url_with_query_and_last_at_in_password() {
        let parsed =
            parse_sql_connection_uri("mysql://root:p%40ss@db.local:3307/app?charset=utf8mb4")
                .expect("合法连接串应能解析");
        assert_eq!(parsed.host, "db.local");
        assert_eq!(parsed.port, "3307");
        assert_eq!(parsed.username, "root");
        assert_eq!(parsed.password, "p@ss");
        assert_eq!(parsed.database, "app");
        assert_eq!(parsed.params, "charset=utf8mb4");
    }

    #[test]
    fn rejects_strings_without_host() {
        assert!(parse_sql_connection_uri("mysql://").is_err());
        assert!(parse_sql_connection_uri("127.0.0.1:3306/app").is_err());
    }

    #[test]
    fn decodes_percent_escapes_without_touching_plain_text() {
        assert_eq!(percent_decode_uri_component("a%2Fb"), "a/b");
        assert_eq!(percent_decode_uri_component("plain"), "plain");
    }
}
