// MongoDB 集合/视图元数据与索引读取。
//
// 统计口径对齐 `db.collection.stats()`：拿不到（视图、权限不足、分片元数据缺失）
// 就留 `None`，不用 0 或估算值冒充。

/// 读取集合/视图元数据。
fn mongo_collection_info(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
) -> fluxdb_core::Result<MongoCollectionInfo> {
    let client = mongo_connect(config)?;
    let database_handle = client.database(database);

    // 1) listCollections 拿类型与视图定义（视图拿不到 stats/count）。
    let filter = doc! {"name": collection};
    let list_handle = database_handle.clone();
    let cursor = mongo_block_on(30, "读取集合元数据", async move {
        list_handle.list_collections().filter(filter).await
    })?;
    let spec = mongo_collect_cursor(cursor)?.into_iter().next();
    let Some(spec) = spec else {
        return Err(Error::new(
            ErrorKind::Query,
            format!("集合 `{collection}` 不存在或无权访问"),
        ));
    };

    let mut info = MongoCollectionInfo {
        name: collection.to_string(),
        ..Default::default()
    };
    match spec.collection_type {
        CollectionType::View => {
            info.kind = MongoCollectionKind::View;
            info.view_on = spec.options.view_on;
            info.view_pipeline = spec.options.pipeline.map(|stages| {
                mongo_json_text(Bson::Array(
                    stages.into_iter().map(Bson::Document).collect(),
                ))
            });
            // 视图没有自己的统计：直接返回类型与定义。
            return Ok(info);
        }
        CollectionType::Timeseries => info.kind = MongoCollectionKind::Timeseries,
        // 未知的新类型按普通集合处理（`CollectionType` 是 non_exhaustive）。
        _ => info.kind = MongoCollectionKind::Collection,
    }

    // 2) stats 拿大小与索引数；权限不足时整段跳过，不阻断页面。
    if let Ok(stats) = mongo_block_on(30, "读取集合统计", async {
        database_handle.run_command(doc! {"collStats": collection}).await
    }) {
        info.avg_document_size = mongo_bson_u64(stats.get("avgObjSize"));
        info.data_size = mongo_bson_u64(stats.get("size"));
        info.storage_size = mongo_bson_u64(stats.get("storageSize"));
        info.index_count = mongo_bson_u64(stats.get("nindexes"));
    }

    // 3) 文档数：`estimatedDocumentCount` 不扫表，先给一个可用的近似值；
    //    精确计数在集合页里按需触发（`count_documents` 大集合代价高）。
    let collection_handle = database_handle.collection::<Document>(collection);
    info.document_count = mongo_block_on(30, "读取集合文档数", async move {
        collection_handle.estimated_document_count().await
    })
    .ok();

    Ok(info)
}

/// `$indexStats` 里一个索引的使用统计。
#[derive(Default)]
struct MongoIndexUsage {
    usage_count: Option<u64>,
    /// `accesses.since`（epoch 毫秒）。
    usage_since: Option<i64>,
    /// `$indexStats.building`：仅在索引构建期间出现。
    building: bool,
}

