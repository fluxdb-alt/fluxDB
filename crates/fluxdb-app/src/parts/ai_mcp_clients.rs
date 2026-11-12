/// 手动接入信息：端点 URL 与可直接粘贴的 JSON 示例（令牌只出现在示例里）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpConnectionDetails {
    pub url: String,
    pub json_example: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpClientsSnapshot {
    pub clients: Vec<McpInstallClientStatus>,
    pub connection: Option<McpConnectionDetails>,
    pub connection_error: Option<String>,
}

impl McpRegistration {
    pub fn connection_details(&self) -> fluxdb_core::Result<McpConnectionDetails> {
        self.validate()?;
        let json_example = serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {&self.server_name: self.http_entry()}
        }))
        .map_err(|_| write_error("生成 MCP 配置示例失败"))?;
        Ok(McpConnectionDetails {
            url: self.url.clone(),
            json_example,
        })
    }

    fn launch(&self) -> fluxdb_storage::McpClientLaunch {
        fluxdb_storage::McpClientLaunch {
            url: self.url.clone(),
            authorization: self.authorization(),
        }
    }
}

/// 把 Codex TOML 的 `[mcp_servers.<name>]` 条目转成 JSON 值，让两种配置格式
/// 共用同一份条目校验（`url` / `http_headers`）。
fn codex_entry_to_json(entry: &toml_edit::Item) -> fluxdb_core::Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    if let Some(url) = entry.get("url") {
        map.insert(
            "url".to_string(),
            url.as_str()
                .map(|text| serde_json::Value::String(text.to_string()))
                // 非字符串值同样要留下"该键存在"的痕迹，交由共享校验拒绝。
                .unwrap_or(serde_json::Value::Null),
        );
    }
    // 旧 stdio 字段仅用于识别本应用遗留条目（见 `is_legacy_fluxdb_stdio`）。
    if let Some(command) = entry.get("command") {
        map.insert(
            "command".to_string(),
            command
                .as_str()
                .map(|text| serde_json::Value::String(text.to_string()))
                .unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(args) = entry.get("args") {
        let items = args
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(|text| serde_json::Value::String(text.to_string()))
                            .unwrap_or(serde_json::Value::Null)
                    })
                    .collect()
            })
            .unwrap_or_default();
        map.insert("args".to_string(), serde_json::Value::Array(items));
    }
    if let Some(headers) = entry.get("http_headers") {
        let table = headers
            .as_table_like()
            .ok_or_else(|| write_error("MCP http_headers 不是表"))?;
        let mut json_headers = serde_json::Map::new();
        for (key, value) in table.iter() {
            json_headers.insert(
                key.to_string(),
                value
                    .as_str()
                    .map(|text| serde_json::Value::String(text.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );
        }
        map.insert(
            "headers".to_string(),
            serde_json::Value::Object(json_headers),
        );
    }
    Ok(serde_json::Value::Object(map))
}

/// 旧版 stdio 条目：启动命令来自 fluxdb 可执行文件（`fluxdb-mcp` 或桌面端 `--mcp-server`）。
///
/// 只用于识别本应用自己写过的槽位，避免把用户自建的同名服务当成可覆盖条目。
fn is_legacy_fluxdb_stdio(entry: &serde_json::Value) -> bool {
    let command = entry
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if command.contains("fluxdb") {
        return true;
    }
    entry
        .get("args")
        .and_then(|v| v.as_array())
        .is_some_and(|args| {
            args.iter()
                .filter_map(|arg| arg.as_str())
                .any(|arg| arg == "--mcp-server")
        })
}

/// 只提取端点与授权头用于归属核对；绝不把客户端整个配置带入状态或日志。
///
/// `None` 表示没有同名条目；同名条目不是 HTTP 服务时返回错误，
/// 由调用方展示原因而不是当作"未注册"。
fn client_launch(
    client: McpClientKind,
    home: &Path,
) -> fluxdb_core::Result<Option<fluxdb_storage::McpClientLaunch>> {
    let Some(text) = read_optional(&client.config_path(home))? else {
        return Ok(None);
    };
    if text.trim().is_empty() {
        return Ok(None);
    }
    let entry = match client {
        McpClientKind::Codex => {
            let value = text
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| write_error("客户端配置不是合法 TOML，未改写"))?;
            if value
                .get("mcp_servers")
                .is_some_and(|v| v.as_table_like().is_none())
            {
                return Err(write_error("mcp_servers 不是表，未改写"));
            }
            let Some(entry) = value
                .get("mcp_servers")
                .and_then(|v| v.get(DEFAULT_MCP_SERVER_NAME))
            else {
                return Ok(None);
            };
            Some(codex_entry_to_json(entry)?)
        }
        McpClientKind::ClaudeCode => {
            let value: serde_json::Value = serde_json::from_str(&text)
                .map_err(|_| write_error("客户端配置不是合法 JSON，未改写"))?;
            if !value.is_object() || value.get("mcpServers").is_some_and(|v| !v.is_object()) {
                return Err(write_error("客户端配置或 mcpServers 不是对象，未改写"));
            }
            value
                .get("mcpServers")
                .and_then(|v| v.get(DEFAULT_MCP_SERVER_NAME))
                .cloned()
        }
    };
    let Some(entry) = entry else {
        return Ok(None);
    };
    let Some(url) = entry.get("url").and_then(|v| v.as_str()) else {
        // 升级前写入的 stdio 条目（旧版本启动参数指向 fluxdb 可执行文件）：
        // 视为本应用的可接管槽位，按「未配置」展示，用户点安装即可换成 HTTP。
        if is_legacy_fluxdb_stdio(&entry) {
            return Ok(None);
        }
        return Err(write_error("同名 fluxdb 条目不是 HTTP 服务，未改写"));
    };
    // 头名大小写不敏感（HTTP 语义）。
    let authorization = entry
        .get("headers")
        .and_then(|v| v.as_object())
        .and_then(|headers| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("authorization"))
                .and_then(|(_, value)| value.as_str())
        })
        .ok_or_else(|| write_error("同名 fluxdb 条目缺少 Authorization 头，未改写"))?;
    Ok(Some(fluxdb_storage::McpClientLaunch {
        url: url.to_string(),
        authorization: authorization.to_string(),
    }))
}

