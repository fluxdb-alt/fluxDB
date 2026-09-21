// ER 关系图「分阶段加载」编排（er-design.md §3.4/§4.2）。
//
// 本文件只做数据库元数据读取与纯数据组装，返回纯数据 ErTableNode / ErForeignKeyEdge，
// 供 desktop 布局与绘制消费。字段/关系缓存的并发编排在 controller 层的 er_catalog.rs
// （AppController 持有 Arc<Mutex<ErCatalogCache>>），本文件是其依赖的底层读取函数。
//
// 分层：本文件不拼 SQL（连接器负责）、不持有任何线程共享缓存、不含绘图逻辑。
// 后台执行原语（background_spawn）在 desktop 侧，与 completion_refresh 同模式。
//
// 注意：本文件被 include! 进 lib.rs，运行在 crate root scope，
// 其依赖的名字（ConnectionConfig/DatabaseKind/ObjectPath/connector_for 等）
// 已在 lib.rs 顶部 use 导入，这里不得再重复 use，否则触发 E0252 重复定义。

/// 阶段 1：只读取表目录（含注释，不含字段/关系），每表字段状态为 NotLoaded。
///
/// 表目录读到即可先展示节点（固定槽位稳定网格），不等待字段与外键。
/// `database`：物理库名（MySQL/PG 必备）。`schema`：非 None（PG）限定单 schema；
/// 为 None 时对 MySQL/SQLite 视为整库、对 PG 遍历该库全部 schema。
/// 节点名：PG 多 schema 拼 `schema.table`，其余方言裸名（与连线对齐命名）。
pub fn load_er_tables_in_background(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
) -> fluxdb_core::Result<Vec<ErTableNode>> {
    let connector = connector_for(config)?;
    let database = match database {
        Some(db) => db.to_string(),
        None => return Ok(Vec::new()),
    };

    // 枚举全部表（仿 backup_restore/scope.rs 的层级遍历；PG 多 schema，其余单 database）。
    let root = ObjectPath {
        connection_id: config.id,
        database: Some(database.clone()),
        schema: None,
        name: String::new(),
        kind: ObjectKind::Schema,
    };
    let summaries = if config.kind == DatabaseKind::Postgres {
        let mut all = Vec::new();
        let database_path = ObjectPath {
            kind: ObjectKind::Database,
            ..root.clone()
        };
        for schema_summary in connector.list_objects(Some(&database_path))? {
            if let Some(selected) = schema
                && schema_summary.path.name != selected
            {
                continue;
            }
            all.extend(connector.list_objects(Some(&schema_summary.path))?);
        }
        all
    } else {
        connector.list_objects(Some(&root))?
    };

    // 只取表；视图首版不入 ER 画布（避免与真实表混淆）。
    let mut tables: Vec<ErTableNode> = Vec::new();
    let db = database.clone();
    for summary in summaries.into_iter().filter(|s| matches!(s.path.kind, ObjectKind::Table)) {
        // 结构化身份：schema + 裸名显式成对保存（含点标识符也安全），展示名由身份生成。
        let reference = ErTableRef {
            database: db.clone(),
            schema: summary.path.schema.clone(),
            name: summary.path.name.clone(),
        };
        let name = reference.display();
        tables.push(ErTableNode {
            name,
            reference,
            comment: summary.comment.clone(),
            // 字段尚未读取：空列表不代表「没有字段」。
            status: ErLoadStatus::NotLoaded,
            columns: Vec::new(),
        });
    }
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tables)
}

/// 阶段 3 底层读取：读取整库外键关系，转成两端表名对齐节点的连线。
/// `database`/`schema` 限定范围与 `load_er_tables_in_background` 一致。
///
/// MySQL/SQLite：批量接口一次返回 (源表名, 外键)；PG 保留逐表（需 schema 拼节点名，
/// 批量默认实现会丢 schema 身份）。调用方负责把结果缓存进关系索引以复用
/// （见 controller 层 er_catalog.rs），本函数只做一次 DB 读取。
pub fn load_er_relations_from_db(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
) -> fluxdb_core::Result<Vec<ErForeignKeyEdge>> {
    let connector = connector_for(config)?;
    load_er_relations_from_db_with(connector.as_ref(), config, database, schema)
}

