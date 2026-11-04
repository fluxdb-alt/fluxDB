// MongoDB shell 语句解析与执行（命令行终端用，只读）。
//
// 目标是让终端里的写法和 Compass / mongosh 一致：
//   use fluxdb_demo
//   show dbs / show collections
//   db.events.find({ event_type: "page_view" }).sort({ duration_ms: -1 }).limit(20)
//   db.events.findOne(...) / countDocuments(...) / distinct("field", ...) / aggregate([...])
//   db.events.getIndexes() / db.events.stats() / db.getCollectionNames()
//
// 边界：
//   - 不做 JS 求值。参数是「放宽了引号与尾逗号的 JSON」，构造器（`ObjectId("…")`、
//     `ISODate("…")`、`NumberLong(…)`）在解析阶段就地换成 Extended JSON，再交给 BSON。
//     `let` / 变量 / 多语句一律不支持，报错说清楚而不是给出半截结果。
//   - 只读。写与管理命令（insert / update / delete / drop / createIndex …）明确拒绝，
//     而不是悄悄执行。
//   - 输出排版对齐 Compass：键蓝色、字符串绿色、数字橙色，类型保留
//     `ObjectId('…')` / `ISODate('…')` 这样的标注。

/// shell 单次最多返回的文档数：与集合工作台共用同一产品上限。
const MONGO_SHELL_LIMIT: i64 = fluxdb_core::MONGO_MAX_DOCUMENTS_PER_QUERY as i64;

/// shell 内部执行产物：文本之外还要把文档结构、条数与截断状态带回给工作台。
///
/// 终端（CLI）只取 `text`；工作台用 `documents` 渲染字段卡片，用计数与截断提示说明完整性，
/// 因此不能只返回 `String`。
#[derive(Default)]
struct MongoShellOutput {
    text: String,
    document_count: Option<u64>,
    truncated: bool,
    /// 同一批结果的文档结构，供工作台渲染字段卡片；CLI 只用 `text`。
    documents: Vec<fluxdb_core::MongoDocument>,
}

impl MongoShellOutput {
    /// 非文档结果（`show dbs`、`count` 等）：只有文本，不冒充文档计数。
    fn text(text: String) -> Self {
        Self {
            text,
            ..Default::default()
        }
    }
}

/// shell 里还会遇到的写 / 管理方法：单独识别是为了给出「本终端不支持写」而不是「拼错了」。
const MONGO_SHELL_WRITE_METHODS: &[&str] = &[
    "insert",
    "insertOne",
    "insertMany",
    "update",
    "updateOne",
    "updateMany",
    "replaceOne",
    "delete",
    "deleteOne",
    "deleteMany",
    "remove",
    "save",
    "bulkWrite",
    "drop",
    "renameCollection",
    "createCollection",
    "createIndex",
    "createIndexes",
    "dropIndex",
    "dropIndexes",
    "dropDatabase",
    "createUser",
    "runCommand",
];

/// 解析后的一条语句。
#[derive(Debug)]
enum MongoShellStatement {
    /// `use <db>`：只切换本终端会话的当前库。
    Use(String),
    /// `show dbs` / `show collections`。
    Show(MongoShellShow),
    /// `db.getCollectionNames()`。
    CollectionNames,
    /// `db.<collection>.<method>(...)`。
    Collection {
        collection: String,
        method: MongoShellMethod,
        args: Vec<String>,
        /// 链式调用（`.sort(...)` 等）；只有 `find` 支持。
        chain: Vec<MongoShellChainCall>,
    },
}

#[derive(Debug)]
enum MongoShellShow {
    Databases,
    Collections,
}

#[derive(Debug)]
enum MongoShellMethod {
    Find,
    FindOne,
    CountDocuments,
    Estimate,
    Distinct,
    Aggregate,
    Indexes,
    Stats,
}

/// 执行一条 shell 语句。
///
/// `database` 是本会话的当前库；`use` 的结果经 `MongoShellResult::database` 回传给调用方，
/// 因为库是会话状态，不能写回连接配置。
fn mongo_shell(
    config: &ConnectionConfig,
    database: &str,
    input: &str,
) -> fluxdb_core::Result<fluxdb_core::MongoShellResult> {
    let text = input.trim().trim_end_matches(';').trim();
    if text.is_empty() {
        return Ok(fluxdb_core::MongoShellResult::default());
    }
    let output = match mongo_shell_parse(text)? {
        MongoShellStatement::Use(name) => {
            return Ok(fluxdb_core::MongoShellResult {
                // 与真实 shell 的提示语一致，界面上不需要再翻译一次。
                output: format!("switched to db {name}"),
                database: Some(name),
                ..Default::default()
            });
        }
        MongoShellStatement::Show(MongoShellShow::Databases) => mongo_shell_databases(config)?,
        MongoShellStatement::Show(MongoShellShow::Collections)
        | MongoShellStatement::CollectionNames => {
            let database = mongo_shell_require_database(database)?;
            mongo_shell_collection_names(config, &database)?
        }
        MongoShellStatement::Collection {
            collection,
            method,
            args,
            chain,
        } => {
            let database = mongo_shell_require_database(database)?;
            mongo_shell_run(config, &database, &collection, method, &args, &chain)?
        }
    };
    Ok(fluxdb_core::MongoShellResult {
        output: output.text,
        database: None,
        document_count: output.document_count,
        truncated: output.truncated,
        documents: output.documents,
    })
}

