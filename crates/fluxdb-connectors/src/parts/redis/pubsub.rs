// Redis Pub/Sub 独立会话能力。
//
// 与 Workbench 的「一次性命令」不同，订阅 / 发布本质是**一条长连接上的流式交互**：
// 一旦 `SUBSCRIBE` 进入订阅态，后续 `PUBLISH` 的消息会以 push 形式持续到达。
// 本模块把「一条专用连接 + 待发命令队列 + 消息读取」封装成一个独立会话，
// 与普通命令执行器解耦——这正是设计文档「Pub/Sub 是独立会话，不混入 Workbench」的落点。
//
// 会话由桌面层持有，周期调用 `poll()` 推进一次：
//   1. 先写出已排队的 subscribe / unsubscribe / publish 命令；
//   2. 再读取当前 socket 上可用的回包（订阅确认 + push 消息），直到读超时。
//   连接与消息状态都在会话内部维护，UI 只负责呈现结果与触发命令入队。
// 会话生命周期随对应 tab 的创建 / 关闭而建立 / 销毁；连接不进入普通连接池
// （订阅连接不能复用），随会话 drop 直接关闭。

// 本文件经 `include!` 并入 crate-root 作用域，Error / ErrorKind / ConnectionConfig / Duration /
// Mutex 已由 lib.rs / 其它 redis 子模块导入，此处仅补充 Arc（crate-root 未导入）。
use std::sync::Arc;

/// Pub/Sub 命令入队：从多线程（UI 线程）把待发命令写进共享队列，
/// 由 `poll` 所在的读取线程统一写出，避免 socket 读写竞态。
type CommandQueue = Arc<Mutex<Vec<String>>>;

/// 一次 `poll` 的产出：订阅状态变更 + 收到的消息。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PubSubPollOutcome {
    /// 订阅 / 取消订阅确认（含通道号与剩余订阅数）。
    pub subscriptions: Vec<PubSubSubscriptionEvent>,
    /// 收到的广播消息（普通 channel 或 pattern）。
    pub messages: Vec<PubSubMessage>,
    /// `PUBLISH` 命令的返回（收到该结果的订阅者数量）。
    pub published: Vec<PubSubPublishResult>,
    /// 命令 / 协议层错误（不影响会话存活）。
    pub errors: Vec<String>,
}

/// 订阅态变更事件。
#[derive(Clone, Debug, PartialEq)]
pub struct PubSubSubscriptionEvent {
    /// true 表示 pattern 订阅（PSUBSCRIBE / PUNSUBSCRIBE）。
    pub pattern: bool,
    /// 是否订阅成功（false 表示取消订阅）。
    pub subscribed: bool,
    /// 通道名或 pattern 名。
    pub channel: String,
    /// 当前剩余订阅数（含 pattern）。
    pub count: i64,
}

/// 一条 Pub/Sub 广播消息。
#[derive(Clone, Debug, PartialEq)]
pub struct PubSubMessage {
    /// 命中的 pattern（普通订阅时为空）。
    pub pattern: Option<String>,
    /// 实际发布所在的通道名。
    pub channel: String,
    /// 消息载荷（原始字节，UI 端决定按文本 / hex 呈现）。
    pub payload: Vec<u8>,
}

/// `PUBLISH` 的执行结果。
#[derive(Clone, Debug, PartialEq)]
pub struct PubSubPublishResult {
    pub channel: String,
    /// 实际收到该消息的订阅者数量。
    pub count: i64,
}

