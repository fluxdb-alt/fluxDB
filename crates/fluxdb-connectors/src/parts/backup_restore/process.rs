fn task_error(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Internal, message)
}
fn io_error(error: impl std::fmt::Display) -> Error {
    task_error(error.to_string())
}
fn canceled(cancel: &AtomicBool) -> fluxdb_core::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(task_error("已取消；目标可能已部分写入，取消不等于回滚"));
    }
    Ok(())
}
fn report(progress: &mut dyn FnMut(DatabaseTaskProgress), stage: &str, message: impl Into<String>) {
    progress(DatabaseTaskProgress {
        stage: stage.into(),
        message: message.into(),
    });
}
/// 持有子进程直到退出；IO 错误和取消同样执行 kill + wait，避免遗留任务。
struct ManagedChild(std::process::Child);
impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn run_client(
    mut command: Command,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(DatabaseTaskProgress),
) -> fluxdb_core::Result<()> {
    canceled(cancel)?;
    command.stderr(Stdio::piped());
    let mut child = ManagedChild(command.spawn().map_err(io_error)?);
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or_else(|| task_error("无法读取客户端日志"))?;
    // 诊断只保留少量尾部行并截断，避免业务 SQL 或凭据进入持久化日志。
    // 继续排空管道，防止客户端在大输出时阻塞。
    let reader = std::thread::spawn(move || {
        let mut reader = stderr;
        let mut output = Vec::new();
        let mut chunk = [0; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if output.len() < 16 * 1024 {
                        output.extend_from_slice(&chunk[..n.min(16 * 1024 - output.len())]);
                    }
                }
            }
        }
        output
    });
    let started = Instant::now();
    let mut last = Instant::now();
    let outcome = loop {
        if let Err(error) = canceled(cancel) {
            break Err(error);
        }
        match child.0.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(task_error(format!(
                        "数据库客户端执行失败（{status}）；目标可能已部分写入"
                    )))
                };
            }
            Err(error) => break Err(io_error(error)),
            Ok(None) => {}
        }
        if last.elapsed() >= Duration::from_secs(1) {
            report(
                progress,
                "执行",
                format!("客户端运行中，已耗时 {} 秒", started.elapsed().as_secs()),
            );
            last = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    drop(child);
    let diagnostics = reader.join().unwrap_or_default();
    if !diagnostics.is_empty() {
        let message = sanitized_client_diagnostics(&diagnostics);
        if outcome.is_err() {
            return Err(task_error(format!(
                "{}；客户端诊断：{message}",
                outcome.unwrap_err().message
            )));
        }
        report(progress, "客户端", message);
    }
    outcome
}

