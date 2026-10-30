// MongoDB 连接与错误映射。
//
// 所有跨层错误在这里收敛成 `fluxdb_core::Error`：驱动错误文本对使用者没有意义，
// 而认证失败 / 超时 / 网络不可达需要被 UI 区分展示与重试判断。

use mongodb::error::{ErrorKind as MongoErrorKind, Error as MongoError};
use mongodb::options::ClientOptions;

/// 进程级共享 runtime：驱动是异步的，Connector trait 是同步的。
///
/// 与 PostgreSQL / SQL Server 一致，全进程只建一个 multi-thread runtime，
/// 避免每次请求都新建线程池。
fn mongo_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("创建 MongoDB tokio runtime 失败")
    })
}

/// 驱动 future 的执行线程栈大小。
///
/// `Runtime::block_on` 是在**调用方线程**上 poll future 的（它只借用 runtime 的
/// worker 去跑 IO），所以决定是否溢栈的是调用方栈，而不是 runtime 的 worker 栈。
/// 调用方多半是 gpui 的后台任务（macOS 上是 GCD worker，栈仅约 512 KiB），而 debug
/// 构建下驱动的 BSON 序列化 / 反序列化帧很深，实测 `listCollections` 序列化请求时
/// 直接擦到保护页 → `abort()`（SIGILL / SIGABRT），不是可捕获的 panic。
/// 因此所有驱动调用都必须放进放大栈的线程；线程创建代价相对一次网络往返可忽略。
const MONGO_OP_STACK_BYTES: usize = 8 * 1024 * 1024;

/// 在共享 runtime 上、放大栈的线程里执行一段异步逻辑。
///
/// 这是唯一允许触碰 `mongo_runtime().block_on` 的入口：任何驱动 future 直接
/// `block_on` 都会继承调用方（可能是 512 KiB 的 GCD worker）的小栈。
/// 线程内 panic 不吞原因，统一转成 `ErrorKind::Internal` 由调用方展示。
fn mongo_run<T: Send>(label: &str, future: impl Future<Output = T> + Send) -> fluxdb_core::Result<T> {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name(format!("fluxdb-mongo-{label}"))
            .stack_size(MONGO_OP_STACK_BYTES)
            .spawn_scoped(scope, move || mongo_runtime().block_on(future))
            .map_err(|error| {
                Error::new(
                    ErrorKind::Internal,
                    format!("启动 MongoDB 工作线程失败：{error}"),
                )
            })?;
        worker.join().map_err(|_| {
            Error::new(
                ErrorKind::Internal,
                format!("{label}工作线程异常退出（多为栈溢出）"),
            )
        })
    })
}

/// 已建好的客户端：同一连接的多次操作复用同一个连接池。
struct CachedMongoClient {
    /// 建这个 client 时用的 URI 指纹。配置换了 URI 就自然重建，不依赖调用方记得失效。
    fingerprint: u64,
    client: MongoClient,
}

/// 全进程的客户端缓存（每个连接最多一份）。
///
/// 复用是压缩往返次数的关键：驱动 client 自带连接池，而每次操作重建 client 会额外付
/// 3 条 TCP 握手（有认证再加 SCRAM），实测 30ms RTT 下同一 `find` 从 174ms 涨到 387ms、
/// 打开集合页从 ~400ms 涨到 ~870ms。Compass 也是整连接只持有一个 client。
///
/// `ponytail:` 缓存不设空闲 TTL，只随断开 / 编辑配置 / 删除连接释放（见
/// `mongo_close_connection_client`）；一个 client 就是一个连接池，长期存活的连接本来就该
/// 留着。若将来出现「连了几百个连接都不断开」的占用问题，再按最近使用时间淘汰。
fn mongo_clients() -> &'static Mutex<HashMap<ConnectionId, CachedMongoClient>> {
    static CLIENTS: OnceLock<Mutex<HashMap<ConnectionId, CachedMongoClient>>> = OnceLock::new();
    CLIENTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 释放某连接缓存的客户端（断开 / 编辑配置 / 删除连接时调用）。