/// 独立 Pub/Sub 会话：持有专用连接 + 命令队列，支持订阅、取消、发布与消息轮询。
///
/// 连接不进入普通连接池（`RedisSession` 的 Drop 才回池；此处直接持有裸 `RedisConnection`，
/// 随会话 drop 关闭）。`poll` 期望「无消息即读超时」，故连接建好后立即把读超时调短。
///
/// ⚠️ 订阅 / 发布分离（设计文档「Socket 订阅 + 独立发布」模型）：订阅连接一旦 `SUBSCRIBE`
/// 就进入订阅态，Redis 只允许在该连接上执行订阅类命令，禁止 `PUBLISH`。因此 `publish` 不会
/// 经订阅 socket 写出，而是在一条全新的普通连接上执行一次 `PUBLISH`，把结果暂存到
/// `pending_published` / `pending_errors`，由下一次 `poll` 合并进 `outcome`。
pub struct RedisPubSubSession {
    connection: RedisConnection,
    /// 目标逻辑数据库编号（0-15）。
    database: u32,
    /// 建连配置：复用于 `publish` 时另开一条普通连接（订阅 socket 不支持 PUBLISH）。
    config: ConnectionConfig,
    /// UI 线程放入、`poll` 写出的待发订阅命令。
    queue: CommandQueue,
    /// 独立连接执行 PUBLISH 的结果（下次 `poll` 合并上屏）。
    pending_published: Vec<PubSubPublishResult>,
    /// 独立连接执行 PUBLISH 的错误（下次 `poll` 合并上屏，不打断会话）。
    pending_errors: Vec<String>,
}

impl RedisPubSubSession {
    /// 建立一条专用订阅连接并选中目标库。
    ///
    /// `poll_timeout` 用于把「无消息」识别为一次空轮询：通常取几百毫秒，既不会漏消息
    /// 也不会让轮询卡住主流程。
    pub fn connect(
        config: &ConnectionConfig,
        database: u32,
        poll_timeout: Duration,
    ) -> fluxdb_core::Result<Self> {
        // 不复用连接池：订阅连接状态（已订阅）不可与普通查询连接互换。
        let mut connection = redis_new_connection(config)?;
        // Plain 直连可改底层 socket 读超时；TLS 底层被 rustls 包裹无法改动，保持默认 5s
        // （该类连接的 Pub/Sub 推送可达性稍降，属已记录限制，不影响功能）。
        connection
            .reader
            .get_mut()
            .set_read_timeout(Some(poll_timeout))
            .map_err(redis_io_error)?;
        redis_select(&mut connection, database)?;
        Ok(Self {
            connection,
            database,
            config: config.clone(),
            queue: Arc::new(Mutex::new(Vec::new())),
            pending_published: Vec::new(),
            pending_errors: Vec::new(),
        })
    }

    /// 当前会话绑定的数据库编号。
    pub fn database(&self) -> u32 {
        self.database
    }

    /// 订阅一个普通通道（入队，下一个 `poll` 生效）。
    pub fn subscribe(&self, channel: &str) {
        self.enqueue_args(&["SUBSCRIBE", channel]);
    }

    /// 按 pattern 订阅（入队，下一个 `poll` 生效）。
    pub fn psubscribe(&self, pattern: &str) {
        self.enqueue_args(&["PSUBSCRIBE", pattern]);
    }

    /// 取消订阅一个普通通道（入队，下一个 `poll` 生效）。
    pub fn unsubscribe(&self, channel: &str) {
        self.enqueue_args(&["UNSUBSCRIBE", channel]);
    }

    /// 取消一个 pattern 订阅（入队，下一个 `poll` 生效）。
    pub fn punsubscribe(&self, pattern: &str) {
        self.enqueue_args(&["PUNSUBSCRIBE", pattern]);
    }

    /// 发布一条消息。
    ///
    /// 在一条全新的普通连接上执行一次 `PUBLISH` 并回填订阅者数量：订阅连接已处于订阅态、
    /// 不允许 `PUBLISH`，故发布必须走独立连接（设计文档的混合模型）。结果暂存由下一次
    /// `poll()` 合并进 `outcome`，成功后由 UI 层更新「发布会话数」提示。
    pub fn publish(&mut self, channel: &str, message: &str) {
        match redis_publish_once(&self.config, self.database, channel, message) {
            Ok(count) => self.pending_published.push(PubSubPublishResult {
                channel: channel.to_string(),
                count,
            }),
            Err(error) => self.pending_errors.push(error.message.clone()),
        }
    }

    fn enqueue_args(&self, args: &[&str]) {
        let Ok(mut queue) = self.queue.lock() else {
            return;
        };
        queue.push(args.iter().map(|s| s.to_string()).collect::<Vec<_>>().join("\u{1}"));
    }

