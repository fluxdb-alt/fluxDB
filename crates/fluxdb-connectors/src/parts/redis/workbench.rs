// Redis 命令执行器（Workbench）实现。
//
// 负责把用户在 Workbench 输入的文本切分成一条条 Redis 命令，
// 逐条在目标 DB 上执行，并把 RESP 回包映射为通用的 CommandReply 结构。
// 不依赖 GPUI，错误信息为中文、直接面向 UI 展示。

/// 把用户输入的整段文本切分成若干条 Redis 命令。
///
/// Redis CLI 习惯是一行一条命令；同时兼容用分号分隔的写法。
/// 空行、以及 `#` / `//` 开头的注释行会被忽略。返回非空命令的原始文本列表。
fn split_redis_commands(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                return Vec::new();
            }
            // 分号分隔多条命令；命令文本里可能含引号，这里不做引号感知，
            // 交由词法切分阶段去处理，仅按行/分号粗分。
            trimmed
                .split(';')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 把一条命令文本切分为 argv（命令字 + 参数）。
///
/// 支持单引号 / 双引号包裹的含空格参数，以及 `\` 转义下一个字符。
/// 返回空 Vec 表示命令为空。
fn tokenize_command(command: &str) -> Vec<String> {
    let mut argv = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(ch) = chars.next() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    in_token = true;
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                    in_token = true;
                }
                c if c.is_whitespace() => {
                    if in_token {
                        argv.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                _ => {
                    current.push(ch);
                    in_token = true;
                }
            },
        }
    }
    if quote.is_some() {
        // 引号未闭合：保留已收集内容，避免整条命令丢失；后续发送时按原样处理。
        current.push_str(" ");
    }
    if in_token || !current.is_empty() {
        argv.push(current);
    }
    argv
}

/// 命令是否需要在 Workbench 中禁用（防止误操作）。
/// 这类命令不被视为「普通用户可执行的应答命令」，返回 true 时跳过执行并提示。
fn is_unsupported_command(argv: &[String]) -> bool {
    let Some(command) = argv.first() else {
        return false;
    };
    let upper = command.to_ascii_uppercase();
    // SELECT 由执行目标决定库，Workbench 不直接切换；
    // SUBSCRIBE/PSUBSCRIBE/MONITOR 会阻塞连接，需要命令结果流式输出，暂不支持。
    matches!(
        upper.as_str(),
        "SELECT" | "SUBSCRIBE" | "PSUBSCRIBE" | "MONITOR"
    )
}

/// 把 RESP 值映射为通用的 CommandReply。递归展开 Array / Bulk。
fn redis_value_to_reply(value: RedisValue) -> CommandReply {
    match value {
        RedisValue::Simple(text) => CommandReply::Status(text),
        RedisValue::Int(number) => CommandReply::Integer(number),
        RedisValue::Bulk(None) => CommandReply::Nil,
        RedisValue::Bulk(Some(bytes)) => CommandReply::Bulk(build_command_bulk(bytes)),
        RedisValue::Array(items) => CommandReply::Array(
            items.into_iter().map(redis_value_to_reply).collect(),
        ),
    }
}

/// 把原始字节包装为 CommandBulk：优先按 UTF-8 文本呈现，不可解码时给 hex 预览。
fn build_command_bulk(bytes: Vec<u8>) -> CommandBulk {
    let byte_len = bytes.len() as u64;
    match String::from_utf8(bytes) {
        Ok(text) => {
            let binary = text.chars().any(|ch| {
                ch.is_control() && ch != '\n' && ch != '\r' && ch != '\t'
            });
            let (bytes_preview_hex, text) = if binary {
                (Some(hex_preview(&text.as_bytes())), None)
            } else {
                (None, Some(text))
            };
            CommandBulk {
                text,
                bytes_preview_hex,
                byte_len,
                binary,
            }
        }
        Err(error) => {
            let raw = error.into_bytes();
            CommandBulk {
                text: None,
                bytes_preview_hex: Some(hex_preview(&raw)),
                byte_len,
                binary: true,
            }
        }
    }
}

/// 生成不可解码内容的十六进制预览（最多 64 字节）。
fn hex_preview(bytes: &[u8]) -> String {
    const MAX: usize = 64;
    let shown = &bytes[..bytes.len().min(MAX)];
    let mut hex = shown
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if bytes.len() > MAX {
        hex.push_str("…");
    }
    hex
}

