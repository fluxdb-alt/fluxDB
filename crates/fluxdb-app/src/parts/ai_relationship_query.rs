// AI 关系查询（`db.get_relationships` / `db.find_join_paths` 的共享内核）。
//
// 一个事实源：物理外键与本地逻辑关系都归一成 [`ErRelationship`]，可用性只由
// `usage()` 判定，JOIN 条件只由 `join_plan()` 产出。这里负责三件宿主逻辑：
//
// 1. **作用域与授权**：目标句柄 → 连接配置 → 库/schema；请求的表必须落在作用域内且
//    通过对象白名单（`sql_object_allowed`）。
// 2. **汇合与去重**：外键派生关系与本地逻辑关系按身份键合并；角色或字段配对不同的
//    关系各自独立，不误去重。
// 3. **覆盖范围**：外键加载状态、字段加载状态、本地目录是否可读，全部原样返回。
//    「没加载到」不是「没有关系」，调用方必须能看到区别。

/// 一次关系查询的固定作用域（工具入参解析 + 授权校验结果）。
#[derive(Debug)]
pub struct ErRelationshipScope {
    pub target: AiSqlTarget,
    pub config: ConnectionConfig,
    pub database: String,
    pub schema: Option<String>,
    pub scope_key: String,
}

/// 合并后的一条关系及其两端表身份。
pub struct ErRelationshipEntry {
    pub relationship: ErRelationship,
    /// 端点表身份；结构里解析不出来的端点为 `None`（由覆盖范围表达，不静默丢弃）。
    pub left: Option<ErTableRef>,
    pub right: Option<ErTableRef>,
    /// 合并来源：`database_constraint` / `logical`。
    pub source: &'static str,
    pub usage: ErUsage,
    /// 与 `usage` 同一判定下生成的 JOIN 计划；不可用时为 `None`。
    pub plan: Option<fluxdb_core::ErJoinPlan>,
}

/// 目录查询的输入条件。
pub struct ErRelationshipRequest {
    /// 返回这些表之间的关系；为空表示按作用域全量判定（路径搜索需要全图）。
    pub tables: Vec<ErTableRef>,
    /// 包含「指向这些表」的入向关系。
    pub incoming: bool,
    /// 包含「这些表指向别处」的出向关系。
    pub outgoing: bool,
    /// 是否同时返回不可用关系（未确认/已拒绝/失效），并保持各自状态区分。
    pub include_unavailable: bool,
    /// 只返回两端都在 `tables` 内的关系（`direction=between` 的语义）。
    pub require_both_ends: bool,
    pub limit: usize,
}

impl ErRelationshipRequest {
    /// 默认只看可用关系（`incoming`/`outgoing` 同时成立 = 入向 ∪ 出向邻接）。
    pub fn adjacent(tables: Vec<ErTableRef>, limit: usize) -> Self {
        Self {
            tables,
            incoming: true,
            outgoing: true,
            include_unavailable: false,
            require_both_ends: false,
            limit,
        }
    }

    /// 只保留两端都在请求集合内的关系。
    pub fn between(tables: Vec<ErTableRef>, limit: usize) -> Self {
        Self {
            require_both_ends: true,
            ..Self::adjacent(tables, limit)
        }
    }

    /// 全作用域判定：路径搜索的中间表不在请求表里，不能只取起终点的邻接。
    pub fn all() -> Self {
        Self {
            tables: Vec::new(),
            incoming: true,
            outgoing: true,
            include_unavailable: false,
            require_both_ends: false,
            limit: usize::MAX,
        }
    }

    fn touches(&self, entry: &ErRelationshipEntry, wanted: &BTreeSet<String>) -> bool {
        if wanted.is_empty() {
            return true;
        }
        let left = entry.relationship.left_entity.as_str();
        let right = entry.relationship.right_entity.as_str();
        if self.require_both_ends {
            return wanted.contains(left) && wanted.contains(right);
        }
        if wanted.contains(left) && wanted.contains(right) {
            return true;
        }
        (self.outgoing && wanted.contains(left)) || (self.incoming && wanted.contains(right))
    }
}

