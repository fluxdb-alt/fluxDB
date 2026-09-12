// PostgreSQL 表操作 SQL 生成（T18，设计 §9.3）。
//
// 与 MySQL 的差别：对象一律 schema 限定（同名表只在指定 schema 内操作）；重命名的新名是
// 单段；清空默认 CONTINUE IDENTITY RESTRICT，用户显式选择才 RESTART/CASCADE；
// 不使用 `session_replication_role` 绕过外键（那不是「禁用外键检查」的等价物）。

struct PostgresTableActionSqlProvider;

impl TableActionSqlProvider for PostgresTableActionSqlProvider {
    fn rename_table_sql(
        &self,
        schema: Option<&str>,
        old_name: &str,
        new_name: &str,
    ) -> Result<String, String> {
        if new_name.contains('.') {
            return Err("重命名只接受单段新表名（不带 schema）".to_string());
        }
        Ok(format!(
            "ALTER TABLE {} RENAME TO {};",
            postgres_action_qualified(schema, old_name),
            quote_postgres_identifier(new_name)
        ))
    }

    /// 复制表：`CREATE TABLE (LIKE ... INCLUDING ALL)` 保留结构/索引/约束/注释，
    /// 由调用方保证随后的序列重绑（LIKE 会把 serial 默认指向**源**序列）。
    fn copy_table_sql(
        &self,
        schema: Option<&str>,
        source_name: &str,
        target_name: &str,
        copy_data: bool,
        _: Option<&str>,
    ) -> Result<String, String> {
        let source = postgres_action_qualified(schema, source_name);
        let target = postgres_action_qualified(schema, target_name);
        let mut statements = vec![format!(
            "CREATE TABLE {target} (LIKE {source} INCLUDING ALL);"
        )];
        // 先重建独立序列（LIKE 会把 serial 默认指向源序列），再按需复制数据并校准序列位置。
        statements.extend(postgres_copy_sequence_statements(schema, target_name));
        if copy_data {
            statements.push(postgres_copy_data_statement(
                schema, &source, &target, target_name,
            ));
        }
        Ok(statements.join("\n"))
    }

    fn drop_table_sql(
        &self,
        kind: ObjectKind,
        schema: Option<&str>,
        table_name: &str,
    ) -> Result<String, String> {
        // 默认 RESTRICT：依赖对象存在时由服务端报错，CASCADE 必须由用户显式选择。
        let name = postgres_action_qualified(schema, table_name);
        match kind {
            ObjectKind::View => Ok(format!("DROP VIEW {name};")),
            _ => Ok(format!("DROP TABLE {name};")),
        }
    }

    fn truncate_table_sql(
        &self,
        schema: Option<&str>,
        table_name: &str,
        restart_identity: bool,
    ) -> Result<String, String> {
        let name = postgres_action_qualified(schema, table_name);
        let identity = if restart_identity {
            "RESTART IDENTITY"
        } else {
            "CONTINUE IDENTITY"
        };
        Ok(format!("TRUNCATE TABLE {name} {identity} RESTRICT;"))
    }

    /// PG 不支持 MySQL 的 FOREIGN_KEY_CHECKS；明确拒绝而不是改用
    /// `session_replication_role`（那会绕过触发器与复制相关约束，语义并不等价）。
    fn with_foreign_key_check(
        &self,
        sql: String,
        foreign_key_check: ForeignKeyCheckMode,
    ) -> Result<String, String> {
        match foreign_key_check {
            ForeignKeyCheckMode::Default => Ok(sql),
            ForeignKeyCheckMode::Enable | ForeignKeyCheckMode::Disable => Err(
                "PostgreSQL 不支持禁用外键检查；请改用 CASCADE 或先处理依赖对象".to_string(),
            ),
        }
    }
}

/// schema 限定名：未给 schema 时不限定（由连接 search_path 决定）。
fn postgres_action_qualified(schema: Option<&str>, name: &str) -> String {
    match schema.map(str::trim).filter(|schema| !schema.is_empty()) {
        Some(schema) => format!(
            "{}.{}",
            quote_postgres_identifier(schema),
            quote_postgres_identifier(name.trim())
        ),
        None => quote_postgres_identifier(name.trim()),
    }
}

