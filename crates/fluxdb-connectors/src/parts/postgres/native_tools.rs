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