    /// 推进一轮：合并独立发布的暂存结果，写出排队订阅命令，再读取当前可用回包 / 消息。
    ///
    /// 订阅确认与 push 消息按 RESP 顺序混合到达，此处统一读取并分类；PUBLISH 结果由独立
    /// 连接完成并暂存，在本轮开头合并进 `outcome`（不再经订阅 socket）。读超时（无消息）
    /// 视为正常的“一轮结束”；真正的 IO / 协议错误才返回 Err。
    pub fn poll(&mut self) -> fluxdb_core::Result<PubSubPollOutcome> {
        let mut outcome = PubSubPollOutcome::default();
        // 0. 合并独立连接上完成的发布结果（PUBLISH 不走订阅 socket）。
        outcome.published.append(&mut self.pending_published);
        outcome.errors.append(&mut self.pending_errors);
        // 1. 写出所有排队命令（每条命令用分隔符拆回 argv 后按 RESP 编码写出）。
        let queued = match self.queue.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => Vec::new(),
        };
        for entry in queued {
            let argv: Vec<&str> = entry.split('\u{1}').collect();
            if argv.is_empty() {
                continue;
            }
            if let Err(error) = self.write_args(&argv) {
                outcome.errors.push(error.message.clone());
            }
        }

        // 2. 读取所有当前可用回包，直到读超时。
        loop {
            match redis_read_value(&mut self.connection.reader) {
                Ok(value) => self.classify(value, &mut outcome),
                Err(error) if is_read_timeout(&error) => break,
                Err(error) => {
                    // 真正的断连 / 协议错误：结束本轮，交由 UI 呈现错误态。
                    outcome.errors.push(error.message.clone());
                    return Ok(outcome);
                }
            }
        }
        Ok(outcome)
    }

    /// 按 RESP 编码把一条命令写出（不读取回包；回包统一在 `poll` 的读取阶段分类）。
    /// 写失败说明连接状态不可信，标记为不可复用。
    fn write_args(&mut self, args: &[&str]) -> fluxdb_core::Result<()> {
        use std::io::Write;
        let mut request = Vec::new();
        request.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
        for arg in args {
            request.extend_from_slice(format!("${}\r\n", arg.as_bytes().len()).as_bytes());
            request.extend_from_slice(arg.as_bytes());
            request.extend_from_slice(b"\r\n");
        }
        self.connection
            .reader
            .get_mut()
            .write_all(&request)
            .map_err(redis_io_error)?;
        self.connection.reader.get_mut().flush().map_err(redis_io_error)?;
        Ok(())
    }

    /// 把一条 RESP 值分类为订阅事件 / 消息 / 发布结果 / 错误。
    fn classify(&self, value: RedisValue, outcome: &mut PubSubPollOutcome) {
        let RedisValue::Array(items) = value else {
            self.classify_scalar(value, outcome);
            return;
        };
        let mut iter = items.into_iter();
        let Some(RedisValue::Bulk(Some(kind_bytes))) = iter.next() else {
            return;
        };
        let kind = String::from_utf8_lossy(&kind_bytes).to_string();
        match kind.as_str() {
            "subscribe" | "psubscribe" | "unsubscribe" | "punsubscribe" => {
                let pattern = kind.starts_with('p');
                let subscribed = kind.starts_with("sub");
                let channel = next_text(&mut iter).unwrap_or_default();
                let count = next_int(&mut iter);
                outcome.subscriptions.push(PubSubSubscriptionEvent {
                    pattern,
                    subscribed,
                    channel,
                    count,
                });
            }
            "message" => {
                let Some(channel) = next_text(&mut iter) else { return };
                let Some(payload) = next_bulk(&mut iter) else { return };
                outcome.messages.push(PubSubMessage {
                    pattern: None,
                    channel,
                    payload,
                });
            }
            "pmessage" => {
                let Some(pattern) = next_text(&mut iter) else { return };
                let Some(channel) = next_text(&mut iter) else { return };
                let Some(payload) = next_bulk(&mut iter) else { return };
                outcome.messages.push(PubSubMessage {
                    pattern: Some(pattern),
                    channel,
                    payload,
                });
            }
            _ => {
                // 其它未知回包：保留原始文本以便排查，但不打断会话。
                outcome.errors.push(format!("未知 Pub/Sub 回包类型: {kind}"));
            }
        }
    }

    /// 处理单个标量值（主要承载 `PUBLISH` 的整数回包）。
    fn classify_scalar(&self, value: RedisValue, outcome: &mut PubSubPollOutcome) {
        match value {
            RedisValue::Int(count) => {
                // PUBLISH 回包：数量本身即「订阅者数」，通道名在命令端已知（UI 层回填）。
                outcome.published.push(PubSubPublishResult {
                    channel: String::new(),
                    count,
                });
            }
            RedisValue::Simple(text) => {
                outcome.errors.push(format!("命令出错: {text}"));
            }
            _ => {}
        }
    }
}

