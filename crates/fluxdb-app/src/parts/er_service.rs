// ER 关系图「画布原型」的加载编排（er-design.md §8 步骤 3）。
//
// 提供后台加载整库 ER 图的 free 函数：一次编排读全库表 / 列 / 外键，
// 返回纯数据 ErGraphData，供 desktop 布局与绘制消费。不依赖控制器 `&self`，
// 因此可由独立线程执行而不阻塞 UI（与 completion_refresh 的
// refresh_index_columns_in_background 同款模式）。
//
// 分层：本文件只做数据库读取编排与纯数据组装，不含任何绘图/SQL 拼接。
//
// 注意：本文件被 include! 进 lib.rs，运行在 crate root scope，
// 其依赖的名字（ConnectionConfig/DatabaseKind/Er* /ObjectPath/connector_for 等）
// 已在 lib.rs 顶部 use 导入，这里不得再重复 use，否则触发 E0252 重复定义。

/// 后台加载整库 ER 图：表（含列、主键、可空、注释）+ 外键连线。
///
/// `database`：物理库名（MySQL/PG 必备）。`schema`：非 None 时（PG）限定单 schema；
/// 为 None 时对 MySQL/SQLite 视为整库、对 PG 则遍历该库全部 schema。
///
/// 返回值 `ErGraphData` 表节点排序稳定（按展示名）。外键每一端表名均经过与
/// 节点名一致的命名规则（PG 拼 `schema.table`，其余裸名），保证连线能对齐节点。
pub fn load_er_graph_in_background(
    config: &ConnectionConfig,
    database: Option<&str>,
    schema: Option<&str>,
) -> fluxdb_core::Result<ErGraphData> {
    let connector = connector_for(config)?;
    let database = match database {
        Some(db) => db.to_string(),
        None => return Ok(ErGraphData::default()),
    };

    // 1) 枚举全部表（仿 backup_restore/scope.rs 的层级遍历；PG 多 schema，其余单 database）。
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
            // 限定单 schema 时只取该 schema；否则遍历全部。
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
    let table_set = summaries
        .into_iter()
        .filter(|s| matches!(s.path.kind, ObjectKind::Table))
        .collect::<Vec<_>>();

    // 节点名。PG 多 schema 时带 schema 前缀区分同名表；其余方言裸名即可。
    let table_name = |kind: DatabaseKind, path: &ObjectPath| -> String {
        if kind == DatabaseKind::Postgres {
            match (&path.schema, path.name.as_str()) {
                (Some(s), name) => format!("{s}.{name}"),
                (None, name) => name.to_string(),
            }
        } else {
            path.name.clone()
        }
    };

    let mut tables: Vec<ErTableNode> = Vec::with_capacity(table_set.len());
    let mut table_paths: Vec<ObjectPath> = Vec::with_capacity(table_set.len());
    let mut table_names: Vec<String> = Vec::with_capacity(table_set.len());
    for summary in &table_set {
        tables.push(ErTableNode {
            name: table_name(config.kind, &summary.path),
            comment: summary.comment.clone(),
            columns: Vec::new(),
        });
        table_paths.push(summary.path.clone());
        table_names.push(summary.path.name.clone());
    }

    // 2) 批量读列（PG/MySQL 单查询，SQLite 底层按表循环），按表分组。
    if !table_names.is_empty() {
        let columns = connector.list_completion_columns_for_tables(
            Some(&database),
            schema,
            &table_names,
        )?;
        // 用裸表名+可选 schema 定位列所属节点。
        for col in columns {
            let display_name = if config.kind == DatabaseKind::Postgres {
                match (&col.schema, col.table.as_str()) {
                    (Some(s), name) => format!("{s}.{name}"),
                    (None, name) => name.to_string(),
                }
            } else {
                col.table.clone()
            };
            if let Some(node) = tables.iter_mut().find(|n| n.name == display_name) {
                node.columns.push(ErColumn {
                    name: col.name,
                    type_name: col.type_name,
                    primary_key: col.primary_key,
                    nullable: col.nullable,
                });
            }
        }
    }

    // 3) 读取外键，转成两端表名对齐节点的连线。
    // MySQL/SQLite：批量接口一次返回 (源表名, 外键)，避免大库 N+1；
    // PG 保留逐表（需 schema 拼节点名，批量默认实现会丢 schema 身份）。
    let mut edges = Vec::new();
    if config.kind == DatabaseKind::Postgres {
        for path in &table_paths {
            for fk in connector.list_foreign_keys(path)? {
                edges.push(ErForeignKeyEdge {
                    name: fk.name,
                    from_table: table_name(config.kind, path),
                    from_column: fk.column,
                    to_table: {
                        // 被引用端表名按同规则落到节点名；PG 无 schema 时补 source schema。
                        match fk.ref_schema {
                            Some(s) => format!("{s}.{}", fk.ref_table),
                            None => {
                                let s = path.schema.clone().unwrap_or_else(|| "public".into());
                                format!("{s}.{}", fk.ref_table)
                            }
                        }
                    },
                    to_column: fk.ref_column,
                });
            }
        }
    } else {
        // MySQL 真批量一次查全库外键；SQLite 走默认逐表实现（本地快，可接受）。
        let batch = connector.list_foreign_keys_for_tables(Some(&database), None, &table_names)?;
        for (src_table, fk) in batch {
            // MySQL/SQLite 节点、被引用表名均为裸名，与批量返回一致。
            edges.push(ErForeignKeyEdge {
                name: fk.name,
                from_table: src_table,
                from_column: fk.column,
                to_table: fk.ref_table,
                to_column: fk.ref_column,
            });
        }
    }

    // 4) 排序稳定（按节点名），保证每次加载结果一致。
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    edges.sort_by(|a, b| {
        (&a.from_table, &a.from_column, &a.to_table, &a.to_column)
            .cmp(&(&b.from_table, &b.from_column, &b.to_table, &b.to_column))
    });

    Ok(ErGraphData { tables, edges })
}