// —— 解析 ——

fn mongo_shell_parse(text: &str) -> fluxdb_core::Result<MongoShellStatement> {
    let tokens = mongo_shell_tokenize(text);
    let Some(head) = tokens.first().map(String::as_str) else {
        return Err(mongo_shell_error("Empty statement"));
    };
    match head {
        "use" => {
            if tokens.len() != 2 {
                return Err(mongo_shell_error("Usage: use <database>"));
            }
            let name = unquote(&tokens[1]);
            if name.is_empty() {
                return Err(mongo_shell_error("Database name cannot be empty"));
            }
            Ok(MongoShellStatement::Use(name.to_string()))
        }
        // 与 mongosh 一样只回一行错误，不刷用法清单；可用写法交给 Tab 补全。
        // 多写的 token 必须报错：`show dbs | show collections` 曾被当成 `show dbs` 静默执行。
        "show" if tokens.len() == 2 => match tokens[1].as_str() {
            "dbs" | "databases" => Ok(MongoShellStatement::Show(MongoShellShow::Databases)),
            "collections" | "tables" => Ok(MongoShellStatement::Show(MongoShellShow::Collections)),
            other => Err(mongo_shell_error(format!("Unknown command: show {other}"))),
        },
        "show" => Err(mongo_shell_error("Usage: show dbs | show collections")),
        // `db` 后面紧跟 `.` 或 `[` 才是方法调用；`db["a.b"]` 里带引号且没有空白，
        // 整条语句会是一个 token，所以按前缀判断而不是按 token 相等。
        "db" => mongo_shell_parse_db_call(text),
        _ if text.starts_with("db.") || text.starts_with("db[") => mongo_shell_parse_db_call(text),
        // JS 语句不在支持范围内：明确说明，而不是把它当成未知命令。
        "let" | "const" | "var" => {
            Err(mongo_shell_error(
                "JavaScript variables are not supported; run db.<collection>.<method>(...) directly",
            ))
        }
        other => Err(mongo_shell_error(format!("Unrecognized statement: {other}"))),
    }
}

/// 按空白切 token，字符串字面量（单 / 双 / 反引号）内部不切。
fn mongo_shell_tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quote.is_some() => {
                current.push(ch);
                escaped = true;
            }
            '\'' | '"' | '`' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
                current.push(ch);
            }
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// 解析 `db` 开头的调用：`db.getCollectionNames()` 或 `db.<集合>.<方法>(...)`。
fn mongo_shell_parse_db_call(text: &str) -> fluxdb_core::Result<MongoShellStatement> {
    let rest = text["db".len()..].trim_start();
    // `db["a.b"]` / `db['a.b']`：集合名含点时必须用方括号。
    let (collection, tail, db_method) = if let Some(inner) = rest.strip_prefix('[') {
        let close = inner
            .find(']')
            .ok_or_else(|| mongo_shell_error("Missing ] after the collection name"))?;
        let collection = unquote(inner[..close].trim()).to_string();
        let tail = inner[close + 1..]
            .trim_start()
            .strip_prefix('.')
            .ok_or_else(|| mongo_shell_error("Usage: db[\"collection\"].<method>(...)"))?;
        (collection, tail, None)
    } else {
        let rest = rest
            .strip_prefix('.')
            .ok_or_else(|| mongo_shell_error("Usage: db.<collection>.<method>(...)"))?;
        match rest.find('.') {
            Some(dot) => (
                unquote(rest[..dot].trim()).to_string(),
                &rest[dot + 1..],
                None,
            ),
            // `db.someMethod()`：没有集合名，只可能是 db 级方法。
            None => (String::new(), rest, Some(rest)),
        }
    };
    if let Some(db_method) = db_method {
        let (name, args, chain) = mongo_shell_split_call(db_method)?;
        if name.eq_ignore_ascii_case("getCollectionNames") {
            if !args.is_empty() || !chain.is_empty() {
                return Err(mongo_shell_error("getCollectionNames() takes no arguments"));
            }
            return Ok(MongoShellStatement::CollectionNames);
        }
        return Err(mongo_shell_error(format!(
            "Unknown db method: {name}; only db.getCollectionNames() is supported"
        )));
    }
    let (name, args, chain) = mongo_shell_split_call(tail)?;
    let method = mongo_shell_method(&name)?;
    Ok(MongoShellStatement::Collection {
        collection,
        method,
        args,
        chain,
    })
}

/// 一次链式调用：方法名（已转小写）+ 参数文本。
type MongoShellChainCall = (String, Vec<String>);

/// 拆出「方法名、参数、链式调用」。
///
/// `find({a:1}).sort({b:-1}).limit(5)` → `("find", ["{a:1}"], [("sort", …), ("limit", …)])`。
/// 只接受「调用 + 若干 `.方法(...)`」的形式；夹着注释、逗号等其它残留一律报语法错误。
fn mongo_shell_split_call(
    text: &str,
) -> fluxdb_core::Result<(String, Vec<String>, Vec<MongoShellChainCall>)> {
    let open = text
        .find('(')
        .ok_or_else(|| mongo_shell_error("Missing ( in the method call"))?;
    let name = text[..open].trim().to_string();
    if name.is_empty() {
        return Err(mongo_shell_error("Method name cannot be empty"));
    }
    let (args, mut rest) = mongo_shell_split_args(&text[open..])?;
    let mut chain = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return Ok((name, args, chain));
        }
        let call = rest
            .strip_prefix('.')
            .ok_or_else(|| mongo_shell_error(format!("Unexpected trailing input: {rest}")))?;
        let open = call
            .find('(')
            .ok_or_else(|| mongo_shell_error(format!("Missing ( in the chained call: {call}")))?;
        let chain_name = call[..open].trim().to_string();
        if chain_name.is_empty() {
            return Err(mongo_shell_error("Missing method name in the chained call"));
        }
        let (chain_args, remaining) = mongo_shell_split_args(&call[open..])?;
        chain.push((chain_name.to_ascii_lowercase(), chain_args));
        rest = remaining;
    }
}