/// 用注入 connector 读取整库关系边（可测试性：配合假连接器观察实际请求）。
pub fn load_er_relations_from_db_with(
    connector: &dyn Connector,
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
) -> fluxdb_core::Result<Vec<ErForeignKeyEdge>> {
    let database = match database {
        Some(db) => db.to_string(),
        None => return Ok(Vec::new()),
    };

    // 需要逐表读取外键的方言（PG）要有表路径/表名；先枚举一次以对齐节点名。
    let mut table_paths: Vec<ObjectPath> = Vec::new();
    let mut table_names: Vec<String> = Vec::new();
    if config.kind == DatabaseKind::Postgres {
        let root = ObjectPath {
            connection_id: config.id,
            database: Some(database.clone()),
            schema: None,
            name: String::new(),
            kind: ObjectKind::Schema,
        };
        let database_path = ObjectPath {
            kind: ObjectKind::Database,
            ..root.clone()
        };
        for schema_summary in connector.list_objects(Some(&database_path))? {
            if let Some(selected) = schema
                && schema_summary.path.name != selected
            {
                continue;
            }
            for summary in connector.list_objects(Some(&schema_summary.path))? {
                if matches!(summary.path.kind, ObjectKind::Table) {
                    table_paths.push(summary.path.clone());
                    table_names.push(summary.path.name.clone());
                }
            }
        }
    } else {
        let root = ObjectPath {
            connection_id: config.id,
            database: Some(database.clone()),
            schema: None,
            name: String::new(),
            kind: ObjectKind::Schema,
        };
        for summary in connector.list_objects(Some(&root))? {
            if matches!(summary.path.kind, ObjectKind::Table) {
                table_names.push(summary.path.name.clone());
            }
        }
    }

    let mut edges = Vec::new();
    if config.kind == DatabaseKind::Postgres {
        for path in &table_paths {
            // 本端结构化身份：所在 schema（未声明时按 pg default public）+ 裸名。
            let from_schema = path
                .schema
                .clone()
                .unwrap_or_else(|| "public".to_string());
            for fk in connector.list_foreign_keys(path)? {
                // 被引用端 schema：FK 元数据带则用之，否则继承本端 schema（同 schema 引用的常见情形）。
                let to_schema = fk
                    .ref_schema
                    .clone()
                    .unwrap_or_else(|| from_schema.clone());
                let from_ref = ErTableRef {
                    database: database.clone(),
                    schema: Some(from_schema.clone()),
                    name: path.name.clone(),
                };
                let to_ref = ErTableRef {
                    database: database.clone(),
                    schema: Some(to_schema.clone()),
                    name: fk.ref_table.clone(),
                };
                let fname = fk.name.clone();
                let from_display = from_ref.display();
                let to_display = to_ref.display();
                // 复合外键：按有序列对逐列拆成边（保留约束名作可靠身份，§6.2）。
                // 不在 PG 拼接成不存在的单列“a, b”，只在有序列对非空时拆分，否则单列回退。
                let pairs = er_expand_fk_columns(&fk.columns, &fk.ref_columns, &fk.column, &fk.ref_column);
                for (src_col, dst_col) in pairs {
                    edges.push(ErForeignKeyEdge {
                        name: fname.clone(),
                        from_table: from_display.clone(),
                        from_column: src_col,
                        to_table: to_display.clone(),
                        to_column: dst_col,
                        from_reference: from_ref.clone(),
                        to_reference: to_ref.clone(),
                    });
                }
            }
        }
    } else {
        let batch = connector.list_foreign_keys_for_tables(Some(&database), None, &table_names)?;
        for (src_table, fk) in batch {
            let from_ref = ErTableRef {
                database: database.clone(),
                schema: schema.map(str::to_string),
                name: src_table.clone(),
            };
            let to_ref = ErTableRef {
                database: database.clone(),
                schema: schema.map(str::to_string),
                name: fk.ref_table.clone(),
            };
            edges.push(ErForeignKeyEdge {
                name: fk.name,
                from_table: from_ref.display(),
                from_column: fk.column,
                to_table: to_ref.display(),
                to_column: fk.ref_column,
                from_reference: from_ref,
                to_reference: to_ref,
            });
        }
    }
    edges.sort_by(|a, b| {
        (&a.from_reference, &a.from_column, &a.to_reference, &a.to_column)
            .cmp(&(&b.from_reference, &b.from_column, &b.to_reference, &b.to_column))
    });
    Ok(edges)
}

