// PostgreSQL 对象树元数据（T06）：真实 pg_catalog 路由，PG 不返回 mock 数据。
//
// 导航模型（app 按「父路径的 kind」分发，connector 决定该层返回什么子项）：
// - `path=None`            → 数据库列表（pg_database，过滤模板/内部库，按 CONNECT 权限可见）；
// - `kind=Database`        → 该库内的 schema 列表（pg_namespace，过滤系统 schema）；
// - `kind=Schema`          → 该 schema 内的表/分区表/外部表/视图/物化视图（pg_class，relkind 过滤）。
// 「同名对象」由 schema 限定天然区分：ObjectPath 用 database+schema+name 三维定位。
// 元数据查询在「目标库的新连接」上执行、用后即弃；不进入会话注册表，
// 保证刷新/断开取舍干净（不残留影响对象树的会话状态）。

/// 对象树入口：按 `path` 分发到数据库/schema/关系列表。
fn pg_list_objects(
    config: &ConnectionConfig,
    path: Option<&ObjectPath>,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    if config.kind != DatabaseKind::Postgres {
        return Err(Error::new(ErrorKind::Connection, "连接类型不匹配"));
    }
    match path {
        None => pg_list_databases(config),
        Some(path) if path.kind == ObjectKind::Database => pg_list_schemas(config, path),
        Some(path) if path.kind == ObjectKind::Schema => pg_list_relations(config, path),
        Some(_) => Err(Error::new(
            ErrorKind::Internal,
            "不支持的 PostgreSQL 对象层级",
        )),
    }
}

/// 数据库列表：维护库建连，按 CONNECT 权限返回用户可见的非模板库。
///
/// 档案 `scope.show_other_databases` 控制可见范围：关闭时仅返回维护库（本库），
/// 打开时返回所有用户可 CONNECT 的非模板库。
fn pg_list_databases(config: &ConnectionConfig) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let show_others = config
        .postgres_profile
        .as_ref()
        .map(|p| p.scope.show_other_databases)
        .unwrap_or(false);
    // 折叠为"是否只显示本库"：维护库为空（用户留空「数据库」）时没有单一"本库"可展示，
    // 一律列出全部可 CONNECT 库，而不是回落 postgres 只显示一库。
    let database = pg_request_database(config, None);
    let profile_db_empty = match &config.postgres_profile {
        Some(p) => p.basic.maintenance_database.trim().is_empty(),
        None => true,
    };
    let maintain_only = !profile_db_empty && !show_others;
    if maintain_only {
        return pg_runtime().block_on(async {
            let _session = pg_connect(config, &database).await?;
            Ok(vec![database_object(config.id, &database)])
        });
    }
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        let rows = session
            .client
            .query(
                "SELECT d.datname FROM pg_catalog.pg_database d \
                 WHERE d.datistemplate = false AND d.datallowconn \
                   AND pg_catalog.has_database_privilege(d.datname, 'CONNECT') \
                 ORDER BY d.datname",
                &[],
            )
            .await
            .map_err(pg_error)?;
        let mut objects = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.get(0);
            objects.push(database_object(config.id, &name));
        }
        Ok(objects)
    })
}

/// schema 列表：连接目标库，默认过滤系统 schema（`pg_*` 前缀与 information_schema）。
///
/// 档案 `scope.show_system_schemas` 打开时一并显示系统 schema。
fn pg_list_schemas(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let show_system = config
        .postgres_profile
        .as_ref()
        .map(|p| p.scope.show_system_schemas)
        .unwrap_or(false);
    let database = path
        .database
        .clone()
        .unwrap_or_else(|| path.name.clone());
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        // 关闭时过滤系统 schema（默认行为）；打开时不加过滤条件，全部 schema 可见。
        let rows = if show_system {
            session
                .client
                .query(
                    "SELECT n.nspname FROM pg_catalog.pg_namespace n \
                     ORDER BY n.nspname",
                    &[],
                )
                .await
                .map_err(pg_error)?
        } else {
            session
                .client
                .query(
                    "SELECT n.nspname FROM pg_catalog.pg_namespace n \
                     WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                     ORDER BY n.nspname",
                    &[],
                )
                .await
                .map_err(pg_error)?
        };
        let mut objects = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.get(0);
            objects.push(ObjectSummary {
                path: ObjectPath {
                    connection_id: config.id,
                    database: Some(database.clone()),
                    schema: None,
                    name: name.clone(),
                    kind: ObjectKind::Schema,
                },
                rows: None,
                modified_at: None,
                comment: None,
            });
        }
        Ok(objects)
    })
}

/// 关系列表：连接目标库，读指定 schema 内的表/分区表/外部表/视图/物化视图与行数估计。
///
/// relkind 映射：`r`普通表/`f`外部表 → Table；`p`分区表 → Table；
/// `v`视图/`m`物化视图 → View（模型无物化/分区专属 kind，统一归入 Table/View 树形仍可区分）。
fn pg_list_relations(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ObjectSummary>> {
    let database = path.database.clone().unwrap_or_else(|| {
        pg_request_database(config, None)
    });
    let schema_name = path
        .name
        .clone();
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        let rows = session
            .client
            .query(
                "SELECT c.relname, c.relkind::text, c.reltuples::bigint, \
                        pg_catalog.obj_description(c.oid, 'pg_class') AS comment \
                 FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relkind IN ('r','p','f','v','m') \
                 ORDER BY c.relname",
                &[&schema_name],
            )
            .await
            .map_err(pg_error)?;
        let mut objects = Vec::with_capacity(rows.len());
        for row in rows {
            let name: String = row.get(0);
            let relkind: String = row.get(1);
            let reltuples: i64 = row.get(2);
            let comment: Option<String> = row.get(3);
            let kind = if matches!(relkind.as_str(), "v" | "m") {
                ObjectKind::View
            } else {
                ObjectKind::Table
            };
            objects.push(ObjectSummary {
                path: ObjectPath {
                    connection_id: config.id,
                    database: Some(database.clone()),
                    schema: Some(path.name.clone()),
                    name,
                    kind,
                },
                // reltuples 是规划器估计值（ANALYZE 后才准），仅作树形行数提示。
                rows: (reltuples >= 0).then_some(reltuples as u64),
                modified_at: None,
                comment: comment.filter(|comment| !comment.trim().is_empty()),
            });
        }
        Ok(objects)
    })
}