/// 列出集合索引。
///
/// 除 `listIndexes` 的定义与选项外，再补两类统计：`collStats.indexSizes` 给大小、
/// `$indexStats` 给使用次数与统计起点。两级统计都允许失败（权限不足、服务端不支持），
/// 失败时字段留 `None`，索引列表本身照常返回。
fn mongo_indexes(
    config: &ConnectionConfig,
    database: &str,
    collection: &str,
) -> fluxdb_core::Result<Vec<MongoIndexInfo>> {
    let client = mongo_connect(config)?;
    let collection_handle = client.database(database).collection::<Document>(collection);
    let cursor = mongo_block_on(30, "读取索引列表", async move {
        collection_handle.list_indexes().await
    })?;

    let sizes = mongo_index_sizes(&client, database, collection);
    let usages = mongo_index_usage(&client, database, collection);

    let mut indexes = mongo_collect_cursor(cursor)?
        .into_iter()
        .map(|model| {
            let options = model.options.unwrap_or_default();
            let name = options.name.clone().unwrap_or_default();
            let usage = usages.get(&name);
            MongoIndexInfo {
                index_type: mongo_index_type(&model.keys, &options).to_string(),
                fields: model
                    .keys
                    .iter()
                    .map(|(field, value)| MongoIndexField {
                        field: field.clone(),
                        value: mongo_index_key_text(value),
                    })
                    .collect(),
                keys_json: mongo_json_text(Bson::Document(model.keys)),
                unique: options.unique.unwrap_or(false),
                sparse: options.sparse.unwrap_or(false),
                partial_filter: options
                    .partial_filter_expression
                    .map(|filter| mongo_json_text(Bson::Document(filter))),
                expire_after_seconds: options
                    .expire_after
                    .and_then(|duration| i64::try_from(duration.as_secs()).ok()),
                has_collation: options.collation.is_some(),
                hidden: options.hidden.unwrap_or(false),
                size_bytes: sizes.get(&name).copied(),
                usage_count: usage.and_then(|usage| usage.usage_count),
                usage_since: usage.and_then(|usage| usage.usage_since),
                // 没有 `$indexStats` 时无法判定构建态，按已就绪处理，不臆造"构建中"。
                ready: !usage.is_some_and(|usage| usage.building),
                name,
            }
        })
        .collect::<Vec<_>>();
    indexes.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(indexes)
}

/// `collStats.indexSizes`：按索引名取字节数。
///
/// 独立部署与副本集可用；权限不足或分片集群元数据缺失时整段返回空表。
fn mongo_index_sizes(
    client: &MongoClient,
    database: &str,
    collection: &str,
) -> HashMap<String, u64> {
    let command = doc! {"collStats": collection};
    let stats = mongo_block_on(30, "读取索引大小", async {
        client.database(database).run_command(command).await
    })
    .ok();
    let Some(stats) = stats else {
        return HashMap::new();
    };
    let Some(Bson::Document(sizes)) = stats.get("indexSizes") else {
        return HashMap::new();
    };
    sizes
        .iter()
        .filter_map(|(name, value)| {
            mongo_bson_u64(Some(value)).map(|size| (name.clone(), size))
        })
        .collect()
}

/// `$indexStats`：按索引名取使用次数、统计起点与构建状态。
fn mongo_index_usage(
    client: &MongoClient,
    database: &str,
    collection: &str,
) -> HashMap<String, MongoIndexUsage> {
    let collection_handle = client.database(database).collection::<Document>(collection);
    let cursor = mongo_block_on(30, "读取索引使用统计", async move {
        collection_handle.aggregate(vec![doc! {"$indexStats": {}}]).await
    })
    .ok();
    let Some(cursor) = cursor else {
        return HashMap::new();
    };
    let Ok(documents) = mongo_collect_cursor(cursor) else {
        return HashMap::new();
    };
    documents
        .into_iter()
        .filter_map(|document| {
            let name = document.get_str("name").ok()?.to_string();
            let accesses = document.get_document("accesses").ok();
            let usage = MongoIndexUsage {
                usage_count: accesses.and_then(|accesses| mongo_bson_u64(accesses.get("ops"))),
                usage_since: accesses.and_then(|accesses| match accesses.get("since") {
                    Some(Bson::DateTime(since)) => Some(since.timestamp_millis()),
                    _ => None,
                }),
                building: matches!(document.get("building"), Some(Bson::Boolean(true))),
            };
            Some((name, usage))
        })
        .collect()
}

/// 索引键值转展示文本：数字方向与字符串类型都直接转文本。
fn mongo_index_key_text(value: &Bson) -> String {
    match value {
        Bson::Int32(number) => number.to_string(),
        Bson::Int64(number) => number.to_string(),
        Bson::Double(number) if number.fract() == 0.0 => format!("{number:.0}"),
        Bson::Double(number) => number.to_string(),
        Bson::String(text) => text.clone(),
        other => mongo_json_text(other.clone()),
    }
}