///
/// 只从缓存移除引用：驱动在最后一个引用 drop 后关闭连接池里的连接。
pub fn mongo_close_connection_client(connection_id: ConnectionId) {
    let Ok(mut clients) = mongo_clients().lock() else {
        return;
    };
    if clients.remove(&connection_id).is_some() {
        tracing::info!(
            target: "fluxdb_connectors",
            connection_id = ?connection_id,
            "MongoDB 客户端已随连接释放"
        );
    }
}

/// URI 指纹：只用于判断「配置是否换了目标」，不回显、不落日志。
fn mongo_uri_fingerprint(uri: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    uri.hash(&mut hasher);
    hasher.finish()
}

/// 取缓存的客户端；没有或 URI 已变时返回 `None`。
fn mongo_cached_client(connection_id: ConnectionId, fingerprint: u64) -> Option<MongoClient> {
    let clients = mongo_clients().lock().ok()?;
    clients
        .get(&connection_id)
        .filter(|cached| cached.fingerprint == fingerprint)
        .map(|cached| cached.client.clone())
}

/// 建连：优先使用结构化档案现场拼 URI（密码不落盘），缺失时退回 `endpoint`。
///
/// 同一连接（同一 URI）复用已建好的 client；只有首次或配置变了才真正建池。
fn mongo_connect(config: &ConnectionConfig) -> fluxdb_core::Result<MongoClient> {
    let uri = match config.mongo_profile.as_ref() {
        Some(profile) => {
            if let Some(message) = profile.validate() {
                return Err(Error::new(ErrorKind::Connection, message));
            }
            profile.to_uri()
        }
        None => match &config.endpoint {
            Endpoint::Uri { uri } => uri.clone(),
            Endpoint::Tcp { host, port, .. } => format!("mongodb://{host}:{port}"),
            Endpoint::SqliteFile { .. } => {
                return Err(Error::new(ErrorKind::Connection, "MongoDB 不支持文件型连接"))
            }
        },
    };

    let fingerprint = mongo_uri_fingerprint(&uri);
    if let Some(client) = mongo_cached_client(config.id, fingerprint) {
        return Ok(client);
    }

    let client = mongo_build_client(&uri)?;
    // 并发冷启动（如同时发起 find 与字段抽样）可能各自建出一个池，后写覆盖先写：
    // 多出来的那个池随最后一个引用 drop 关闭，浪费只发生在首屏一次。
    if let Ok(mut clients) = mongo_clients().lock() {
        clients.insert(
            config.id,
            CachedMongoClient {
                fingerprint,
                client: client.clone(),
            },
        );
    }
    Ok(client)
}

/// 真正建一个客户端（解析 URI + 构造，均需在大栈线程里跑）。
///
/// 超时由驱动自身的 `connectTimeoutMS` / `serverSelectionTimeoutMS` 控制。
/// `ClientOptions::parse` 的 SRV/TXT 解析会落到 runtime 的阻塞线程池，单纯的
/// `tokio::time::timeout` 只能取消 future、无法中断已经卡住的 DNS 调用（实测可
/// 无限挂住），所以整段解析放到独立线程，调用方按墙钟等待并在超时后放弃该线程。
/// 解析与客户端构造必须在同一次大栈执行里：两者都要 poll 驱动代码。
fn mongo_build_client(uri: &str) -> fluxdb_core::Result<MongoClient> {
    let uri = uri.to_string();
    // 解析线程：SRV/TXT 查询可能卡在系统解析器里，调用方不能跟着一起等。
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("fluxdb-mongo-parse".to_string())
        .stack_size(MONGO_OP_STACK_BYTES)
        .spawn(move || {
            let client = mongo_runtime().block_on(async move {
                let options = ClientOptions::parse(&uri).await.map_err(mongo_error)?;
                // 只建客户端不会真正握手（无 IO），但驱动要在 runtime 上下文里登记
                // 连接池任务，因此仍回到共享 runtime 上构造；认证/网络问题交给首次命令暴露。
                MongoClient::with_options(options).map_err(mongo_error)
            });
            let _ = tx.send(client);
        })
        .map_err(|error| {
            Error::new(ErrorKind::Internal, format!("启动 MongoDB 解析线程失败：{error}"))
        })?;

    rx.recv_timeout(Duration::from_secs(MONGO_URI_PARSE_TIMEOUT_SECS))
        .map_err(|_| Error::new(ErrorKind::Timeout, "解析 MongoDB 连接串超时（DNS 或 SRV 查询无响应）"))?
}