/// 按顶层逗号切参数，返回参数文本与右括号之后的剩余部分。
///
/// 括号 / 方括号 / 花括号 / 字符串内部的逗号不算分隔符——JSON 参数里到处都是。
fn mongo_shell_split_args(text: &str) -> fluxdb_core::Result<(Vec<String>, &str)> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut end = None;
    for (offset, ch) in text.char_indices() {
        match ch {
            '(' if offset == 0 && quote.is_none() => continue,
            '\'' | '"' | '`' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
                current.push(ch);
            }
            '(' | '[' | '{' if quote.is_none() => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' if quote.is_none() => {
                if ch == ')' && depth == 0 {
                    end = Some(offset + 1);
                    break;
                }
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if quote.is_none() && depth == 0 => args.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    let Some(end) = end else {
        return Err(mongo_shell_error("Missing ) in the method call"));
    };
    args.push(current);
    let args = args
        .into_iter()
        .map(|arg| arg.trim().to_string())
        .filter(|arg| !arg.is_empty())
        .collect();
    Ok((args, &text[end..]))
}

fn mongo_shell_method(name: &str) -> fluxdb_core::Result<MongoShellMethod> {
    let lower = name.to_ascii_lowercase();
    let method = match lower.as_str() {
        "find" => MongoShellMethod::Find,
        "findone" => MongoShellMethod::FindOne,
        "count" | "countdocuments" => MongoShellMethod::CountDocuments,
        "estimateddocumentcount" => MongoShellMethod::Estimate,
        "distinct" => MongoShellMethod::Distinct,
        "aggregate" => MongoShellMethod::Aggregate,
        "getindexes" => MongoShellMethod::Indexes,
        "stats" => MongoShellMethod::Stats,
        other => {
            let is_write = MONGO_SHELL_WRITE_METHODS
                .iter()
                .any(|candidate| candidate.to_ascii_lowercase() == other);
            return Err(mongo_shell_error(if is_write {
                format!(
                    "The command line only supports read-only queries; {name} is not supported. \
                     Use the collection workbench or import to write data"
                )
            } else {
                // 单行：多行清单在终端里是一整块红字，按 Tab 看候选更接近 mongosh。
                format!("Unknown method: {name}")
            }));
        }
    };
    Ok(method)
}

/// 去掉成对引号（单 / 双 / 反引号）。
fn unquote(text: &str) -> &str {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if matches!(first, b'"' | b'\'' | b'`') && first == last {
            return &text[1..text.len() - 1];
        }
    }
    text
}

fn mongo_shell_error(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Query, message.into())
}

// —— 执行 ——

fn mongo_shell_require_database(database: &str) -> fluxdb_core::Result<String> {
    let database = database.trim();
    if database.is_empty() {
        return Err(mongo_shell_error("No database selected: use <database>"));
    }
    Ok(database.to_string())
}

/// `show dbs`：库名 + 磁盘占用，与 mongosh 一致；拿不到占用就只列库名。
fn mongo_shell_databases(config: &ConnectionConfig) -> fluxdb_core::Result<MongoShellOutput> {
    let client = mongo_connect(config)?;
    let admin = client.database("admin");
    let reply = mongo_block_on(30, "listDatabases", async move {
        admin.run_command(doc! {"listDatabases": 1}).await
    });
    let Ok(reply) = reply else {
        // 权限不足时退化成只列库名（`listDatabaseNames` 只需要 listDatabases 的部分权限）。
        let client = mongo_connect(config)?;
        let mut names: Vec<String> = mongo_block_on(30, "listDatabases", async move {
            client.list_database_names().await
        })?;
        names.sort();
        return Ok(MongoShellOutput::text(names.join("\n")));
    };
    let entries = reply
        .get_array("databases")
        .map(|values| values.as_slice())
        .unwrap_or_default();
    let mut lines: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            let document = entry.as_document()?;
            let name = document.get_str("name").ok()?.to_string();
            let size = document
                .get_i64("sizeOnDisk")
                .ok()
                .map(|bytes| format!("  {}", mongo_shell_bytes(bytes as f64)))
                .unwrap_or_default();
            Some(format!("{name}{size}"))
        })
        .collect();
    lines.sort();
    Ok(MongoShellOutput::text(lines.join("\n")))
}

/// 字节数写成 mongosh 风格（`1.000GB` / `20.5MB` / `512B`）。
fn mongo_shell_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{}B", value as u64)
    } else {
        format!("{value:.3}{}", UNITS[unit])
    }
}

fn mongo_shell_collection_names(
    config: &ConnectionConfig,
    database: &str,
) -> fluxdb_core::Result<MongoShellOutput> {
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let names = mongo_block_on(30, "listCollections", async move {
        client.database(&database).list_collection_names().await
    })?;
    let mut names: Vec<String> = names
        .into_iter()
        .filter(|name| !name.starts_with("system."))
        .collect();
    names.sort();
    Ok(MongoShellOutput::text(names.join("\n")))
}

