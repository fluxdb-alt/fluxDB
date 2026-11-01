// MongoDB 文档读取与数据库/集合的增删。

/// 列出可见数据库。系统库（admin/local/config）不展示。
fn mongo_list_databases(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let client = mongo_connect(config)?;
    let names = mongo_block_on(30, "读取 MongoDB 数据库列表", async move {
        client.list_database_names().await
    })?;

    let mut names: Vec<String> = names
        .into_iter()
        .filter(|name| !MONGO_SYSTEM_DATABASES.contains(&name.as_str()))
        .collect();
    names.sort();
    // 超上限只截断展示，不报错：浏览器用途，用户可用搜索定位。
    names.truncate(MONGO_SIDEBAR_DATABASE_LIMIT);
    Ok(names
        .into_iter()
        .map(|name| database_object(config.id, &name))
        .collect())
}

/// 列出库内集合与视图。视图标记为 `ObjectKind::View`（只读）。
fn mongo_list_collections(
    config: &ConnectionConfig,
    connection_id: ConnectionId,
    database: &str,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let client = mongo_connect(config)?;
    let database_handle = client.database(database);

    // `listCollections` 一次拿到类型（collection / view / timeseries），比逐个探测省往返。
    let cursor = mongo_block_on(30, "读取 MongoDB 集合列表", async move {
        database_handle.list_collections().await
    })?;

    let mut objects = Vec::new();
    for spec in mongo_collect_cursor(cursor)? {
        if spec.name.starts_with("system.") {
            continue;
        }
        // 时间序列集合按普通集合处理（可读可写），只有视图只读。
        let kind = match spec.collection_type {
            CollectionType::View => ObjectKind::View,
            // 时间序列可读可写；未知的新类型也按普通集合处理（`CollectionType` 是 non_exhaustive）。
            _ => ObjectKind::Collection,
        };
        objects.push(ObjectSummary {
            path: ObjectPath {
                connection_id,
                database: Some(database.to_string()),
                schema: None,
                name: spec.name,
                kind,
            },
            rows: None,
            modified_at: None,
            comment: None,
            stable: None,
        });
    }
    objects.sort_by(|left, right| left.path.name.cmp(&right.path.name));
    Ok(objects)
}

/// 把驱动游标读空（游标本身按批拉取，单批失败即整体失败，不静默丢数据）。
fn mongo_collect_cursor<T>(cursor: mongodb::Cursor<T>) -> fluxdb_core::Result<Vec<T>>
where
    T: serde::de::DeserializeOwned + Send,
{
    mongo_block_on(120, "cursor", async move {
        use futures_util::TryStreamExt as _;
        cursor.try_collect::<Vec<T>>().await
    })
}

