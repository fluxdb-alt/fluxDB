fn mysql_qualified_table(database: &str, table: &str) -> String {
    format!(
        "{}.{}",
        fluxdb_core::MySqlDialect.quote_identifier(database),
        fluxdb_core::MySqlDialect.quote_identifier(table)
    )
}

fn mysql_indexes(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<Vec<IndexInfo>> {
    let database = mysql_database_for_path(config, path)?;
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let has_expression = mysql_statistics_has_expression(&mut connection).await;
        let rows = sqlx::query(mysql_indexes_query(has_expression))
            .bind(&database)
            .bind(&path.name)
            .fetch_all(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;
        mysql_indexes_from_rows(rows).map(|(indexes, _)| indexes)
    })
}

/// 索引行 → `IndexInfo` + 结构同步所需的附加属性。
///
/// `column_name` 在函数索引行为 NULL（键项是表达式），此时用 `expression` 文本占位，
/// 并在附加属性里标记 `has_expression`，结构同步会跳过这类索引。
fn mysql_indexes_from_rows(
    rows: Vec<MySqlRow>,
) -> fluxdb_core::Result<(Vec<IndexInfo>, BTreeMap<String, fluxdb_core::IndexAttributes>)> {
    let mut indexes: BTreeMap<String, IndexInfo> = BTreeMap::new();
    let mut attributes: BTreeMap<String, fluxdb_core::IndexAttributes> = BTreeMap::new();
    for row in rows {
        let name: String = row.try_get("index_name").map_err(mysql_error)?;
        let column: Option<String> = row.try_get("column_name").map_err(mysql_error)?;
        let expression: Option<String> = row.try_get("expression").map_err(mysql_error)?;
        let non_unique: i64 = row.try_get("non_unique").map_err(mysql_error)?;
        let index_type: String = row.try_get("index_type").map_err(mysql_error)?;
        let comment: String = row.try_get("index_comment").map_err(mysql_error)?;
        let attrs = attributes
            .entry(name.clone())
            .or_insert_with(|| fluxdb_core::IndexAttributes {
                index_type: Some(index_type).filter(|value| !value.is_empty()),
                predicate: None,
                has_expression: false,
            });
        let attrs = attrs.clone();
        let entry = indexes.entry(name.clone()).or_insert_with(|| IndexInfo {
            is_primary: name == "PRIMARY",
            name,
            columns: Vec::new(),
            is_unique: non_unique == 0,
            index_type: attrs.index_type.clone(),
            comment: Some(comment).filter(|value| !value.is_empty()),
        });
        match column {
            Some(column) => entry.columns.push(column),
            None => {
                if let Some(attrs) = attributes.get_mut(&entry.name) {
                    attrs.has_expression = true;
                }
                if let Some(expression) = expression.filter(|value| !value.is_empty()) {
                    entry.columns.push(expression);
                }
            }
        }
    }
    Ok((indexes.into_values().collect(), attributes))
}

/// MySQL 8.0 才在 `information_schema.statistics` 提供 `EXPRESSION` 列（函数索引）。
/// 检测失败按旧版处理，避免把不支持的列写进查询。
async fn mysql_statistics_has_expression(connection: &mut MySqlConnection) -> bool {
    let result = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM information_schema.columns \
         WHERE table_schema = 'information_schema' \
           AND table_name = 'STATISTICS' AND column_name = 'EXPRESSION'",
    )
    .fetch_one(&mut *connection)
    .await;
    match result {
        Ok(count) => count > 0,
        Err(error) => {
            tracing::debug!(
                target: "fluxdb_connectors",
                op = "mysql_statistics_has_expression",
                error = %error,
                "无法检测 statistics.expression，按 MySQL 5.7 兼容模式读取索引"
            );
            false
        }
    }
}