fn mongo_shell_run(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    method: MongoShellMethod,
    args: &[String],
    chain: &[MongoShellChainCall],
) -> fluxdb_core::Result<MongoShellOutput> {
    match method {
        MongoShellMethod::Find => mongo_shell_find(config, database, collection, args, chain),
        _ if !chain.is_empty() => Err(mongo_shell_error(
            "Only find supports chaining (.sort() / .skip() / .limit())",
        )),
        MongoShellMethod::FindOne => mongo_shell_find_one(config, database, collection, args),
        MongoShellMethod::CountDocuments => mongo_shell_count(config, database, collection, args),
        MongoShellMethod::Estimate => mongo_shell_estimate(config, database, collection),
        MongoShellMethod::Distinct => mongo_shell_distinct(config, database, collection, args),
        MongoShellMethod::Aggregate => mongo_shell_aggregate(config, database, collection, args),
        MongoShellMethod::Indexes => mongo_shell_indexes(config, database, collection),
        MongoShellMethod::Stats => mongo_shell_stats(config, database, collection),
    }
}

fn mongo_shell_find(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    positional: &[String],
    chain: &[MongoShellChainCall],
) -> fluxdb_core::Result<MongoShellOutput> {
    let request = mongo_shell_find_request(collection, positional, chain)?;
    let page = mongo_find(config, database, &request, &|| false)?;
    let text = mongo_shell_documents_text(&page.documents);
    Ok(MongoShellOutput {
        text,
        document_count: Some(page.returned),
        // `has_more` 同时覆盖「超过 500 条」和「触到字节预算」，两者都意味着还有结果没取回。
        truncated: page.has_more,
        documents: page.documents,
    })
}

fn mongo_shell_find_one(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    positional: &[String],
) -> fluxdb_core::Result<MongoShellOutput> {
    let rewrite = |text: Option<&String>| {
        mongo_shell_rewrite_constructors(text.map(String::as_str).unwrap_or(""))
    };
    let request = fluxdb_core::MongoFindRequest {
        collection: collection.to_string(),
        filter: rewrite(positional.first()),
        projection: rewrite(positional.get(1)),
        limit: 1,
        ..Default::default()
    };
    let page = mongo_find(config, database, &request, &|| false)?;
    let document = page.documents.into_iter().next();
    let text = match document.as_ref() {
        Some(document) => mongo_shell_pretty(&mongo_shell_parse_text("Result", &document.json)?, 0),
        // mongosh 的 findOne 无结果时打印 null。
        None => "null".to_string(),
    };
    Ok(MongoShellOutput {
        text,
        document_count: Some(u64::from(document.is_some())),
        truncated: false,
        documents: document.into_iter().collect(),
    })
}

/// 组 `find` 请求：位置参数是 filter / projection，链式是 sort / skip / limit。
fn mongo_shell_find_request(
    collection: &str,
    positional: &[String],
    chain: &[MongoShellChainCall],
) -> fluxdb_core::Result<fluxdb_core::MongoFindRequest> {
    // 条件先过构造器改写：`find({_id: ObjectId("…")})` 是最常见的写法，
    // 不改写的话 `mongo_find` 会把它当成非法 JSON。
    let rewrite = |text: Option<&String>| {
        mongo_shell_rewrite_constructors(text.map(String::as_str).unwrap_or(""))
    };
    let mut request = fluxdb_core::MongoFindRequest {
        collection: collection.to_string(),
        filter: rewrite(positional.first()),
        projection: rewrite(positional.get(1)),
        limit: MONGO_SHELL_LIMIT as u64,
        ..Default::default()
    };
    for (name, values) in chain {
        let value = mongo_shell_rewrite_constructors(
            values.first().map(String::as_str).unwrap_or(""),
        );
        match name.as_str() {
            "sort" => request.sort = value,
            "skip" => request.skip = mongo_shell_integer(&value, "skip")?,
            "limit" => {
                request.limit = mongo_shell_integer(&value, "limit")?.clamp(1, MONGO_SHELL_LIMIT as u64)
            }
            other => {
                return Err(mongo_shell_error(format!(
                    "find does not support .{other}(); supported: .sort() / .skip() / .limit()"
                )));
            }
        }
    }
    Ok(request)
}

fn mongo_shell_integer(text: &str, label: &str) -> fluxdb_core::Result<u64> {
    let text = text.trim();
    text.parse::<u64>()
        .map_err(|_| mongo_shell_error(format!("{label} must be a non-negative integer, got {text}")))
}

fn mongo_shell_count(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    positional: &[String],
) -> fluxdb_core::Result<MongoShellOutput> {
    let filter = mongo_shell_document("Filter", positional.first().map(String::as_str).unwrap_or(""))?;
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let count = mongo_block_on(60, "count", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .count_documents(filter)
            .await
    })?;
    Ok(MongoShellOutput::text(count.to_string()))
}

fn mongo_shell_estimate(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
) -> fluxdb_core::Result<MongoShellOutput> {
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let count = mongo_block_on(30, "estimatedDocumentCount", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .estimated_document_count()
            .await
    })?;
    Ok(MongoShellOutput::text(count.to_string()))
}