/// 基于关系索引计算「当前表关联 ER」的包含表集合（纯函数，供多视图复用缓存）。
///
/// 以 `center` 为中心沿外键（入向/出向）扩 `depth` 跳；`extra` 为单节点式展开显式
/// 加入的种子（与 center 同等作为起点各扩 depth 跳）。邻域判定只依赖关系边集合，
/// 不读取任何字段，因此可复用共享关系索引（er_catalog.rs）而不用每次全库读表。
/// 匹配一律按结构化身份（ErTableRef）进行，正确处理跨 schema 同名表与含点标识符，
/// 不遗漏跨 schema 入向/出向邻居。返回包含表的展示名集合（与节点 name 对齐）。
pub fn er_neighborhood_included_tables(
    center: &ErTableRef,
    depth: u8,
    extra: &BTreeSet<ErTableRef>,
    edges: &[ErForeignKeyEdge],
) -> BTreeSet<String> {
    let mut included: BTreeSet<ErTableRef> = extra.clone();
    included.insert(center.clone());
    let mut frontier: Vec<ErTableRef> = included.iter().cloned().collect();
    for _ in 0..depth {
        let mut next: Vec<ErTableRef> = Vec::new();
        for t in &frontier {
            for edge in edges {
                let neighbor = if edge.from_reference == *t {
                    Some(edge.to_reference.clone())
                } else if edge.to_reference == *t {
                    Some(edge.from_reference.clone())
                } else {
                    None
                };
                if let Some(n) = neighbor
                    && !included.contains(&n)
                {
                    included.insert(n.clone());
                    next.push(n);
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    included.into_iter().map(|r| r.display()).collect()
}

/// 兼容入口：一次读取整库表+关系（表字段状态为 NotLoaded），供既有测试/整库首帧缓存。
///
/// 新路径按阶段走 load_er_tables_in_background + 按需字段 + 关系索引缓存；
/// 本函数保留用于批量构造整库图的纯读取（不含字段）。
/// 一次性读取指定（裸）表字段（供兼容整库入口 / 备用）。返回按 (display 名 -> 列) 分组。
fn load_er_columns_for_tables_from_db(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    bare_tables: &[String],
) -> fluxdb_core::Result<BTreeMap<String, Vec<ErColumn>>> {
    if bare_tables.is_empty() {
        return Ok(BTreeMap::new());
    }
    let connector = connector_for(config)?;
    let columns = connector.list_completion_columns_for_tables_with_cancel(
        database,
        schema,
        bare_tables,
        &|| false,
    )?;
    // 按 display 名（与 ErTableNode.name 对齐：PG schema.table，其余裸名）分组列。
    let mut by_display: BTreeMap<String, Vec<ErColumn>> = BTreeMap::new();
    for col in columns {
        let display = if config.kind == DatabaseKind::Postgres {
            match (&col.schema, col.table.as_str()) {
                (Some(s), name) => format!("{s}.{name}"),
                (None, name) => name.to_string(),
            }
        } else {
            col.table.clone()
        };
        by_display.entry(display).or_default().push(ErColumn {
            name: col.name,
            type_name: col.type_name,
            primary_key: col.primary_key,
            nullable: col.nullable,
        });
    }
    Ok(by_display)
}

/// 兼容入口：一次读取整库表 + 字段 + 关系（供既有测试/整库首帧缓存 / 备用）。
///
/// 新路径按阶段走 load_er_tables_in_background + 按需字段 + 关系索引缓存；
/// 本函数保留为「一次性全量读取（含字段）」的等价物，验证真实库端到端链路。
pub fn load_er_graph_in_background(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
) -> fluxdb_core::Result<ErGraphData> {
    let mut tables = load_er_tables_in_background(config, database, schema)?;
    // 查询用裸表名（PG 展示名 `schema.table` 拆出裸名；其余即裸名）。
    let bare_query: Vec<String> = if config.kind == DatabaseKind::Postgres {
        tables
            .iter()
            .map(|t| {
                t.name
                    .rsplit_once('.')
                    .map(|(_, b)| b.to_string())
                    .unwrap_or_else(|| t.name.clone())
            })
            .collect()
    } else {
        tables.iter().map(|t| t.name.clone()).collect()
    };
    let by_display = load_er_columns_for_tables_from_db(config, database, schema, &bare_query)?;
    for table in tables.iter_mut() {
        // 空字段也是「已读」（无该表列 ≠ 未读）。
        table.status = ErLoadStatus::Loaded;
        table.columns = by_display.get(&table.name).cloned().unwrap_or_default();
    }
    let edges = load_er_relations_from_db(config, database, schema)?;
    Ok(ErGraphData {
        tables,
        edges,
        relation_status: ErLoadStatus::Loaded,
    })
}

/// 兼容入口：以 `center` 展示名（或裸名）为中心 `depth` 跳邻域子图 + 显式 `extra` 种子。
///
/// 旧版直接全库读取再过滤；新路径改用共享关系索引缓存（er_catalog.rs）。本函数
/// 保留给既有测试使用，语义不变（含表集由 er_neighborhood_included_tables 计算）。
/// `center`/`extra` 接受展示名（PG `schema.table`）或裸名：按展示名精确匹配到结构化身份，
/// 裸名在无可疑（单 schema）场景下按同 schema/继承前缀解析，避免跨 schema 误匹配。
pub fn load_er_neighborhood_in_background(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
    center: &str,
    depth: u8,
    extra: &BTreeSet<String>,
) -> fluxdb_core::Result<ErGraphData> {
    let full = load_er_graph_in_background(config, database, schema)?;
    // 展示名 → 结构化身份解析表（唯一）。
    let display_to_ref: std::collections::HashMap<&str, &ErTableRef> = full
        .tables
        .iter()
        .map(|t| (t.name.as_str(), &t.reference))
        .collect();
    let resolve = |display: &str| -> Option<ErTableRef> {
        display_to_ref
            .get(display)
            .map(|r| (*r).clone())
            .or_else(|| {
                // 裸名（无 schema 前缀）：限定当前 schema（或 PG 缺省 public）内匹配。
                let wanted = ErTableRef {
                    database: database.unwrap_or("").to_string(),
                    schema: schema.map(str::to_string),
                    name: display.to_string(),
                };
                // 让 wanted 的 schema 参与实例化查找：按 (schema, name) 精确匹配。
                full.tables
                    .iter()
                    .find(|t| t.reference.schema == wanted.schema && t.reference.name == display)
                    .map(|t| t.reference.clone())
            })
    };
    let center_ref = resolve(center).unwrap_or(ErTableRef {
        database: database.unwrap_or("").to_string(),
        schema: schema.map(str::to_string),
        name: center.to_string(),
    });
    let extra_refs: BTreeSet<ErTableRef> = extra.iter().filter_map(|s| resolve(s)).collect();
    let included = er_neighborhood_included_tables(&center_ref, depth, &extra_refs, &full.edges);
    Ok(ErGraphData {
        tables: full
            .tables
            .into_iter()
            .filter(|t| included.contains(&t.name))
            .collect(),
        edges: full
            .edges
            .into_iter()
            .filter(|e| included.contains(&e.from_table) && included.contains(&e.to_table))
            .collect(),
        relation_status: ErLoadStatus::Loaded,
    })
}

/// 把外键的有序列对展开为 (源列, 目标列) 对（§6.2 复合约束身份）。
///
/// - 有序列对（`columns`/`ref_columns`）非空且长度一致时按 `zip` 逐列展开，保持数据库原始序号：
///   复合外键不会被拼接成不存在的单列“a, b”，而是按每列生成一条边（复用同一约束名作身份，
///   供同表对多条约束区分）。
/// - 否则单列回退（用 `column`/`ref_column`）。
pub fn er_expand_fk_columns(
    columns: &[String],
    ref_columns: &[String],
    column: &str,
    ref_column: &str,
) -> Vec<(String, String)> {
    if !columns.is_empty() && !ref_columns.is_empty() && columns.len() == ref_columns.len() {
        columns
            .iter()
            .cloned()
            .zip(ref_columns.iter().cloned())
            .collect()
    } else {
        vec![(column.to_string(), ref_column.to_string())]
    }
}

#[cfg(test)]
mod er_service_tests {
    use super::er_expand_fk_columns;

    #[test]
    fn composite_fk_expands_ordered_pairs_not_joined_single() {
        // 复合外键：按有序列逐列展开，不逗号拼接成单列（PG 旧行为会在 ER 连线里指向不存在的列）。
        let pairs = er_expand_fk_columns(
            &["tenant_id".into(), "customer_id".into()],
            &["tenant_id".into(), "id".into()],
            "tenant_id",
            "tenant_id",
        );
        assert_eq!(
            pairs,
            vec![
                ("tenant_id".to_string(), "tenant_id".to_string()),
                ("customer_id".to_string(), "id".to_string()),
            ],
            "复合外键须按原始序号逐列展开"
        );
    }

    #[test]
    fn single_and_mismatched_fk_fall_back() {
        // 单列：直接一列。
        assert_eq!(
            er_expand_fk_columns(&[], &[], "product_id", "id"),
            vec![("product_id".to_string(), "id".to_string())]
        );
        // 列数不匹配：不 zip（避免错位），回退单列。
        assert_eq!(
            er_expand_fk_columns(&["a".into(), "b".into()], &["x".into()], "a", "x"),
            vec![("a".to_string(), "x".to_string())]
        );
    }
}