/// 下一项按文本解码。
fn next_text(iter: &mut std::vec::IntoIter<RedisValue>) -> Option<String> {
    iter.next().map(|v| match v {
        RedisValue::Bulk(Some(b)) => String::from_utf8_lossy(&b).to_string(),
        other => redis_value_text(other),
    })
}

/// 下一项按原始字节返回。
fn next_bulk(iter: &mut std::vec::IntoIter<RedisValue>) -> Option<Vec<u8>> {
    iter.next().and_then(|v| match v {
        RedisValue::Bulk(Some(b)) => Some(b),
        RedisValue::Bulk(None) => Some(Vec::new()),
        _ => None,
    })
}

/// 下一项按整数解码。
fn next_int(iter: &mut std::vec::IntoIter<RedisValue>) -> i64 {
    iter.next()
        .and_then(|v| match v {
            RedisValue::Int(n) => Some(n),
            _ => None,
        })
        .unwrap_or(0)
}

/// 读超时（无消息可读）按“本轮结束”处理。兼容 macOS（Operation timed out）与
/// Linux/Windows（Resource temporarily unavailable）的 io 错误文案。
fn is_read_timeout(error: &Error) -> bool {
    let text = error.message.to_lowercase();
    text.contains("timed out")
        || text.contains("resource temporarily unavailable")
        || text.contains("operation now in progress")
}

/// 用一条全新的普通连接执行一次 `PUBLISH`，返回收到该消息的订阅者数量。
///
/// PUBLISH 必须走独立连接而不能经订阅 socket：订阅连接一旦进入订阅态，Redis 只允许
/// 订阅类命令，执行 PUBLISH 会报 "ERR Can't execute 'publish': only (P|S)SUBSCRIBE ..."。
/// 每次发布新开一条连接用完即关，与设计文档「Socket 订阅 + 独立发布」的混合模型一致。
fn redis_publish_once(
    config: &ConnectionConfig,
    database: u32,
    channel: &str,
    message: &str,
) -> fluxdb_core::Result<i64> {
    let mut connection = redis_new_connection(config)?;
    redis_select(&mut connection, database)?;
    write_command_args(&mut connection, &["PUBLISH", channel, message])?;
    match redis_read_value(&mut connection.reader)? {
        // 正常回包为整数（订阅者数）；错误回包 `-ERR` 已由 redis_read_value 转成 Err。
        RedisValue::Int(count) => Ok(count),
        RedisValue::Simple(error_text) => Err(Error::new(
            ErrorKind::Query,
            format!("Pub/Sub 发布失败: {error_text}"),
        )),
        _ => Err(Error::new(ErrorKind::Query, "Pub/Sub 发布返回格式异常")),
    }
}

/// 按 RESP 编码把一条命令写出到连接（普通命令路径，不读回包；回包由调用方读取）。
fn write_command_args(connection: &mut RedisConnection, args: &[&str]) -> fluxdb_core::Result<()> {
    use std::io::Write;
    let mut request = Vec::new();
    request.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
    for arg in args {
        request.extend_from_slice(format!("${}\r\n", arg.as_bytes().len()).as_bytes());
        request.extend_from_slice(arg.as_bytes());
        request.extend_from_slice(b"\r\n");
    }
    connection
        .reader
        .get_mut()
        .write_all(&request)
        .map_err(redis_io_error)?;
    connection
        .reader
        .get_mut()
        .flush()
        .map_err(redis_io_error)?;
    Ok(())
}