/// 端点字段加载结果与覆盖范围事实。
pub struct ErColumnLoad {
    /// 实体 ID → 已加载列。
    pub columns: BTreeMap<String, Vec<ErColumn>>,
    /// 实体 ID → 加载状态。
    pub status: BTreeMap<String, ErLoadStatus>,
    /// 字段加载被表数量上限截断时为 true：其余关系未被判定，不能当成不可用。
    pub truncated: bool,
}

/// 关系筛选结果与覆盖范围。
pub struct ErRelationshipSelection {
    pub entries: Vec<ErRelationshipEntry>,
    pub total_matched: usize,
    pub truncated: bool,
    pub columns: ErColumnLoad,
}

/// 单次关系工具允许按需加载的端点表数量上限。
///
/// 可用性判定要求两端字段都能读到（否则无法证明列存在且类型可比较），因此库级关系
/// 判定天然要按端点表数加载字段。超出上限时相关关系判为「未判定」而不是「不可用」，
/// 并在 coverage 里给出 `column_load_truncated`，绝不静默缩小覆盖范围。
pub const ER_AI_MAX_COLUMN_TABLES: usize = 256;

/// 解析并校验查询作用域（含对象白名单与库/schema 边界）。
pub fn er_relationship_scope(
    resolver: &dyn SqlExecutionResolver,
    target: &AiSqlTarget,
    requested: &[serde_json::Value],
) -> Result<ErRelationshipScope, SqlExecError> {
    let config = resolver
        .connection_config(target.connection_id)
        .ok_or_else(|| {
            SqlExecError::new(
                "connection_not_found",
                format!("连接不在本轮授权范围内: {}", target.connection_id.0),
            )
        })?;
    if config.kind == fluxdb_core::DatabaseKind::Redis {
        return Err(SqlExecError::new(
            "unsupported_database",
            "关系工具只适用于关系型数据库",
        ));
    }
    let tables = parse_er_table_refs(target, requested)?;
    if tables.is_empty() {
        return Err(SqlExecError::new(
            "invalid_arguments",
            "至少需要一张明确定位的表",
        ));
    }
    let database = tables
        .first()
        .map(|table| table.database.clone())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SqlExecError::new(
                "target_database_required",
                "target 必须带库，或表定位中给出 database；合法格式：conn:<id>[/db:<database>][/schema:<schema>]",
            )
        })?;
    if let Some(target_database) = &target.database
        && target_database != &database
    {
        return Err(SqlExecError::new(
            "scope_mismatch",
            "表定位的 database 超出当前 target 作用域；删除表定位里的 database，或让 target 的 db 与之一致",
        ));
    }
    let schema = target.schema.clone();
    for table in &tables {
        if let Some(target_schema) = &target.schema
            && table.schema.as_deref() != Some(target_schema.as_str())
        {
            return Err(SqlExecError::new(
                "scope_mismatch",
                "表定位的 schema 超出当前 target 作用域；删除表定位里的 schema，或让 target 的 schema 与之一致",
            ));
        }
        if !resolver.sql_object_allowed(target, &table.name) {
            return Err(SqlExecError::new(
                "object_not_allowed",
                format!("对象 {} 不在授权范围内", table.name),
            ));
        }
    }
    let scope_key =
        fluxdb_core::er_relationship_scope_key(config.id, &database, schema.as_deref());
    Ok(ErRelationshipScope {
        target: target.clone(),
        config,
        database,
        schema,
        scope_key,
    })
}

/// 把工具入参里的结构化表定位解析为表身份（database/schema 缺省时取 target）。
pub fn parse_er_table_refs(
    target: &AiSqlTarget,
    requested: &[serde_json::Value],
) -> Result<Vec<ErTableRef>, SqlExecError> {
    let mut tables = Vec::with_capacity(requested.len());
    for value in requested {
        let object = value
            .as_object()
            .ok_or_else(|| SqlExecError::new("invalid_arguments", "表定位必须是对象"))?;
        let name = object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| SqlExecError::new("invalid_arguments", "表定位缺少 name"))?;
        tables.push(ErTableRef {
            database: object
                .get("database")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| target.database.clone())
                .unwrap_or_default(),
            schema: object
                .get("schema")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| target.schema.clone()),
            name: name.to_string(),
        });
    }
    Ok(tables)
}

