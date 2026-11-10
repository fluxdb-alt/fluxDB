// MongoDB 连接级服务器面板：serverStatus / hello / $currentOp 读取与 killOp 处置。
//
// 只做服务端只读观测；终止操作在 UI、控制器和连接器三处拦截只读连接。

/// `$currentOp` 一次最多展示的操作数；超出部分直接丢弃，不做分页。
const MONGO_CURRENT_OP_LIMIT: i64 = 200;

/// 概览指标白名单：`serverStatus` 的层级路径 + 是否「非零即告警」。
///
/// 只放排障常用且各版本稳定的字段；界面按路径把 `.` 换成 `_` 作为 i18n key。
const MONGO_OVERVIEW_METRICS: &[(&[&str], bool)] = &[
    (&["opcounters", "insert"], false),
    (&["opcounters", "query"], false),
    (&["opcounters", "update"], false),
    (&["opcounters", "delete"], false),
    (&["opcounters", "getmore"], false),
    (&["opcounters", "command"], false),
    (&["connections", "current"], false),
    (&["connections", "available"], false),
    (&["connections", "totalCreated"], false),
    (&["connections", "active"], false),
    (&["mem", "resident"], false),
    (&["mem", "virtual"], false),
    (&["network", "bytesIn"], false),
    (&["network", "bytesOut"], false),
    (&["globalLock", "activeClients", "total"], false),
    (&["globalLock", "currentQueue", "total"], true),
    (&["asserts", "warning"], true),
    (&["asserts", "msg"], true),
    (&["asserts", "user"], true),
];

/// 读取实例状态与当前操作。`$currentOp` 失败只填 `operations_error`，概览照常返回。
fn mongo_load_server_activity(
    config: &ConnectionConfig,
) -> fluxdb_core::Result<MongoServerActivity> {
    let client = mongo_connect(config)?;
    let admin = client.database("admin");

    let hello =
        mongo_block_on(20, "读取 MongoDB 拓扑信息", async { admin.run_command(doc! {"hello": 1}).await })?;
    let status = mongo_block_on(20, "读取 MongoDB 服务器状态", async {
        admin.run_command(doc! {"serverStatus": 1}).await
    })
    .map_err(|error| {
        // serverStatus 都读不到说明连接不可用，整页无内容可展示。
        Error::new(
            error.kind,
            format!("读取 MongoDB 服务器状态失败：{}", error.message),
        )
    })?;

    let mut activity = MongoServerActivity {
        version: status.get_str("version").unwrap_or("").to_string(),
        edition: mongo_status_edition(&status),
        topology: mongo_hello_topology(&hello),
        replica_set_name: hello.get_str("setName").unwrap_or("").to_string(),
        member_state: mongo_member_state(hello.get_i32("myState").ok()),
        uptime_secs: mongo_bson_u64(status.get("uptime")),
        connections_current: mongo_path_u64(&status, &["connections", "current"]).unwrap_or(0),
        connections_available: mongo_path_u64(&status, &["connections", "available"]),
        metrics: mongo_overview_metrics(&status),
        ..MongoServerActivity::default()
    };

    match mongo_current_operations(&client) {
        Ok(operations) => activity.operations = operations,
        // 权限不足（`inprog` / `$currentOp` 需 allUsers 权限）时只影响「当前操作」子页。
        Err(error) => activity.operations_error = Some(error.message),
    }

    Ok(activity)
}

/// 拓扑判定口径与「显示连接信息」弹框一致。
fn mongo_hello_topology(hello: &Document) -> MongoTopology {
    if hello.get_str("msg") == Ok("isdbgrid") {
        MongoTopology::Sharded
    } else if hello.get_str("setName").is_ok() {
        MongoTopology::ReplicaSet
    } else if hello.contains_key("isWritablePrimary") || hello.contains_key("ismaster") {
        MongoTopology::Standalone
    } else {
        MongoTopology::Unknown
    }
}