/// 执行 `find`。
///
/// 取 `limit + 1` 条判定 `has_more`，避免为了翻页额外做全表 count；
/// `total` 只在结果集不足一页时填充（此时已知总数），其余保持 `None`，不伪造统计。
fn mongo_find(
    config: &ConnectionConfig,
    database: &str,
    request: &MongoFindRequest,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<MongoDocumentsPage> {
    if request.collection.trim().is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定集合名"));
    }
    // 查询条件接受 Compass 的宽松写法（`{ age: 41 }`），这是一次只读请求；
    // 写文档的入口仍只认严格 JSON（见 `mongo_parse_object_inner` 的说明）。
    let filter = mongo_parse_query_object("Filter", &request.filter)?;
    let projection = mongo_parse_query_object("Projection", &request.projection)?;
    let sort = mongo_parse_query_object("Sort", &request.sort)?;
    mongo_validate_sort(&sort)?;
    let collation = mongo_parse_collation(&request.collation)?;
    let hint = mongo_parse_hint(&request.hint)?;

    // `limit = 0` 表示调用方未填页大小，按默认页处理；直接夹到 1 会让人误判集合只有
    // 一条数据（真实Atlas用例即如此），因此这里显式给默认值而不是静默取 1。
    let limit = match request.limit {
        0 => fluxdb_core::Pagination::DEFAULT_LIMIT,
        value => value,
    }
    .clamp(1, MONGO_MAX_DOCUMENTS_PER_QUERY);
    let client = mongo_connect(config)?;
    let collection = client
        .database(database)
        .collection::<Document>(&request.collection);

    let mut find = collection.find(filter);
    if !projection.is_empty() {
        find = find.projection(projection);
    }
    if !sort.is_empty() {
        find = find.sort(sort);
    }
    if let Some(collation) = collation {
        find = find.collation(collation);
    }
    if let Some(hint) = hint {
        find = find.hint(hint);
    }
    if let Some(max_time_ms) = request.max_time_ms.filter(|value| *value > 0) {
        find = find.max_time(Duration::from_millis(max_time_ms));
    }
    let find = find
        .skip(request.skip)
        // 多取一条用于判定 has_more。
        .limit(limit.saturating_add(1) as i64)
        .batch_size(limit.min(200) as u32);

    if should_cancel() {
        return Err(Error::new(ErrorKind::Cancelled, "查询已取消"));
    }

    // 游标读取必须在放大栈的线程里一次做完：驱动解码结果文档的代码同样跑在调用方
    // 线程上，逐条 `block_on` 既会继承小栈，也会退化成「每条文档开一个线程」。
    // 单次读取上限是 `limit + 1`（≤ 501 条），因此取消只在进入前判断即可。
    let mut cursor = mongo_block_on(60, "find", async move { find.await })?;
    let (documents, fetched) = mongo_block_on(60, "cursor", async move {
        use futures_util::TryStreamExt as _;
        let mut documents = Vec::new();
        let mut fetched: u64 = 0;
        while let Some(document) = cursor.try_next().await? {
            fetched += 1;
            if fetched > limit {
                // 多取到的那条只用于判定还有下一页，不返回。
                break;
            }
            documents.push(MongoDocument {
                id: mongo_document_id(&document),
                json: mongo_json_text(Bson::Document(document)),
            });
        }
        Ok((documents, fetched))
    })?;

    let has_more = fetched > limit;
    // 不足一页时总数可确定（skip + 本页条数），无需再查。
    let total = (!has_more).then_some(request.skip + documents.len() as u64);
    Ok(MongoDocumentsPage {
        returned: documents.len() as u64,
        documents,
        total,
        has_more,
    })
}

/// 执行聚合管道。`pipeline` 必须是 Extended JSON **数组**文本，元素为各个 stage 文档。
///
/// 结果集上限与 `find` 一致（`MONGO_MAX_DOCUMENTS_PER_QUERY`），避免聚合把内存打满；
/// 超出部分不返回，由 `has_more` 告知 UI。
fn mongo_aggregate(
    config: &ConnectionConfig,
    path: &ObjectPath,
    pipeline: &str,
) -> fluxdb_core::Result<MongoDocumentsPage> {
    let database = path
        .database
        .as_deref()
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Query, "请指定数据库"))?;
    let collection = path.name.trim();
    if collection.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定集合名"));
    }
    let text = pipeline.trim();
    // 空管道等价于「不筛选」，直接返回前 N 条。
    let stages: Vec<Document> = if text.is_empty() {
        Vec::new()
    } else {
        let value = mongo_parse_json("聚合管道", text, true)?;
        let value = Bson::try_from(value).map_err(|error| {
            Error::new(
                ErrorKind::Query,
                format!("聚合管道不是合法的 Extended JSON：{error}"),
            )
        })?;
        match value {
            Bson::Array(items) => items
                .into_iter()
                .map(|item| match item {
                    Bson::Document(stage) => Ok(stage),
                    _ => Err(Error::new(ErrorKind::Query, "聚合管道的每个阶段必须是对象")),
                })
                .collect::<fluxdb_core::Result<Vec<_>>>()?,
            _ => {
                return Err(Error::new(
                    ErrorKind::Query,
                    "聚合管道必须是 Extended JSON 数组",
                ));
            }
        }
    };

    let limit = MONGO_MAX_DOCUMENTS_PER_QUERY;
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let cursor = mongo_block_on(60, "MongoDB 聚合", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .aggregate(stages)
            .await
    })?;
    let documents = mongo_collect_cursor(cursor)?;
    let has_more = documents.len() as u64 > limit;
    let documents = documents
        .into_iter()
        .take(limit as usize)
        .map(|document| MongoDocument {
            id: mongo_document_id(&document),
            json: mongo_json_text(Bson::Document(document)),
        })
        .collect::<Vec<_>>();
    Ok(MongoDocumentsPage {
        returned: documents.len() as u64,
        documents,
        total: None,
        has_more,
    })
}