/// 汇合目录：物理外键派生关系 + 本地逻辑关系，按身份键去重。
///
/// 返回 `(合并后的关系, 外键加载状态, 本地目录读取说明)`。
pub fn er_merged_relationships(
    scope: &ErRelationshipScope,
    source: &dyn ErRelationshipSource,
) -> Result<(Vec<ErRelationship>, ErLoadStatus, String), SqlExecError> {
    let snapshot =
        source
            .foreign_keys(&scope.config, &scope.database, scope.schema.as_deref())
            .map_err(|error| {
                SqlExecError::new("metadata_error", format!("物理外键读取失败: {error}"))
            })?;
    let foreign = fluxdb_core::er_foreign_key_relationships(&snapshot.edges);
    let (logical, catalog_status) = match source.local_service(&scope.scope_key) {
        Some(service) => match service.list() {
            Ok(relationships) => (relationships, "loaded".to_string()),
            Err(error) => (Vec::new(), format!("unavailable: {error}")),
        },
        None => (Vec::new(), "unavailable".to_string()),
    };
    // 身份键索引：外键先入；本地逻辑关系命中同键时合并，而不是再返回一条。
    let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
    let mut merged: Vec<ErRelationship> = Vec::with_capacity(foreign.len() + logical.len());
    for relationship in foreign {
        by_key.insert(fluxdb_core::er_relationship_identity_key(&relationship), merged.len());
        merged.push(relationship);
    }
    for relationship in logical {
        let key = fluxdb_core::er_relationship_identity_key(&relationship);
        match by_key.get(&key).copied() {
            Some(index) => {
                // 同一条关系既有物理约束又有本地条目：保留用户写的说明、过滤条件与角色，
                // 约束事实以数据库为准补齐（本地未声明约束时才补，不覆盖已有声明）。
                let mut relationship = relationship;
                if relationship.enforcement.kind == fluxdb_core::ErEnforcementKind::None
                    && let Some(constraint) = merged[index].enforcement.constraint_ref.clone()
                {
                    relationship.enforcement.kind = merged[index].enforcement.kind;
                    relationship.enforcement.constraint_ref = Some(constraint);
                    relationship.enforcement.enforced = merged[index].enforcement.enforced;
                }
                merged[index] = relationship;
            }
            None => {
                by_key.insert(key, merged.len());
                merged.push(relationship);
            }
        }
    }
    Ok((merged, snapshot.status, catalog_status))
}

/// 按请求筛选合并后的关系，并给出端点表身份与 `usage()` 结论。
pub fn er_select_relationships(
    scope: &ErRelationshipScope,
    source: &dyn ErRelationshipSource,
    merged: Vec<ErRelationship>,
    request: &ErRelationshipRequest,
    should_cancel: &dyn Fn() -> bool,
) -> Result<ErRelationshipSelection, SqlExecError> {
    let wanted: BTreeSet<String> = request
        .tables
        .iter()
        .map(fluxdb_core::er_entity_id)
        .collect();
    // `tables` 为空 = 全作用域判定（路径搜索需要整张图）。
    let selected: Vec<ErRelationship> = merged
        .into_iter()
        .filter(|relationship| {
            wanted.is_empty()
                || wanted.contains(&relationship.left_entity)
                || wanted.contains(&relationship.right_entity)
        })
        .collect();
    // 端点表身份：请求表 + 命中关系的对端表都要解析，否则相邻关系会因“对端未加载”
    // 被误判成不完整而消失。解析不出来时由覆盖范围（端点缺失列表）表达。
    let mut endpoints: BTreeMap<String, ErTableRef> = request
        .tables
        .iter()
        .map(|table| (fluxdb_core::er_entity_id(table), table.clone()))
        .collect();
    for relationship in &selected {
        for entity in [&relationship.left_entity, &relationship.right_entity] {
            if endpoints.contains_key(entity) {
                continue;
            }
            if let Some((database, schema, name)) = fluxdb_core::er_entity_id_parts(entity) {
                endpoints.insert(
                    entity.clone(),
                    ErTableRef {
                        database: if database.is_empty() {
                            scope.database.clone()
                        } else {
                            database
                        },
                        schema: schema.or_else(|| scope.schema.clone()),
                        name,
                    },
                );
            }
        }
    }
    let columns = load_er_endpoint_columns(scope, source, &endpoints, should_cancel)?;
    let mut entries: Vec<ErRelationshipEntry> = Vec::new();
    for relationship in selected {
        let context = er_usage_context(&relationship, &endpoints, &columns);
        let usage = relationship.usage(&context);
        let plan = relationship.join_plan(&context);
        let entry = ErRelationshipEntry {
            source: er_entry_source(&relationship),
            left: endpoints.get(&relationship.left_entity).cloned(),
            right: endpoints.get(&relationship.right_entity).cloned(),
            relationship,
            usage,
            plan,
        };
        if !request.touches(&entry, &wanted) {
            continue;
        }
        if !request.include_unavailable && !entry.usage.join_candidate {
            continue;
        }
        entries.push(entry);
    }
    entries.sort_by(|left, right| {
        left.relationship
            .left_entity
            .cmp(&right.relationship.left_entity)
            .then_with(|| left.relationship.right_entity.cmp(&right.relationship.right_entity))
            .then_with(|| left.relationship.role.cmp(&right.relationship.role))
    });
    let total_matched = entries.len();
    let truncated = total_matched > request.limit;
    entries.truncate(request.limit);
    Ok(ErRelationshipSelection {
        entries,
        total_matched,
        truncated,
        columns,
    })
}