/// 列出全部已适配客户端的展示行（不含注册判定，由调用方用权威解析结果填充）。
fn install_client_statuses(home: &Path) -> Vec<McpInstallClientStatus> {
    McpClientKind::all()
        .iter()
        .copied()
        .map(|client| {
            let config_path = client.config_path(home);
            McpInstallClientStatus {
                key: client.key().to_string(),
                label: client.label().to_string(),
                config_path: config_path.display().to_string(),
                config_exists: config_path.is_file(),
                registered: false,
                managed: false,
                up_to_date: false,
                error: None,
            }
        })
        .collect()
}

pub fn mcp_clients_snapshot(
    storage: &fluxdb_storage::FileStorage,
    home: &Path,
    registration: fluxdb_core::Result<McpRegistration>,
) -> fluxdb_core::Result<McpClientsSnapshot> {
    let managed = storage.load_mcp_client_registrations()?;
    let desired = registration.as_ref().ok().map(|r| r.launch());
    let clients = install_client_statuses(home)
        .into_iter()
        .map(|mut status| {
            status.managed = managed.contains_key(&status.key);
            if let Some(client) = client_from_key(&status.key) {
                match client_launch(client, home) {
                    Ok(current) => {
                        status.registered = current.is_some();
                        status.up_to_date = desired.is_some() && current == desired;
                        if let Some(expected) = managed.get(&status.key) {
                            if current.as_ref().is_some_and(|v| v != expected) {
                                status.error = Some(
                                    "同名配置已被修改，已停止自动维护；可重新安装覆盖为本应用命令".into(),
                                );
                            }
                        }
                    }
                    Err(error) => status.error = Some(error.message),
                }
            }
            status
        })
        .collect();
    let (connection, connection_error) = match registration.and_then(|r| r.connection_details()) {
        Ok(details) => (Some(details), None),
        Err(error) => (None, Some(error.message)),
    };
    Ok(McpClientsSnapshot {
        clients,
        connection,
        connection_error,
    })
}