/// 抽样文档给补全采集字段：`$sample` 走服务端随机抽样，比 `find` 前 N 条更能覆盖字段。
///
/// 对齐 Compass 的 schema 采样：`promoteValues: false` 保留 BSON 类型，`secondaryPreferred`
/// 优先打从节点，不挤占主库。抽样条数由调用方给（补全只需几百条足够）。
fn mongo_sample_documents(
    config: &ConnectionConfig,
    path: &ObjectPath,
    size: u32,
) -> fluxdb_core::Result<Vec<String>> {
    let database = path
        .database
        .as_deref()
        .filter(|database| !database.trim().is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Query, "请指定数据库"))?;
    let collection = path.name.trim();
    if collection.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定集合名"));
    }
    let size = size.clamp(1, fluxdb_core::MONGO_MAX_DOCUMENTS_PER_QUERY as u32);
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let pipeline = vec![doc! {
        "$sample": { "size": size as i64 }
    }];
    let cursor = mongo_block_on(30, "MongoDB 采样", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            // 抽样只用于补全，读从节点即可。
            .aggregate(pipeline)
            .selection_criteria(SelectionCriteria::ReadPreference(
                ReadPreference::SecondaryPreferred {
                    options: Default::default(),
                },
            ))
            .await
    })?;
    Ok(mongo_collect_cursor(cursor)?
        .into_iter()
        .map(|document| mongo_json_text(Bson::Document(document)))
        .collect())
}

/// 排序校验：值只能是 1 / -1（`$meta` 等文本排序暂不支持，显式拒绝）。
fn mongo_validate_sort(sort: &Document) -> fluxdb_core::Result<()> {
    for (field, value) in sort {
        let direction = match value {
            Bson::Int32(number) => *number as i64,
            Bson::Int64(number) => *number,
            Bson::Double(number) => *number as i64,
            _ => {
                return Err(Error::new(
                    ErrorKind::Query,
                    format!("Sort 字段 `{field}` 的方向必须是 1 或 -1"),
                ));
            }
        };
        if direction != 1 && direction != -1 {
            return Err(Error::new(
                ErrorKind::Query,
                format!("Sort 字段 `{field}` 的方向必须是 1 或 -1"),
            ));
        }
    }
    Ok(())
}

/// 排序规则：Extended JSON 对象直接交给驱动的 `Collation` 反序列化。
///
/// 不自己逐字段翻译：`Collation` 的字段（strength / caseLevel / numericOrdering …）
/// 与 wire protocol 一一对应，手写映射只会漏字段、也容易把拼写错误静默吞成默认值。
/// 因此这里只做「空 → 不设置」和「解析失败 → 明确指出是 Collation 的问题」。
fn mongo_parse_collation(text: &str) -> fluxdb_core::Result<Option<Collation>> {
    let document = mongo_parse_query_object("Collation", text)?;
    if document.is_empty() {
        return Ok(None);
    }
    mongodb::bson::from_document::<Collation>(document)
        .map(Some)
        .map_err(|error| {
            Error::new(
                ErrorKind::Query,
                format!("Collation 无法识别（locale 必填）：{error}"),
            )
        })
}

/// 索引提示：`"indexName"` 或 `{ field: 1 }`（与 Compass 的 Index Hint 一致）。
fn mongo_parse_hint(text: &str) -> fluxdb_core::Result<Option<Hint>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match mongo_parse_json("Index Hint", trimmed, true)? {
        serde_json::Value::String(name) if !name.trim().is_empty() => {
            Ok(Some(Hint::Name(name.trim().to_string())))
        }
        value @ serde_json::Value::Object(_) => match Bson::try_from(value) {
            Ok(Bson::Document(keys)) if !keys.is_empty() => Ok(Some(Hint::Keys(keys))),
            _ => Err(Error::new(
                ErrorKind::Query,
                "Index Hint 必须是索引名或非空的键定义对象",
            )),
        },
        _ => Err(Error::new(
            ErrorKind::Query,
            "Index Hint 必须是索引名或键定义对象，如 \"_id_\" 或 { \"age\": 1 }",
        )),
    }
}

