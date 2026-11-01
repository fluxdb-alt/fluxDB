//! MongoDB 真实连接集成测试。
//!
//! 仅在显式提供 `FLUXDB_MONGO_URI` 时运行；默认跳过，避免常规单测依赖外部实例。
//! 凭据只从环境变量读入内存（`SecretRef::inline`），不写文件、不打印到日志。
//!
//! 运行示例（凭据由调用方从自己的凭据文件注入，不要写进仓库）：
//! ```text
//! FLUXDB_MONGO_URI='mongodb+srv://user:pass@cluster/db?retryWrites=true' \
//!   cargo test --locked -p fluxdb-connectors --test mongo_live -- --nocapture
//! ```
//!
//! 测试自建 `fluxdb_smoke_*` 集合，结束时删除，不触碰既有数据。

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

use fluxdb_connectors::connector_for;
use fluxdb_core::{
    ConnectionConfig, ConnectionId, DatabaseKind, Endpoint, MongoConnectionProfile,
    MongoFindRequest, MongoImportOptions, MongoImportPreviewRequest, MongoImportRequest,
    ObjectKind, ObjectPath,
};

/// 从 `FLUXDB_MONGO_URI` 构造连接配置；未设置时返回 `None`（用例跳过）。
///
/// 走与 UI 导入完全相同的 `MongoConnectionProfile::from_uri`，因此本测试同时覆盖
/// URI 解析 → 档案 → 拨号 → 命令执行的完整链路。
fn live_config() -> Option<ConnectionConfig> {
    let uri = std::env::var("FLUXDB_MONGO_URI").ok()?;
    let uri = uri.trim();
    if uri.is_empty() {
        return None;
    }
    let profile = MongoConnectionProfile::from_uri(uri)
        .unwrap_or_else(|error| panic!("FLUXDB_MONGO_URI 解析失败: {error}"));
    Some(ConnectionConfig {
        id: ConnectionId(9_901),
        name: "MongoDB live".into(),
        kind: DatabaseKind::MongoDb,
        endpoint: Endpoint::Tcp {
            host: profile
                .basic
                .hosts
                .first()
                .map(|host| host.host.clone())
                .unwrap_or_default(),
            port: profile
                .basic
                .hosts
                .first()
                .map(|host| host.port)
                .unwrap_or(27_017),
            database: Some(profile.basic.default_database.clone()),
        },
        options: BTreeMap::new(),
        redis_profile: None,
        mysql_profile: None,
        postgres_profile: None,
        dameng_profile: None,
        sqlserver_profile: None,
        mongo_profile: Some(profile),
        oracle_profile: None,
    })
}

/// 目标库：优先 URI 里的默认库，回落到 `FLUXDB_MONGO_DB`，再回落到 `fluxdb_smoke`。
fn live_database(config: &ConnectionConfig) -> String {
    config
        .mongo_profile
        .as_ref()
        .map(|profile| profile.basic.default_database.clone())
        .filter(|database| !database.is_empty())
        .or_else(|| std::env::var("FLUXDB_MONGO_DB").ok())
        .unwrap_or_else(|| "fluxdb_smoke".to_string())
}

/// 不取消的查询（UI 侧由取消标志驱动，测试里固定为 `false`）。
fn never_cancel() -> &'static dyn Fn() -> bool {
    &NO_CANCEL
}

static NO_CANCEL: fn() -> bool = || false;
static ALWAYS_CANCEL: fn() -> bool = || true;

fn collection_path(connection_id: ConnectionId, database: &str, collection: &str) -> ObjectPath {
    ObjectPath {
        connection_id,
        database: Some(database.to_string()),
        schema: None,
        name: collection.to_string(),
        kind: ObjectKind::Collection,
    }
}