/// 按需加载端点字段；命中缓存不重复查询，未命中才访问连接器。
fn load_er_endpoint_columns(
    scope: &ErRelationshipScope,
    source: &dyn ErRelationshipSource,
    endpoints: &BTreeMap<String, ErTableRef>,
    should_cancel: &dyn Fn() -> bool,
) -> Result<ErColumnLoad, SqlExecError> {
    let mut tables: Vec<ErTableRef> = endpoints.values().cloned().collect();
    tables.sort();
    let truncated = tables.len() > ER_AI_MAX_COLUMN_TABLES;
    if truncated {
        tables.truncate(ER_AI_MAX_COLUMN_TABLES);
    }
    let mut columns: BTreeMap<String, Vec<ErColumn>> = BTreeMap::new();
    let mut status: BTreeMap<String, ErLoadStatus> = BTreeMap::new();
    if !tables.is_empty() {
        if should_cancel() {
            return Err(SqlExecError::new("cancelled", "关系读取已取消"));
        }
        let batch = source
            .table_columns(&scope.config, &tables)
            .map_err(|error| SqlExecError::new("metadata_error", format!("字段读取失败: {error}")))?;
        for (display, loaded, state) in batch.tables {
            let Some((entity, _)) = endpoints
                .iter()
                .find(|(_, table)| table.display() == display)
            else {
                continue;
            };
            columns.insert(entity.clone(), loaded);
            status.insert(entity.clone(), state);
        }
    }
    Ok(ErColumnLoad {
        columns,
        status,
        truncated,
    })
}