/// `hello.myState` → 成员角色。副本集外该字段缺失，返回空串。
fn mongo_member_state(state: Option<i32>) -> String {
    match state {
        Some(1) => "PRIMARY".into(),
        Some(2) => "SECONDARY".into(),
        Some(7) => "ARBITER".into(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// `serverStatus.modules` 含 `enterprise` 才是企业版。
fn mongo_status_edition(status: &Document) -> String {
    let is_enterprise = status
        .get_array("modules")
        .map(|modules| modules.iter().any(|module| module.as_str() == Some("enterprise")))
        .unwrap_or(false);
    if is_enterprise { "Enterprise" } else { "Community" }.into()
}

fn mongo_overview_metrics(status: &Document) -> Vec<ServerMetric> {
    MONGO_OVERVIEW_METRICS
        .iter()
        .filter_map(|(path, warn_nonzero)| {
            let value = mongo_path_u64(status, path)?;
            Some(ServerMetric {
                key: path.join("."),
                value: value.to_string(),
                warning: *warn_nonzero && value > 0,
            })
        })
        .collect()
}

/// 按 `serverStatus` 的层级路径取非负整数；路径缺失或类型不符返回 `None`。
fn mongo_path_u64(document: &Document, path: &[&str]) -> Option<u64> {
    let mut current: &Bson = document.get(*path.first()?)?;
    for key in &path[1..] {
        current = current.as_document()?.get(*key)?;
    }
    mongo_bson_u64(Some(current))
}

/// 读 `admin.$currentOp`：只取活跃或已运行 ≥1 秒的操作，按运行时长倒序。
fn mongo_current_operations(client: &MongoClient) -> fluxdb_core::Result<Vec<MongoOperation>> {
    let admin = client.database("admin");
    let pipeline = vec![
        doc! {"$currentOp": {"allUsers": true, "idleConnections": false}},
        // 非活跃且未超 1 秒的连接对排障没有价值，直接过滤在服务端。
        doc! {"$match": {"$or": [{"active": true}, {"secs_running": {"$gte": 1}}]}},
        doc! {"$sort": {"secs_running": -1}},
        doc! {"$limit": MONGO_CURRENT_OP_LIMIT},
    ];
    let cursor = mongo_block_on(20, "读取 MongoDB 当前操作", async { admin.aggregate(pipeline).await })?;
    Ok(mongo_collect_cursor(cursor)?
        .iter()
        .map(mongo_operation_from_doc)
        .collect())
}

fn mongo_operation_from_doc(document: &Document) -> MongoOperation {
    MongoOperation {
        opid: match document.get("opid") {
            Some(Bson::Int32(value)) => value.to_string(),
            Some(Bson::Int64(value)) => value.to_string(),
            Some(Bson::String(value)) => value.clone(),
            _ => String::new(),
        },
        client: document.get_str("client").unwrap_or("").to_string(),
        op: document.get_str("op").unwrap_or("").to_string(),
        namespace: document.get_str("ns").unwrap_or("").to_string(),
        description: mongo_operation_description(document),
        secs_running: document
            .get("secs_running")
            .and_then(|value| mongo_bson_u64(Some(value)))
            .unwrap_or(0),
        waiting_for_lock: document.get_bool("waitingForLock").unwrap_or(false),
        plan_summary: document.get_str("planSummary").unwrap_or("").to_string(),
    }
}

/// 操作描述：优先 `desc`，否则用命令名（`command` 文档的第一个键）。
fn mongo_operation_description(document: &Document) -> String {
    if let Ok(desc) = document.get_str("desc")
        && !desc.is_empty()
    {
        return desc.to_string();
    }
    document
        .get_document("command")
        .ok()
        .and_then(|command| command.keys().next().cloned())
        .unwrap_or_default()
}

fn mongo_kill_operation(config: &ConnectionConfig, opid: &str) -> fluxdb_core::Result<()> {
    if config.is_read_only() {
        return Err(Error::new(
            ErrorKind::Unsupported,
            "只读连接不允许终止 MongoDB 操作",
        ));
    }
    let opid = opid.trim();
    if opid.is_empty() {
        return Err(Error::new(ErrorKind::Query, "操作 ID 不能为空"));
    }
    let opid_number = opid
        .parse::<i64>()
        .map_err(|_| Error::new(ErrorKind::Query, format!("操作 ID `{opid}` 不是数字")))?;
    let client = mongo_connect(config)?;
    let admin = client.database("admin");
    mongo_block_on(20, "终止 MongoDB 操作", async move {
        admin.run_command(doc! {"killOp": 1, "op": opid_number}).await
    })
    .map(|_| ())
}

#[cfg(test)]
mod mongo_server_tests {
    use super::*;

    fn status() -> Document {
        doc! {
            "version": "7.0.45",
            "uptime": 3600i64,
            "connections": {"current": 5i32, "available": 995i32, "totalCreated": 10i64},
            "opcounters": {"insert": 3i64, "query": 12i64},
            "globalLock": {"currentQueue": {"total": 2i32}, "activeClients": {"total": 1i32}},
            "asserts": {"warning": 0i32, "msg": 1i32},
            "modules": [],
        }
    }

    fn activity_config(read_only: bool) -> ConnectionConfig {
        let mut options = BTreeMap::new();
        if read_only {
            options.insert("read_only".to_string(), "true".to_string());
        }
        ConnectionConfig {
            id: ConnectionId(9),
            name: "mongo".into(),
            kind: DatabaseKind::MongoDb,
            endpoint: Endpoint::Tcp {
                host: "127.0.0.1".into(),
                port: 1,
                database: None,
            },
            options,
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            sqlserver_profile: None,
            mongo_profile: None,
            oracle_profile: None,
        }
    }

    #[test]
    fn overview_metrics_follow_whitelist_and_alert_rules() {
        let metrics = mongo_overview_metrics(&status());
        let find = |key: &str| {
            metrics
                .iter()
                .find(|metric| metric.key == key)
                .unwrap_or_else(|| panic!("缺少指标 {key}"))
        };
        assert_eq!(find("connections.current").value, "5");
        assert_eq!(find("opcounters.query").value, "12");
        assert!(!find("connections.current").warning);
        assert!(!find("asserts.warning").warning, "0 不告警");
        assert!(find("asserts.msg").warning, "非零断言告警");
        assert!(find("globalLock.currentQueue.total").warning);
        assert!(
            !metrics.iter().any(|metric| metric.key == "version"),
            "只输出白名单指标"
        );
    }

    #[test]
    fn hello_maps_topology_and_member_state() {
        assert_eq!(
            mongo_hello_topology(&doc! {"setName": "rs0", "myState": 1i32}),
            MongoTopology::ReplicaSet
        );
        assert_eq!(
            mongo_hello_topology(&doc! {"msg": "isdbgrid"}),
            MongoTopology::Sharded
        );
        assert_eq!(
            mongo_hello_topology(&doc! {"isWritablePrimary": true}),
            MongoTopology::Standalone
        );
        assert_eq!(mongo_hello_topology(&Document::new()), MongoTopology::Unknown);
        assert_eq!(mongo_member_state(Some(1)), "PRIMARY");
        assert_eq!(mongo_member_state(Some(2)), "SECONDARY");
        assert_eq!(mongo_member_state(None), "");
    }

    #[test]
    fn operation_document_parses_fields_and_falls_back_to_command_name() {
        let operation = mongo_operation_from_doc(&doc! {
            "opid": 42i32,
            "client": "10.0.0.1:5000",
            "op": "query",
            "ns": "shop.orders",
            "secs_running": 12i64,
            "waitingForLock": true,
            "planSummary": "COLLSCAN",
            "command": {"find": "orders"},
        });
        assert_eq!(operation.opid, "42");
        assert_eq!(operation.client, "10.0.0.1:5000");
        assert_eq!(operation.namespace, "shop.orders");
        assert_eq!(operation.description, "find");
        assert_eq!(operation.secs_running, 12);
        assert!(operation.waiting_for_lock);
        assert_eq!(operation.plan_summary, "COLLSCAN");
        assert!(operation.is_slow());
    }

    #[test]
    fn operation_uses_desc_when_present_and_tolerates_missing_fields() {
        let operation = mongo_operation_from_doc(&doc! {"desc": "conn1234", "opid": "host:1"});
        assert_eq!(operation.description, "conn1234");
        assert_eq!(operation.opid, "host:1");
        assert_eq!(operation.secs_running, 0);
        assert!(!operation.is_slow());
        assert!(!operation.waiting_for_lock);
    }

    #[test]
    fn read_only_connection_rejects_kill_before_connecting() {
        let config = activity_config(true);
        assert_eq!(
            mongo_kill_operation(&config, "42").unwrap_err().kind,
            ErrorKind::Unsupported
        );
    }

    #[test]
    fn invalid_opid_rejected_before_connecting() {
        let config = activity_config(false);
        assert_eq!(
            mongo_kill_operation(&config, "  ").unwrap_err().kind,
            ErrorKind::Query
        );
        assert_eq!(
            mongo_kill_operation(&config, "abc").unwrap_err().kind,
            ErrorKind::Query
        );
    }
}
