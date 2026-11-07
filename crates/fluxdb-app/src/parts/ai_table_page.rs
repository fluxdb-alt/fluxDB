// 页面查询只接受结构化筛选，不接受 SQL、路径、连接配置或凭据。
use fluxdb_core::{FilterOp, SortDirection};
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AiTableFilter {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AiTableSort {
    pub field: String,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AiTableQuery {
    pub page_ref: String,
    pub filters: Vec<AiTableFilter>,
    pub sort: Vec<AiTableSort>,
    pub execute: bool,
    pub limit: u64,
}

impl AiTableQuery {
    pub fn specs(
        &self,
        columns: &[Column],
    ) -> fluxdb_core::Result<(Vec<FilterSpec>, Vec<SortSpec>)> {
        let invalid = || editor_error("page_query_invalid");
        if self.filters.len() > 20 || self.sort.len() > 8 || !(1..=1000).contains(&self.limit) {
            return Err(invalid());
        }
        let known = |field: &str| columns.iter().any(|column| column.name == field);
        let mut filters = Vec::new();
        for filter in &self.filters {
            if !known(&filter.field)
                || filter.values.len() > 50
                || filter.values.iter().any(|value| value.len() > 4096)
            {
                return Err(invalid());
            }
            let (op, count) = match filter.op.as_str() {
                "eq" => (FilterOp::Eq, 1),
                "ne" => (FilterOp::NotEq, 1),
                "lt" => (FilterOp::LessThan, 1),
                "le" => (FilterOp::LessThanOrEqual, 1),
                "gt" => (FilterOp::GreaterThan, 1),
                "ge" => (FilterOp::GreaterThanOrEqual, 1),
                "contains" => (FilterOp::Contains, 1),
                "is_null" => (FilterOp::IsNull, 0),
                "is_not_null" => (FilterOp::IsNotNull, 0),
                "in" if !filter.values.is_empty() => (FilterOp::InList, filter.values.len()),
                "not_in" if !filter.values.is_empty() => (FilterOp::NotInList, filter.values.len()),
                _ => return Err(invalid()),
            };
            if filter.values.len() != count {
                return Err(invalid());
            }
            filters.push(FilterSpec {
                field: filter.field.clone(),
                op,
                enabled: true,
                values: filter.values.iter().cloned().map(CellValue::Text).collect(),
            });
        }
        let mut sort = Vec::new();
        for item in &self.sort {
            if !known(&item.field) {
                return Err(invalid());
            }
            sort.push(SortSpec {
                field: item.field.clone(),
                direction: if item.descending {
                    SortDirection::Desc
                } else {
                    SortDirection::Asc
                },
            });
        }
        Ok((filters, sort))
    }
}

#[derive(Clone, Debug)]
pub enum AiTablePageAction {
    Browse {
        connection_id: ConnectionId,
        parent: Option<ObjectPath>,
        offset: usize,
    },
    Open(ObjectPath),
    Query(AiTableQuery),
    Find(String),
}

/// 未绑定执行目标时，这些工具在本轮目录中不可用：它们只能对绑定连接执行，
/// 暴露出来只会诱导模型反复猜 `conn:<id>`。
pub const UNBOUND_SQL_TOOLS: &[&str] = &[
    "db.run",
    "db.search_objects",
    "db.describe_object",
    "db.get_relationships",
    "db.find_join_paths",
    "er.propose_relationships",
    "redis.scan",
    "redis.inspect",
    "redis.execute",
    "redis.subscribe",
    "redis.server_info",
];

/// 从目录移除无法执行的 SQL/Redis 工具；页面发现工具与只读元数据保持可用。
pub fn without_unbound_sql_tools(catalog: &mut ToolCatalog) {
    let disabled = UNBOUND_SQL_TOOLS
        .iter()
        .map(|id| (*id).to_string())
        .collect();
    catalog.without_tools(&disabled);
}

const FIND_TABLE_MAX_RESULTS: usize = 50;
const FIND_TABLE_INDEX_CANDIDATES: usize = 200;
const FIND_TABLE_MAX_DATABASES_PER_CONNECTION: usize = 12;
const FIND_TABLE_MAX_TABLES_PER_DATABASE: u64 = 30;
const FIND_TABLE_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// 系统库不参与表名检索（用户不会把业务表放这里），排序时后置避免占满名额。
fn is_system_schema_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "information_schema" | "mysql" | "performance_schema" | "sys" | "admin" | "local"
    )
}

/// 按名称实时扫描已保存 SQL 连接中的表/视图（只读元数据，不读取数据行）。
///
/// 完整遍历每个连接的库列表并受上限约束；命中系统库/超预算/超名额时显式返回
/// `truncated`，不让模型把不完整结果当成"不存在"。单个连接不可用只记入
/// `unavailable_connections`，不阻断其余连接。
fn scan_tables_in_connections(
    configs: &[ConnectionConfig],
    name: &str,
    cancel: &fluxdb_core::CancelFlag,
    limit: usize,
) -> Result<serde_json::Value, &'static str> {
    let needle = name.trim().to_ascii_lowercase();
    if needle.is_empty() || needle.len() > 256 || limit == 0 {
        return Err("page_query_invalid");
    }
    let started = std::time::Instant::now();
    let mut items = Vec::new();
    let mut truncated = false;
    let mut unavailable = Vec::new();
    'connections: for config in configs {
        if cancel.is_cancelled() {
            return Err("page_query_cancelled");
        }
        if started.elapsed() > FIND_TABLE_BUDGET {
            truncated = true;
            break;
        }
        let databases = match list_objects_for_connection(config, None) {
            Ok(objects) => objects
                .into_iter()
                .filter(|object| matches!(object.path.kind, ObjectKind::Database))
                .collect::<Vec<_>>(),
            Err(_) => {
                unavailable.push(config.name.clone());
                continue;
            }
        };
        let (user_databases, system_databases): (Vec<_>, Vec<_>) = databases
            .into_iter()
            .partition(|object| !is_system_schema_name(&object.path.name));
        for (searched, database) in user_databases
            .into_iter()
            .chain(system_databases)
            .enumerate()
        {
            if searched >= FIND_TABLE_MAX_DATABASES_PER_CONNECTION {
                truncated = true;
                break 'connections;
            }
            if cancel.is_cancelled() {
                return Err("page_query_cancelled");
            }
            if started.elapsed() > FIND_TABLE_BUDGET {
                truncated = true;
                break 'connections;
            }
            let Ok(connector) = fluxdb_connectors::connector_for(config) else {
                continue;
            };
            let Ok(tables) = connector.list_completion_tables_with_cancel(
                Some(&database.path.name),
                None,
                &needle,
                FIND_TABLE_MAX_TABLES_PER_DATABASE,
                &|| cancel.is_cancelled(),
            ) else {
                continue;
            };
            for table in tables {
                // 连接器内部使用子序列模糊匹配；这里只保留名字包含关键字的命中，
                // 让候选列表可预测，避免把无关的跳字符匹配当作定位依据。
                if !table.name.to_ascii_lowercase().contains(&needle) {
                    continue;
                }
                if items.len() >= limit {
                    truncated = true;
                    break 'connections;
                }
                items.push(serde_json::json!({
                    "connection_id": config.id.0,
                    "connection_name": config.name,
                    "database": table.database.or_else(|| Some(database.path.name.clone())),
                    "schema": table.schema,
                    "name": table.name,
                    "kind": format!("{:?}", table.kind),
                }));
            }
        }
    }
    let exact_matches = items
        .iter()
        .filter(|item| {
            item["name"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&needle))
        })
        .count();
    Ok(serde_json::json!({
        "items": items,
        "total": items.len(),
        "exact_matches": exact_matches,
        "truncated": truncated,
        "unavailable_connections": unavailable,
        "scope": "已保存 SQL 连接的表/视图元数据（名字包含匹配）；不含数据行，不执行 SQL",
        "next_step": if exact_matches == 1 {
            "用该对象的 connection_id/database/schema/name/kind 原样调用 page.open_table"
        } else if exact_matches > 1 {
            "存在多个精确匹配，先让用户选择目标，不要自行决定"
        } else if truncated {
            "结果被截断：请用 page.browse_objects 在目标连接逐级展开确认，不要猜目标"
        } else {
            "没有包含匹配；可换关键词重试一次，或用 page.browse_objects 逐级浏览"
        },
    }))
}