fn mongo_shell_distinct(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    positional: &[String],
) -> fluxdb_core::Result<MongoShellOutput> {
    let Some(field) = positional.first().map(|arg| unquote(arg.trim()).to_string()) else {
        return Err(mongo_shell_error("Usage: distinct(\"field\", [filter])"));
    };
    let filter = mongo_shell_document("Filter", positional.get(1).map(String::as_str).unwrap_or(""))?;
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let values = mongo_block_on(30, "distinct", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .distinct(&field, filter)
            .await
    })?;
    Ok(MongoShellOutput::text(mongo_shell_array(&values, 0)))
}

fn mongo_shell_aggregate(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    positional: &[String],
) -> fluxdb_core::Result<MongoShellOutput> {
    let stages = mongo_shell_pipeline(positional.first().map(String::as_str).unwrap_or("[]"))?;
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    // 边读边排版并同时卡条数 / 字节预算：聚合可以先产出海量文档，
    // 先 `collect` 再 `take(500)` 会在截断前就把整份结果堆进内存。
    let (text, documents, count, truncated) = mongo_block_on(60, "aggregate", async move {
        use futures_util::TryStreamExt as _;
        let mut cursor = client
            .database(&database)
            .collection::<Document>(&collection)
            .aggregate(stages)
            .await?;
        let limit = MONGO_SHELL_LIMIT as usize;
        let mut out = String::from("[\n");
        let mut documents = Vec::new();
        let mut truncated = false;
        while let Some(document) = cursor.try_next().await? {
            if documents.len() >= limit {
                truncated = true;
                break;
            }
            let piece = mongo_shell_pretty(&Bson::Document(document.clone()), 2);
            // 至少保留第一篇：单篇就超过预算时也要让用户看到内容，而不是回一个空数组。
            if !documents.is_empty()
                && out.len() + piece.len() > fluxdb_core::MONGO_MAX_RESULT_BYTES
            {
                truncated = true;
                break;
            }
            // 结构化形态与文本同一批文档：工作台据此渲染字段卡片。
            documents.push(fluxdb_core::MongoDocument {
                id: mongo_document_id(&document),
                json: mongo_json_text(Bson::Document(document)),
            });
            if documents.len() > 1 {
                // 与 `find` 一致：文档之间空一行，长结果里也能一眼看出边界。
                out.push_str(",\n\n");
            }
            out.push_str("  ");
            out.push_str(&piece);
        }
        let count = documents.len() as u64;
        if count == 0 {
            out = "[]".to_string();
        } else {
            out.push_str("\n]");
        }
        Ok((out, documents, count, truncated))
    })?;
    Ok(MongoShellOutput {
        text,
        document_count: Some(count),
        truncated,
        documents,
    })
}

fn mongo_shell_indexes(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
) -> fluxdb_core::Result<MongoShellOutput> {
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let cursor = mongo_block_on(30, "listIndexes", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .list_indexes()
            .await
    })?;
    // 与 mongosh 的 `getIndexes()` 一致：返回索引定义文档（`key` + 选项），不是名字列表。
    let values: Vec<Bson> = mongo_collect_cursor(cursor)?
        .into_iter()
        .filter_map(|model| {
            serde_json::to_value(&model)
                .ok()
                .and_then(|value| Bson::try_from(value).ok())
        })
        .collect();
    Ok(MongoShellOutput::text(mongo_shell_array(&values, 0)))
}

fn mongo_shell_stats(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
) -> fluxdb_core::Result<MongoShellOutput> {
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let stats = mongo_block_on(30, "collStats", async move {
        client
            .database(&database)
            .run_command(doc! {"collStats": collection})
            .await
    })?;
    Ok(MongoShellOutput::text(mongo_shell_pretty(&Bson::Document(stats), 0)))
}

// —— 参数解析 ——

/// shell 参数文本 → BSON 文档。空文本等价于 `{}`。
fn mongo_shell_document(label: &str, text: &str) -> fluxdb_core::Result<Document> {
    match mongo_shell_parse_text(label, text)? {
        Bson::Document(document) => Ok(document),
        _ => Err(mongo_shell_error(format!("{label} must be an object"))),
    }
}

/// shell 文本 → BSON 值：先替换构造器，再按（放宽的）JSON 解析。
fn mongo_shell_parse_text(label: &str, text: &str) -> fluxdb_core::Result<Bson> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Bson::Document(Document::new()));
    }
    let text = mongo_shell_rewrite_constructors(text);
    let value = mongo_parse_json(label, &text, true)?;
    Bson::try_from(value).map_err(|error| {
        mongo_shell_error(format!("{label} is not valid Extended JSON: {error}"))
    })
}

/// 聚合管道：必须是数组。
///
/// `$out` / `$merge` 会把聚合结果写回集合，和终端的只读边界冲突，解析阶段就拒绝；
/// 放在这里是因为 CLI 与命令工作台共用同一入口。
fn mongo_shell_pipeline(text: &str) -> fluxdb_core::Result<Vec<Document>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    match mongo_shell_parse_text("Pipeline", text)? {
        Bson::Array(items) => items
            .into_iter()
            .map(|item| match item {
                Bson::Document(stage) => {
                    if let Some(stage_name) = stage.keys().find(|key| {
                        key.eq_ignore_ascii_case("$out") || key.eq_ignore_ascii_case("$merge")
                    }) {
                        return Err(mongo_shell_error(format!(
                            "The command line only supports read-only queries; {stage_name} is not supported"
                        )));
                    }
                    Ok(stage)
                }
                _ => Err(mongo_shell_error("Each pipeline stage must be an object")),
            })
            .collect(),
        _ => Err(mongo_shell_error("Pipeline must be an array, e.g. [{\"$match\": {...}}]")),
    }
}