/// 旧版用 `NULL AS expression` 占位，保持 `mysql_indexes_from_rows` 的行结构不变。
fn mysql_indexes_query(has_expression: bool) -> &'static str {
    if has_expression {
        r#"
    SELECT
        CAST(index_name AS CHAR) AS index_name,
        CAST(column_name AS CHAR) AS column_name,
        CAST(expression AS CHAR) AS expression,
        CAST(non_unique AS SIGNED) AS non_unique,
        CAST(index_type AS CHAR) AS index_type,
        CAST(index_comment AS CHAR) AS index_comment
    FROM information_schema.statistics
    WHERE table_schema = ? AND table_name = ?
    ORDER BY index_name, seq_in_index
    "#
    } else {
        r#"
    SELECT
        CAST(index_name AS CHAR) AS index_name,
        CAST(column_name AS CHAR) AS column_name,
        CAST(NULL AS CHAR) AS expression,
        CAST(non_unique AS SIGNED) AS non_unique,
        CAST(index_type AS CHAR) AS index_type,
        CAST(index_comment AS CHAR) AS index_comment
    FROM information_schema.statistics
    WHERE table_schema = ? AND table_name = ?
    ORDER BY index_name, seq_in_index
    "#
    }
}

fn mysql_foreign_keys(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    mysql_foreign_keys_with_cancel(config, path, &|| false)
}

fn mysql_foreign_keys_with_cancel(
    config: &ConnectionConfig,
    path: &ObjectPath,
    should_cancel: &dyn Fn() -> bool,
) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    let database = mysql_database_for_path(config, path)?;
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let Some(mut connection) = await_with_cancel(plan.connect(), should_cancel)
            .await? else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "connect", "MySQL metadata connection cancelled");
            return Ok(Vec::new());
        };
        let rows = await_with_cancel(sqlx::query(
            r#"
            SELECT
                CAST(constraint_name AS CHAR) AS constraint_name,
                CAST(column_name AS CHAR) AS column_name,
                CAST(referenced_table_schema AS CHAR) AS referenced_table_schema,
                CAST(referenced_table_name AS CHAR) AS referenced_table_name,
                CAST(referenced_column_name AS CHAR) AS referenced_column_name
            FROM information_schema.key_column_usage
            WHERE table_schema = ?
              AND table_name = ?
              AND referenced_table_name IS NOT NULL
            ORDER BY constraint_name, ordinal_position
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection), should_cancel)
        .await
        .map_err(mysql_error)?;
        let Some(rows) = rows else {
            tracing::debug!(target: "fluxdb_connectors", op = "metadata_cancel", kind = "foreign_keys", table = %path.name, "MySQL metadata query cancelled");
            drop(connection);
            return Ok(Vec::new());
        };
        connection.close().await.map_err(mysql_error)?;

        mysql_foreign_keys_from_rows(rows)
    })
}

fn mysql_foreign_keys_from_rows(rows: Vec<MySqlRow>) -> fluxdb_core::Result<Vec<ForeignKeyInfo>> {
    rows.into_iter()
        .map(|row| {
            Ok(ForeignKeyInfo {
                name: row.try_get("constraint_name").map_err(mysql_error)?,
                column: row.try_get("column_name").map_err(mysql_error)?,
                ref_schema: Some(
                    row.try_get::<String, _>("referenced_table_schema")
                        .map_err(mysql_error)?,
                )
                .filter(|value| !value.is_empty()),
                ref_table: row.try_get("referenced_table_name").map_err(mysql_error)?,
                ref_column: row.try_get("referenced_column_name").map_err(mysql_error)?,
                columns: Vec::new(),
                ref_columns: Vec::new(),
            })
        })
        .collect()
}