/// 解析连接串（含 SRV 查询）的墙钟上限。
const MONGO_URI_PARSE_TIMEOUT_SECS: u64 = 20;

/// 在共享 runtime 上执行一次驱动操作，并加墙钟上限。
///
/// 驱动 3.x 的操作是 action builder（只实现 `IntoFuture`，不是 `Future`），调用方
/// 必须先包成 `async {}` 再传进来。超时统一收敛为 `ErrorKind::Timeout`，避免驱动
/// 内部长时间等待把同步的 Connector 调用挂住。
///
/// 游标的**读取**（`next` / `try_collect`）同样要走这里：驱动解码结果文档的代码
/// 也是在调用方线程上跑的，不能图省事直接 `mongo_runtime().block_on`。
fn mongo_block_on<T: Send>(
    seconds: u64,
    label: &str,
    future: impl Future<Output = mongodb::error::Result<T>> + Send,
) -> fluxdb_core::Result<T> {
    // 线程执行结果本身就是 `Result`，这里再展开一层，把「线程启动/退出失败」
    // 与「驱动操作失败」合成同一个错误通道。
    mongo_run(label, async move {
        tokio::time::timeout(Duration::from_secs(seconds), future)
            .await
            .map_err(|_| Error::new(ErrorKind::Timeout, format!("{label}超时")))?
            .map_err(mongo_error)
    })
    .and_then(std::convert::identity)
}

/// 把驱动错误映射成领域错误。
///
/// 分区口径：认证 → `Authentication`，选主/网络/DNS → `Connection`，命令与写入失败
/// → `Query`，服务端能力不足 → `Unsupported`。默认分支保留原始文本，不吞错误。
fn mongo_error(error: MongoError) -> Error {
    let message = mongo_error_message(&error);
    let kind = match &*error.kind {
        // 认证失败单独归类：UI 要提示检查用户名/密码/authSource，且不应自动重试。
        MongoErrorKind::Authentication { .. } => ErrorKind::Authentication,
        // DNS 失败对 Atlas SRV 常意味着网络/防火墙问题，归 Connection 更利于重试判断。
        MongoErrorKind::DnsResolve { .. }
        | MongoErrorKind::ServerSelection { .. }
        | MongoErrorKind::Io(_)
        | MongoErrorKind::ConnectionPoolCleared { .. }
        | MongoErrorKind::InvalidTlsConfig { .. }
        | MongoErrorKind::Shutdown => ErrorKind::Connection,
        MongoErrorKind::Command(_)
        | MongoErrorKind::Write(_)
        | MongoErrorKind::BulkWrite(_)
        | MongoErrorKind::InsertMany(_)
        | MongoErrorKind::InvalidArgument { .. }
        | MongoErrorKind::InvalidResponse { .. }
        | MongoErrorKind::Transaction { .. } => ErrorKind::Query,
        MongoErrorKind::IncompatibleServer { .. }
        | MongoErrorKind::SessionsNotSupported
        | MongoErrorKind::MissingResumeToken
        | MongoErrorKind::GridFs(_) => ErrorKind::Unsupported,
        _ => ErrorKind::Internal,
    };
    Error::new(kind, message)
}

/// 提取驱动错误里对用户有意义的部分：服务端消息优先，其次错误码，最后整体文本。
fn mongo_error_message(error: &MongoError) -> String {
    match &*error.kind {
        MongoErrorKind::Command(command) => {
            let mut text = command.message.clone();
            if !command.code_name.is_empty() {
                text.push_str(&format!("（{} {}）", command.code_name, command.code));
            }
            text
        }
        MongoErrorKind::Authentication { .. } => {
            "MongoDB 认证失败：请检查用户名、密码与 authSource".to_string()
        }
        MongoErrorKind::DnsResolve { message, .. } => format!("解析 MongoDB 主机失败：{message}"),
        MongoErrorKind::ServerSelection { message, .. } => format!("无法选择 MongoDB 节点：{message}"),
        MongoErrorKind::Io(io) => format!("MongoDB 网络错误：{io}"),
        _ => {
            let text = error.to_string();
            // 驱动会把连接串回显到错误里；密码已由 `to_uri` 编码，但错误文本仍可能含 userinfo，
            // 这里统一抹掉 `user:pass@` 形式的口令再上抛。
            redact_mongo_uri_password(&text)
        }
    }
}