/// 创建集合。MongoDB 的库由首个集合隐式创建，因此这里也承担"建库"职责。
fn mongo_create_collection(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
) -> fluxdb_core::Result<()> {
    let database = database.trim();
    let collection = collection.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请填写数据库名"));
    }
    if collection.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请填写集合名"));
    }
    // MongoDB 命名的硬限制：库名不能含 `/\."$` 与空字符；集合名不能含 `$` 且不能为空。
    if database.contains(['/', '\\', '.', '"', '$', '\0']) {
        return Err(Error::new(
            ErrorKind::Query,
            "数据库名不能包含 / \\ . \" $ 或空字符",
        ));
    }
    if collection.contains(['$', '\0']) {
        return Err(Error::new(ErrorKind::Query, "集合名不能包含 $ 或空字符"));
    }

    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    mongo_block_on(30, "创建集合", async move {
        client
            .database(&database)
            .create_collection(collection)
            .await
    })
    .map(|_| ())
}

/// 删除集合。
fn mongo_drop_collection(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
) -> fluxdb_core::Result<()> {
    if database.trim().is_empty() || collection.trim().is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定数据库与集合"));
    }
    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    mongo_block_on(30, "删除集合", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .drop()
            .await
    })
    .map(|_| ())
}

/// 重命名集合（同库内）。
///
/// `renameCollection` 只能对 `admin` 库执行，且两个名字都必须带库名前缀；
/// 视图不可重命名（服务端会拒绝），因此这里不额外区分，错误原样上抛。
fn mongo_rename_collection(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    new_name: &str,
) -> fluxdb_core::Result<()> {
    let database = database.trim();
    let collection = collection.trim();
    let new_name = new_name.trim();
    if database.is_empty() || collection.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定数据库与集合"));
    }
    if new_name.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请填写新的集合名"));
    }
    if new_name == collection {
        return Err(Error::new(ErrorKind::Query, "集合名没有变化"));
    }
    if new_name.contains(['$', '\0']) {
        return Err(Error::new(ErrorKind::Query, "集合名不能包含 $ 或空字符"));
    }
    if MONGO_SYSTEM_DATABASES.contains(&database) {
        return Err(Error::new(ErrorKind::Query, "系统库不允许修改集合"));
    }

    let client = mongo_connect(config)?;
    let from = format!("{database}.{collection}");
    let to = format!("{database}.{new_name}");
    mongo_block_on(60, "重命名集合", async move {
        client
            .database("admin")
            .run_command(doc! {
                "renameCollection": from,
                "to": to,
                "dropTarget": false,
            })
            .await
    })
    .map(|_| ())
}

/// 整篇替换文档：`_id` 取自原文档的原始 BSON 值，新文档的 `_id` 必须与之相等。
///
/// 用原文档的 `_id`（而不是 UI 上的可读 id 文本）做定位条件，ObjectId / 字符串 /
/// 内嵌文档都能精确命中；同时拒绝改 `_id`——MongoDB 不支持更新 `_id`，这里提前报错
/// 而不是让驱动抛一个难以理解的写错误。
fn mongo_replace_document(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    before_json: &str,
    after_json: &str,
) -> fluxdb_core::Result<()> {
    let database = database.trim();
    let collection = collection.trim();
    if database.is_empty() || collection.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定数据库与集合"));
    }
    let before = mongo_parse_object("原文档", before_json)?;
    let after = mongo_parse_object("新文档", after_json)?;
    let Some(id) = before.get("_id").cloned() else {
        return Err(Error::new(
            ErrorKind::Query,
            "原文档缺少 _id，无法定位要修改的文档",
        ));
    };
    match after.get("_id") {
        Some(next) if next == &id => {}
        Some(_) => {
            return Err(Error::new(ErrorKind::Query, "不支持修改 _id"));
        }
        None => return Err(Error::new(ErrorKind::Query, "新文档不能删除 _id")),
    }

    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let result = mongo_block_on(60, "修改文档", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .replace_one(doc! {"_id": id}, &after)
            .await
    })?;
    // 匹配数为 0：文档在编辑期间被别处删掉或改了 `_id`，不能静默当成成功。
    if result.matched_count == 0 {
        return Err(Error::new(
            ErrorKind::Query,
            "文档不存在或已被删除，未做任何修改",
        ));
    }
    Ok(())
}