/// 当前 Unix 秒级时间戳（供每条执行记录的时间展示；源布局曾误用 elapsed 秒数，此处修正）。
fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

impl RedisConnector {
    /// 执行一次 Workbench 请求：对每条命令在目标 DB 上运行并收集结果。
    ///
    /// 单条命令失败（如 WRONGTYPE / 键不存在）不会中断后续命令；真正的
    /// IO / 连接错误才整体返回 Err。`continue_on_error` 控制是否跳过后续命令。
    pub fn execute_command_workbench(
        &self,
        request: &CommandWorkbenchRequest,
    ) -> fluxdb_core::Result<CommandWorkbenchExecution> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Workbench 需要连接配置上下文",
            ));
        };
        let target = match &request.target {
            CommandExecutionTarget::Redis { database, .. } => *database,
            _ => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "不支持的命令执行目标",
                ));
            }
        };
        let started_at = Instant::now();
        let commands = split_redis_commands(&request.text);
        let mut connection = redis_connect(config)?;

        let mut items = Vec::with_capacity(commands.len());
        let mut summary = CommandExecutionSummary {
            total: commands.len(),
            success: 0,
            failed: 0,
            skipped: 0,
        };
        let mut aborted = false;
        for raw in &commands {
            let argv = tokenize_command(raw);
            let command_started = Instant::now();

            let (status, reply) = if aborted {
                (CommandExecutionStatus::Skipped, CommandReply::Nil)
            } else if argv.is_empty() {
                (CommandExecutionStatus::Failed, CommandReply::Error("空命令".to_string()))
            } else if is_unsupported_command(&argv) {
                summary.failed += 1;
                (
                    CommandExecutionStatus::Failed,
                    CommandReply::Error(format!(
                        "命令「{}」在 Workbench 中暂不支持",
                        argv[0]
                    )),
                )
            } else {
                match self.execute_single_command(&mut connection, target, &argv) {
                    Ok(reply) => {
                        summary.success += 1;
                        (CommandExecutionStatus::Success, reply)
                    }
                    Err(error) => {
                        if !request.continue_on_error {
                            aborted = true;
                        }
                        summary.failed += 1;
                        (CommandExecutionStatus::Failed, CommandReply::Error(error.message.clone()))
                    }
                }
            };

            if status == CommandExecutionStatus::Skipped {
                summary.skipped += 1;
            }

            items.push(CommandExecutionItem {
                command: raw.clone(),
                argv_preview: argv,
                status,
                reply,
                elapsed_ms: command_started.elapsed().as_millis() as u64,
                size_limit_exceeded: false,
            });
        }

        Ok(CommandWorkbenchExecution {
            id: 0,
            target: request.target.clone(),
            text: request.text.clone(),
            commands: items,
            summary,
            run_mode: request.run_mode,
            results_mode: request.results_mode,
            started_at_unix_secs: unix_now_secs(),
            elapsed_ms: started_at.elapsed().as_millis() as u64,
        })
    }

    /// 按「执行单元 = 单条命令」的语义执行一次 Workbench 请求。
    ///
    /// 与 `execute_command_workbench`（聚合为一张总卡片）不同，这里把输入文本切分成
    /// 单条命令后，在同一个连接上逐条执行，并**每条命令生成一个独立的
    /// `CommandWorkbenchExecution`**（每条只含一条 `CommandExecutionItem`）。
    /// 返回的列表顺序即命令执行顺序，供 App 层逐条追加为历史卡片。
    /// 单条命令失败不会中断后续命令（各自独立成卡）。
    pub fn execute_command_workbench_commands(
        &self,
        request: &CommandWorkbenchRequest,
    ) -> fluxdb_core::Result<Vec<CommandWorkbenchExecution>> {
        let Some(config) = self.config.as_ref() else {
            return Err(Error::new(
                ErrorKind::Connection,
                "Redis Workbench 需要连接配置上下文",
            ));
        };
        let target = match request.target {
            CommandExecutionTarget::Redis { database, .. } => database,
            _ => {
                return Err(Error::new(
                    ErrorKind::Unsupported,
                    "不支持的命令执行目标",
                ));
            }
        };
        let commands = split_redis_commands(&request.text);
        // 复用单个连接逐条执行，避免每条命令都重新建立连接。
        let mut connection = redis_connect(config)?;
        let mut executions = Vec::with_capacity(commands.len());
        for raw in &commands {
            let argv = tokenize_command(raw);
            let command_started = Instant::now();
            let (status, reply) = if argv.is_empty() {
                (
                    CommandExecutionStatus::Failed,
                    CommandReply::Error("空命令".to_string()),
                )
            } else if is_unsupported_command(&argv) {
                (
                    CommandExecutionStatus::Failed,
                    CommandReply::Error(format!(
                        "命令「{}」在 Workbench 中暂不支持",
                        argv[0]
                    )),
                )
            } else {
                match self.execute_single_command(&mut connection, target, &argv) {
                    Ok(reply) => (CommandExecutionStatus::Success, reply),
                    Err(error) => (
                        CommandExecutionStatus::Failed,
                        CommandReply::Error(error.message.clone()),
                    ),
                }
            };
            let elapsed_ms = command_started.elapsed().as_millis() as u64;
            executions.push(CommandWorkbenchExecution {
                id: 0,
                target: request.target.clone(),
                text: raw.clone(),
                commands: vec![CommandExecutionItem {
                    command: raw.clone(),
                    argv_preview: argv,
                    status,
                    reply,
                    elapsed_ms,
                    size_limit_exceeded: false,
                }],
                summary: CommandExecutionSummary {
                    total: 1,
                    success: usize::from(status == CommandExecutionStatus::Success),
                    failed: usize::from(status == CommandExecutionStatus::Failed),
                    skipped: usize::from(status == CommandExecutionStatus::Skipped),
                },
                run_mode: request.run_mode,
                results_mode: request.results_mode,
                started_at_unix_secs: unix_now_secs(),
                elapsed_ms,
            });
        }
        Ok(executions)
    }

    /// 在指定 DB 上执行单条命令并映射回包。
    fn execute_single_command(
        &self,
        connection: &mut RedisConnection,
        database: u32,
        argv: &[String],
    ) -> fluxdb_core::Result<CommandReply> {
        redis_select(connection, database)?;
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        let value = connection.command(&args)?;
        Ok(redis_value_to_reply(value))
    }
}