/// 批量读取整个库的外键，返回 `(源表名, 外键)`；复合键每列占一行。
/// 一次 connection 查全库，替代逐表 N+1，供 ER 等全库关系拓扑场景使用。
/// `tables` 为空时返回全库 FK；非空时仅返回这些表的外键。
fn mysql_foreign_keys_for_tables(
    config: &ConnectionConfig,
    database: &str,
    tables: &[String],
) -> fluxdb_core::Result<Vec<(String, ForeignKeyInfo)>> {
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let rows = if tables.is_empty() {
            sqlx::query(
                r#"
                SELECT
                    CAST(table_name AS CHAR) AS table_name,
                    CAST(constraint_name AS CHAR) AS constraint_name,
                    CAST(column_name AS CHAR) AS column_name,
                    CAST(referenced_table_schema AS CHAR) AS referenced_table_schema,
                    CAST(referenced_table_name AS CHAR) AS referenced_table_name,
                    CAST(referenced_column_name AS CHAR) AS referenced_column_name
                FROM information_schema.key_column_usage
                WHERE table_schema = ?
                  AND referenced_table_name IS NOT NULL
                ORDER BY table_name, constraint_name, ordinal_position
                "#,
            )
            .bind(database)
            .fetch_all(&mut connection)
            .await
            .map_err(mysql_error)?
        } else {
            let mut placeholders = Vec::with_capacity(tables.len());
            let mut values = Vec::with_capacity(tables.len() + 1);
            values.push(database.to_string());
            for _ in tables {
                placeholders.push("?");
            }
            let table_list = placeholders.join(", ");
            let sql = format!(
                "SELECT
                    CAST(table_name AS CHAR) AS table_name,
                    CAST(constraint_name AS CHAR) AS constraint_name,
                    CAST(column_name AS CHAR) AS column_name,
                    CAST(referenced_table_schema AS CHAR) AS referenced_table_schema,
                    CAST(referenced_table_name AS CHAR) AS referenced_table_name,
                    CAST(referenced_column_name AS CHAR) AS referenced_column_name
                 FROM information_schema.key_column_usage
                 WHERE table_schema = ? AND table_name IN ({table_list})
                   AND referenced_table_name IS NOT NULL
                 ORDER BY table_name, constraint_name, ordinal_position"
            );
            let mut query = sqlx::query(&sql);
            for v in values {
                query = query.bind(v);
            }
            for t in tables {
                query = query.bind(t);
            }
            query.fetch_all(&mut connection).await.map_err(mysql_error)?
        };
        connection.close().await.map_err(mysql_error)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push((
                row.try_get::<String, _>("table_name").map_err(mysql_error)?,
                ForeignKeyInfo {
                    name: row.try_get("constraint_name").map_err(mysql_error)?,
                    column: row.try_get("column_name").map_err(mysql_error)?,
                    ref_schema: Some(
                        row.try_get::<String, _>("referenced_table_schema")
                            .map_err(mysql_error)?,
                    )
                    .filter(|value| !value.is_empty()),
                    ref_table: row.try_get("referenced_table_name").map_err(mysql_error)?,
                    ref_column: row.try_get("referenced_column_name").map_err(mysql_error)?,
                    columns: Vec::new(),
                    ref_columns: Vec::new(),
                },
            ));
        }
        Ok(out)
    })
}

fn mysql_triggers(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    let database = mysql_database_for_path(config, path)?;
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let rows = sqlx::query(
            r#"
            SELECT
                CAST(trigger_name AS CHAR) AS trigger_name,
                CAST(event_manipulation AS CHAR) AS event_manipulation,
                CAST(action_timing AS CHAR) AS action_timing,
                CAST(action_statement AS CHAR) AS action_statement
            FROM information_schema.triggers
            WHERE trigger_schema = ? AND event_object_table = ?
            ORDER BY trigger_name
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;

        mysql_triggers_from_rows(rows)
    })
}

fn mysql_triggers_from_rows(rows: Vec<MySqlRow>) -> fluxdb_core::Result<Vec<TriggerInfo>> {
    rows.into_iter()
        .map(|row| {
            Ok(TriggerInfo {
                name: row.try_get("trigger_name").map_err(mysql_error)?,
                event: row.try_get("event_manipulation").map_err(mysql_error)?,
                timing: row.try_get("action_timing").map_err(mysql_error)?,
                body: Some(row.try_get("action_statement").map_err(mysql_error)?),
            })
        })
        .collect()
}

fn mysql_table_ddl(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<String> {
    let database = mysql_database_for_path(config, path)?;
    let sql = format!(
        "SHOW CREATE TABLE {}.{}",
        fluxdb_core::MySqlDialect.quote_identifier(&database),
        fluxdb_core::MySqlDialect.quote_identifier(&path.name)
    );
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let row = sqlx::query(&sql)
            .fetch_one(&mut connection)
            .await
            .map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;
        row.try_get::<String, _>(1).map_err(mysql_error)
    })
}