/// 删除一篇文档：定位条件取自原文档的原始 BSON `_id`，与整篇替换用同一套规则。
///
/// 删除数为 0 说明文档已被别处删掉或 `_id` 已改变，必须报错而不是当成成功——
/// 界面会据此提示"文档不存在"，而不是假装删掉了再刷新出一模一样的列表。
fn mongo_delete_document(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    document_json: &str,
) -> fluxdb_core::Result<()> {
    let database = database.trim();
    let collection = collection.trim();
    if database.is_empty() || collection.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定数据库与集合"));
    }
    let document = mongo_parse_object("文档", document_json)?;
    let Some(id) = document.get("_id").cloned() else {
        return Err(Error::new(
            ErrorKind::Query,
            "文档缺少 _id，无法定位要删除的文档",
        ));
    };

    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let result = mongo_block_on(60, "删除文档", async move {
        client
            .database(&database)
            .collection::<Document>(&collection)
            .delete_one(doc! {"_id": id})
            .await
    })?;
    if result.deleted_count == 0 {
        return Err(Error::new(
            ErrorKind::Query,
            "文档不存在或已被删除，未做任何修改",
        ));
    }
    Ok(())
}

/// 插入一篇新文档，返回新文档的 `_id`（Extended JSON 文本）。
///
/// `_id` 可省略（「复制文档」传来的副本已剥掉 `_id`），由服务端生成；也可显式给出
/// ObjectId（界面「新增文档」预填的那个）。返回它是因为界面需要确认「插进去的到底是
/// 哪一篇」，而不是只报一句成功。
fn mongo_insert_document(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
    document_json: &str,
) -> fluxdb_core::Result<String> {
    let database = database.trim();
    let collection = collection.trim();
    if database.is_empty() || collection.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定数据库与集合"));
    }
    let document = mongo_parse_object("文档", document_json)?;
    // 自带 `_id` 允许，但必须是 ObjectId：界面「新增文档」会预填一个，插完可立即拿去查询。
    // 其它类型（尤其内嵌文档 / 数组）拒绝——那多半是把 `_id` 当成普通字段误填，
    // 与其让它在集合里制造一个和既有数据对不上的主键，不如当场说明。
    if let Some(id) = document
        .get("_id")
        .filter(|id| !matches!(id, Bson::ObjectId(_)))
    {
        return Err(Error::new(
            ErrorKind::Query,
            format!(
                "_id 必须是 ObjectId（形如 {{\"$oid\": \"…\"}} 或 24 位十六进制字符串），当前是 {id:?}"
            ),
        ));
    }

    let client = mongo_connect(config)?;
    let database = database.to_string();
    let collection = collection.to_string();
    let inserted_id = mongo_block_on(60, "新增文档", async move {
        // 先拿 BSON 值再转回扩展 JSON：让它按真实类型（ObjectId 等）呈现。
        client
            .database(&database)
            .collection::<Document>(&collection)
            .insert_one(&document)
            .await
            .map(|result| result.inserted_id)
    })?;
    // 返回的是 `_id` 的值本身（如 `{"$oid":"…"}`），不是包着 `_id` 的整篇文档：
    // 调用方多半要拿它拼过滤条件，多一层包装会变成一个匹配不到的查询。
    Ok(inserted_id.into_relaxed_extjson().to_string())
}

/// 删除数据库（连带其中全部集合）。
fn mongo_drop_database(config: &ConnectionConfig, database: &str) -> fluxdb_core::Result<()> {
    let database = database.trim();
    if database.is_empty() {
        return Err(Error::new(ErrorKind::Query, "请指定数据库"));
    }
    if MONGO_SYSTEM_DATABASES.contains(&database) {
        return Err(Error::new(ErrorKind::Query, "系统库不允许删除"));
    }
    let client = mongo_connect(config)?;
    let database = database.to_string();
    mongo_block_on(60, "删除数据库", async move {
        client.database(&database).drop().await
    })
    .map(|_| ())
}

/// 执行数据库命令。接受 Extended JSON 对象文本（如 `{"ping": 1}`、`{"count": "orders"}`）。
fn mongo_run_command(
    config: &ConnectionConfig,
    database: &str,
    command: &str,
) -> fluxdb_core::Result<String> {
    let document = mongo_parse_object("命令", command)?;
    if document.is_empty() {
        return Err(Error::new(ErrorKind::Query, "命令不能为空"));
    }
    let client = mongo_connect(config)?;
    let database = if database.trim().is_empty() {
        "admin".to_string()
    } else {
        database.to_string()
    };
    mongo_block_on(60, "执行 MongoDB 命令", async move {
        client.database(&database).run_command(document).await
    })
    .map(mongo_command_text)
}