/// 索引类型判定，口径与 Compass `getIndexType` 一致。
fn mongo_index_type(keys: &Document, options: &IndexOptions) -> &'static str {
    let key_value = |expected: &str| {
        keys.values()
            .any(|value| matches!(value, Bson::String(text) if text == expected))
    };
    if options.sphere_2d_index_version.is_some()
        || key_value("2d")
        || key_value("2dsphere")
        || key_value("geoHaystack")
    {
        "geospatial"
    } else if key_value("hashed") {
        "hashed"
    } else if options.text_index_version.is_some() {
        "text"
    } else if key_value("columnstore") {
        // 列存储键是通配索引的特例，必须先于 wildcard 判断。
        "columnstore"
    } else if keys
        .keys()
        .any(|field| field == "$**" || field.contains(".$**"))
    {
        "wildcard"
    } else if options.clustered() == Some(true) {
        "clustered"
    } else {
        "regular"
    }
}

/// 连接信息：「显示连接信息」弹框的数据。
///
/// 只汇总服务端真实返回的字段：`hello` 给拓扑/成员，`buildInfo` 给版本与发行版，
/// `listDatabases` 给库大小。任一命令失败只留空对应字段（不阻断弹框、不编造值）。
fn mongo_server_info(config: &ConnectionConfig) -> fluxdb_core::Result<MongoServerInfo> {
    let client = mongo_connect(config)?;
    let admin = client.database("admin");

    // 1) hello：拓扑判定与成员列表。
    let hello = mongo_block_on(20, "读取 MongoDB 拓扑信息", async {
        admin.run_command(doc! {"hello": 1}).await
    })
    .map_err(|error| {
        // 连 hello 都失败说明连接本身不可用，此时弹框无内容可展示。
        Error::new(error.kind, format!("读取 MongoDB 连接信息失败：{}", error.message))
    })?;

    let mut info = MongoServerInfo {
        topology: if hello.get_str("msg") == Ok("isdbgrid") {
            MongoTopology::Sharded
        } else if hello.get_str("setName").is_ok() {
            MongoTopology::ReplicaSet
        } else if hello.contains_key("isWritablePrimary") || hello.contains_key("ismaster") {
            MongoTopology::Standalone
        } else {
            MongoTopology::Unknown
        },
        replica_set_name: hello.get_str("setName").ok().map(str::to_string),
        primary: hello.get_str("primary").ok().map(str::to_string),
        hosts: hello
            .get_array("hosts")
            .map(|hosts| {
                hosts
                    .iter()
                    .filter_map(|host| host.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        max_wire_version: hello.get_i32("maxWireVersion").ok(),
        uptime_secs: mongo_bson_u64(hello.get("uptime")),
        ..Default::default()
    };

    // 2) buildInfo：版本与发行版（`modules` 含 enterprise 才是企业版）。
    if let Ok(build) = mongo_block_on(20, "读取 MongoDB 版本信息", async {
        admin.run_command(doc! {"buildInfo": 1}).await
    }) {
        info.version = build.get_str("version").ok().map(str::to_string);
        let is_enterprise = build
            .get_array("modules")
            .map(|modules| modules.iter().any(|module| module.as_str() == Some("enterprise")))
            .unwrap_or(false);
        info.edition = Some(if is_enterprise { "Enterprise" } else { "Community" }.to_string());
    }

    // 3) listDatabases：库大小。权限不足时整段跳过。
    if let Ok(list) = mongo_block_on(20, "读取数据库列表", async {
        client.list_databases().await
    }) {
        info.databases = list
            .into_iter()
            .map(|database| MongoDatabaseStat {
                name: database.name,
                size_on_disk: Some(database.size_on_disk),
                empty: database.empty,
            })
            .collect();
    }

    Ok(info)
}

/// BSON 数值（int32/int64/double）转 u64；负数与非数值返回 `None`。
fn mongo_bson_u64(value: Option<&Bson>) -> Option<u64> {
    match value? {
        Bson::Int32(number) => u64::try_from(*number).ok(),
        Bson::Int64(number) => u64::try_from(*number).ok(),
        Bson::Double(number) if *number >= 0.0 => Some(*number as u64),
        _ => None,
    }
}