/// 把 shell 构造器换成 Extended JSON。
///
/// 只认白名单，且保持「不做求值」的边界：参数原样当作 JSON 标量或字符串。
fn mongo_shell_rewrite_constructors(text: &str) -> String {
    const CONSTRUCTORS: &[(&str, &str)] = &[
        ("ObjectId", "$oid"),
        ("ISODate", "$date"),
        ("NumberLong", "$numberLong"),
        ("NumberInt", "$numberInt"),
        ("NumberDecimal", "$numberDecimal"),
        ("UUID", "$uuid"),
    ];
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    // 字符串字面量内部原样拷贝：`"call ObjectId(1)"` 里的字样不是构造器。
    let mut quote: Option<char> = None;
    'outer: while index < text.len() {
        let ch = text[index..].chars().next().unwrap_or('\0');
        if let Some(open) = quote {
            out.push(ch);
            index += ch.len_utf8();
            if ch == '\\' {
                // 转义序列整段拷贝，避免把 `\"` 当字符串结束。
                if let Some(next) = text[index..].chars().next() {
                    out.push(next);
                    index += next.len_utf8();
                }
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '"' | '\'' | '`') {
            quote = Some(ch);
            out.push(ch);
            index += ch.len_utf8();
            continue;
        }
        for (name, key) in CONSTRUCTORS {
            let Some(rest) = text[index..].strip_prefix(name) else {
                continue;
            };
            // 前一个字符是标识符字符时不算构造器（`MyObjectId(`）。
            if let Some(previous) = text[..index].chars().next_back()
                && (previous.is_alphanumeric() || previous == '_' || previous == '.')
            {
                continue;
            }
            let Some(after_name) = rest.trim_start().strip_prefix('(') else {
                continue;
            };
            let Some(close) = after_name.find(')') else {
                continue;
            };
            let argument = after_name[..close].trim();
            // Extended JSON 里 `$oid` / `$date` / `$numberLong` 等的值都必须是字符串，
            // 而 shell 写法允许裸数字（`NumberLong(7)`），所以一律按字符串序列化。
            let value = match serde_json::to_string(unquote(argument)) {
                Ok(quoted) => quoted,
                Err(_) => continue,
            };
            out.push_str(&format!("{{\"{key}\": {value}}}"));
            index += name.len() + (rest.len() - after_name.len()) + close + 1;
            continue 'outer;
        }
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

// —— 输出排版 ——

fn mongo_shell_documents_text(documents: &[fluxdb_core::MongoDocument]) -> String {
    let mut out = String::new();
    for (position, document) in documents.iter().enumerate() {
        // 多篇文档之间留空行：工作台是一整块文本，紧贴着排会让人分不清
        // 上一篇的 `}` 和下一篇的 `{` 属于两条记录。
        if position > 0 {
            out.push('\n');
        }
        match mongo_shell_parse_text("Result", &document.json) {
            Ok(value) => {
                out.push_str(&mongo_shell_pretty(&value, 0));
                out.push('\n');
            }
            // 服务端返回的 EJSON 解析不了属于内部问题：原样给出，不让结果凭空消失。
            Err(_) => {
                out.push_str(&document.json);
                out.push('\n');
            }
        }
    }
    if out.is_empty() {
        // 与 mongosh 的空结果一致：什么都不打印（提示符会紧跟下一行）。
        return String::new();
    }
    out.trim_end_matches('\n').to_string()
}

fn mongo_shell_array(values: &[Bson], indent: usize) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }
    let mut out = String::from("[\n");
    for (position, value) in values.iter().enumerate() {
        out.push_str(&" ".repeat(indent + 2));
        out.push_str(&mongo_shell_pretty(value, indent + 2));
        if position + 1 < values.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(&" ".repeat(indent));
    out.push(']');
    out
}

/// Compass 风格排版：键蓝色、字符串绿色、数字橙色，类型带 `ObjectId('…')` 这类标注。
fn mongo_shell_pretty(value: &Bson, indent: usize) -> String {
    match value {
        Bson::Document(document) => mongo_shell_pretty_document(document, indent),
        Bson::Array(values) => mongo_shell_array(values, indent),
        scalar => mongo_shell_scalar(scalar),
    }
}

fn mongo_shell_pretty_document(document: &Document, indent: usize) -> String {
    if document.is_empty() {
        return "{}".to_string();
    }
    let mut out = String::from("{\n");
    let keys: Vec<&str> = document.keys().map(String::as_str).collect();
    for (position, key) in keys.iter().enumerate() {
        out.push_str(&" ".repeat(indent + 2));
        out.push_str(&mongo_shell_color_key(key));
        out.push_str(": ");
        out.push_str(&mongo_shell_pretty(&document[*key], indent + 2));
        if position + 1 < keys.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(&" ".repeat(indent));
    out.push('}');
    out
}

/// 键名按 Compass 的写法：标识符形态不加引号，其余的加引号。
fn mongo_shell_color_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$');
    let text = if bare {
        key.to_string()
    } else {
        format!("\"{key}\"")
    };
    format!("\x1b[34m{text}\x1b[0m")
}

fn mongo_shell_scalar(value: &Bson) -> String {
    match value {
        Bson::String(text) => format!("\x1b[32m'{}'\x1b[0m", text.replace('\\', "\\\\").replace('\'', "\\'")),
        Bson::Symbol(text) => format!("\x1b[32m'{text}'\x1b[0m"),
        Bson::Int32(number) => format!("\x1b[33m{number}\x1b[0m"),
        Bson::Int64(number) => format!("\x1b[33m{number}\x1b[0m"),
        Bson::Double(number) => format!("\x1b[33m{number}\x1b[0m"),
        Bson::Decimal128(number) => format!("\x1b[33m{number}\x1b[0m"),
        Bson::Boolean(flag) => format!("\x1b[36m{flag}\x1b[0m"),
        Bson::Null => "\x1b[90mnull\x1b[0m".to_string(),
        Bson::ObjectId(id) => format!("ObjectId('\x1b[32m{}\x1b[0m')", id.to_hex()),
        Bson::DateTime(date) => format!(
            "ISODate('\x1b[32m{}\x1b[0m')",
            date.try_to_rfc3339_string().unwrap_or_default()
        ),
        Bson::Binary(binary) => format!("BinData({}, '{}')", u8::from(binary.subtype), {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&binary.bytes)
        }),
        Bson::Timestamp(timestamp) => format!(
            "Timestamp({{ t: {}, i: {} }})",
            timestamp.time, timestamp.increment
        ),
        Bson::RegularExpression(regex) => {
            format!("/{}/{regex_flags}", regex.pattern, regex_flags = regex.options)
        }
        Bson::MinKey => "MinKey()".to_string(),
        Bson::MaxKey => "MaxKey()".to_string(),
        Bson::Undefined => "undefined".to_string(),
        // 文档 / 数组只可能经由递归进入本函数；其余冷门类型（DbPointer、JS 代码）直接给 EJSON。
        Bson::Document(_) | Bson::Array(_) => mongo_shell_pretty(value, 0),
        _ => serde_json::to_string(&value.clone().into_relaxed_extjson()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod mongo_shell_tests {
    use super::*;

    /// 去掉 ANSI 颜色：断言只看文本内容，颜色另有专门用例。
    fn plain(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                // CSI 到 `m` 结束。
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
                continue;
            }
            out.push(ch);
        }
        out
    }

    #[test]
    fn parses_use_and_reports_the_switched_database() {
        let config = unreachable_config();
        let result = mongo_shell(&config, "", "use fluxdb_demo").expect("use 必须只切换会话库");
        assert_eq!(result.database.as_deref(), Some("fluxdb_demo"));
        assert_eq!(result.output, "switched to db fluxdb_demo");

        // 缺库名 / 多参数都是用法错误，不能悄悄选一个库。
        assert!(mongo_shell(&config, "", "use").is_err());
        assert!(mongo_shell(&config, "", "use a b").is_err());
    }

    #[test]
    fn parses_find_chain_into_a_find_request() {
        let (name, args, chain) = mongo_shell_split_call(
            r#"find({ event_type: "page_view" }).sort({ duration_ms: -1 }).limit(20)"#,
        )
        .expect("链式调用必须可解析");
        assert_eq!(name, "find");
        assert_eq!(args, vec![r#"{ event_type: "page_view" }"#.to_string()]);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].0, "sort");
        assert_eq!(chain[1], ("limit".to_string(), vec!["20".to_string()]));

        let request = mongo_shell_find_request("events", &args, &chain).expect("组请求");
        assert_eq!(request.collection, "events");
        assert_eq!(request.filter, r#"{ event_type: "page_view" }"#);
        assert_eq!(request.sort, "{ duration_ms: -1 }");
        assert_eq!(request.limit, 20);
    }

    #[test]
    fn pipeline_rejects_write_stages() {
        // 只读终端不允许把结果写回集合；CLI 与命令工作台共用这条入口。
        let out = mongo_shell_pipeline(r#"[{ "$match": {} }, { "$out": "archive" }]"#)
            .expect_err("$out 必须被拒绝");
        assert!(out.message.contains("read-only"), "{}", out.message);
        assert!(
            mongo_shell_pipeline(r#"[{ "$merge": { "into": "archive" } }]"#).is_err(),
            "$merge 必须被拒绝"
        );
        assert!(mongo_shell_pipeline(r#"[{ "$match": {} }]"#).is_ok());
    }

    #[test]
    fn rewrites_shell_constructors_into_extended_json() {
        let text = mongo_shell_rewrite_constructors(
            r#"{ _id: ObjectId("66e000000000000000000001"), at: ISODate("2024-01-01T00:00:00Z"), n: NumberLong(7) }"#,
        );
        assert!(!text.contains("ObjectId("), "{text}");
        let document = mongo_shell_document("Filter", &text).expect("改写成 EJSON 后必须可解析");
        assert!(matches!(
            document.get("_id"),
            Some(Bson::ObjectId(_))
        ), "{document:?}");
        assert!(matches!(document.get("at"), Some(Bson::DateTime(_))));
        assert_eq!(document.get("n"), Some(&Bson::Int64(7)));
    }

    #[test]
    fn constructor_rewrite_does_not_touch_strings_or_longer_identifiers() {
        // 字符串里的同名字样必须原样保留；`MyObjectId(` 不是构造器。
        let text = mongo_shell_rewrite_constructors(r#"{ note: "call ObjectId(1)", id: MyObjectId("x") }"#);
        assert!(text.contains(r#""call ObjectId(1)""#), "{text}");
        assert!(text.contains(r#"MyObjectId("x")"#), "{text}");
    }

    #[test]
    fn rejects_write_and_unknown_methods_with_distinct_messages() {
        let write = mongo_shell_method("deleteMany").expect_err("写方法必须拒绝");
        assert!(write.message.contains("read-only"), "{}", write.message);

        let unknown = mongo_shell_method("findAll").expect_err("未知方法必须拒绝");
        assert!(unknown.message.contains("Unknown method"), "{}", unknown.message);
        // 单行：不要多行用法清单（终端里会糊成一大块红字）。
        assert!(!unknown.message.contains('\n'), "{}", unknown.message);
    }

    #[test]
    fn rejects_extra_show_tokens_instead_of_running_the_first_two() {
        // `show dbs | show collections` 曾被当成 `show dbs` 静默执行。
        let error = mongo_shell_parse("show dbs | show collections").expect_err("多余 token 必须报错");
        assert!(error.message.contains("Usage"), "{}", error.message);

        let error = mongo_shell_parse("show indexes").expect_err("未知 show 子命令必须报错");
        assert!(error.message.contains("Unknown command"), "{}", error.message);
    }

    #[test]
    fn rejects_javascript_and_stray_trailing_syntax() {
        let error = mongo_shell_parse("let x = db.events.find()").expect_err("不支持 JS 变量");
        assert!(error.message.contains("JavaScript"), "{}", error.message);

        let error = mongo_shell_parse("db.events.find() extra").expect_err("多余内容必须报错");
        assert!(error.message.contains("Unexpected"), "{}", error.message);

        let error = mongo_shell_parse("db.events.find(").expect_err("缺右括号必须报错");
        assert!(error.message.contains("Missing )"), "{}", error.message);
    }

    #[test]
    fn parses_quoted_and_bracketed_collection_names() {
        let statement = mongo_shell_parse(r#"db["my.events"].countDocuments()"#).expect("方括号集合名");
        let MongoShellStatement::Collection { collection, .. } = statement else {
            panic!("应解析为集合方法");
        };
        assert_eq!(collection, "my.events");

        let statement = mongo_shell_parse(r#"db["plain"].find()"#).expect("引号集合名");
        let MongoShellStatement::Collection { collection, .. } = statement else {
            panic!("应解析为集合方法");
        };
        assert_eq!(collection, "plain");
    }

    #[test]
    fn parses_show_and_collection_names() {
        assert!(matches!(
            mongo_shell_parse("show dbs").expect("show dbs"),
            MongoShellStatement::Show(MongoShellShow::Databases)
        ));
        assert!(matches!(
            mongo_shell_parse("show collections").expect("show collections"),
            MongoShellStatement::Show(MongoShellShow::Collections)
        ));
        assert!(matches!(
            mongo_shell_parse("db.getCollectionNames()").expect("getCollectionNames"),
            MongoShellStatement::CollectionNames
        ));
        assert!(mongo_shell_parse("db.getCollectionNames(1)").is_err(), "不接受参数");
    }

    #[test]
    fn pretty_output_matches_compass_style() {
        let document = doc! {
            "_id": Bson::ObjectId(mongodb::bson::oid::ObjectId::parse_str("66e000000000000000000001").unwrap()),
            "event_type": "page_view",
            "success": false,
            "duration_ms": 20,
            "nested": {"path": "/"},
        };
        let painted = mongo_shell_pretty(&Bson::Document(document), 0);
        // 颜色只在键 / 标量上，结构字符保持干净。
        assert!(painted.contains("\x1b[34m_id\x1b[0m"), "{painted}");
        assert!(painted.contains("ObjectId('"), "ObjectId 类型标注");
        let text = plain(&painted);
        assert!(text.contains("event_type: 'page_view'"), "{text}");
        assert!(text.contains("success: false"), "{text}");
        assert!(text.contains("duration_ms: 20"), "{text}");
        assert!(text.contains("nested: {"), "嵌套对象要折行: {text}");
        assert!(text.contains("path: '/'"), "{text}");
    }

    #[test]
    fn documents_are_separated_by_a_blank_line() {
        let documents = vec![
            fluxdb_core::MongoDocument {
                id: "1".into(),
                json: r#"{"_id": 1}"#.into(),
            },
            fluxdb_core::MongoDocument {
                id: "2".into(),
                json: r#"{"_id": 2}"#.into(),
            },
        ];
        let text = plain(&mongo_shell_documents_text(&documents));
        // 两条记录之间必须是空行，不能是 `}\n{` 紧贴。
        assert!(text.contains("}\n\n{"), "{text}");
    }

    #[test]
    fn byte_sizes_read_like_mongosh() {
        assert_eq!(mongo_shell_bytes(512.0), "512B");
        assert_eq!(mongo_shell_bytes(1024.0), "1.000KB");
        assert_eq!(mongo_shell_bytes(1024.0 * 1024.0 * 20.5), "20.500MB");
    }

    /// 指向不可达地址的连接：本组用例不连库，只验证解析与排版。
    fn unreachable_config() -> ConnectionConfig {
        ConnectionConfig {
            id: fluxdb_core::ConnectionId(1),
            name: "unit".into(),
            kind: fluxdb_core::DatabaseKind::MongoDb,
            endpoint: fluxdb_core::Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1,
                database: Some("demo".into()),
            },
            options: std::collections::BTreeMap::new(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }
}