/// 显式安装即接管客户端配置里的同名键：写入本应用当前实例的启动命令并登记管理权。
///
/// 同名键是本应用在客户端配置中的固定槽位，用户点击安装表达的是「这里归 fluxdb」，
/// 因此不再与上次记录比对；无法解析的配置和同名非 HTTP 条目（可能是用户自有的远端
/// 服务）仍然拒绝改写。
pub fn connect_mcp_client_registration(
    storage: &fluxdb_storage::FileStorage,
    home: &Path,
    client: McpClientKind,
    registration: &McpRegistration,
) -> fluxdb_core::Result<()> {
    registration.validate()?;
    let mut managed = storage.load_mcp_client_registrations()?;
    // 先解析同名条目：配置不可解析或同名条目不是 HTTP 服务时拒绝改写。
    client_launch(client, home)?;
    register(client, home, registration)?;
    managed.insert(client.key().into(), registration.launch());
    storage.save_mcp_client_registrations(&managed)
}

/// 撤销接入：删除同名键并放弃管理权，其它 server 与客户端原有配置不动。
pub fn remove_mcp_client_registration(
    storage: &fluxdb_storage::FileStorage,
    home: &Path,
    client: McpClientKind,
) -> fluxdb_core::Result<()> {
    let mut managed = storage.load_mcp_client_registrations()?;
    unregister(client, home, DEFAULT_MCP_SERVER_NAME)?;
    managed.remove(client.key());
    storage.save_mcp_client_registrations(&managed)
}