#[cfg(test)]
mod workbench_tests {
    use super::*;

    #[test]
    fn split_commands_ignores_blank_and_comments() {
        let commands = split_redis_commands(
            "SET a 1\n\n# 注释\n// 也注释\nGET a;\nDEL a",
        );
        assert_eq!(commands, vec!["SET a 1", "GET a", "DEL a"]);
    }

    #[test]
    fn tokenize_handles_quoted_whitespace_args() {
        let argv = tokenize_command(r#"SET key "hello world" 'single'"#);
        assert_eq!(argv, vec!["SET", "key", "hello world", "single"]);
    }

    #[test]
    fn tokenize_handles_escaped_char() {
        let argv = tokenize_command(r#"SET k a\ b"#);
        assert_eq!(argv, vec!["SET", "k", "a b"]);
    }

    #[test]
    fn unsupported_commands_detected() {
        let argv = tokenize_command("SUBSCRIBE chan");
        assert!(is_unsupported_command(&argv));
        let argv = tokenize_command("SELECT 0");
        assert!(is_unsupported_command(&argv));
        let argv = tokenize_command("GET key");
        assert!(!is_unsupported_command(&argv));
    }

    #[test]
    fn bulk_binary_gets_hex_preview() {
        let bulk = build_command_bulk(vec![0x00, 0x01, 0x02, 0xff]);
        assert!(bulk.binary);
        assert!(bulk.text.is_none());
        assert_eq!(bulk.byte_len, 4);
        assert!(bulk.bytes_preview_hex.is_some());
    }

    #[test]
    fn bulk_utf8_stays_text() {
        let bulk = build_command_bulk("你好 redis".as_bytes().to_vec());
        assert!(!bulk.binary);
        assert_eq!(bulk.text.as_deref(), Some("你好 redis"));
        assert!(bulk.bytes_preview_hex.is_none());
    }
}