/// 按名称或描述关键词在已保存 SQL 连接中查找表/视图。
///
/// 优先复用 SQL 补全索引（表名、表注释、字段名/注释都在索引里），只读本地元数据；
/// 索引未就绪、已过期或没有命中时回退实时目录扫描。索引为空不能直接当成“不存在”，
/// 否则未预热的大库、新库或部分作用域会漏结果。
impl AppController {
    pub fn find_table_in_connections(
        &self,
        configs: &[ConnectionConfig],
        name: &str,
        cancel: &fluxdb_core::CancelFlag,
    ) -> Result<serde_json::Value, &'static str> {
        let needle = name.trim().to_ascii_lowercase();
        if needle.is_empty() || needle.len() > 256 {
            return Err("page_query_invalid");
        }
        let started = std::time::Instant::now();
        let mut items = Vec::new();
        let mut sources = Vec::new();
        let mut fallback_configs = Vec::new();
        for config in configs {
            if cancel.is_cancelled() {
                return Err("page_query_cancelled");
            }
            if started.elapsed() > FIND_TABLE_BUDGET {
                break;
            }
            let indexed = self.ai_table_index_search(config, name, FIND_TABLE_INDEX_CANDIDATES);
            match indexed {
                Some((matches, scope_gaps)) => {
                    let found = !matches.is_empty();
                    sources.push(serde_json::json!({
                        "connection_id": config.id.0,
                        "source": "completion_index",
                        // 有失效/未建索引的作用域时实时补齐，不能用部分索引冒充全量结果。
                        "coverage": if scope_gaps || !found {
                            "index_plus_live"
                        } else {
                            "index_only"
                        },
                    }));
                    if !found {
                        // 索引可用但没有命中：实时扫描一次，避免未建索引的库被误判为不存在。
                        fallback_configs.push(config.clone());
                    }
                    items.extend(
                        matches
                            .into_iter()
                            .map(|hit| metadata_hit_to_table_item(config, hit))
                            .collect::<Vec<_>>(),
                    );
                    if scope_gaps {
                        fallback_configs.push(config.clone());
                    }
                }
                // 索引没有命中时不能断言不存在：它可能只覆盖部分库/schema。
                _ => fallback_configs.push(config.clone()),
            }
        }
        let mut seen_fallbacks = BTreeSet::new();
        fallback_configs.retain(|config| seen_fallbacks.insert(config.id));
        let mut truncated = false;
        let mut unavailable = Vec::new();
        if !fallback_configs.is_empty() && items.len() < FIND_TABLE_MAX_RESULTS {
            if started.elapsed() > FIND_TABLE_BUDGET {
                truncated = true;
            } else {
                let remaining = FIND_TABLE_MAX_RESULTS - items.len();
                let scanned =
                    scan_tables_in_connections(&fallback_configs, name, cancel, remaining)?;
                sources.push(serde_json::json!({
                    "source": "live_scan",
                    "connection_ids": fallback_configs.iter().map(|c| c.id.0).collect::<Vec<_>>(),
                }));
                truncated |= scanned["truncated"].as_bool().unwrap_or(false);
                if let Some(values) = scanned["items"].as_array() {
                    items.extend(values.iter().cloned());
                }
                if let Some(values) = scanned["unavailable_connections"].as_array() {
                    push_unique_strings(values, &mut unavailable);
                }
            }
        }
        dedupe_table_items(&mut items);
        if items.len() > FIND_TABLE_MAX_RESULTS {
            items.truncate(FIND_TABLE_MAX_RESULTS);
            truncated = true;
        }
        let exact_matches = items
            .iter()
            .filter(|item| {
                item["name"]
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case(&needle))
            })
            .count();
        Ok(serde_json::json!({
            "items": items,
            "total": items.len(),
            "exact_matches": exact_matches,
            "truncated": truncated,
            "unavailable_connections": unavailable,
            "sources": sources,
            "scope": "已保存 SQL 连接的表/视图元数据（表名、表注释、字段名/注释）；不含数据行，不执行 SQL",
            "next_step": if exact_matches == 1 {
                "用该对象的 connection_id/database/schema/name/kind 原样调用 page.open_table"
            } else if exact_matches > 1 {
                "存在多个精确匹配，先让用户选择目标，不要自行决定"
            } else if truncated {
                "结果被截断：请用 page.browse_objects 在目标连接逐级展开确认，不要猜目标"
            } else if !items.is_empty() {
                "没有精确同名对象：结合 comment / matched_columns 选择；不确定时询问用户，不要猜"
            } else {
                "没有匹配；可换关键词重试一次，或用 page.browse_objects 逐级浏览"
            },
        }))
    }
}

fn push_unique_strings(values: &[serde_json::Value], output: &mut Vec<String>) {
    for value in values {
        if let Some(text) = value.as_str()
            && !output.iter().any(|existing| existing == text)
        {
            output.push(text.to_string());
        }
    }
}