/// 让复制出来的表拥有**独立序列**。
///
/// `LIKE ... INCLUDING ALL` 会把源表的 serial 默认值（`nextval('源序列')`）一起复制，
/// 于是目标表的插入会推进源表的序列、源表插入又可能与目标表主键冲突。
/// 这里用 DO 块在目标 schema 内为每个 nextval 默认列新建序列并重绑，identity 列
/// （`attidentity <> ''`）自带独立生成器，不在此列。
fn postgres_copy_sequence_statements(schema: Option<&str>, target_name: &str) -> Vec<String> {
    let target_literal = target_name.trim().replace('\'', "''");
    let schema_filter = postgres_copy_schema_filter(schema);
    vec![format!(
        "DO $$\n\
         DECLARE r record; new_seq text;\n\
         BEGIN\n\
           FOR r IN\n\
             SELECT a.attname AS column_name,\n\
                    n.nspname AS schema_name,\n\
                    format('%I.%I', n.nspname, c.relname) AS table_name\n\
             FROM pg_catalog.pg_attribute a\n\
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid\n\
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n\
             JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum\n\
             WHERE {schema_filter} AND c.relname = '{target_literal}'\n\
               AND a.attnum > 0 AND NOT a.attisdropped AND a.attidentity = ''\n\
               AND pg_catalog.pg_get_expr(d.adbin, d.adrelid) LIKE 'nextval(%'\n\
           LOOP\n\
             new_seq := r.column_name || '_seq';\n\
             EXECUTE format('CREATE SEQUENCE %I.%I OWNED BY %s.%I',\n\
                            r.schema_name, new_seq, r.table_name, r.column_name);\n\
             EXECUTE format('ALTER TABLE %s ALTER COLUMN %I SET DEFAULT nextval(%L::regclass)',\n\
                            r.table_name, r.column_name,\n\
                            format('%I.%I', r.schema_name, new_seq));\n\
           END LOOP;\n\
         END $$;"
    )]
}
/// 复制数据：生成列不参与写入、identity 列用 OVERRIDING SYSTEM VALUE 保留显式值，
/// 复制完成后把各自序列校准到当前最大键值，避免副本后续插入与已复制数据冲突。
fn postgres_copy_data_statement(
    schema: Option<&str>,
    source: &str,
    target: &str,
    target_name: &str,
) -> String {
    let schema_filter = postgres_copy_schema_filter(schema);
    let target_literal = target_name.trim().replace('\'', "''");
    format!(
        "DO $$\n\
         DECLARE r record; colnames text; has_identity boolean; identity_clause text;\n\
                 copied_max bigint; seq_name text;\n\
         BEGIN\n\
           SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY a.attnum),\n\
                  bool_or(a.attidentity <> '')\n\
             INTO colnames, has_identity\n\
           FROM pg_catalog.pg_attribute a\n\
           JOIN pg_catalog.pg_class c ON c.oid = a.attrelid\n\
           JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n\
           WHERE {schema_filter} AND c.relname = '{target_literal}'\n\
             AND a.attnum > 0 AND NOT a.attisdropped AND a.attgenerated = '';\n\
           IF colnames IS NULL THEN RETURN; END IF;\n\
           identity_clause := CASE WHEN has_identity THEN ' OVERRIDING SYSTEM VALUE' ELSE '' END;\n\
           EXECUTE format('INSERT INTO {target} (%s)%s SELECT %s FROM {source}',\n\
                          colnames, identity_clause, colnames);\n\
           FOR r IN\n\
             SELECT a.attname AS column_name, format('%I.%I', n.nspname, c.relname) AS table_name\n\
             FROM pg_catalog.pg_attribute a\n\
             JOIN pg_catalog.pg_class c ON c.oid = a.attrelid\n\
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace\n\
             LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum\n\
             WHERE {schema_filter} AND c.relname = '{target_literal}'\n\
               AND a.attnum > 0 AND NOT a.attisdropped\n\
               AND (a.attidentity <> ''\n\
                    OR pg_catalog.pg_get_expr(d.adbin, d.adrelid) LIKE 'nextval(%')\n\
           LOOP\n\
             seq_name := pg_get_serial_sequence(r.table_name, r.column_name);\n\
             IF seq_name IS NOT NULL THEN\n\
               EXECUTE format('SELECT max(%I) FROM %s', r.column_name, r.table_name) INTO copied_max;\n\
               IF copied_max IS NOT NULL THEN\n\
                 PERFORM setval(seq_name, copied_max);\n\
               END IF;\n\
             END IF;\n\
           END LOOP;\n\
         END $$;"
    )
}

/// 目标表所在的 schema 过滤条件（未指定 schema 时按当前 search_path）。
fn postgres_copy_schema_filter(schema: Option<&str>) -> String {
    match schema.map(str::trim).filter(|schema| !schema.is_empty()) {
        Some(schema) => format!("n.nspname = '{}'", schema.replace('\'', "''")),
        None => "n.nspname = ANY(current_schemas(false))".to_string(),
    }
}
