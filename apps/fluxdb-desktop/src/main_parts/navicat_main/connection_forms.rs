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
            if matches!(event, fluxdb_app::AppEvent::ConnectionUpdated(_)) {
                self.open_connection_from_sidebar(config.id, cx);
            }
            self.saving_connection = false;
            cx.notify();
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
            self.open_connection_from_sidebar(config.id, cx);
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
        self.new_connection_inputs.password.update(cx, |input, cx| {
            input.set_masked(true, window, cx);
        });
        self.new_connection_inputs
            .focus_field(first_connection_field(kind), window, cx);
        cx.notify();
    }

    /// 打开新建/编辑连接弹框时，把表单里的 SSH 认证方式 / 云提供方同步到对应 Select 光标，
    /// 保证下拉与表单字段一致（SelectState 独立维护光标）。
    fn sync_new_connection_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 下拉值已与表单字段一致，不再经过展示文案转换。
        let ssh_auth = self.new_connection_form.ssh_auth.clone();
        let cloud_provider = self.new_connection_form.cloud_provider.clone();
        self.new_connection_inputs
            .ssh_auth_select
            .update(cx, |select, cx| {
                select.set_selected_value(&ssh_auth, window, cx);
            });
        self.new_connection_inputs
            .cloud_provider_select
            .update(cx, |select, cx| {
                select.set_selected_value(&cloud_provider, window, cx);
            });
        // SSL 模式下拉回填：选项值与表单字段值相同，直接同步光标。
        let pg_ssl_mode = self.new_connection_form.pg_tls_ssl_mode.clone();
        self.new_connection_inputs
            .pg_ssl_mode_select
            .update(cx, |select, cx| {
                select.set_selected_value(&pg_ssl_mode, window, cx);
            });
        let mysql_ssl_mode = self.new_connection_form.mysql_tls_ssl_mode.clone();
        self.new_connection_inputs
            .mysql_ssl_mode_select
            .update(cx, |select, cx| {
                select.set_selected_value(&mysql_ssl_mode, window, cx);
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
            ConnectionField::UrlParams => form.url_params = value,
            ConnectionField::SqlitePath => form.sqlite_path = value,
            ConnectionField::MongoAuthDb => form.mongo_auth_db = value,
            ConnectionField::TlsCa => form.tls_ca = value,
            ConnectionField::TlsClientCert => form.tls_client_cert = value,
            ConnectionField::TlsClientKey => form.tls_client_key = value,
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
            // —— MySQL / TiDB 专用 ——
            ConnectionField::MysqlCharset => form.mysql_charset = value,
            ConnectionField::MysqlProxyType => form.mysql_proxy_type = value,
            ConnectionField::MysqlSshConnectTimeout => form.mysql_ssh_connect_timeout_secs = value,
            ConnectionField::MysqlSshKeepalive => form.mysql_ssh_keepalive_secs = value,
            ConnectionField::MysqlProxyHost => form.mysql_proxy_host = value,
            ConnectionField::MysqlProxyPort => form.mysql_proxy_port = value,
            ConnectionField::MysqlProxyUsername => form.mysql_proxy_username = value,
            ConnectionField::MysqlProxyPassword => form.mysql_proxy_password = value,
            ConnectionField::MysqlConnectTimeout => form.mysql_connect_timeout_secs = value,
            ConnectionField::MysqlQueryTimeout => form.mysql_query_timeout_secs = value,
            ConnectionField::MysqlIdleTtl => form.mysql_idle_ttl_secs = value,
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
            ConnectionField::CloudSubscription => form.cloud_subscription = value,
            ConnectionField::CloudResource => form.cloud_resource = value,
            ConnectionField::DiscoveryUri => form.discovery_uri = value,
        }
        cx.notify();
    }

    /// 设置布尔开关字段的编辑值（TLS/SSH 启用、TLS 校验、Cluster 只读等）。
    fn set_connection_toggle_field(
        &mut self,
        field: ConnectionToggleField,
        value: bool,
        cx: &mut Context<Self>,
    ) {
        let form = &mut self.new_connection_form;
        form.test_status = None;
        match field {
            ConnectionToggleField::TlsEnabled => form.tls_enabled = value,
            ConnectionToggleField::TlsVerify => form.tls_verify = value,
            ConnectionToggleField::SshEnabled => form.ssh_enabled = value,
            ConnectionToggleField::ClusterAllowReadonly => form.cluster_allow_readonly = value,
            ConnectionToggleField::MysqlProxyEnabled => form.mysql_proxy_enabled = value,
            ConnectionToggleField::MysqlTcpKeepalive => form.mysql_tcp_keepalive = value,
            ConnectionToggleField::PgTcpKeepalive => form.pg_tcp_keepalive = value,
            ConnectionToggleField::PgShowOtherDatabases => form.pg_show_other_databases = value,
            ConnectionToggleField::PgShowSystemSchemas => form.pg_show_system_schemas = value,
            ConnectionToggleField::MongoSrv => form.mongo_srv = value,
            ConnectionToggleField::ReadOnly => form.read_only = value,
        }
        cx.notify();
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
            DatabaseKind::MySql | DatabaseKind::TiDb => {
                let profile = form.build_mysql_profile();
                if let Some(message) = profile.validate() {
                    return Err(message);
                }
            }
            // PostgreSQL 同样走结构化档案校验（TLS 模式与证书、SSH/代理、超时）。
            DatabaseKind::Postgres => {
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
                if !profile.basic.instance.trim().is_empty() {
                    return Err(tr("app.connection.sqlserver.instance_requires_port").to_string());
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
        if !form.url_params.trim().is_empty() {
            options.insert("url_params".to_string(), form.url_params.trim().to_string());
            // Redis 没有连接串，用同一个「参数」输入框承载 tls / tls_insecure /
            // tls_server_name / sentinel_master 这几个开关，逐个拆成独立选项。
            if kind == DatabaseKind::Redis {
                for (name, value) in redis_option_pairs(&form.url_params) {
                    options.insert(name, value);
                }
            }
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
            | DatabaseKind::Redis
            | DatabaseKind::Postgres
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

        // Redis 专用：把表单里的 TLS/SSH/Advanced 字段组装成结构化档案，
        // 作为拨号 / 保存的主来源（同时保留扁平 options 作兼容冗余）。
        let redis_profile = if kind == DatabaseKind::Redis {
            Some(form.build_redis_profile())
        } else {
            None
        };

        // MySQL/TiDB 专用：把表单里的 TLS/SSH/Proxy/Advanced 字段组装成结构化档案。
        let mysql_profile = if matches!(kind, DatabaseKind::MySql | DatabaseKind::TiDb) {
            Some(form.build_mysql_profile())
        } else {
            None
        };

        // PostgreSQL 专用：组装结构化档案——缺它 PG 连接无法拨号（连接器要求 profile）。
        let postgres_profile = if kind == DatabaseKind::Postgres {
            Some(form.build_postgres_profile())
        } else {
            None
        };
        let dameng_profile = if kind == DatabaseKind::Dameng {
            Some(form.build_dameng_profile())
        } else {
            None
        };
        // SQL Server 档案是拨号、加密模式与 schema 作用域的事实来源；
        // 密码仍在 storage 落盘前按连接 ID 写入加密凭据表。
        let sqlserver_profile = if kind == DatabaseKind::SqlServer {
            Some(form.build_sqlserver_profile())
        } else {
            None
        };
        // Oracle 档案是拨号目标（Service Name / SID）、登录身份与 Wallet 的事实来源；
        // 密码与 Wallet 口令在落盘前由 storage 按连接 ID 写入加密凭据表。
        let oracle_profile = if kind == DatabaseKind::Oracle {
            Some(form.build_oracle_profile())
        } else {
            None
        };

        ConnectionDraft {
            name: form.name.trim().to_string(),
            kind,
            endpoint,
            options,
            redis_profile,
            mysql_profile,
            postgres_profile,
            dameng_profile,
            sqlserver_profile,
            // MongoDB 档案是拨号事实来源：缺它连接器只能退回 endpoint 拼 URI，会丢 SRV/认证库。
            mongo_profile: if kind == DatabaseKind::MongoDb {
                Some(form.build_mongo_profile())
            } else {
                None
            },
            oracle_profile,
        }
    }

    /// Redis 连接串导入 / 云自动发现入口：把 URI 与当前云 provider 交给 app 层解析。
    /// 解析结果（`AppEvent::RedisConnectionDiscovered`）异步回流后回填表单（见下）。
    fn import_redis_connection_string(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.new_connection_kind else {
            return;
        };
        if kind != DatabaseKind::Redis {
            return;
        }
        let connection_string = self.new_connection_form.discovery_uri.clone();
        let provider = self.new_connection_form.cloud_provider.clone();
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
                    controller.dispatch(AppCommand::DiscoverRedisConnection {
                        provider,
                        connection_string,
                    })
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
            // 连接串里没有的云 provider 保留用户当前选择。
            let current_provider = self.new_connection_form.cloud_provider.clone();
            let form = &mut self.new_connection_form;
            if profile.cloud.provider.is_empty() && !current_provider.is_empty() {
                form.cloud_provider = current_provider;
            }
            form.apply_profile(profile);
            // 连接名缺省用解析出的名称；已有填写的名字保留。
            if form.name.trim().is_empty() {
                form.name = draft.name.clone();
            }
        }
        self.redis_discovery_pending_sync = true;
    }
}

/// 从 `key=value&key2=value2` 里挑出 Redis 认识的连接选项。
/// 只放行白名单里的键，避免把任意参数灌进连接配置。
fn redis_option_pairs(raw: &str) -> Vec<(String, String)> {
    // ai_key_prefixes：AI Redis 工具的 Key 授权前缀（逗号分隔），供参数授权校验使用。
    const ALLOWED: [&str; 5] = [
        "tls",
        "tls_insecure",
        "tls_server_name",
        "sentinel_master",
        "ai_key_prefixes",
    ];
    raw.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .filter(|(name, value)| ALLOWED.contains(&name.as_str()) && !value.is_empty())
        .collect()
}