fn metadata_hit_to_table_item(
    config: &ConnectionConfig,
    hit: MetadataSearchHit,
) -> serde_json::Value {
    match hit.payload {
        MetadataEntryPayload::Table(table) => serde_json::json!({
            "connection_id": config.id.0,
            "connection_name": config.name,
            "database": table.database,
            "schema": table.schema,
            "name": hit.title,
            "kind": hit.kind.as_str(),
            "comment": hit.description.map(|comment| {
                comment.chars().take(256).collect::<String>()
            }),
            "matched_columns": hit.matched_keywords.into_iter().take(16).collect::<Vec<_>>(),
        }),
    }
}

fn dedupe_table_items(items: &mut Vec<serde_json::Value>) {
    let mut seen = BTreeSet::new();
    items.retain(|item| {
        seen.insert((
            item["connection_id"].as_u64(),
            item["database"].as_str().map(str::to_string),
            item["schema"].as_str().map(str::to_string),
            item["name"].as_str().map(str::to_string),
            item["kind"].as_str().map(str::to_string),
        ))
    });
}

struct AiTablePageReply {
    result: Option<Result<serde_json::Value, &'static str>>,
    waker: Option<std::task::Waker>,
}

/// UI 操作由后台工具发起，结果只返回有界定位/结构/状态，不含数据行。
pub struct AiTablePageRequest {
    pub action: AiTablePageAction,
    pub turn_id: String,
    allowed_connections: Vec<ConnectionId>,
    configs: Vec<ConnectionConfig>,
    cancel: fluxdb_core::CancelFlag,
    deadline: std::time::Instant,
    reply: Mutex<AiTablePageReply>,
}

impl std::fmt::Debug for AiTablePageRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiTablePageRequest")
            .field("turn_id", &self.turn_id)
            .finish_non_exhaustive()
    }
}

