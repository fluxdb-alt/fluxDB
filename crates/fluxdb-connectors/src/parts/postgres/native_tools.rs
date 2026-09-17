// PostgreSQL 原生脚本（T24）：检测需要 psql 的脚本 + 安全构造 psql 调用参数。
//
// 含 `COPY ... FROM STDIN` 数据块或 psql 元命令（`\copy`、`\set` 等）的 pg_dump 文本不能按分号
// 拆分执行；本模块只做「检测」与「参数构造」两件纯逻辑（可单测），实际子进程执行在 app 层，
// 且 psql 不存在/版本不符需给清晰错误（见集中人工验收）。密码绝不出现在 argv，只经环境变量。

use fluxdb_core::PostgresSslMode;

/// 脚本是否需要 psql 原生模式：存在 `COPY ... FROM STDIN`（数据块以 `\.` 结束）或 psql 元命令。
///
/// 为纯函数便于单测：单引号字符串、双引号标识符、行/块注释内容先被抹成空白，再在「代码掩码」上
/// 检测 psql 元命令与 `COPY ... FROM STDIN`，避免被字符串/注释里的假象误导。
pub fn pg_script_needs_native_mode(script: &str) -> bool {
    let masked = pg_mask_strings_and_comments(script);
    pg_masked_has_meta_command(&masked) || pg_masked_has_copy_stdin(&masked)
}

/// 把字符串/标识符/注释内容替换为空格（保留换行与真实代码字符），返回等长掩码。
fn pg_mask_strings_and_comments(script: &str) -> Vec<u8> {
    let bytes = script.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment: u32 = 0;
    let blank = |masked: &mut Vec<u8>, at: usize| {
        if masked[at] != b'\n' && masked[at] != b'\r' {
            masked[at] = b' ';
        }
    };
    while index < bytes.len() {
        let byte = bytes[index];
        if in_line_comment {
            if byte == b'\n' {
                in_line_comment = false;
            } else {
                blank(&mut masked, index);
            }
            index += 1;
            continue;
        }
        if in_block_comment > 0 {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                blank(&mut masked, index);
                blank(&mut masked, index + 1);
                in_block_comment -= 1;
                index += 2;
                continue;
            }
            if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
                in_block_comment += 1;
                blank(&mut masked, index);
                blank(&mut masked, index + 1);
                index += 2;
                continue;
            }
            blank(&mut masked, index);
            index += 1;
            continue;
        }
        if in_single {
            blank(&mut masked, index);
            if byte == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    blank(&mut masked, index + 1);
                    index += 2;
                    continue;
                }
                in_single = false;
            }
            index += 1;
            continue;
        }
        if in_double {
            blank(&mut masked, index);
            if byte == b'"' {
                in_double = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                in_line_comment = true;
                blank(&mut masked, index);
                blank(&mut masked, index + 1);
                index += 2;
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                in_block_comment += 1;
                blank(&mut masked, index);
                blank(&mut masked, index + 1);
                index += 2;
                continue;
            }
            b'\'' => {
                in_single = true;
                blank(&mut masked, index);
            }
            b'"' => {
                in_double = true;
                blank(&mut masked, index);
            }
            _ => {}
        }
        index += 1;
    }
    masked
}

/// 掩码上检测 psql 元命令：某行首个非空白字符是 `\`。
fn pg_masked_has_meta_command(masked: &[u8]) -> bool {
    let text = String::from_utf8_lossy(masked);
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with('\\')
    })
}

/// 掩码上检测 `COPY ... FROM STDIN`（大小写不敏感，跨空白/换行；`from stdin` 须在同一语句内）。
fn pg_masked_has_copy_stdin(masked: &[u8]) -> bool {
    let lowered = String::from_utf8_lossy(masked).to_ascii_lowercase();
    let mut rest = lowered.as_str();
    while let Some(position) = rest.find("copy") {
        let after = &rest[position..];
        if let Some(from_index) = after.find("from stdin") {
            if !after[..from_index].contains(';') {
                return true;
            }
        }
        rest = &rest[position + 4..];
    }
    false
}