/// 端点字段已加载，且每个配对/过滤列都存在、类型可比较 → 关系可判定。
///
/// 任一端点缺失、字段未读到、列不存在、类型未知或不可比较都置 `coverage_incomplete`
/// 或不可判定；`usage()` 因此不会把「结构没读到」当成「没有这条关系」。
pub fn er_usage_context(
    relationship: &ErRelationship,
    endpoints: &BTreeMap<String, ErTableRef>,
    columns: &ErColumnLoad,
) -> ErUsageContext {
    let left_id = relationship.left_entity.as_str();
    let right_id = relationship.right_entity.as_str();
    let left_state = columns.status.get(left_id);
    let right_state = columns.status.get(right_id);
    let left_columns = columns.columns.get(left_id);
    let right_columns = columns.columns.get(right_id);
    let loaded = |state: Option<&ErLoadStatus>, loaded: Option<&Vec<ErColumn>>| {
        matches!(state, Some(ErLoadStatus::Loaded)) && loaded.is_some_and(|list| !list.is_empty())
    };
    let context = ErUsageContext {
        left_entity_present: loaded(left_state, left_columns)
            && endpoints.contains_key(left_id),
        right_entity_present: loaded(right_state, right_columns)
            && endpoints.contains_key(right_id),
        // 覆盖不完整只针对「因端点表数量上限而根本没去加载」的两端：已加载成功的关系
        // 不能因为库里表多就被一并判成未判定。
        coverage_incomplete: (endpoints.contains_key(left_id) && !columns.status.contains_key(left_id))
            || (endpoints.contains_key(right_id) && !columns.status.contains_key(right_id)),
        pairs_resolvable: true,
        filters_resolvable: true,
    };
    let left_reference = endpoints.get(left_id);
    let right_reference = endpoints.get(right_id);
    let left_types = er_column_types(left_reference, columns.columns.get(left_id));
    let right_types = er_column_types(right_reference, columns.columns.get(right_id));
    let mut pairs_resolvable = context.left_entity_present && context.right_entity_present;
    for pair in &relationship.column_pairs {
        if !fluxdb_core::er_column_types_comparable(
            left_types.get(&pair.left_column).map(String::as_str),
            right_types.get(&pair.right_column).map(String::as_str),
        ) {
            pairs_resolvable = false;
            break;
        }
    }
    let mut filters_resolvable = context.left_entity_present && context.right_entity_present;
    for filter in &relationship.required_filters {
        let (types, present) = match filter.side {
            fluxdb_core::ErRelationSide::Left => (&left_types, context.left_entity_present),
            fluxdb_core::ErRelationSide::Right => (&right_types, context.right_entity_present),
        };
        let Some(column_type) = types.get(&filter.column_id) else {
            filters_resolvable = false;
            break;
        };
        if !present || !fluxdb_core::er_literal_matches_column(&filter.literal, Some(column_type))
        {
            filters_resolvable = false;
            break;
        }
    }
    ErUsageContext {
        pairs_resolvable,
        filters_resolvable,
        ..context
    }
}

