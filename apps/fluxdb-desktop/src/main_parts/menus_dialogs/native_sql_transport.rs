fn resolved_credentials(config: &ConnectionConfig) -> (String, u16, String, String) {
    // PG 与 MySQL 凭据分属不同档案/options 键（PG 用 host/maintenance_database/username/password，
    // MySQL 用 root/剩下扁平键），按连接类型选择对应归一化，避免 PG 拿到 MySQL 默认 root。
    let resolved = match config.kind {
        DatabaseKind::Postgres => config.postgres_resolved(),
        _ => config.mysql_resolved(),
    };
    let (host, port) = match &resolved.endpoint {
        Endpoint::Tcp { host, port, .. } => (host.clone(), *port),
        Endpoint::SqliteFile { .. } => (String::new(), 0),
        Endpoint::Uri { .. } => (String::new(), 0),
    };
    let user = resolved
        .options
        .get("username")
        .cloned()
        .unwrap_or_else(|| {
            if config.kind == DatabaseKind::Postgres {
                String::new()
            } else {
                "root".to_string()
            }
        });
    let password = resolved
        .options
        .get("password")
        .cloned()
        .unwrap_or_default();
    (host, port, user, password)
}

/// 由 SSH 档案选项映射连接器隧道认证方式（私钥优先，其次密码，再次 agent）。
fn pg_ssh_auth_from_options(ssh: &fluxdb_core::PostgresSshOptions) -> fluxdb_app::SshTunnelAuth {
    let key_path = ssh.private_key.value().unwrap_or_default();
    if !key_path.trim().is_empty() {
        fluxdb_app::SshTunnelAuth::Key {
            private_key_path: key_path.to_string(),
        }
    } else if !ssh.password.value().unwrap_or_default().is_empty() {
        fluxdb_app::SshTunnelAuth::Password
    } else {
        fluxdb_app::SshTunnelAuth::Agent
    }
}

/// 选一个本地空闲端口（绑定 127.0.0.1:0 取系统分配端口后释放，供 ssh -L 使用）。
fn pg_pick_free_local_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| anyhow::anyhow!("无法分配本地端口：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| anyhow::anyhow!("读取本地端口失败：{error}"))?
        .port();
    drop(listener);
    Ok(port)
}

/// 等 SSH 隧道就绪：轮询连接 127.0.0.1:local_port，直到成功或超时/取消/隧道进程退出。
fn pg_wait_tunnel_ready(
    local_port: u16,
    tunnel_child: &mut std::process::Child,
    cancel_flag: &Arc<AtomicBool>,
    timeout_ms: u64,
) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if cancel_flag.load(Ordering::Relaxed) {
            return false;
        }
        if let Ok(Some(_)) = tunnel_child.try_wait() {
            return false; // 隧道进程已退出（建连失败）
        }
        if std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], local_port)),
            Duration::from_millis(300),
        )
        .is_ok()
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}

/// 建立 SSH 隧道子进程（供 pg_dump/psql 复用）：选空闲端口 → `ssh -N -L`（密码认证经 sshpass -e）
/// → 等就绪。返回 (隧道子进程, 本地端口)；调用方负责在工具结束后 kill+wait 回收。
pub fn pg_start_ssh_tunnel(
    ssh: &fluxdb_core::PostgresSshOptions,
    remote_host: &str,
    remote_port: u16,
    cancel_flag: &Arc<AtomicBool>,
) -> anyhow::Result<(std::process::Child, u16)> {
    let local_port = pg_pick_free_local_port()?;
    let auth = pg_ssh_auth_from_options(ssh);
    let tunnel = fluxdb_app::pg_ssh_tunnel_invocation(
        &ssh.host,
        ssh.port,
        &ssh.username,
        &auth,
        remote_host,
        remote_port,
        local_port,
        ssh.keepalive_interval_secs,
    );
    let password_auth = matches!(auth, fluxdb_app::SshTunnelAuth::Password);
    let mut tunnel_cmd = if password_auth {
        Command::new("sshpass")
    } else {
        Command::new(&tunnel.program)
    };
    if password_auth {
        tunnel_cmd.arg("-e").arg(&tunnel.program);
    }
    tunnel_cmd.args(&tunnel.args);
    if password_auth {
        let pass = ssh.password.value().unwrap_or_default();
        if !pass.is_empty() {
            tunnel_cmd.env("SSHPASS", pass);
        }
    }
    let mut child = tunnel_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| anyhow::anyhow!("启动 ssh 隧道失败：{error}"))?;
    if !pg_wait_tunnel_ready(local_port, &mut child, cancel_flag, 8000) {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("SSH 隧道建立失败或超时");
    }
    Ok((child, local_port))
}
