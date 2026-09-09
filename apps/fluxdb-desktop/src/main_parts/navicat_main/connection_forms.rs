impl NavicatMain {
    fn confirm_delete_connection(&mut self, cx: &mut Context<Self>) {
        let Some(connection_id) = self.pending_delete_connection.take() else {
            return;
        };

        self.dispatch(AppCommand::DeleteConnection(connection_id), cx);
        let _ = self
            .storage
            .save_connections(&self.controller.connection_configs());
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
        if let Err(message) = self.validate_new_connection(kind) {
            self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(message));
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
                let has_credentials = existing.credential_ref.is_some()
                    || config.options.contains_key("password")
                    || config.redis_profile.as_ref().is_some_and(|profile| {
                        profile.basic.password.inline.is_some()
                            || profile.ssh.password.inline.is_some()
                            || profile.ssh.passphrase.inline.is_some()
                    })
                    || config.mysql_profile.as_ref().is_some_and(|profile| {
                        profile.basic.password.inline.is_some()
                            || profile.ssh().is_some_and(|ssh| {
                                ssh.password.inline.is_some() || ssh.passphrase.inline.is_some()
                            })
                            || profile.proxy().is_some_and(|proxy| proxy.password.inline.is_some())
                    });
                if has_credentials {
                    config.credential_ref = Some(format!("gdb.connection.{}", connection_id.0));
                }
            }
            let event = self
                .controller
                .dispatch(AppCommand::UpdateConnection(config.clone()));
            let _ = self
                .storage
                .save_connections(&self.controller.connection_configs());
            self.new_connection_kind = None;
            self.new_connection_password_visible = false;
            self.editing_connection_id = None;
            if matches!(event, fluxdb_app::AppEvent::ConnectionUpdated(_)) {
                self.open_connection_from_sidebar(config.id, cx);
            }
            cx.notify();
            return;
        }

        let test_config = draft.clone().into_config(fluxdb_core::ConnectionId(0));
        match self
            .controller
            .dispatch(AppCommand::TestConnection(test_config))
        {
            fluxdb_app::AppEvent::ConnectionTested(_, Ok(())) => {}
            fluxdb_app::AppEvent::ConnectionTested(_, Err(error)) => {
                self.new_connection_form.test_status =
                    Some(ConnectionTestStatus::Error(error.to_string()));
                cx.notify();
                return;
            }
            fluxdb_app::AppEvent::Failed(error) => {
                self.new_connection_form.test_status = Some(ConnectionTestStatus::Error(format!(
                    "{}：{}",
                    error.title, error.message
                )));
                cx.notify();
                return;
            }
            _ => {
                self.new_connection_form.test_status =
                    Some(ConnectionTestStatus::Error("连接失败，未保存".to_string()));
                cx.notify();
                return;
            }
        }
        let event = self
            .controller
            .dispatch(AppCommand::CreateConnection(draft));
        let _ = self
            .storage
            .save_connections(&self.controller.connection_configs());
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
        // 表单字段值 → 下拉展示文案。
        let ssh_auth = match self.new_connection_form.ssh_auth.as_str() {
            "private_key" => "私钥".to_string(),
            _ => "密码".to_string(),
        };
        let cloud_provider = match self.new_connection_form.cloud_provider.as_str() {
            "azure" => "Azure".to_string(),
            "redis-cloud" => "Redis Cloud".to_string(),
            _ => "不使用云".to_string(),
        };
        self.new_connection_inputs.ssh_auth_select.update(cx, |select, cx| {
            select.set_selected_value(&ssh_auth, window, cx);
        });
        self.new_connection_inputs.cloud_provider_select.update(cx, |select, cx| {
            select.set_selected_value(&cloud_provider, window, cx);
        });
    }

    fn cancel_new_connection(&mut self, cx: &mut Context<Self>) {
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
            // —— MySQL / TiDB 专用 ——
            ConnectionField::MysqlTlsSslMode => form.mysql_tls_ssl_mode = value,
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
        }
        cx.notify();
    }

    fn choose_sqlite_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 SQLite 数据库文件".into()),
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
                            format!("选择 SQLite 文件失败：{error}"),
                        ));
                        cx.notify();
                    }
                    Err(error) => {
                        this.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                            format!("选择 SQLite 文件失败：{error}"),
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
        prompt: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(prompt.into()),
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
                            format!("选择文件失败：{error}"),
                        ));
                        cx.notify();
                    }
                    Err(error) => {
                        this.new_connection_form.test_status = Some(ConnectionTestStatus::Error(
                            format!("选择文件失败：{error}"),
                        ));
                        cx.notify();
                    }
                });
            })
            .ok();
        }));
    }

    fn test_new_connection(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.new_connection_kind else {
            return;
        };
        match self.validate_new_connection(kind) {
            Ok(()) => {
                let config = self
                    .new_connection_draft(kind)
                    .into_config(fluxdb_core::ConnectionId(0));
                self.new_connection_form.test_status =
                    Some(ConnectionTestStatus::Pending("正在测试连接...".to_string()));
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
                                    ConnectionTestStatus::Success("测试连接成功".to_string())
                                }
                                fluxdb_app::AppEvent::ConnectionTested(_, Err(error)) => {
                                    ConnectionTestStatus::Error(error.to_string())
                                }
                                fluxdb_app::AppEvent::Failed(error) => ConnectionTestStatus::Error(
                                    format!("{}：{}", error.title, error.message),
                                ),
                                _ => ConnectionTestStatus::Error("测试连接失败".to_string()),
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
            return Err("请填写连接名称".to_string());
        }
        match kind {
            DatabaseKind::Sqlite => {
                if form.sqlite_path.trim().is_empty() {
                    return Err("请选择或输入 SQLite 文件路径".to_string());
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
            _ => {
                if form.host.trim().is_empty() {
                    return Err("请填写主机".to_string());
                }
                parse_port(&form.port)?;
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
            DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::Redis => Endpoint::Tcp {
                host: form.host.trim().to_string(),
                port: form
                    .port
                    .parse()
                    .unwrap_or_else(|_| database_default_port_u16(kind)),
                database: non_empty_option(&form.database),
            },
            DatabaseKind::Sqlite => Endpoint::SqliteFile {
                path: form.sqlite_path.trim().into(),
                read_only: false,
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

        ConnectionDraft {
            name: form.name.trim().to_string(),
            kind,
            endpoint,
            credential_ref: None,
            options,
            redis_profile,
            mysql_profile,
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
            self.new_connection_form.test_status =
                Some(ConnectionTestStatus::Error("请先输入 Redis 连接串".to_string()));
            cx.notify();
            return;
        }
        self.new_connection_form.test_status =
            Some(ConnectionTestStatus::Pending("正在解析连接串...".to_string()));
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
                                    "已导入 Redis 连接串".to_string(),
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
                                Some(ConnectionTestStatus::Error("连接串解析失败".to_string()));
                        }
                    }
                    this._redis_discover_task = None;
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }

    /// 把连接串解析出的草稿（携带 `redis_profile`）回填进新建/编辑表单。
    ///
    /// 注意：异步回调里拿不到 `Window`，此处只更新表单数据 + 打「待同步输入框」标记；
    /// 真正的输入框实体同步放在 `NavicatMain::render`（有 `Window`）里消费该标记。
    fn apply_redis_discovered_draft(
        &mut self,
        draft: &ConnectionDraft,
        _cx: &mut Context<Self>,
    ) {
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
    const ALLOWED: [&str; 4] = ["tls", "tls_insecure", "tls_server_name", "sentinel_master"];
    raw.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .filter(|(name, value)| ALLOWED.contains(&name.as_str()) && !value.is_empty())
        .collect()
}