/// 列 ID → 类型名。键必须是列 ID（关系里存的是列 ID，不是裸列名），
/// 否则关系引用的列永远匹配不上，所有关系都会被判成「定义不完整」。
fn er_column_types(
    reference: Option<&ErTableRef>,
    columns: Option<&Vec<ErColumn>>,
) -> BTreeMap<String, String> {
    match (reference, columns) {
        (Some(reference), Some(columns)) => columns
            .iter()
            .filter_map(|column| {
                column
                    .type_name
                    .clone()
                    .map(|type_name| (fluxdb_core::er_column_id(reference, &column.name), type_name))
            })
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// 关系来源标签：带数据库约束引用的关系标 `database_constraint`，其余为 `logical`。
pub fn er_entry_source(relationship: &ErRelationship) -> &'static str {
    if relationship.enforcement.constraint_ref.is_some() {
        "database_constraint"
    } else {
        "logical"
    }
}
#[cfg(test)]
mod ai_relationship_query_tests {
    use super::*;

    fn source() -> FakeErSource {
        FakeErSource::new()
            .with_column(
                "orders",
                vec![
                    test_column("id", "bigint"),
                    test_column("customer_id", "bigint"),
                    test_column("ship_to_id", "bigint"),
                ],
            )
            .with_column(
                "customers",
                vec![test_column("id", "bigint"), test_column("is_deleted", "tinyint")],
            )
            .with_column(
                "items",
                vec![test_column("id", "bigint"), test_column("order_id", "bigint")],
            )
    }

    fn select(source: &FakeErSource, request: ErRelationshipRequest) -> ErRelationshipSelection {
        let scope = test_scope(&er_test_connection());
        let (merged, _, _) = er_merged_relationships(&scope, source).expect("汇合关系目录");
        er_select_relationships(&scope, source, merged, &request, &|| false)
            .expect("筛选关系")
    }

    fn roles(selection: &ErRelationshipSelection) -> Vec<String> {
        selection
            .entries
            .iter()
            .map(|entry| entry.relationship.role.clone())
            .collect()
    }

    #[test]
    fn foreign_key_and_matching_logical_relationship_are_merged_into_one_entry() {
        let mut logical = confirmed_logical("orders", "customers", "fk_orders_customer", &[("customer_id", "id")]);
        logical.enforcement = fluxdb_core::ErRelationshipEnforcement {
            kind: fluxdb_core::ErEnforcementKind::None,
            constraint_ref: None,
            enforced: None,
        };
        let source = source()
            .with_edges(vec![test_edge(
                "orders",
                "customers",
                "fk_orders_customer",
                "customer_id",
                "id",
            )])
            .with_logical(vec![logical]);

        let selection = select(&source, ErRelationshipRequest::adjacent(vec![test_table("orders")], 50));

        assert_eq!(selection.entries.len(), 1, "同一关系不得重复返回");
        let entry = &selection.entries[0];
        assert_eq!(entry.source, "database_constraint");
        assert_eq!(
            entry.relationship.enforcement.constraint_ref.as_deref(),
            Some("fk_orders_customer"),
            "合并后仍要保留物理约束证据"
        );
        assert!(entry.usage.join_candidate);
    }

    #[test]
    fn different_role_or_pairs_keeps_relationships_separate() {
        let mut source = source();
        source.logical = vec![
            confirmed_logical("orders", "customers", "buyer", &[("customer_id", "id")]),
            confirmed_logical("orders", "customers", "shipper", &[("ship_to_id", "id")]),
        ];

        let selection = select(&source, ErRelationshipRequest::adjacent(vec![test_table("orders")], 50));

        assert_eq!(roles(&selection), vec!["buyer", "shipper"]);
    }

    #[test]
    fn direction_separates_incoming_from_outgoing() {
        let mut source = source();
        source.logical = vec![
            confirmed_logical("orders", "customers", "out", &[("customer_id", "id")]),
            confirmed_logical("items", "orders", "in", &[("order_id", "id")]),
        ];
        let orders = vec![test_table("orders")];

        let outgoing = select(
            &source,
            ErRelationshipRequest {
                incoming: false,
                ..ErRelationshipRequest::adjacent(orders.clone(), 50)
            },
        );
        let incoming = select(
            &source,
            ErRelationshipRequest {
                incoming: true,
                outgoing: false,
                ..ErRelationshipRequest::adjacent(orders.clone(), 50)
            },
        );

        assert_eq!(roles(&outgoing), vec!["out"]);
        assert_eq!(roles(&incoming), vec!["in"]);
    }

    #[test]
    fn between_only_returns_relationships_inside_the_requested_set() {
        let mut source = source();
        source.logical = vec![
            confirmed_logical("orders", "customers", "buyer", &[("customer_id", "id")]),
            confirmed_logical("items", "orders", "order_of", &[("order_id", "id")]),
            confirmed_logical("regions", "items", "region_items", &[("id", "id")]),
        ];
        let requested = vec![test_table("orders"), test_table("customers")];

        let adjacent = select(&source, ErRelationshipRequest::adjacent(requested.clone(), 50));
        let between = select(&source, ErRelationshipRequest::between(requested, 50));

        assert_eq!(
            roles(&adjacent),
            vec!["order_of", "buyer"],
            "adjacent 取入向 ∪ 出向邻接"
        );
        assert_eq!(
            roles(&between),
            vec!["buyer"],
            "between 只保留两端都在请求集合内的关系"
        );
    }

    #[test]
    fn unconfirmed_rejected_and_stale_are_distinguished_by_reason_codes() {
        let mut proposed = confirmed_logical("orders", "customers", "buyer", &[("customer_id", "id")]);
        proposed.review.state = ErReviewState::Proposed;
        proposed.review.confirmed_revision = None;
        let mut rejected = confirmed_logical("orders", "items", "item_of", &[("id", "id")]);
        rejected.review.state = ErReviewState::Rejected;
        let mut stale = confirmed_logical("orders", "items", "stale_link", &[("id", "id")]);
        stale.validity.state = fluxdb_core::ErValidityState::Stale;
        let mut source = source();
        source.logical = vec![proposed, rejected, stale];

        let default_view = select(&source, ErRelationshipRequest::adjacent(vec![test_table("orders")], 50));
        assert!(
            default_view.entries.is_empty(),
            "默认只返回可用关系，未确认/已拒绝/失效都不返回"
        );

        let with_unusable = select(
            &source,
            ErRelationshipRequest {
                include_unavailable: true,
                ..ErRelationshipRequest::adjacent(vec![test_table("orders")], 50)
            },
        );
        let reasons: BTreeMap<String, Vec<String>> = with_unusable
            .entries
            .iter()
            .map(|entry| (entry.relationship.role.clone(), entry.usage.reason_codes.clone()))
            .collect();
        assert_eq!(reasons.get("buyer").map(Vec::as_slice), Some(["confirmation_required".to_string()].as_slice()));
        assert_eq!(reasons.get("item_of").map(Vec::as_slice), Some(["rejected".to_string()].as_slice()));
        assert!(
            reasons
                .get("stale_link")
                .is_some_and(|codes| codes.iter().any(|code| code == "stale")),
            "{reasons:?}：结构失效的原因码要与「未确认」「已拒绝」区分"
        );
    }

    #[test]
    fn unloaded_endpoints_are_not_reported_as_no_relationship() {
        let mut source = FakeErSource::new()
            .with_column("orders", vec![test_column("customer_id", "bigint")]);
        source.logical = vec![confirmed_logical("orders", "customers", "buyer", &[("customer_id", "id")])];

        let selection = select(
            &source,
            ErRelationshipRequest {
                include_unavailable: true,
                ..ErRelationshipRequest::adjacent(vec![test_table("orders")], 50)
            },
        );

        let entry = &selection.entries[0];
        assert!(!entry.usage.join_candidate);
        assert!(
            entry.usage.reason_codes.contains(&"incomplete_definition".to_string()),
            "结构未加载必须表达为「未判定」，而不是「没有关系」"
        );
        assert_eq!(
            selection.columns.status.get(&fluxdb_core::er_entity_id(&test_table("customers"))),
            Some(&ErLoadStatus::NotLoaded)
        );
    }

    #[test]
    fn incomparable_pair_types_block_usage() {
        let mut source = FakeErSource::new()
            .with_column("orders", vec![test_column("customer_id", "bigint")])
            .with_column("customers", vec![test_column("id", "varchar")]);
        source.logical = vec![confirmed_logical("orders", "customers", "buyer", &[("customer_id", "id")])];

        let selection = select(&source, ErRelationshipRequest::adjacent(vec![test_table("orders")], 50));

        assert!(
            selection.entries.is_empty(),
            "类型不可比较的关系不得作为可用候选返回"
        );
    }

    #[test]
    fn foreign_key_load_failure_is_surfaced_instead_of_empty_catalog() {
        let mut source = source();
        source.foreign_error = true;
        let scope = test_scope(&er_test_connection());

        let error = er_merged_relationships(&scope, &source).expect_err("读取失败必须上报");

        assert_eq!(error.code, "metadata_error");
    }

    #[test]
    fn pending_foreign_key_load_is_reported_as_loading() {
        let mut source = source();
        source.foreign_status = ErLoadStatus::Loading;
        let scope = test_scope(&er_test_connection());

        let (_, status, _) = er_merged_relationships(&scope, &source).expect("汇合关系目录");

        assert_eq!(status, ErLoadStatus::Loading);
    }

    #[test]
    fn scope_rejects_tables_outside_target() {
        let resolver = test_resolver();
        let target = AiSqlTarget {
            connection_id: ConnectionId(1),
            database: Some("sales".into()),
            schema: None,
        };
        let outside = serde_json::json!({ "database": "other", "name": "orders" });

        let error = er_relationship_scope(resolver.as_ref(), &target, &[outside]).expect_err("越界必须拒绝");

        assert_eq!(error.code, "scope_mismatch");
    }

    #[test]
    fn object_white_list_denies_unauthorized_table() {
        let resolver = test_resolver_with_scope(&["orders"]);
        let target = AiSqlTarget {
            connection_id: ConnectionId(1),
            database: Some("sales".into()),
            schema: None,
        };
        let denied = serde_json::json!({ "name": "payroll" });

        let error = er_relationship_scope(resolver.as_ref(), &target, &[denied]).expect_err("白名单外必须拒绝");

        assert_eq!(error.code, "object_not_allowed");
    }

    #[test]
    fn unknown_connection_is_rejected() {
        let resolver = test_resolver();
        let target = AiSqlTarget {
            connection_id: ConnectionId(99),
            database: Some("sales".into()),
            schema: None,
        };

        let error = er_relationship_scope(resolver.as_ref(), &target, &[serde_json::json!({ "name": "orders" })])
            .expect_err("未授权连接必须拒绝");

        assert_eq!(error.code, "connection_not_found");
    }
}