/// 设计表专用快照：一次 MySQL 连接读取列、索引、外键、触发器与 DDL。
fn mysql_load_table_design_metadata(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<fluxdb_core::TableDesignMetadata> {
    let database = mysql_database_for_path(config, path)?;
    let plan = mysql_connection_plan(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| Error::new(ErrorKind::Internal, error.to_string()))?;

    runtime.block_on(async {
        let mut connection = plan.connect().await?;
        let columns = mysql_columns(&mut connection, &database, &path.name).await?;
        let has_expression = mysql_statistics_has_expression(&mut connection).await;
        let index_rows = sqlx::query(mysql_indexes_query(has_expression))
            .bind(&database)
            .bind(&path.name)
            .fetch_all(&mut connection)
            .await
            .map_err(mysql_error)?;
        let (index_rows, index_attributes) = mysql_indexes_from_rows(index_rows)?;
        let foreign_key_rows = sqlx::query(
            r#"
            SELECT
                CAST(constraint_name AS CHAR) AS constraint_name,
                CAST(column_name AS CHAR) AS column_name,
                CAST(referenced_table_schema AS CHAR) AS referenced_table_schema,
                CAST(referenced_table_name AS CHAR) AS referenced_table_name,
                CAST(referenced_column_name AS CHAR) AS referenced_column_name
            FROM information_schema.key_column_usage
            WHERE table_schema = ?
              AND table_name = ?
              AND referenced_table_name IS NOT NULL
            ORDER BY constraint_name, ordinal_position
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        let foreign_keys = mysql_foreign_keys_from_rows(foreign_key_rows)?;
        // 引用动作单独读取：共享的外键投影不携带动作，避免改动所有调用方。
        let action_rows = sqlx::query(
            r#"
            SELECT
                CAST(constraint_name AS CHAR) AS constraint_name,
                CAST(delete_rule AS CHAR) AS delete_rule,
                CAST(update_rule AS CHAR) AS update_rule
            FROM information_schema.referential_constraints
            WHERE constraint_schema = ? AND table_name = ?
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        let foreign_key_actions = action_rows
            .into_iter()
            .filter_map(|row| {
                let name: String = row.try_get("constraint_name").ok()?;
                let on_delete: Option<String> = row.try_get("delete_rule").ok();
                let on_update: Option<String> = row.try_get("update_rule").ok();
                Some((
                    name,
                    fluxdb_core::ForeignKeyActions {
                        on_delete,
                        on_update,
                    },
                ))
            })
            .collect();
        // CHECK 约束需要 MySQL 8.0.16+ / MariaDB 10.2+；读取失败降级为「未读取」，
        // 结构同步据此跳过 CHECK 比对，不会误删目标端约束。
        let check_rows = sqlx::query(
            r#"
            SELECT
                CAST(tc.constraint_name AS CHAR) AS constraint_name,
                CAST(cc.check_clause AS CHAR) AS check_clause
            FROM information_schema.table_constraints tc
            JOIN information_schema.check_constraints cc
              ON cc.constraint_schema = tc.constraint_schema
             AND cc.constraint_name = tc.constraint_name
            WHERE tc.table_schema = ? AND tc.table_name = ?
              AND tc.constraint_type = 'CHECK'
            ORDER BY tc.constraint_name
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await;
        let (checks, checks_loaded) = match check_rows {
            Ok(rows) => (
                rows.into_iter()
                    .filter_map(|row| {
                        let name: String = row.try_get("constraint_name").ok()?;
                        let expression: String = row.try_get("check_clause").ok()?;
                        Some(fluxdb_core::CheckMeta {
                            name,
                            expression,
                            definition: String::new(),
                        })
                    })
                    .collect::<Vec<_>>(),
                true,
            ),
            Err(error) => {
                tracing::debug!(
                    target: "fluxdb_connectors",
                    op = "metadata_checks",
                    table = %path.name,
                    error = %error,
                    "MySQL CHECK 约束元数据不可用，跳过 CHECK 比对"
                );
                (Vec::new(), false)
            }
        };
        let trigger_rows = sqlx::query(
            r#"
            SELECT
                CAST(trigger_name AS CHAR) AS trigger_name,
                CAST(event_manipulation AS CHAR) AS event_manipulation,
                CAST(action_timing AS CHAR) AS action_timing,
                CAST(action_statement AS CHAR) AS action_statement
            FROM information_schema.triggers
            WHERE trigger_schema = ? AND event_object_table = ?
            ORDER BY trigger_name
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        let triggers = mysql_triggers_from_rows(trigger_rows)?;
        // 设计表快照单独读取默认值：completion 查询路径不需要，避免改动共享列模型。
        let default_rows = sqlx::query(
            r#"
            SELECT
                CAST(column_name AS CHAR) AS column_name,
                CAST(column_default AS CHAR) AS column_default
            FROM information_schema.columns
            WHERE table_schema = ? AND table_name = ? AND column_default IS NOT NULL
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        let column_defaults = default_rows
            .into_iter()
            .filter_map(|row| {
                let name: String = row.try_get("column_name").ok()?;
                let default: String = row.try_get("column_default").ok()?;
                Some((name, default))
            })
            .collect();
        // 自增列、生成列与表注释：跨方言建表需要，和默认值一样在 design 快照里单独读取。
        let extra_rows = sqlx::query(
            r#"
            SELECT
                CAST(column_name AS CHAR) AS column_name,
                CAST(extra AS CHAR) AS extra,
                CAST(generation_expression AS CHAR) AS generation_expression
            FROM information_schema.columns
            WHERE table_schema = ? AND table_name = ?
            "#,
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_all(&mut connection)
        .await
        .map_err(mysql_error)?;
        let auto_increment_columns = extra_rows
            .iter()
            .filter_map(|row| {
                let name: String = row.try_get("column_name").ok()?;
                let extra: String = row.try_get("extra").ok()?;
                extra
                    .to_ascii_lowercase()
                    .contains("auto_increment")
                    .then_some(name)
            })
            .collect::<std::collections::BTreeSet<_>>();
        // `DEFAULT_GENERATED` 表示默认值表达式，不是生成列，必须排除。
        let generated_columns = extra_rows
            .iter()
            .filter_map(|row| {
                let name: String = row.try_get("column_name").ok()?;
                let extra: String = row.try_get::<String, _>("extra").ok()?.to_ascii_lowercase();
                if !extra.contains("generated") || extra.contains("default_generated") {
                    return None;
                }
                let expression: Option<String> =
                    row.try_get("generation_expression").ok().filter(|value: &String| !value.is_empty());
                Some((
                    name,
                    fluxdb_core::GeneratedColumn {
                        expression: expression?,
                        stored: Some(extra.contains("stored")),
                    },
                ))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let table_comment: Option<String> = sqlx::query_scalar(
            "SELECT CAST(table_comment AS CHAR) FROM information_schema.tables \
             WHERE table_schema = ? AND table_name = ?",
        )
        .bind(&database)
        .bind(&path.name)
        .fetch_optional(&mut connection)
        .await
        .map_err(mysql_error)?
        .filter(|comment: &String| !comment.trim().is_empty());
        let ddl_sql = format!(
            "SHOW CREATE TABLE {}.{}",
            fluxdb_core::MySqlDialect.quote_identifier(&database),
            fluxdb_core::MySqlDialect.quote_identifier(&path.name)
        );
        let ddl_row = sqlx::query(&ddl_sql)
            .fetch_one(&mut connection)
            .await
            .map_err(mysql_error)?;
        let ddl: String = ddl_row.try_get(1).map_err(mysql_error)?;
        connection.close().await.map_err(mysql_error)?;
        Ok(fluxdb_core::TableDesignMetadata {
            columns: columns_to_completion(Some(&database), None, &path.name, columns),
            indexes: index_rows,
            foreign_keys,
            triggers,
            column_defaults,
            auto_increment_columns,
            generated_columns,
            index_attributes,
            foreign_key_actions,
            checks,
            checks_loaded,
            table_comment,
            ddl,
            details_loaded: true,
        })
    })
}

fn mysql_database_for_path(
    config: &ConnectionConfig,
    path: &ObjectPath,
) -> fluxdb_core::Result<String> {
    path.database
        .clone()
        .or_else(|| match &config.endpoint {
            Endpoint::Tcp { database, .. } => database.clone(),
            _ => None,
        })
        .filter(|database| !database.is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Connection, "缺少数据库上下文"))
}