/// 启动只更新已选择、仍存在且与上次写入一致的条目，不接管手动配置。
pub fn sync_mcp_client_registrations(
    storage: &fluxdb_storage::FileStorage,
    home: &Path,
    registration: &McpRegistration,
) -> fluxdb_core::Result<Vec<String>> {
    let mut managed = storage.load_mcp_client_registrations()?;
    let mut changed = Vec::new();
    let mut errors = Vec::new();
    for client in McpClientKind::all().iter().copied() {
        let Some(expected) = managed.get(client.key()).cloned() else {
            continue;
        };
        let result = (|| {
            let Some(current) = client_launch(client, home)? else {
                // 用户在客户端移除了条目，不在后台重新添加。
                return Ok(());
            };
            if current != expected {
                return Err(write_error("同名配置已改变，未自动覆盖"));
            }
            let fact = register(client, home, registration)?;
            managed.insert(client.key().into(), registration.launch());
            if fact.changed {
                changed.push(client.label().to_string());
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(client = client.key(), "MCP 客户端维护失败，保留原配置");
            errors.push(format!("{}：{}", client.label(), error.message));
        }
    }
    storage.save_mcp_client_registrations(&managed)?;
    if !errors.is_empty() {
        return Err(write_error(errors.join("；")));
    }
    Ok(changed)
}

impl AppController {
    /// 客户端注册信息来自正在运行的应用内 MCP 端点；开关关闭时没有可写入的地址。
    fn mcp_registration(&self) -> fluxdb_core::Result<McpRegistration> {
        let slot = self
            .mcp_http
            .lock()
            .map_err(|_| write_error("MCP 服务状态不可用"))?;
        let Some(handle) = slot.as_ref() else {
            let start_error = self
                .mcp_http_error
                .lock()
                .ok()
                .and_then(|error| error.clone());
            return Err(write_error(
                start_error.unwrap_or_else(|| "本地 MCP 服务未开启，请先打开开关".into()),
            ));
        };
        let endpoint = handle.endpoint();
        let registration = McpRegistration {
            server_name: DEFAULT_MCP_SERVER_NAME.into(),
            url: endpoint.url.clone(),
            token: endpoint.token.clone(),
        };
        registration.validate()?;
        Ok(registration)
    }

    fn dispatch_mcp_clients(&mut self, command: AppCommand) -> AppEvent {
        let result = (|| {
            let storage = self.ai_storage()?;
            let home = home_dir()?;
            match command {
                AppCommand::RefreshMcpClients => {
                    // 首次打开时先初始化隔离存储根，命令示例与实际注册使用同一份端点。
                    storage.load_mcp_client_registrations()?;
                    let snapshot = mcp_clients_snapshot(&storage, &home, self.mcp_registration())?;
                    Ok(AppEvent::AiMcpClientsLoaded(snapshot))
                }
                AppCommand::SetMcpClientRegistration {
                    client_key,
                    registered,
                } => {
                    let client = client_from_key(&client_key)
                        .ok_or_else(|| write_error("未适配的客户端，请使用通用接入"))?;
                    if registered {
                        let registration = self.mcp_registration()?;
                        connect_mcp_client_registration(&storage, &home, client, &registration)?;
                    } else {
                        remove_mcp_client_registration(&storage, &home, client)?;
                    }
                    Ok(AppEvent::AiMcpClientsSynced {
                        clients: vec![client.label().into()],
                    })
                }
                AppCommand::SyncMcpClients => {
                    if storage.load_mcp_client_registrations()?.is_empty() {
                        return Ok(AppEvent::AiMcpClientsSynced { clients: vec![] });
                    }
                    // 启动时的后台维护：端点关闭时无事可做，不能因此弹出错误。
                    let Ok(registration) = self.mcp_registration() else {
                        return Ok(AppEvent::AiMcpClientsSynced { clients: vec![] });
                    };
                    let clients = sync_mcp_client_registrations(&storage, &home, &registration)?;
                    Ok(AppEvent::AiMcpClientsSynced { clients })
                }
                _ => unreachable!(),
            }
        })();
        result.unwrap_or_else(|error: fluxdb_core::Error| AppEvent::AiSettingsError(error.message))
    }
}

#[cfg(test)]
mod registration_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_home(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "fluxdb-mcp-clients-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).expect("create home");
        home
    }

    fn registration() -> McpRegistration {
        McpRegistration {
            server_name: DEFAULT_MCP_SERVER_NAME.into(),
            url: "http://127.0.0.1:47821/mcp".into(),
            token: "test-token".into(),
        }
    }

    #[test]
    fn registers_http_endpoint_and_detects_it_as_current() {
        let home = temp_home("http");
        let registration = registration();
        let expected = registration.launch();

        register(McpClientKind::Codex, &home, &registration).expect("register codex");
        register(McpClientKind::ClaudeCode, &home, &registration).expect("register claude");

        for client in McpClientKind::all() {
            let current = client_launch(*client, &home)
                .expect("read client config")
                .expect("entry must exist");
            assert_eq!(current, expected, "{} 应被识别为当前端点", client.key());
        }

        // 旧 stdio 字段必须被清掉，避免两套传输并存。
        let claude = std::fs::read_to_string(McpClientKind::ClaudeCode.config_path(&home))
            .expect("read claude config");
        let claude: serde_json::Value = serde_json::from_str(&claude).expect("json");
        let entry = &claude["mcpServers"][DEFAULT_MCP_SERVER_NAME];
        assert_eq!(entry["type"], "http");
        assert!(entry.get("command").is_none(), "不应残留 command");

        unregister(McpClientKind::ClaudeCode, &home, DEFAULT_MCP_SERVER_NAME)
            .expect("unregister claude");
        assert!(
            client_launch(McpClientKind::ClaudeCode, &home)
                .expect("read after unregister")
                .is_none()
        );
    }

    #[test]
    fn legacy_stdio_slot_is_takeover_ready() {
        let home = temp_home("legacy");
        std::fs::write(
            McpClientKind::ClaudeCode.config_path(&home),
            r#"{"mcpServers":{"fluxdb":{"type":"stdio","command":"/Applications/FluxDB.app/Contents/MacOS/fluxdb","args":["--mcp-server"]}}}"#,
        )
        .expect("seed legacy config");
        assert!(
            client_launch(McpClientKind::ClaudeCode, &home)
                .expect("legacy own entry must not be treated as foreign")
                .is_none()
        );

        // 显式安装必须能接管旧槽位，否则用户无法从 stdio 迁移到 HTTP。
        register(McpClientKind::ClaudeCode, &home, &registration())
            .expect("take over legacy slot");
        let current = client_launch(McpClientKind::ClaudeCode, &home)
            .expect("read after install")
            .expect("http entry");
        assert_eq!(current, registration().launch());
    }

    #[test]
    fn rejects_non_loopback_or_header_breaking_endpoints() {
        let mut registration = registration();
        registration.url = "http://10.0.0.1:47821/mcp".into();
        assert!(registration.validate().is_err(), "非回环地址必须拒绝");

        registration.url = "http://127.0.0.1:47821/mcp".into();
        registration.token = "a\r\nX: b".into();
        assert!(registration.validate().is_err(), "令牌不得破坏请求头");
    }
}