/// psql 调用参数（用于 `Command`，不经 shell）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgPsqlInvocation {
    /// 可执行文件（默认 `psql`）。
    pub program: String,
    /// argv（不含可执行文件），如 `--no-psqlrc -v ON_ERROR_STOP=1 -h host -p 5432 -U user -d db -f file`。
    pub args: Vec<String>,
    /// 需注入的环境变量（PGPASSWORD 等）；不进入 argv，避免泄漏到进程列表。
    pub env: Vec<(String, String)>,
}

/// 构造 psql 调用：非交互 + 可选继续错误 + 目标库 + TLS 模式；密码只在 env，绝不进 argv（设计 §11.2）。
///
/// `ssl_mode` 决定是否追加 `sslmode=` 选项（disable/require/verify-ca/verify-full）；
/// `Prefer`（PG 默认，缺省先尝试 SSL）不显式传参。注意：本函数只按直连/TLS 语义生成连接参数，
/// SSH 隧道场景由调用方另建隧道并把 `host` 指向隧道本地端口（SSH+TLS 原生工具链待人工验证）。
pub fn pg_psql_invocation(
    host: &str,
    port: u16,
    user: &str,
    database: &str,
    script_path: &str,
    password: Option<&str>,
    stop_on_error: bool,
    ssl_mode: PostgresSslMode,
) -> PgPsqlInvocation {
    let args = vec![
        "--no-psqlrc".to_string(),
        "-q".to_string(),
        "-v".to_string(),
        if stop_on_error {
            "ON_ERROR_STOP=1".to_string()
        } else {
            "ON_ERROR_STOP=off".to_string()
        },
        "-h".to_string(),
        host.to_string(),
        "-p".to_string(),
        port.to_string(),
        "-U".to_string(),
        user.to_string(),
        "-d".to_string(),
        database.to_string(),
        "-f".to_string(),
        script_path.to_string(),
    ];
    // 密码只走 env；TLS 模式同样只走 env（PGSSLMODE，psql/pg_dump 都认），不进 argv——
    // psql/pg_dump 无 `--sslmode` CLI 开关，`-c sslmode=..` 会被当作用户 SQL 而非连接参数。
    let mut env = Vec::new();
    if let Some(mode) = pg_sslmode_value(ssl_mode) {
        env.push(("PGSSLMODE".to_string(), mode.to_string()));
    }
    if let Some(password) = password.filter(|password| !password.is_empty()) {
        env.push(("PGPASSWORD".to_string(), password.to_string()));
    }
    PgPsqlInvocation {
        program: "psql".to_string(),
        args,
        env,
    }
}

/// 把连接档案的 TLS 模式映射为 psql/pg_dump 的 `sslmode` 值；`Prefer` 返回 None（省略）。
pub fn pg_sslmode_value(ssl_mode: PostgresSslMode) -> Option<&'static str> {
    match ssl_mode {
        PostgresSslMode::Disabled => Some("disable"),
        PostgresSslMode::Require => Some("require"),
        PostgresSslMode::VerifyCa => Some("verify-ca"),
        PostgresSslMode::VerifyFull => Some("verify-full"),
        // Prefer 是服务端默认，显式传属冗余；省略等价。
        PostgresSslMode::Prefer => None,
    }
}

/// pg_dump 调用参数（用于 `Command`，不经 shell）。密码只经 `PGPASSWORD` 环境变量。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PgDumpInvocation {
    /// 可执行文件（默认 `pg_dump`，可配绝对路径）。
    pub program: String,
    /// argv（不含可执行文件）。
    pub args: Vec<String>,
    /// 需注入的环境变量（PGPASSWORD/PGSSLMODE）。
    pub env: Vec<(String, String)>,
}

/// pg_dump 备份范围：结构/数据/完整。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PgDumpScope {
    /// 仅结构（`--schema-only`）。
    SchemaOnly,
    /// 仅数据（`--data-only`）。
    DataOnly,
    /// 结构与数据（默认）。
    Full,
}