/// 抹掉错误文本里 `scheme://user:pass@host` 的口令片段。
fn redact_mongo_uri_password(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("://") {
        let (head, tail) = rest.split_at(start + 3);
        out.push_str(head);
        // authority 段到第一个分隔符为止。
        let end = tail
            .find(['/', '?', ' ', '"', '\'', ','])
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rsplit_once('@') {
            Some((userinfo, host)) => {
                let user = userinfo.split(':').next().unwrap_or("");
                out.push_str(user);
                out.push_str(":***@");
                out.push_str(host);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// 解析查询文本为 JSON 值。
///
/// `relaxed` 为真时先按严格 JSON 解析，失败再按 JSON5 解析（无引号键、单引号、注释、
/// 尾逗号都收），与 Compass 查询栏一致；为假时只认严格 JSON，写文档走这条路——
/// JSON5 会把 `NaN` / `Infinity` 静默读成 `null`，写入时那就是**静默改数据**。
///
/// 严格 JSON 先试一遍是必要的，不只是省事：JSON5 把所有数字读成 f64，
/// `{"age": 41}` 会落成 BSON Double 而匹配不到存成 Int32 的字段。
fn mongo_parse_json(label: &str, text: &str, relaxed: bool) -> fluxdb_core::Result<serde_json::Value> {
    let strict_error = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    if !relaxed {
        return Err(Error::new(
            ErrorKind::Query,
            format!("{label} 不是合法 JSON：{strict_error}"),
        ));
    }
    // JSON5 独有的非有限数字会被 serde_json 读成 null，等于把查询条件悄悄换掉。
    // 这里用朴素子串判断：能走到这一步的文本已经被严格 JSON 拒过，误判只会多报一次错，
    // 不会改坏语义（字符串里含 `NaN` 的宽松写法确实会被误拒，代价可以接受）。
    if text.contains("NaN") || text.contains("Infinity") {
        return Err(Error::new(
            ErrorKind::Query,
            format!("{label} 不支持 NaN / Infinity"),
        ));
    }
    // 失败时报严格 JSON 的错：JSON5 的错误位置对用户更难对上号，
    // 而触发退化的写法本身通常就落在严格 JSON 报的那个位置上。
    json5::from_str::<serde_json::Value>(text)
        .map_err(|_| Error::new(ErrorKind::Query, format!("{label} 不是合法 JSON：{strict_error}")))
}

/// 解析 Extended JSON 对象；非对象输入直接报错（`find` 的 filter/projection/sort 都是对象）。
fn mongo_parse_object(label: &str, text: &str) -> fluxdb_core::Result<Document> {
    mongo_parse_object_inner(label, text, false)
}

/// 同 [`mongo_parse_object`]，但接受 Compass 查询栏的宽松写法；只用于查询条件，不用于写文档。
fn mongo_parse_query_object(label: &str, text: &str) -> fluxdb_core::Result<Document> {
    mongo_parse_object_inner(label, text, true)
}

fn mongo_parse_object_inner(label: &str, text: &str, relaxed: bool) -> fluxdb_core::Result<Document> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Document::new());
    }
    let value = mongo_parse_json(label, trimmed, relaxed)?;
    if !value.is_object() {
        return Err(Error::new(
            ErrorKind::Query,
            format!("{label} 必须是 JSON 对象"),
        ));
    }
    // Extended JSON（Canonical / Relaxed 都接受）。
    match Bson::try_from(value) {
        Ok(Bson::Document(document)) => Ok(document),
        Ok(_) => Err(Error::new(
            ErrorKind::Query,
            format!("{label} 必须是 JSON 对象"),
        )),
        Err(error) => Err(Error::new(
            ErrorKind::Query,
            format!("{label} 不是合法的 Extended JSON：{error}"),
        )),
    }
}

/// BSON 值转 Relaxed Extended JSON 文本（保留 ObjectId / Date / Decimal128 等类型标注）。
fn mongo_json_text(value: Bson) -> String {
    serde_json::to_string_pretty(&value.into_relaxed_extjson())
        .unwrap_or_else(|_| "{}".to_string())
}

/// 文档 `_id` 的可读标识：ObjectId 取 24 位十六进制，其它类型用其 Extended JSON 文本。
fn mongo_document_id(document: &Document) -> String {
    match document.get("_id") {
        Some(Bson::ObjectId(oid)) => oid.to_hex(),
        Some(Bson::String(value)) => value.clone(),
        Some(other) => mongo_json_text(other.clone()),
        None => String::new(),
    }
}

/// 驱动命令结果转统一文本；`ok: 0` 由驱动转成 `Command` 错误，这里只处理成功结果。
fn mongo_command_text(document: Document) -> String {
    mongo_json_text(Bson::Document(document))
}

#[cfg(test)]
mod mongo_wire_tests {
    use super::*;

    #[test]
    fn cached_client_is_reused_until_released() {
        // 复用与失效是这次改动唯一的行为契约：URI 不变就命中，URI 变了或连接被释放就重建。
        // 构造走 `mongo_build_client`（自带大栈线程）：测试线程栈同样放不下驱动代码。
        let uri = "mongodb://127.0.0.1:27017";
        let client = mongo_build_client(uri).expect("构造客户端");
        let connection_id = ConnectionId(4_242);
        let fingerprint = mongo_uri_fingerprint(uri);

        assert!(mongo_cached_client(connection_id, fingerprint).is_none());
        mongo_clients().lock().expect("缓存锁").insert(
            connection_id,
            CachedMongoClient {
                fingerprint,
                client,
            },
        );
        assert!(mongo_cached_client(connection_id, fingerprint).is_some());
        // 同 id 但 URI 变了：必须按新 URI 重建，不能复用旧池。
        assert!(mongo_cached_client(connection_id, fingerprint ^ 1).is_none());

        mongo_close_connection_client(connection_id);
        assert!(mongo_cached_client(connection_id, fingerprint).is_none());
    }

    #[test]
    fn uri_fingerprint_tracks_every_uri_difference() {
        // 主机/库/参数任一变化都要换指纹，否则改了配置仍会连到旧目标。
        assert_ne!(
            mongo_uri_fingerprint("mongodb://h1:27017/db"),
            mongo_uri_fingerprint("mongodb://h2:27017/db")
        );
        assert_ne!(
            mongo_uri_fingerprint("mongodb://h:27017/db"),
            mongo_uri_fingerprint("mongodb://h:27017/other")
        );
        assert_eq!(
            mongo_uri_fingerprint("mongodb://h:27017/db"),
            mongo_uri_fingerprint("mongodb://h:27017/db")
        );
    }

    #[test]
    fn redacts_password_in_uri_like_text() {
        let text = "connection string contained mongodb://alice:s3cret@db.example.com:27017/?x=1 failed";
        let redacted = redact_mongo_uri_password(text);
        assert!(!redacted.contains("s3cret"), "{redacted}");
        assert!(redacted.contains("alice:***@db.example.com:27017"), "{redacted}");
    }

    #[test]
    fn parses_extended_json_object_and_rejects_non_object() {
        let document = mongo_parse_object("filter", r#"{"a": {"$oid": "671700000000000000000001"}}"#)
            .expect("应解析 Extended JSON");
        assert!(matches!(document.get("a"), Some(Bson::ObjectId(_))));

        assert!(mongo_parse_object("filter", "[1,2]").is_err());
        assert!(mongo_parse_object("filter", "{bad").is_err());
        // 空文本等价于 `{}`。
        assert!(mongo_parse_object("filter", "  ").unwrap().is_empty());
    }

    #[test]
    fn accepts_compass_style_relaxed_json() {
        // Compass 查询栏接受的无引号键 / 单引号 / 尾逗号，查询条件都要能跑。
        let document = mongo_parse_query_object("Filter", "{ age: 41 }").expect("无引号键");
        assert_eq!(document.get("age"), Some(&Bson::Int32(41)));

        let document =
            mongo_parse_query_object("Filter", "{'status': 'active',}").expect("单引号 + 尾逗号");
        assert_eq!(document.get("status").and_then(Bson::as_str), Some("active"));

        // 运算符键同样不带引号，这是查询栏里最常见的写法。
        let document = mongo_parse_query_object("Filter", "{age: {$gt: 40}}").expect("无引号运算符");
        assert!(matches!(document.get("age"), Some(Bson::Document(_))));

        // 非有限数字会被 JSON5 静默读成 null，宁可报错也不能改成「等于 null」。
        assert!(mongo_parse_query_object("Filter", "{age: NaN}").is_err());
    }

    #[test]
    fn write_paths_still_require_strict_json() {
        // 写文档走严格解析：宽松解析只是为了查询输入，不能顺带放宽写入。
        assert!(mongo_parse_object("文档", "{ age: 41 }").is_err());
        assert!(mongo_parse_object("文档", r#"{"age": 41}"#).is_ok());
    }

    #[test]
    fn strict_json_numbers_keep_integer_type() {
        // 严格 JSON 优先：否则 `41` 会被 JSON5 读成 f64 再落成 Double，
        // 查不到存成 Int32 的同名字段。
        let document = mongo_parse_object("filter", r#"{"age": 41}"#).unwrap();
        assert_eq!(document.get("age"), Some(&Bson::Int32(41)));
    }

    #[test]
    fn document_id_prefers_readable_forms() {
        let oid = mongodb::bson::oid::ObjectId::parse_str("671700000000000000000001").unwrap();
        assert_eq!(
            mongo_document_id(&doc! {"_id": oid}),
            "671700000000000000000001"
        );
        assert_eq!(mongo_document_id(&doc! {"_id": "abc"}), "abc");
        assert_eq!(mongo_document_id(&doc! {"x": 1}), "");
    }

    #[test]
    fn collation_requires_locale_and_accepts_compass_syntax() {
        // 空 = 不设置：面板留空不能变成「非法 collation」把查询拦下。
        assert!(mongo_parse_collation("").unwrap().is_none());
        assert!(mongo_parse_collation("  ").unwrap().is_none());

        let collation = mongo_parse_collation("{ locale: 'en', strength: 2 }")
            .expect("宽松写法应该能解析")
            .expect("非空 collation");
        assert_eq!(collation.locale, "en");

        // locale 是 Collation 的必填字段：缺了要明确报错，不能静默当成默认规则。
        let error = mongo_parse_collation("{ strength: 2 }").unwrap_err();
        assert!(error.to_string().contains("Collation"), "{error}");
    }

    #[test]
    fn hint_accepts_index_name_or_key_document() {
        assert!(mongo_parse_hint("").unwrap().is_none());
        assert!(mongo_parse_hint("   ").unwrap().is_none());

        // 索引名（Compass 里写成字符串）与键定义两种形态都要认。
        assert!(matches!(
            mongo_parse_hint("\"_id_\"").unwrap(),
            Some(Hint::Name(name)) if name == "_id_"
        ));
        assert!(matches!(
            mongo_parse_hint("{ age: 1 }").unwrap(),
            Some(Hint::Keys(keys)) if keys.get("age").is_some()
        ));

        // 空对象当成「没填」会把用户的明确输入吞掉，直接报错更清楚。
        assert!(mongo_parse_hint("{}").is_err());
        assert!(mongo_parse_hint("42").is_err());
    }

    #[test]
    fn json_text_keeps_bson_types() {
        let oid = mongodb::bson::oid::ObjectId::parse_str("671700000000000000000001").unwrap();
        let text = mongo_json_text(Bson::Document(doc! {"_id": oid, "n": 1i32}));
        assert!(text.contains("$oid"), "{text}");
        assert!(text.contains("671700000000000000000001"), "{text}");
    }
}