/// 原生客户端 stderr 只取尾部短诊断，保留错误对象，不记录大段数据。
fn sanitized_client_diagnostics(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = Vec::new();
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut line = line.to_string();
        for keyword in ["password=", "pwd=", "token=", "secret=", "authorization="] {
            if let Some(position) = line.to_ascii_lowercase().find(keyword) {
                line.truncate(position + keyword.len());
                line.push_str("<redacted>");
                break;
            }
        }
        if line.len() > 240 {
            line.truncate(240);
            line.push_str("…");
        }
        lines.push(line);
        if lines.len() == 4 {
            break;
        }
    }
    if lines.is_empty() {
        return "客户端产生诊断输出，但没有可展示的错误摘要".into();
    }
    let message = lines.join(" / ");
    if message.len() > 700 {
        format!("{}…", &message[..message
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i <= 600)
            .last()
            .unwrap_or(0)])
    } else {
        message
    }
}
fn resolved(config: &ConnectionConfig) -> ConnectionConfig {
    match config.kind {
        DatabaseKind::Postgres => config.postgres_resolved(),
        DatabaseKind::MySql | DatabaseKind::TiDb => config.mysql_resolved(),
        _ => config.clone(),
    }
}
/// 原生工具共享连接传输。不能保持档案语义的组合明确拒绝，绝不悄悄降级 TLS。
fn native_connection(
    config: &ConnectionConfig,
) -> fluxdb_core::Result<(String, u16, String, String, Option<SshTunnel>)> {
    let config = resolved(config);
    let Endpoint::Tcp { host, port, .. } = &config.endpoint else {
        return Err(task_error("原生客户端需要 TCP 连接"));
    };
    let mut tunnel = None;
    if ssh_enabled(&config.options) {
        let auth = ssh_auth_from_options(&config.options, true)
            .ok_or_else(|| task_error("SSH 认证信息缺失"))?;
        let jump = config.options.get("ssh_host").cloned().unwrap_or_default();
        let jump_port = config
            .options
            .get("ssh_port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(22);
        tunnel = Some(open_tunnel_with(
            (&jump, jump_port),
            &auth,
            (host, *port),
            SshTunnelOptions {
                verify_host_key: true,
                ..Default::default()
            },
        )?);
    }
    let actual_port = tunnel.as_ref().map(|t| t.local_port).unwrap_or(*port);
    Ok((
        host.clone(),
        actual_port,
        config.options.get("username").cloned().unwrap_or_default(),
        config.options.get("password").cloned().unwrap_or_default(),
        tunnel,
    ))
}
fn mysql_tls(
    command: &mut Command,
    config: &ConnectionConfig,
    mariadb: bool,
    tunneled: bool,
) -> fluxdb_core::Result<()> {
    let profile = config
        .mysql_profile
        .clone()
        .unwrap_or_else(|| fluxdb_core::MysqlConnectionProfile::from_options(&config.options));
    if let Some(error) = profile.validate() {
        return Err(task_error(error));
    }
    let tls = &profile.tls;
    if !tls.sni.is_empty() || (tunneled && tls.enabled && tls.verify) {
        return Err(task_error(
            "原生 MySQL 工具暂不支持此 SNI / SSH 主机名校验组合，请使用可保持 TLS 校验的直连配置",
        ));
    }
    if mariadb {
        if tls.enabled {
            command.arg("--ssl");
            if tls.verify {
                command.arg("--ssl-verify-server-cert");
            }
        } else {
            command.arg("--skip-ssl");
        }
    } else {
        let mode = if !tls.enabled {
            "DISABLED"
        } else if tls.verify {
            "VERIFY_IDENTITY"
        } else {
            match tls.ssl_mode {
                fluxdb_core::MysqlSslMode::Disabled => "DISABLED",
                fluxdb_core::MysqlSslMode::Preferred => "PREFERRED",
                fluxdb_core::MysqlSslMode::Required => "REQUIRED",
            }
        };
        command.arg(format!("--ssl-mode={mode}"));
    }
    for (flag, secret) in [
        ("--ssl-ca", &tls.ca),
        ("--ssl-cert", &tls.client_cert),
        ("--ssl-key", &tls.client_key),
    ] {
        if let Some(value) = secret.value().filter(|v| !v.is_empty()) {
            command.arg(format!("{flag}={value}"));
        }
    }
    Ok(())
}
fn postgres_env(
    command: &mut Command,
    config: &ConnectionConfig,
    tunneled: bool,
) -> fluxdb_core::Result<()> {
    let profile = config
        .postgres_profile
        .clone()
        .unwrap_or_else(|| fluxdb_core::PostgresConnectionProfile::from_options(&config.options));
    if let Some(error) = profile.validate() {
        return Err(task_error(error));
    }
    if !profile.tls.server_name.is_empty() {
        return Err(task_error(
            "原生 PostgreSQL 工具暂不支持独立 TLS server_name",
        ));
    }
    for (key, secret) in [
        ("PGSSLROOTCERT", &profile.tls.ca),
        ("PGSSLCERT", &profile.tls.client_cert),
        ("PGSSLKEY", &profile.tls.client_key),
    ] {
        if let Some(value) = secret.value().filter(|v| !v.is_empty()) {
            command.env(key, value);
        }
    }
    if tunneled {
        command.env("PGHOSTADDR", "127.0.0.1");
    }
    command.env("PGCONNECT_TIMEOUT", "10");
    Ok(())
}

#[cfg(test)]
mod process_tests {
    use super::*;

    #[test]
    fn failure_includes_sanitized_client_diagnostics() {
        let mut command = Command::new("sh");
        command.arg("-c").arg("echo \"mysqldump: Couldn't execute test\" >&2; exit 2");
        let cancel = AtomicBool::new(false);

        let error = run_client(command, &cancel, &mut |_| {})
            .expect_err("非零退出必须失败");

        assert!(error.message.contains("exit status: 2"));
        assert!(error.message.contains("Couldn't execute test"));
    }

    #[test]
    fn diagnostics_are_truncated_and_redacted() {
        let bytes = b"password=should-not-leak
FIRST
SECOND
THIRD
FOURTH
FIFTH";
        let message = sanitized_client_diagnostics(bytes);

        assert!(!message.contains("should-not-leak"));
        assert!(message.contains("FIFTH"));
        assert!(message.contains("SECOND"));
    }
}