/// 构造 pg_dump 调用（plain + inserts 单文件 .sql，可由 psql 恢复）。
///
/// - `scope`：结构/数据/完整（`--schema-only`/`--data-only`/默认）。
/// - `owner`/`acl`：是否导出属主/ACL；为 false 时加 `--no-owner`/`--no-acl`（默认不导出属主/ACL，
///   使备份可在其它环境恢复；显式勾选才保留）。
/// - `tables`：非空按 `-t schema.table` 只导出选中表（空=整库）。
/// - 密码与 TLS 模式只经 env（PGPASSWORD/PGSSLMODE），绝不进 argv。
#[allow(clippy::too_many_arguments)]
pub fn pg_dump_invocation(
    program: &str,
    host: &str,
    port: u16,
    user: &str,
    database: &str,
    password: Option<&str>,
    ssl_mode: PostgresSslMode,
    scope: PgDumpScope,
    owner: bool,
    acl: bool,
    tables: &[String],
) -> PgDumpInvocation {
    let mut args = vec![
        "--format=plain".to_string(),
        "--inserts".to_string(),
        "-h".to_string(),
        host.to_string(),
        "-p".to_string(),
        port.to_string(),
        "-U".to_string(),
        user.to_string(),
        "-d".to_string(),
        database.to_string(),
    ];
    match scope {
        PgDumpScope::SchemaOnly => args.push("--schema-only".to_string()),
        PgDumpScope::DataOnly => args.push("--data-only".to_string()),
        PgDumpScope::Full => {}
    }
    // 默认不导出属主/ACL（便于跨环境恢复）；显式勾选才保留。
    if !owner {
        args.push("--no-owner".to_string());
    }
    if !acl {
        args.push("--no-acl".to_string());
    }
    for table in tables {
        args.push("-t".to_string());
        args.push(table.clone());
    }
    let mut env = Vec::new();
    if let Some(mode) = pg_sslmode_value(ssl_mode) {
        env.push(("PGSSLMODE".to_string(), mode.to_string()));
    }
    if let Some(password) = password.filter(|password| !password.is_empty()) {
        env.push(("PGPASSWORD".to_string(), password.to_string()));
    }
    PgDumpInvocation {
        program: program.to_string(),
        args,
        env,
    }
}

/// 校验 pg_dump 客户端主版本不低于服务器主版本（PG 禁止用更旧的 pg_dump 备份新服务器）。
///
/// 入参见 `pg_dump --version` 与 `SHOW server_version` 解析出的主版本号（如 `16.15` → 16）。
/// 任一侧未知（None）时不做判断（放行，避免误拦），返回 true。
pub fn pg_dump_version_compatible(tool_major: Option<u32>, server_major: Option<u32>) -> bool {
    match (tool_major, server_major) {
        (Some(tool), Some(server)) => tool >= server,
        _ => true,
    }
}

/// 从 `pg_dump --version` 输出解析主版本号，如 `pg_dump (PostgreSQL) 16.15` → 16。
pub fn pg_tool_major_version(version_output: &str) -> Option<u32> {
    let digits: String = version_output
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.split('.').next().and_then(|major| major.parse().ok())
}

/// 读取服务端主版本号（`SHOW server_version` 首段数字，如 `16.15` → 16），供原生备份前
/// 校验 pg_dump 客户端不低于服务端主版本（PG 禁止用更旧客户端备份）。
pub fn pg_server_major_version(config: &ConnectionConfig) -> fluxdb_core::Result<Option<u32>> {
    pg_runtime().block_on(async {
        let session = pg_connect(config, &pg_request_database(config, None)).await?;
        let row = session
            .client
            .query_one("SELECT current_setting('server_version')", &[])
            .await
            .map_err(pg_error)?;
        let version: String = row.get(0);
        Ok(pg_tool_major_version(&version))
    })
}


/// 供 pg_dump/psql 等原生客户端持有的 SSH 隧道句柄。
///
/// 句柄存活期间本地端口可用；Drop 会停止监听并等待桥线程退出。具体 SSH 实现保持在
/// connectors 内部，应用层不需要安装 `ssh`/`sshpass`，也不会接触连接器内部会话类型。
pub struct PgNativeSshTunnel {
    tunnel: SshTunnel,
}

impl PgNativeSshTunnel {
    pub fn local_port(&self) -> u16 {
        self.tunnel.local_port
    }
}

/// 使用与 PostgreSQL 驱动相同的 libssh2、认证、known_hosts、超时和心跳规则建立隧道。
pub fn pg_open_native_ssh_tunnel(
    ssh: &fluxdb_core::PostgresSshOptions,
    target_host: &str,
    target_port: u16,
    inherited_connect_timeout_secs: u32,
) -> fluxdb_core::Result<PgNativeSshTunnel> {
    let auth = pg_ssh_auth(ssh);
    let options = SshTunnelOptions {
        connect_timeout_secs: if ssh.connect_timeout_secs > 0 {
            ssh.connect_timeout_secs
        } else {
            inherited_connect_timeout_secs
        },
        keepalive_interval_secs: ssh.keepalive_interval_secs,
        verify_host_key: true,
    };
    let tunnel = open_tunnel_with(
        (&ssh.host, ssh.port),
        &auth,
        (target_host, target_port),
        options,
    )?;
    Ok(PgNativeSshTunnel { tunnel })
}