#[test]
fn mongo_live_ping_create_query_metadata_and_cleanup() {
    let Some(config) = live_config() else {
        eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
        return;
    };
    let connection_id = config.id;
    let database = live_database(&config);
    let connector = connector_for(&config).expect("connector");
    connector
        .test_connection(&config)
        .expect("认证与选主必须成功");

    // 每个用例独立集合名，避免并发或历史残留互相污染。
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let collection = format!("fluxdb_smoke_{suffix}");

    connector
        .mongo_create_collection(connection_id, &database, &collection)
        .expect("创建集合");
    let path = collection_path(connection_id, &database, &collection);

    // 写入三类 BSON 值：ObjectId / 日期 / 数字，验证 Extended JSON 不丢类型。
    let insert = format!(
        r#"{{ "insert": "{collection}", "documents": [
            {{ "_id": "smoke-1", "name": "alpha", "count": 1, "when": {{ "$date": "2024-01-01T00:00:00Z" }} }},
            {{ "_id": "smoke-2", "name": "beta", "count": 2, "nested": {{ "tags": ["x", "y"] }} }}
        ] }}"#
    );
    // 服务端返回的是命令回执（`n` 为写入条数），扩展 JSON 里没有 "inserted" 字样。
    let inserted = connector
        .mongo_run_command(connection_id, &database, &insert)
        .expect("插入文档");
    assert!(
        serde_json::from_str::<serde_json::Value>(&inserted)
            .map(|reply| reply["ok"] == 1.0 && reply["n"] == 2)
            .unwrap_or(false),
        "插入回执必须确认成功且写入两条: {inserted}"
    );

    let page = connector
        .mongo_find(&path, &MongoFindRequest::new(&collection), never_cancel())
        .expect("查询文档");
    assert_eq!(page.returned, 2, "应返回两条文档");

    let filtered = MongoFindRequest {
        filter: r#"{ "name": "beta" }"#.to_string(),
        ..MongoFindRequest::new(&collection)
    };
    let page = connector
        .mongo_find(&path, &filtered, never_cancel())
        .expect("按条件查询");
    assert_eq!(page.returned, 1);
    let json = &page.documents[0].json;
    assert!(json.contains("beta"), "{json}");
    assert!(json.contains("tags"), "嵌套数组必须原样返回: {json}");

    // 投影只返回指定字段：验证字段级裁剪在真实服务端生效。
    let projected = MongoFindRequest {
        filter: r#"{ "_id": "smoke-2" }"#.to_string(),
        projection: r#"{ "nested.tags": 1, "_id": 0 }"#.to_string(),
        ..MongoFindRequest::new(&collection)
    };
    let page = connector
        .mongo_find(&path, &projected, never_cancel())
        .expect("投影查询");
    assert_eq!(page.returned, 1);
    assert!(page.documents[0].json.contains("tags"));
    assert!(
        !page.documents[0].json.contains("\"name\""),
        "投影之外的字段不应返回: {}",
        page.documents[0].json
    );

    // 取消标志必须让查询提前返回错误，而不是继续拉数据。
    let cancelled =
        connector.mongo_find(&path, &MongoFindRequest::new(&collection), &ALWAYS_CANCEL);
    assert!(cancelled.is_err(), "取消后的查询必须失败");

    // 整篇替换：以原 `_id` 定位，写回后按 `_id` 复查，确保改动真的落到服务端。
    connector
        .mongo_replace_document(
            connection_id,
            &database,
            &collection,
            r#"{ "_id": "smoke-1", "name": "alpha", "count": 1 }"#,
            r#"{ "_id": "smoke-1", "name": "alpha", "count": 42, "extra": true }"#,
        )
        .expect("整篇替换");
    let replaced = connector
        .mongo_find(
            &path,
            &MongoFindRequest {
                filter: r#"{ "_id": "smoke-1" }"#.to_string(),
                ..MongoFindRequest::new(&collection)
            },
            never_cancel(),
        )
        .expect("复查替换结果");
    assert_eq!(replaced.returned, 1);
    let json = &replaced.documents[0].json;
    assert!(json.contains("42") && json.contains("extra"), "{json}");

    // `_id` 是定位条件：改了它必须被拒绝，服务端数据保持原样。
    let rejected = connector.mongo_replace_document(
        connection_id,
        &database,
        &collection,
        r#"{ "_id": "smoke-1", "name": "alpha" }"#,
        r#"{ "_id": "smoke-9", "name": "alpha" }"#,
    );
    assert!(rejected.is_err(), "不允许修改 _id: {rejected:?}");

    let info = connector.mongo_collection_info(&path).expect("集合信息");
    assert!(info.document_count.unwrap_or_default() >= 2, "{info:?}");

    // 服务端 5.0+ 对同名同选项的 createCollection 是幂等的（返回 ok:1），
    // 连接器按服务端语义透传：重复创建不报错，集合内容不受影响。
    connector
        .mongo_create_collection(connection_id, &database, &collection)
        .expect("重复创建同名集合必须按服务端幂等语义成功");
    let info = connector
        .mongo_collection_info(&path)
        .expect("重复创建后集合信息");
    assert!(info.document_count.unwrap_or_default() >= 2, "{info:?}");

    // 删除文档：同样以原 `_id` 定位；删完按 `_id` 复查确认真的没了。
    // 放在最后做，避免影响上面「文档数 >= 2」的断言。
    let insert_one = format!(
        r#"{{ "insert": "{collection}", "documents": [ {{ "_id": "smoke-del", "name": "gamma" }} ] }}"#
    );
    connector
        .mongo_run_command(connection_id, &database, &insert_one)
        .expect("插入待删文档");
    let doomed = connector
        .mongo_find(
            &path,
            &MongoFindRequest {
                filter: r#"{ "_id": "smoke-del" }"#.to_string(),
                ..MongoFindRequest::new(&collection)
            },
            never_cancel(),
        )
        .expect("删除前复查");
    assert_eq!(doomed.returned, 1, "待删文档必须存在");
    connector
        .mongo_delete_document(
            connection_id,
            &database,
            &collection,
            &doomed.documents[0].json,
        )
        .expect("删除文档");
    let gone = connector
        .mongo_find(
            &path,
            &MongoFindRequest {
                filter: r#"{ "_id": "smoke-del" }"#.to_string(),
                ..MongoFindRequest::new(&collection)
            },
            never_cancel(),
        )
        .expect("删除后复查");
    assert_eq!(gone.returned, 0, "删掉的文档不能再查到");
    // 再删一次必须报错而不是静默成功，否则界面会谎报「已删除」。
    let repeat = connector.mongo_delete_document(
        connection_id,
        &database,
        &collection,
        &doomed.documents[0].json,
    );
    assert!(repeat.is_err(), "重复删除必须失败: {repeat:?}");

    // 新增文档：不带 `_id`，由服务端生成；返回的 `_id` 必须能精确查回刚插的那篇，
    // 且内容与传入的 Extended JSON 语义一致（BSON 类型标注不能被降级）。
    let inserted = connector
        .mongo_insert_document(
            connection_id,
            &database,
            &collection,
            r#"{ "name": "smoke-copy", "at": { "$date": "2026-10-22T08:30:00.000Z" }, "amt": { "$numberDecimal": "100" } }"#,
        )
        .expect("新增文档");
    assert!(inserted.contains("$oid"), "返回新 _id 的值本身: {inserted}");
    let fetched = connector
        .mongo_find(
            &path,
            &MongoFindRequest {
                filter: format!(r#"{{ "_id": {inserted} }}"#),
                ..MongoFindRequest::new(&collection)
            },
            never_cancel(),
        )
        .expect("按返回的 _id 复查");
    assert_eq!(fetched.returned, 1, "新增的文档必须能按返回的 _id 查到");
    assert!(
        fetched.documents[0].json.contains("$numberDecimal"),
        "Decimal128 类型标注必须保留: {}",
        fetched.documents[0].json
    );
    // 自带 ObjectId 必须原样采用：界面「新增文档」会预填一个，返回的 `_id` 得就是它。
    let preset_id = r#"{"$oid":"6517000000000000000000aa"}"#;
    let preset = connector
        .mongo_insert_document(
            connection_id,
            &database,
            &collection,
            &format!(r#"{{ "_id": {preset_id}, "name": "smoke-preset" }}"#),
        )
        .expect("自带 ObjectId 的新增文档");
    assert!(
        preset.contains("6517000000000000000000aa"),
        "返回的 _id 必须就是传入的那个: {preset}"
    );
    // 非 ObjectId 的 `_id` 必须拒绝：界面只预填 ObjectId，其它类型多半是把 `_id`
    // 当普通字段误填，插进去会与集合既有主键类型对不上。
    let with_id = connector.mongo_insert_document(
        connection_id,
        &database,
        &collection,
        r#"{ "_id": "smoke-copy", "name": "dup" }"#,
    );
    assert!(
        with_id.is_err(),
        "非 ObjectId 的 _id 必须被拒绝: {with_id:?}"
    );

    connector
        .mongo_drop_collection(connection_id, &database, &collection)
        .expect("删除集合");
    // MongoDB 对不存在的集合执行 find 会返回空结果而不是报错（服务端语义），
    // 因此「已删除」用集合列表验证，而不是靠查询失败推断。
    let after = connector
        .mongo_find(&path, &MongoFindRequest::new(&collection), never_cancel())
        .expect("查询已删除的集合返回空结果");
    assert_eq!(after.returned, 0, "{after:?}");
    let remaining = connector
        .list_objects(Some(&ObjectPath {
            connection_id,
            database: Some(database.clone()),
            schema: None,
            name: database.clone(),
            kind: ObjectKind::Database,
        }))
        .expect("读取集合列表");
    assert!(
        remaining
            .iter()
            .all(|object| object.path.name != collection),
        "删除后集合不应再出现在列表里: {remaining:?}"
    );
}

#[test]
fn mongo_live_server_info_reports_real_topology() {
    let Some(config) = live_config() else {
        eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
        return;
    };
    let connector = connector_for(&config).expect("connector");
    let info = connector
        .mongo_server_info(config.id)
        .expect("读取连接信息");
    assert!(
        info.version.is_some(),
        "buildInfo.version 必须有值: {info:?}"
    );
    assert_ne!(
        info.topology,
        fluxdb_core::MongoTopology::Unknown,
        "hello 必须能判定拓扑: {info:?}"
    );
    assert!(
        !info.databases.is_empty(),
        "listDatabases 至少返回一个可见库: {info:?}"
    );
}

#[test]
fn mongo_live_aggregate_and_indexes() {
    let Some(config) = live_config() else {
        eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
        return;
    };
    let connection_id = config.id;
    let database = live_database(&config);
    let connector = connector_for(&config).expect("connector");
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let collection = format!("fluxdb_smoke_agg_{suffix}");
    connector
        .mongo_create_collection(connection_id, &database, &collection)
        .expect("创建集合");
    let path = collection_path(connection_id, &database, &collection);

    let insert = format!(
        r#"{{ "insert": "{collection}", "documents": [
            {{ "group": "a", "value": 1 }}, {{ "group": "a", "value": 2 }}, {{ "group": "b", "value": 5 }}
        ] }}"#
    );
    connector
        .mongo_run_command(connection_id, &database, &insert)
        .expect("插入文档");

    let pipeline = r#"[{ "$group": { "_id": "$group", "total": { "$sum": "$value" } } }, { "$sort": { "_id": 1 } }]"#;
    let page = connector.mongo_aggregate(&path, pipeline).expect("聚合");
    assert_eq!(page.returned, 2, "两个分组: {page:?}");
    assert!(
        page.documents
            .iter()
            .any(|document| document.json.contains("3")),
        "{page:?}"
    );

    let indexes = connector.mongo_indexes(&path).expect("索引");
    assert_eq!(indexes.len(), 1, "新集合只有默认 _id 索引: {indexes:?}");
    assert!(indexes[0].name == "_id_", "{indexes:?}");

    connector
        .mongo_drop_collection(connection_id, &database, &collection)
        .expect("清理集合");
}