impl AiTablePageRequest {
    pub fn active(&self) -> bool {
        !self.cancel.is_cancelled()
            && std::time::Instant::now() < self.deadline
            && self.reply.lock().is_ok_and(|reply| reply.result.is_none())
    }
    pub fn allows(&self, connection: ConnectionId) -> bool {
        self.allowed_connections.contains(&connection)
    }
    pub fn matches_connection(&self, config: &ConnectionConfig) -> bool {
        self.configs.iter().any(|expected| expected == config)
    }
    pub fn configs(&self) -> Vec<ConnectionConfig> {
        self.configs.clone()
    }
    pub fn cancel_flag(&self) -> fluxdb_core::CancelFlag {
        self.cancel.clone()
    }
    pub fn complete(&self, result: Result<serde_json::Value, &'static str>) {
        if let Ok(mut reply) = self.reply.lock()
            && reply.result.is_none()
        {
            reply.result = Some(if self.cancel.is_cancelled() {
                Err("page_query_cancelled")
            } else if std::time::Instant::now() >= self.deadline {
                Err("page_query_timeout")
            } else {
                result
            });
            if let Some(waker) = reply.waker.take() {
                waker.wake();
            }
        }
    }
    async fn wait(&self) -> Result<serde_json::Value, &'static str> {
        std::future::poll_fn(|cx| {
            if self.cancel.is_cancelled() {
                self.complete(Err("page_query_cancelled"));
            } else if std::time::Instant::now() >= self.deadline {
                self.complete(Err("page_query_timeout"));
            }
            let Ok(mut reply) = self.reply.lock() else {
                return std::task::Poll::Ready(Err("page_query_unavailable"));
            };
            if let Some(result) = reply.result.take() {
                // 保留终态标志，迟到动作不能在工具已返回后继续执行。
                reply.result = Some(Err("page_query_unavailable"));
                std::task::Poll::Ready(result)
            } else {
                reply.waker = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await
    }
}

struct AiTablePageWaitGuard(Arc<AiTablePageRequest>);
impl Drop for AiTablePageWaitGuard {
    fn drop(&mut self) {
        self.0.complete(Err("page_query_cancelled"));
    }
}

struct AiTablePageTool {
    descriptor: fluxdb_core::ToolDescriptor,
    actions: Arc<dyn UiActionSink>,
    connections: Vec<ConnectionId>,
    configs: Vec<ConnectionConfig>,
}

impl fluxdb_core::AiTool for AiTablePageTool {
    fn descriptor(&self) -> &fluxdb_core::ToolDescriptor {
        &self.descriptor
    }
    fn execute(
        &self,
        ctx: fluxdb_core::ToolExecutionContext,
        arguments: String,
    ) -> fluxdb_core::AiFuture<'_, fluxdb_core::ToolReceipt> {
        Box::pin(async move {
            let started_at_ms = now_ms();
            let result = async {
                let value: serde_json::Value =
                    serde_json::from_str(&arguments).map_err(|_| "page_query_invalid")?;
                let schema = serde_json::from_str(&self.descriptor.input_schema_json)
                    .map_err(|_| "page_query_invalid")?;
                validate_schema(&schema, &value).map_err(|_| "page_query_invalid")?;
                let action = match self.descriptor.id.as_str() {
                    "page.find_table" => {
                        let name = value["table"].as_str().ok_or("page_query_invalid")?;
                        AiTablePageAction::Find(name.to_string())
                    }
                    "page.browse_objects" => {
                        let id = ConnectionId(
                            value["connection_id"]
                                .as_u64()
                                .ok_or("page_query_invalid")?,
                        );
                        if !self.connections.contains(&id) {
                            return Err("page_query_unavailable");
                        }
                        let parent = value
                            .get("parent")
                            .map(|v| serde_json::from_value(v.clone()))
                            .transpose()
                            .map_err(|_| "page_query_invalid")?;
                        AiTablePageAction::Browse {
                            connection_id: id,
                            parent,
                            offset: value["offset"].as_u64().unwrap_or(0).min(100_000) as usize,
                        }
                    }
                    "page.open_table" => {
                        let object: ObjectPath = serde_json::from_value(value["object"].clone())
                            .map_err(|_| "page_query_invalid")?;
                        if !self.connections.contains(&object.connection_id) {
                            return Err("page_query_unavailable");
                        }
                        AiTablePageAction::Open(object)
                    }
                    "page.query_table" => {
                        let query: AiTableQuery =
                            serde_json::from_value(value).map_err(|_| "page_query_invalid")?;
                        AiTablePageAction::Query(query)
                    }
                    _ => return Err("page_query_invalid"),
                };
                let request = Arc::new(AiTablePageRequest {
                    action,
                    turn_id: ctx.turn_id.clone(),
                    allowed_connections: self.connections.clone(),
                    configs: self.configs.clone(),
                    cancel: ctx.cancel.clone(),
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(25),
                    reply: Mutex::new(AiTablePageReply {
                        result: None,
                        waker: None,
                    }),
                });
                let _guard = AiTablePageWaitGuard(request.clone());
                if !request.active() {
                    return Err("page_query_cancelled");
                }
                self.actions
                    .submit(UiAction::TablePage(request.clone()))
                    .map_err(|_| "page_query_unavailable")?;
                request.wait().await
            }
            .await;
            let (status, value, code) = match result {
                Ok(value) => (fluxdb_core::ToolReceiptStatus::Succeeded, value, None),
                Err(code) => (
                    if ctx.cancel.is_cancelled() {
                        fluxdb_core::ToolReceiptStatus::Cancelled
                    } else {
                        fluxdb_core::ToolReceiptStatus::Failed
                    },
                    serde_json::json!({"error":code,"retryable":false}),
                    Some(code.into()),
                ),
            };
            Ok(fluxdb_core::ToolReceipt {
                call_id: ctx.call_id,
                tool_id: self.descriptor.id.clone(),
                status,
                result_json: value.to_string(),
                error_code: code,
                affected_rows: None,
                started_at_ms,
                finished_at_ms: now_ms(),
            })
        })
    }
}

pub fn add_ai_table_page_tools(
    catalog: &mut ToolCatalog,
    connections: &[ConnectionConfig],
    actions: Arc<dyn UiActionSink>,
) -> fluxdb_core::Result<()> {
    let path = serde_json::json!({"type":"object","additionalProperties":false,
    "required":["connection_id","database","schema","name","kind"],"properties":{
        "connection_id":{"type":"integer","minimum":1},
        "database":{"type":["string","null"]},"schema":{"type":["string","null"]},
        "name":{"type":"string","minLength":1,"maxLength":512},
        "kind":{"type":"string","enum":["Database","Schema","Table","View"]}
    }});
    let specs = [
        (
            "page.find_table",
            "按表名、表注释或字段名/注释关键词在已保存的 SQL 连接中只读查找，返回连接、库、schema、完整对象定位与命中字段（最多50条）。用户给出表名或只描述业务含义（如“订单增长”）时先用一次；多个精确匹配必须让用户选择。只查元数据，不读取数据行、不执行 SQL；失败后不要猜连接 ID 重试。",
            serde_json::json!({"type":"object","additionalProperties":false,"required":["table"],
                "properties":{"table":{"type":"string","minLength":1,"maxLength":256,
                    "description":"表名，或描述业务含义的关键词（也可用字段名/注释线索）"}}}),
        ),
        (
            "page.browse_objects",
            "逐级打开侧边栏连接/库/schema，返回当前层最多50个真实对象。parent只能使用上次返回的完整path，不能猜；同名目标需澄清。不会读取数据行。",
            serde_json::json!({"type":"object","additionalProperties":false,"required":["connection_id"],
                "properties":{"connection_id":{"type":"integer","minimum":1},"parent":path.clone(),
                "offset":{"type":"integer","minimum":0,"maximum":100000}}}),
        ),
        (
            "page.open_table",
            "选中并打开已发现的表/视图，返回page_ref和真实列结构；复用已有表页，不查询数据、不覆盖现有条件或未提交修改。",
            serde_json::json!({"type":"object","additionalProperties":false,"required":["object"],
                "properties":{"object":path}}),
        ),
        (
            "page.query_table",
            "在已打开的page_ref上填写AND筛选和排序。用户要求查询时execute=true，只填条件时false。列名必须来自真实结构；limit最多1000。每轮最多实际查询一次；结果只在表页展示，不返回行值。现有不同条件必须由用户先处理，不可覆盖。",
            serde_json::json!({"type":"object","additionalProperties":false,
                "required":["page_ref","filters","sort","execute","limit"],"properties":{
                "page_ref":{"type":"string","minLength":1,"maxLength":128},
                "execute":{"type":"boolean"},"limit":{"type":"integer","minimum":1,"maximum":1000},
                "filters":{"type":"array","maxItems":20,"items":{"type":"object","additionalProperties":false,
                    "required":["field","op","values"],"properties":{
                    "field":{"type":"string","minLength":1,"maxLength":512},
                    "op":{"type":"string","enum":["eq","ne","lt","le","gt","ge","contains","is_null","is_not_null","in","not_in"]},
                    "values":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":4096}}}}},
                "sort":{"type":"array","maxItems":8,"items":{"type":"object","additionalProperties":false,
                    "required":["field","descending"],"properties":{
                    "field":{"type":"string","minLength":1,"maxLength":512},"descending":{"type":"boolean"}}}}
            }}),
        ),
    ];
    let ids = connections
        .iter()
        .filter(|c| {
            matches!(
                c.kind,
                DatabaseKind::Sqlite
                    | DatabaseKind::Postgres
                    | DatabaseKind::Kingbase
                    | DatabaseKind::MySql
                    | DatabaseKind::TiDb
                    | DatabaseKind::MariaDb
                    | DatabaseKind::Dameng
                    | DatabaseKind::SqlServer
            )
        })
        .map(|c| c.id)
        .collect::<Vec<_>>();
    for (id, description, schema) in specs {
        catalog.add_tool_with_shape_and_effect(
            Box::new(AiTablePageTool {
                descriptor: fluxdb_core::ToolDescriptor {
                    id: id.into(),
                    version: 1,
                    description: description.into(),
                    risk: fluxdb_core::ToolRisk::Read,
                    input_schema_json: schema.to_string(),
                },
                actions: actions.clone(),
                connections: ids.clone(),
                configs: connections.to_vec(),
            }),
            ToolOutputShape::Opaque,
            Some(EffectClass::LocalUiMutation),
        )?;
    }
    Ok(())
}

impl AppController {
    fn ai_table_config(&self, object: &ObjectPath) -> fluxdb_core::Result<&ConnectionConfig> {
        let config = self
            .connection_config(object.connection_id)
            .ok_or_else(|| editor_error("page_query_unavailable"))?;
        self.validate_ai_database_resource(config)?;
        if !matches!(
            config.kind,
            DatabaseKind::Sqlite
                | DatabaseKind::MySql
                | DatabaseKind::TiDb
                | DatabaseKind::MariaDb
                | DatabaseKind::Postgres
                | DatabaseKind::Kingbase
                | DatabaseKind::Dameng
                | DatabaseKind::SqlServer
        ) {
            return Err(editor_error("page_query_unsupported"));
        }
        Ok(config)
    }

    fn browse_ai_table_objects(
        &mut self,
        connection_id: ConnectionId,
        parent: Option<ObjectPath>,
    ) -> AppEvent {
        let probe = parent.clone().unwrap_or(ObjectPath {
            connection_id,
            database: None,
            schema: None,
            name: String::new(),
            kind: ObjectKind::Database,
        });
        let result = (|| {
            let config = self.ai_table_config(&probe)?;
            if probe.connection_id != connection_id {
                return Err(editor_error("page_query_invalid"));
            }
            if let Some(parent) = parent.as_ref()
                && (!matches!(parent.kind, ObjectKind::Database | ObjectKind::Schema)
                    || !self
                        .state
                        .connections
                        .iter()
                        .find(|c| c.config.id == connection_id)
                        .is_some_and(|c| c.objects.iter().any(|o| &o.path == parent)))
            {
                return Err(editor_error("page_query_unavailable"));
            }
            list_objects_for_connection(config, parent.as_ref())
        })();
        match result {
            Ok(objects) => AppEvent::AiTableObjectsLoaded {
                connection_id,
                parent,
                objects,
            },
            Err(error) => self.fail(error),
        }
    }

    fn ai_table_columns(&self, object: &ObjectPath) -> fluxdb_core::Result<Vec<Column>> {
        let config = self.ai_table_config(object)?;
        if !matches!(object.kind, ObjectKind::Table | ObjectKind::View)
            || !self
                .state
                .connections
                .iter()
                .find(|c| c.config.id == object.connection_id)
                .is_some_and(|c| c.objects.iter().any(|item| &item.path == object))
        {
            return Err(editor_error("page_query_unavailable"));
        }
        let columns = connector_for(config)?.list_completion_columns(
            object.database.as_deref(),
            object.schema.as_deref(),
            &object.name,
        )?;
        if columns.is_empty() || columns.len() > 512 {
            return Err(editor_error("page_query_unsupported"));
        }
        Ok(columns
            .into_iter()
            .map(|c| Column {
                name: c.name,
                type_name: c.type_name,
                nullable: c.nullable,
                primary_key: c.primary_key,
                comment: c.comment,
            })
            .collect())
    }

    fn load_ai_table_page(
        &self,
        object: &ObjectPath,
        query: &AiTableQuery,
    ) -> fluxdb_core::Result<DataPage> {
        let columns = self.ai_table_columns(object)?;
        let (filters, sort) = query.specs(&columns)?;
        self.load_data_page(object, Pagination::new(0, query.limit), &sort, &filters, None)
    }
}

#[cfg(test)]
mod ai_table_page_tests {
    use super::*;

    fn columns() -> Vec<Column> {
        ["id", "status", "created_at"]
            .into_iter()
            .map(|name| Column {
                name: name.into(),
                type_name: Some("text".into()),
                nullable: true,
                primary_key: false,
                comment: None,
            })
            .collect()
    }

    fn query(filters: serde_json::Value, sort: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "page_ref":"table-page:turn:1", "execute":true, "limit":100,
            "filters":filters, "sort":sort
        })
    }

    fn completion_table(
        schema: &str,
        name: &str,
        comment: Option<&str>,
    ) -> CompletionTable {
        CompletionTable {
            database: Some("main".into()),
            schema: Some(schema.into()),
            name: name.into(),
            kind: ObjectKind::Table,
            comment: comment.map(str::to_string),
        }
    }

    fn completion_column(table: &str, name: &str, comment: Option<&str>) -> CompletionColumn {
        CompletionColumn {
            database: Some("main".into()),
            schema: Some("public".into()),
            table: table.into(),
            name: name.into(),
            type_name: Some("text".into()),
            nullable: true,
            primary_key: false,
            comment: comment.map(str::to_string),
            stable: None,
        }
    }

    fn index_config(path: PathBuf) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId(1),
            name: "indexed".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: fluxdb_core::Endpoint::SqliteFile {
                path,
                read_only: true,
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            mongo_profile: None,
            sqlserver_profile: None,
            oracle_profile: None,
        }
    }

    fn index_snapshot(connection_id: ConnectionId, database: &str, table: &str) -> CompletionIndexSnapshot {
        CompletionIndexSnapshot {
            connection_id,
            database: Some(database.into()),
            schema: Some("public".into()),
            tables: vec![TableRef {
                database: Some(database.into()),
                schema: Some("public".into()),
                name: table.into(),
                kind: ObjectKind::Table,
                rows: None,
                comment: None,
            }],
            columns: Vec::new(),
            routines: Vec::new(),
            triggers: Vec::new(),
            meta: CompletionIndexMeta {
                app_index_version: COMPLETION_INDEX_VERSION,
                db_kind: DatabaseKind::Sqlite,
                last_indexed_at: unix_timestamp_secs(),
                last_verified_at: unix_timestamp_secs(),
                ttl_seconds: COMPLETION_INDEX_TTL_SECONDS,
                dirty: false,
                table_count: 1,
                table_fingerprints: Vec::new(),
            },
        }
    }

    fn hit_table(hit: &MetadataSearchHit) -> &TableRef {
        match &hit.payload {
            MetadataEntryPayload::Table(table) => table,
        }
    }

    #[test]
    fn structured_filters_map_to_real_columns_and_sort_directions() {
        let query: AiTableQuery = serde_json::from_value(query(
            serde_json::json!([
                {"field":"status","op":"eq","values":["failed"]},
                {"field":"created_at","op":"ge","values":["2026-01-01"]},
                {"field":"id","op":"in","values":["1","2"]}
            ]),
            serde_json::json!([{"field":"created_at","descending":true}]),
        ))
        .expect("query");

        let (filters, sort) = query.specs(&columns()).expect("valid filters");
        assert_eq!(filters.len(), 3);
        assert_eq!(filters[0].op, FilterOp::Eq);
        assert_eq!(filters[2].op, FilterOp::InList);
        assert_eq!(sort[0].direction, SortDirection::Desc);
    }

    #[test]
    fn unbound_catalog_exposes_no_sql_execution_tools() {
        let mut specs = standard_sql_tool_specs().expect("SQL specs");
        specs.extend([
            ToolSpec::from_descriptor(&fluxdb_core::ToolDescriptor {
                id: "redis.scan".into(),
                version: 1,
                description: "redis scan".into(),
                risk: fluxdb_core::ToolRisk::Read,
                input_schema_json: r#"{"type":"object"}"#.into(),
            })
            .expect("redis"),
            ToolSpec::from_descriptor(&fluxdb_core::ToolDescriptor {
                id: "page.find_table".into(),
                version: 1,
                description: "find table".into(),
                risk: fluxdb_core::ToolRisk::Read,
                input_schema_json: r#"{"type":"object"}"#.into(),
            })
            .expect("page"),
        ]);
        let mut catalog = ToolCatalog::new(specs);
        without_unbound_sql_tools(&mut catalog);

        for id in UNBOUND_SQL_TOOLS {
            assert!(catalog.describe(id).is_none(), "{id}");
            assert!(catalog.executable(id).is_none(), "{id}");
            assert!(catalog.prepare(id, "{}").is_err(), "{id}");
        }
        assert!(catalog.describe("page.find_table").is_some());
    }

    #[test]
    fn late_relationship_tools_do_not_reintroduce_unbound_execution() {
        let mut specs = standard_sql_tool_specs().expect("SQL specs");
        for id in ["db.get_relationships", "db.find_join_paths"] {
            specs.push(
                ToolSpec::from_descriptor(&fluxdb_core::ToolDescriptor {
                    id: id.into(),
                    version: 1,
                    description: id.into(),
                    risk: fluxdb_core::ToolRisk::Read,
                    input_schema_json: r#"{"type":"object"}"#.into(),
                })
                .expect("relationship spec"),
            );
        }
        let mut catalog = ToolCatalog::new(specs);
        // 模拟运行时装配顺序：SQL 目录先建立，关系工具随后追加，最后才收窄。
        without_unbound_sql_tools(&mut catalog);
        for id in ["db.get_relationships", "db.find_join_paths"] {
            assert!(catalog.describe(id).is_none(), "{id}");
        }
    }

    #[test]
    fn find_table_returns_real_locations_without_rows() {
        let path =
            std::env::temp_dir().join(format!("fluxdb-find-table-{}.sqlite", uuid::Uuid::new_v4()));
        let conn = rusqlite::Connection::open(&path).expect("SQLite fixture");
        conn.execute_batch(
            "CREATE TABLE \"3d_countries\" (id TEXT, match_node TEXT);
             INSERT INTO \"3d_countries\" VALUES ('AD', 'hz');",
        )
        .expect("fixture table");
        drop(conn);
        let config = ConnectionConfig {
            id: ConnectionId(1),
            name: "docker".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: fluxdb_core::Endpoint::SqliteFile {
                path: path.clone(),
                read_only: true,
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            mongo_profile: None,
            sqlserver_profile: None,
            oracle_profile: None,
        };
        let controller = AppController::new();
        let result = controller
            .find_table_in_connections(&[config], "3d_countries", &fluxdb_core::CancelFlag::new())
            .expect("find table");
        assert_eq!(result["exact_matches"], 1);
        assert_eq!(result["items"][0]["connection_name"], "docker");
        assert_eq!(result["items"][0]["database"], "main");
        assert_eq!(result["items"][0]["kind"], "Table");
        assert_eq!(result["truncated"], false);
        assert_eq!(result["sources"][0]["source"], "live_scan");
        assert!(!result.to_string().contains("'hz'"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn metadata_index_finds_tables_by_name_comment_and_column() {
        let mut index = CompletionIndex::default();
        let connection_id = ConnectionId(1);
        index.insert_tables(
            connection_id,
            Some("main"),
            Some("public"),
            vec![
                completion_table("public", "orders", Some("订单主表，记录客户下单信息")),
                completion_table("public", "order_items", Some("line items")),
                completion_table("public", "customers", Some("buyer master data")),
            ],
            DatabaseKind::Sqlite,
        );
        index.replace_table_columns(
            connection_id,
            Some("main"),
            Some("public"),
            "orders",
            vec![completion_column("orders", "order_status", Some("fulfillment state"))],
            DatabaseKind::Sqlite,
        );

        let by_name = index.search_fresh_metadata(connection_id, "order", 10, unix_timestamp_secs());
        assert_eq!(hit_table(&by_name[0]).name, "orders");
        assert!(
            by_name
                .iter()
                .any(|hit| hit_table(hit).name == "order_items")
        );

        let by_comment =
            index.search_fresh_metadata(connection_id, "客户", 10, unix_timestamp_secs());
        assert_eq!(by_comment.len(), 1);
        assert_eq!(hit_table(&by_comment[0]).name, "orders");

        // 整句描述先无命中时退到中文二元词，能找到“订单/客户”等线索。
        let by_description = index.search_fresh_metadata(
            connection_id,
            "订单增长与客户下单",
            10,
            unix_timestamp_secs(),
        );
        assert_eq!(hit_table(&by_description[0]).name, "orders");

        let by_column =
            index.search_fresh_metadata(connection_id, "status", 10, unix_timestamp_secs());
        assert_eq!(by_column.len(), 1);
        assert_eq!(hit_table(&by_column[0]).name, "orders");

        index.replace_table_columns(
            connection_id,
            Some("main"),
            Some("public"),
            "orders",
            vec![completion_column("orders", "shipment_state", None)],
            DatabaseKind::Sqlite,
        );
        assert!(
            index
                .search_fresh_metadata(connection_id, "status", 10, unix_timestamp_secs())
                .is_empty(),
            "旧列名不能继续命中"
        );
        assert_eq!(
            hit_table(
                &index.search_fresh_metadata(
                    connection_id,
                    "shipment",
                    10,
                    unix_timestamp_secs()
                )[0]
            )
            .name,
            "orders"
        );

        assert!(
            index
                .search_fresh_metadata(connection_id, "", 10, unix_timestamp_secs())
                .is_empty()
        );
        assert_eq!(
            index
                .search_fresh_metadata(connection_id, "orders", 1, unix_timestamp_secs())
                .len(),
            1
        );
    }

    #[test]
    fn stale_index_scopes_are_not_used_by_ai_search() {
        let mut index = CompletionIndex::default();
        let connection_id = ConnectionId(1);
        index.insert_tables(
            connection_id,
            Some("main"),
            Some("public"),
            vec![completion_table("public", "orders", None)],
            DatabaseKind::Sqlite,
        );
        index.replace_table_columns(
            connection_id,
            Some("main"),
            Some("public"),
            "orders",
            vec![completion_column("orders", "id", None)],
            DatabaseKind::Sqlite,
        );
        index.mark_dirty(connection_id, Some("main"), Some("public"));

        assert!(
            index
                .search_fresh_metadata(connection_id, "orders", 10, unix_timestamp_secs())
                .is_empty()
        );
        assert!(!index.has_fresh_connection_index(connection_id, unix_timestamp_secs()));
        // 列仍然在内存里，但所属作用域已 dirty，AI 不能拿旧结构当事实。
        assert!(
            index
                .search_fresh_metadata(connection_id, "id", 10, unix_timestamp_secs())
                .is_empty()
        );
    }

    #[test]
    fn search_scope_uses_parent_metadata_when_table_schema_has_none() {
        let mut index = CompletionIndex::default();
        let connection_id = ConnectionId(1);
        // 预热按整库写入元信息，但表自身带 search_path 的 schema。
        index.insert_tables(
            connection_id,
            Some("postgres"),
            None,
            vec![CompletionTable {
                database: Some("postgres".into()),
                ..completion_table("public", "orders", None)
            }],
            DatabaseKind::Postgres,
        );
        index.replace_table_columns(
            connection_id,
            Some("postgres"),
            Some("public"),
            "orders",
            vec![completion_column("orders", "id", None)],
            DatabaseKind::Postgres,
        );
        assert_eq!(
            index
                .search_fresh_metadata(connection_id, "orders", 10, unix_timestamp_secs())
                .len(),
            1
        );
        index.mark_dirty(connection_id, Some("postgres"), None);
        assert!(
            index
                .search_fresh_metadata(connection_id, "orders", 10, unix_timestamp_secs())
                .is_empty(),
            "整库失效后不能继续使用父作用域里的旧表"
        );
    }

    #[test]
    fn column_keywords_work_when_columns_are_indexed_per_database() {
        let mut index = CompletionIndex::default();
        let connection_id = ConnectionId(1);
        index.insert_tables(
            connection_id,
            Some("postgres"),
            None,
            vec![CompletionTable {
                database: Some("postgres".into()),
                ..completion_table("public", "orders", None)
            }],
            DatabaseKind::Postgres,
        );
        // 预热批次按整库取列，列落在 (postgres, None)。
        index.replace_table_columns(
            connection_id,
            Some("postgres"),
            None,
            "orders",
            vec![completion_column("orders", "order_status", None)],
            DatabaseKind::Postgres,
        );
        let matches =
            index.search_fresh_metadata(connection_id, "order_status", 10, unix_timestamp_secs());
        assert_eq!(matches.len(), 1);
        assert_eq!(hit_table(&matches[0]).schema.as_deref(), Some("public"));
        assert_eq!(matches[0].matched_keywords, vec!["order_status"]);
    }

    #[test]
    fn find_table_prefers_ready_index_without_connecting() {
        let config = index_config(PathBuf::from("missing-for-index-test.sqlite"));
        let mut controller = AppController::new();
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config.clone()]));
        if let Ok(mut index) = controller.completion_index.lock() {
            index.insert_tables(
                config.id,
                Some("main"),
                Some("public"),
                vec![completion_table(
                    "public",
                    "orders",
                    Some("customer purchase records"),
                )],
                DatabaseKind::Sqlite,
            );
            index.replace_table_columns(
                config.id,
                Some("main"),
                Some("public"),
                "orders",
                vec![completion_column("orders", "order_status", None)],
                DatabaseKind::Sqlite,
            );
        }

        let result = controller
            .find_table_in_connections(
                std::slice::from_ref(&config),
                "orders",
                &fluxdb_core::CancelFlag::new(),
            )
            .expect("index search");
        assert_eq!(result["sources"].as_array().map(Vec::len), Some(1));
        assert_eq!(result["sources"][0]["source"], "completion_index");
        assert_eq!(result["sources"][0]["coverage"], "index_only");
        assert_eq!(result["items"][0]["name"], "orders");
        assert_eq!(result["items"][0]["comment"], "customer purchase records");

        // 索引未命中时仍回退实时扫描，不能把“未建索引”当成对象不存在。
        let scanned = controller
            .find_table_in_connections(
                std::slice::from_ref(&config),
                "missing_table",
                &fluxdb_core::CancelFlag::new(),
            )
            .expect("fallback scan");
        assert!(
            scanned["sources"]
                .as_array()
                .is_some_and(|sources| sources
                    .iter()
                    .any(|source| source["source"] == "live_scan")),
            "索引没命中时必须回退实时扫描"
        );
        assert_eq!(scanned["items"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn persisted_indexes_are_loaded_across_database_scopes() {
        let root = std::env::temp_dir().join(format!(
            "fluxdb-index-scopes-{}",
            uuid::Uuid::new_v4()
        ));
        let storage = fluxdb_storage::FileStorage::new(root.clone());
        let config = index_config(root.join("target.sqlite"));
        for database in ["main", "analytics"] {
            let snapshot = index_snapshot(config.id, database, &format!("{database}_events"));
            storage
                .save_completion_index(&config, Some(database), Some("public"), &snapshot)
                .expect("save snapshot");
        }

        let snapshots = storage
            .load_completion_indexes_for_connection(&config)
            .expect("load snapshots");
        assert_eq!(snapshots.len(), 2);
        let mut names = snapshots
            .iter()
            .flat_map(|snapshot| snapshot.tables.iter().map(|table| table.name.clone()))
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["analytics_events", "main_events"]);

        let mut controller = AppController::new();
        controller.set_completion_index_storage(storage.clone());
        controller.load_persisted_completion_indexes(&config);
        let loaded = controller
            .completion_index
            .lock()
            .expect("completion index")
            .search_fresh_metadata(config.id, "events", 10, unix_timestamp_secs());
        assert!(
            loaded
                .iter()
                .any(|hit| hit_table(hit).name == "analytics_events"),
            "跨库快照应合并进同一个连接索引"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn index_search_reports_multiple_exact_matches_across_scopes() {
        let root = std::env::temp_dir().join(format!(
            "fluxdb-index-exact-{}",
            uuid::Uuid::new_v4()
        ));
        let storage = fluxdb_storage::FileStorage::new(root.clone());
        let config = index_config(root.join("target.sqlite"));
        for database in ["main", "analytics"] {
            let snapshot = index_snapshot(config.id, database, "orders");
            storage
                .save_completion_index(&config, Some(database), Some("public"), &snapshot)
                .expect("save snapshot");
        }

        let mut controller = AppController::new();
        controller.set_completion_index_storage(storage);
        let result = controller
            .find_table_in_connections(
                std::slice::from_ref(&config),
                "orders",
                &fluxdb_core::CancelFlag::new(),
            )
            .expect("index search");
        assert_eq!(result["exact_matches"], 2);
        assert_eq!(result["items"].as_array().map(Vec::len), Some(2));
        assert_eq!(result["sources"][0]["source"], "completion_index");
        assert_eq!(result["sources"][0]["coverage"], "index_only");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn refreshing_table_list_removes_stale_schema_entries() {
        let mut index = CompletionIndex::default();
        let connection_id = ConnectionId(1);
        index.insert_tables(
            connection_id,
            Some("postgres"),
            None,
            vec![CompletionTable {
                database: Some("postgres".into()),
                ..completion_table("public", "old_orders", None)
            }],
            DatabaseKind::Postgres,
        );
        index.insert_tables(
            connection_id,
            Some("postgres"),
            None,
            vec![CompletionTable {
                database: Some("postgres".into()),
                ..completion_table("public", "orders", None)
            }],
            DatabaseKind::Postgres,
        );
        assert!(
            index
                .search_fresh_metadata(
                    connection_id,
                    "old_orders",
                    10,
                    unix_timestamp_secs()
                )
                .is_empty(),
            "整库刷新后旧 schema 条目必须清除"
        );
        assert_eq!(
            hit_table(
                &index.search_fresh_metadata(
                    connection_id,
                    "orders",
                    10,
                    unix_timestamp_secs()
                )[0]
            )
            .name,
            "orders"
        );
    }

    #[test]
    fn level0_refresh_keeps_target_table_and_all_databases() {
        let path = std::env::temp_dir().join(format!(
            "fluxdb-level0-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let storage = fluxdb_storage::FileStorage::new(path.clone());
        let mut controller = AppController::new();
        controller.set_ai_storage(storage);
        controller.dispatch(AppCommand::ReplaceConnections(vec![ConnectionConfig {
            id: ConnectionId(1),
            name: "docker".into(),
            kind: DatabaseKind::Sqlite,
            endpoint: fluxdb_core::Endpoint::SqliteFile {
                path: std::env::temp_dir().join("unused.sqlite"),
                read_only: true,
            },
            options: Default::default(),
            redis_profile: None,
            mysql_profile: None,
            postgres_profile: None,
            dameng_profile: None,
            mongo_profile: None,
            sqlserver_profile: None,
            oracle_profile: None,
        }]));
        let table = ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("gaea".into()),
            schema: None,
            name: "3d_countries".into(),
            kind: ObjectKind::Table,
        };
        let event = controller.dispatch(AppCommand::RememberAiTableObject(
            table.clone(),
        ));
        assert!(matches!(event, AppEvent::ObjectsLoaded(..)), "{event:?}");
        let databases = ["gaea", "llm"]
            .into_iter()
            .map(|name| ObjectSummary {
                path: ObjectPath {
                    connection_id: ConnectionId(1),
                    database: Some(name.into()),
                    schema: None,
                    name: name.into(),
                    kind: ObjectKind::Database,
                },
                rows: None,
                modified_at: None,
                comment: None,
                stable: None,
            })
            .collect();
        let event = controller.dispatch(AppCommand::FinishAiTableObjects {
            connection_id: ConnectionId(1),
            parent: None,
            objects: databases,
        });
        assert!(matches!(event, AppEvent::ObjectsLoaded(..)), "{event:?}");
        let objects = &controller
            .state()
            .connections
            .iter()
            .find(|c| c.config.id == ConnectionId(1))
            .expect("connection")
            .objects;
        assert!(objects.iter().any(|item| item.path == table), "target table lost");
        for name in ["gaea", "llm"] {
            assert!(
                objects.iter().any(|item| {
                    item.path.kind == ObjectKind::Database && item.path.name == name
                }),
                "missing database {name}"
            );
        }
        let _ = std::fs::remove_dir_all(path);
    }

    fn data_page(limit: u64) -> DataPage {
        DataPage {
            columns: vec![Column {
                name: "id".into(),
                type_name: Some("text".into()),
                nullable: false,
                primary_key: true,
                comment: None,
            }],
            rows: Vec::new(),
            offset: 0,
            limit,
            has_more: false,
            incomplete: None,
        }
    }

    #[test]
    fn stale_page_load_is_discarded_after_newer_operation() {
        let mut controller = AppController::new();
        let config = index_config(std::env::temp_dir().join("token-test.sqlite"));
        let _ = controller.dispatch(AppCommand::ReplaceConnections(vec![config]));
        let event = controller.dispatch(AppCommand::OpenDataEditor(ObjectPath {
            connection_id: ConnectionId(1),
            database: Some("main".into()),
            schema: None,
            name: "orders".into(),
            kind: ObjectKind::Table,
        }));
        let AppEvent::TabOpened(tab_id) = event else {
            panic!("{event:?}");
        };

        let AppEvent::DataPageLoadStarted {
            generation: first, ..
        } = controller.dispatch(AppCommand::BeginDataPageLoad(tab_id))
        else {
            panic!("first load");
        };
        let AppEvent::DataPageLoadStarted {
            generation: second,
            ..
        } = controller.dispatch(AppCommand::BeginDataPageLoad(tab_id))
        else {
            panic!("second load");
        };
        assert!(second > first);

        // 旧结果不得覆盖新操作。
        assert!(matches!(
            controller.dispatch(AppCommand::FinishDataPageLoad {
                tab_id,
                generation: first,
                result: Ok(data_page(10)),
            }),
            AppEvent::DataPageLoadDiscarded { .. }
        ));
        let editor = controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if tab.id == tab_id => Some(editor),
                _ => None,
            })
            .expect("data editor");
        assert!(editor.loading, "旧结果不能提前结束新加载");
        // 最新结果正常写回。
        assert!(matches!(
            controller.dispatch(AppCommand::FinishDataPageLoad {
                tab_id,
                generation: second,
                result: Ok(data_page(20)),
            }),
            AppEvent::DataLoaded(..)
        ));
        let editor = controller
            .state()
            .tabs
            .iter()
            .find_map(|tab| match &tab.kind {
                TabKind::DataEditor(editor) if tab.id == tab_id => Some(editor),
                _ => None,
            })
            .expect("data editor");
        assert_eq!(editor.page.as_ref().map(|page| page.limit), Some(20));
        assert!(!editor.loading);
    }

    #[test]
    fn unknown_columns_bad_arity_and_unbounded_queries_are_rejected() {
        for value in [
            query(
                serde_json::json!([{"field":"missing","op":"eq","values":["x"]}]),
                serde_json::json!([]),
            ),
            query(
                serde_json::json!([{"field":"id","op":"between","values":["1","2"]}]),
                serde_json::json!([]),
            ),
            query(
                serde_json::json!([{"field":"id","op":"eq","values":[]}]),
                serde_json::json!([]),
            ),
            serde_json::json!({
                "page_ref":"table-page:turn:1","execute":true,"limit":1001,
                "filters":[],"sort":[]
            }),
        ] {
            let query: AiTableQuery = serde_json::from_value(value).expect("shape");
            assert_eq!(
                query.specs(&columns()).expect_err("invalid").message,
                "page_query_invalid"
            );
        }
    }
}