/// SSH 隧道子进程调用参数：把远端 `host:port` 经 `ssh -L local:host:port` 映射到本地端口，
/// 供 psql/pg_dump 等原生工具在 SSH 下连接（工具看到的是 `127.0.0.1:local_port`）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshTunnelInvocation {
    /// `ssh` 可执行文件。
    pub program: String,
    /// argv（不含可执行文件）。
    pub args: Vec<String>,
    /// 需注入的环境变量（如 SSHPASS 由调用方经 sshpass 传入；本构造不直接放密码）。
    pub env: Vec<(String, String)>,
    /// 本地监听端口（远端映射到这个端口，供 psql/pg_dump 连接）。
    pub local_port: u16,
}

/// SSH 认证方式（构建 `ssh` 参数用）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SshTunnelAuth {
    /// 公钥认证：`ssh -i <keyfile>`。
    Key { private_key_path: String },
    /// 密码认证：需外部 sshpass 注入 SSHPASS（`ssh` 自身不接受明文密码 argv）。
    Password,
    /// 无认证（依赖 ssh-agent / 默认密钥）。
    Agent,
}

/// 构造 `ssh -N -L local:target -p port [-i key] user@jump` 隧道调用（不经 shell）。
///
/// `local_port` 由调用方（选择一个空闲端口）决定；工具改用 `127.0.0.1:local_port` 连接。
/// 密码认证不在此注入密码（ssh 不支持 argv 密码），由调用方经 sshpass+SSHPASS 环境变量处理，
/// 本函数仅标记需要密码认证（返回 `Password` 时调用方须自行注入）。保持通道至子进程结束由
/// 调用方负责（子进程存活期间不 kill）。
#[allow(clippy::too_many_arguments)]
pub fn pg_ssh_tunnel_invocation(
    ssh_host: &str,
    ssh_port: u16,
    ssh_user: &str,
    auth: &SshTunnelAuth,
    target_host: &str,
    target_port: u16,
    local_port: u16,
    keepalive_interval_secs: u32,
) -> SshTunnelInvocation {
    let mut args = vec![
        "-N".to_string(),
        "-L".to_string(),
        format!("{local_port}:{target_host}:{target_port}"),
        "-p".to_string(),
        ssh_port.to_string(),
        // 非交互：失败即退，避免等待输入卡住；已知主机首次接受（与项目 SSH 既有策略一致）。
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
    ];
    if keepalive_interval_secs > 0 {
        args.push("-o".to_string());
        args.push(format!("ServerAliveInterval={keepalive_interval_secs}"));
    }
    if let SshTunnelAuth::Key { private_key_path } = auth {
        args.push("-i".to_string());
        args.push(private_key_path.clone());
    }
    if matches!(auth, SshTunnelAuth::Password) {
        // 密码认证需 sshpass；在此关闭 BatchMode 让 ssh 允许密码（由 sshpass 喂入）。
        // 注：password 经 SSHPASS 环境变量，不落 argv。
        if let Some(index) = args.iter().position(|a| a == "BatchMode=yes") {
            args[index] = "BatchMode=no".to_string();
        }
    }
    args.push(format!("{ssh_user}@{ssh_host}"));
    SshTunnelInvocation {
        program: "ssh".to_string(),
        args,
        env: Vec::new(),
        local_port,
    }
}

/// SSH/隧道场景的 libpq 拨号地址分离：`PGHOSTADDR` 指定实际拨号地址（隧道本地 127.0.0.1），
/// 而 `-h`/`host` 仍为真实远端主机名供 TLS 校验（设计 §11.3：host/hostaddr 分别保持证书身份与拨号地址）。
pub fn pg_hostaddr_env(hostaddr: &str, port: u16) -> Vec<(String, String)> {
    vec![
        ("PGHOSTADDR".to_string(), hostaddr.to_string()),
        ("PGPORT".to_string(), port.to_string()),
    ]
}