/// 命令行终端的 shell 语句在真库上跑通：`use` / `show dbs` / `show collections` /
/// `find` / `findOne` / `countDocuments` / `distinct` / `aggregate` / `getIndexes` / `stats`。
#[test]
fn mongo_live_shell_statements() {
    let Some(config) = live_config() else {
        eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
        return;
    };
    let connection_id = config.id;
    let database = live_database(&config);
    let connector = connector_for(&config).expect("connector");
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let collection = format!("fluxdb_shell_{suffix}");
    connector
        .mongo_create_collection(connection_id, &database, &collection)
        .expect("创建集合");
    let insert = format!(
        r#"{{ "insert": "{collection}", "documents": [
            {{ "_id": "shell-1", "group": "a", "value": 1 }},
            {{ "_id": "shell-2", "group": "a", "value": 2 }},
            {{ "_id": "shell-3", "group": "b", "value": 5 }}
        ] }}"#
    );
    connector
        .mongo_run_command(connection_id, &database, &insert)
        .expect("插入文档");

    let shell = |text: &str| {
        connector
            .mongo_shell(connection_id, &database, text)
            .unwrap_or_else(|error| panic!("{text} 执行失败: {error:?}"))
            .output
    };

    // `use` 只回传切换后的库，不写回连接配置。
    let switched = connector
        .mongo_shell(connection_id, &database, "use another_db")
        .expect("use");
    assert_eq!(switched.database.as_deref(), Some("another_db"));
    assert_eq!(switched.output, "switched to db another_db");

    assert!(
        shell("show dbs").contains(&database),
        "show dbs 要列出当前库"
    );
    assert!(
        shell("show collections").contains(&collection),
        "show collections 要列出新建集合"
    );
    assert!(shell("db.getCollectionNames()").contains(&collection));

    let found = shell(&format!(
        r#"db.{collection}.find({{ group: "a" }}).sort({{ value: -1 }}).limit(1)"#
    ));
    assert!(found.contains("shell-2"), "排序后取第一条: {found}");
    assert!(!found.contains("shell-3"), "过滤条件不生效: {found}");

    let one = shell(&format!("db.{collection}.findOne({{ _id: \"shell-3\" }})"));
    assert!(one.contains("shell-3"), "{one}");
    assert_eq!(
        shell(&format!("db.{collection}.findOne({{ _id: \"missing\" }})")),
        "null"
    );

    assert_eq!(shell(&format!("db.{collection}.countDocuments()")), "3");
    assert_eq!(
        shell(&format!(
            r#"db.{collection}.countDocuments({{ group: "a" }})"#
        )),
        "2"
    );
    assert_eq!(
        shell(&format!("db.{collection}.estimatedDocumentCount()")),
        "3"
    );

    let distinct = shell(&format!(r#"db.{collection}.distinct("group")"#));
    assert!(
        distinct.contains('a') && distinct.contains('b'),
        "{distinct}"
    );

    let aggregated = shell(&format!(
        r#"db.{collection}.aggregate([{{ "$group": {{ "_id": "$group", "total": {{ "$sum": "$value" }} }} }}])"#
    ));
    assert!(
        aggregated.contains('3') && aggregated.contains('5'),
        "{aggregated}"
    );

    let indexes = shell(&format!("db.{collection}.getIndexes()"));
    assert!(indexes.contains("_id_"), "{indexes}");

    let stats = shell(&format!("db.{collection}.stats()"));
    assert!(stats.contains("count"), "{stats}");

    // 写方法在解析层拒绝，不能悄悄执行。
    let write = connector
        .mongo_shell(
            connection_id,
            &database,
            &format!("db.{collection}.deleteMany({{}})"),
        )
        .expect_err("写方法必须拒绝");
    assert!(write.message.contains("read-only"), "{}", write.message);

    connector
        .mongo_drop_collection(connection_id, &database, &collection)
        .expect("清理集合");
}

/// 回归：驱动操作必须在放大栈的工作线程上执行。
///
/// 曾经的故障是 gpui 后台任务（macOS GCD worker，栈约 512 KiB）里直接
/// `Runtime::block_on` 驱动 future，debug 构建下 `listCollections` 的 BSON 序列化
/// 帧擦到保护页 → SIGILL（不是 panic，无法捕获）。这里用 192 KiB 栈的线程复现
/// 同样的调用方环境：连接器内部若漏掉放大栈，本用例会直接把测试进程打挂。
///
/// 覆盖三条曾经直接在调用方线程 poll 的路径：建连、集合列表、`find` 游标解码。
#[test]
fn mongo_live_operations_survive_small_caller_stack() {
    let Some(config) = live_config() else {
        eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
        return;
    };
    let connection_id = config.id;
    let database = live_database(&config);
    let label = format!("读取 {database} 集合列表");
    let worker = std::thread::Builder::new()
        .name("small-stack-caller".to_string())
        .stack_size(192 * 1024)
        .spawn(move || {
            let connector = connector_for(&config).expect("connector");
            let parent = ObjectPath {
                connection_id,
                database: Some(database.clone()),
                schema: None,
                name: database.clone(),
                kind: ObjectKind::Database,
            };
            let objects = connector.list_objects(Some(&parent))?;
            // 顺带验证游标解码：挑一个普通集合查一页，走 `find` + 结果反序列化。
            let collection = objects
                .iter()
                .find(|object| object.path.kind == ObjectKind::Collection);
            if let Some(collection) = collection {
                connector.mongo_find(
                    &collection.path,
                    &MongoFindRequest::new(&collection.path.name),
                    never_cancel(),
                )?;
            }
            Ok::<usize, fluxdb_core::Error>(objects.len())
        })
        .expect("启动小栈调用线程");
    let count = worker
        .join()
        .unwrap_or_else(|_| panic!("{label}的工作线程异常退出"))
        .unwrap_or_else(|error| panic!("{label}失败: {error}"));
    // Atlas 的 demo 库至少有集合；这里只要求调用活着返回，不绑定具体数量。
    assert!(count > 0, "{label}返回空结果，链路可能没真正执行");
}

/// CSV / JSON / JSONL 三种文件的导入链路：类型推断、ObjectId 还原、坏文档跳过并继续。
///
/// 关键断言是「`_id` 必须是 ObjectId 而不是字符串」——这正是 CSV 直接构造 `Bson`
/// 而不是拼 Extended JSON 文本的原因；以及「重复 `_id` 只让一条失败，其余照常写入」，
/// 这是 `ordered: false` 的语义。
#[test]
fn mongo_live_import_documents_from_csv_and_json() {
    let Some(config) = live_config() else {
        eprintln!("跳过：未设置 FLUXDB_MONGO_URI");
        return;
    };
    let connection_id = config.id;
    let database = live_database(&config);
    let connector = connector_for(&config).expect("connector");
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis();
    let collection = format!("fluxdb_smoke_import_{suffix}");
    connector
        .mongo_create_collection(connection_id, &database, &collection)
        .expect("创建集合");
    let path = collection_path(connection_id, &database, &collection);

    // 临时文件放在系统临时目录下的独立子目录，测完删除。
    let dir = std::env::temp_dir().join(format!("fluxdb_mongo_import_{suffix}"));
    std::fs::create_dir_all(&dir).expect("创建临时目录");

    let first_id = "6571c1b0a1b2c3d4e5f60001";
    let second_id = "6571c1b0a1b2c3d4e5f60002";
    let csv_path = dir.join("docs.csv");
    std::fs::write(
        &csv_path,
        format!(
            "_id,name,count,price,active,joined\n\
             {first_id},alpha,7,1.5,true,2024-01-02\n\
             {first_id},duplicate,1,0.5,false,2024-01-03\n\
             {second_id},beta,90,2.25,f,2024-01-04T05:06:07Z\n"
        ),
    )
    .expect("写入 CSV");

    let options = MongoImportOptions::new();
    let preview = connector
        .mongo_preview_import(&MongoImportPreviewRequest {
            object: path.clone(),
            source: csv_path.clone(),
            options: options.clone(),
        })
        .expect("CSV 预览");
    assert_eq!(preview.format, fluxdb_core::MongoImportFormat::Csv);
    let detected = |name: &str| {
        preview
            .fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| field.detected.clone())
            .unwrap_or_default()
    };
    assert_eq!(detected("_id"), "objectId", "{preview:?}");
    assert_eq!(detected("count"), "int32", "{preview:?}");
    assert_eq!(detected("price"), "double", "{preview:?}");
    assert_eq!(detected("active"), "boolean", "{preview:?}");
    assert_eq!(detected("joined"), "date", "{preview:?}");

    let cancel = AtomicBool::new(false);
    let report = connector
        .mongo_import_documents(
            &MongoImportRequest {
                object: path.clone(),
                source: csv_path.clone(),
                options: options.clone(),
            },
            &cancel,
            &mut |_| {},
        )
        .expect("CSV 导入");
    assert_eq!(report.inserted, 2, "重复 _id 只失败一条: {report:?}");
    assert_eq!(report.failed, 1, "{report:?}");
    assert!(!report.cancelled, "{report:?}");

    // 主键必须是 ObjectId：按十六进制串再查一次就能区分「ObjectId」与「字符串」。
    let page = connector
        .mongo_find(
            &path,
            &MongoFindRequest {
                filter: format!(r#"{{ "_id": {{ "$oid": "{first_id}" }} }}"#),
                ..MongoFindRequest::new(&collection)
            },
            never_cancel(),
        )
        .expect("按 ObjectId 查询");
    assert_eq!(page.returned, 1, "ObjectId 主键应能查到文档: {page:?}");
    let document = &page.documents[0].json;
    assert!(document.contains("alpha"), "{document}");
    assert!(document.contains("1.5"), "{document}");

    // JSON 数组：Extended JSON 的 `$oid` 与 `$date` 走同一条还原路径。
    let json_path = dir.join("docs.json");
    std::fs::write(
        &json_path,
        format!(
            r#"[{{ "_id": {{ "$oid": "{third}" }}, "kind": "array", "at": {{ "$date": "2024-05-06T07:08:09Z" }} }},
                {{ "kind": "array-generated" }}]"#,
            third = "6571c1b0a1b2c3d4e5f60003"
        ),
    )
    .expect("写入 JSON");
    let report = connector
        .mongo_import_documents(
            &MongoImportRequest {
                object: path.clone(),
                source: json_path.clone(),
                options: options.clone(),
            },
            &cancel,
            &mut |_| {},
        )
        .expect("JSON 导入");
    assert_eq!(report.inserted, 2, "{report:?}");
    assert_eq!(report.failed, 0, "{report:?}");

    // JSONL：每行一篇，含一行坏 JSON，验证坏文档跳过而其余写入。
    let jsonl_path = dir.join("docs.jsonl");
    std::fs::write(
        &jsonl_path,
        "{\"kind\": \"lines\", \"n\": 1}\n{ this is not json }\n{\"kind\": \"lines\", \"n\": 2}\n",
    )
    .expect("写入 JSONL");
    let preview = connector
        .mongo_preview_import(&MongoImportPreviewRequest {
            object: path.clone(),
            source: jsonl_path.clone(),
            options: options.clone(),
        })
        .expect("JSONL 预览");
    assert_eq!(preview.format, fluxdb_core::MongoImportFormat::JsonLines);
    let report = connector
        .mongo_import_documents(
            &MongoImportRequest {
                object: path.clone(),
                source: jsonl_path.clone(),
                options,
            },
            &cancel,
            &mut |_| {},
        )
        .expect("JSONL 导入");
    assert_eq!(report.inserted, 2, "{report:?}");
    assert_eq!(report.failed, 1, "{report:?}");
    assert!(
        report.errors.iter().any(|error| error.contains("第 2 行")),
        "错误应带行号: {report:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    connector
        .mongo_drop_collection(connection_id, &database, &collection)
        .expect("清理集合");
}
