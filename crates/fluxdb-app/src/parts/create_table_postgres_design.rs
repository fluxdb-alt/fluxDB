// PostgreSQL 设计表差异执行（T17，设计 §9.2）。
//
// 只生成**精确的差异动作**，不重建整表：普通字段修改不会碰到分区/RLS/排除约束等
// 图形编辑器尚不能表示的定义，因为它们根本不出现在生成语句里。
// 类型转换不自动编造 USING 表达式——PG 能隐式转换的会成功，不能的由服务端报错并
// 提示用户手写 USING，避免静默截断数据。

/// 设计表 → 有序 PG 差异语句；无变化返回空列表（无修改无 SQL）。
pub(crate) fn create_table_postgres_design_statements(
    create: &CreateTableState,
) -> Result<Vec<String>, String> {
    let CreateTableMode::Design {
        object,
        original,
        original_ddl,
    } = &create.mode
    else {
        return Ok(Vec::new());
    };
    let mut statements = Vec::new();
    let schema = postgres_design_schema(create, object);
    let old_table = postgres_qualified_name(schema.as_deref(), object.name.trim());
    let new_table_name = create.table_name.trim();
    let table = postgres_qualified_name(schema.as_deref(), new_table_name);

    // 1) 表改名：PG 的 RENAME TO 只能接新名（不带 schema）。
    if !object.name.trim().eq_ignore_ascii_case(new_table_name) {
        statements.push(format!(
            "ALTER TABLE {old_table} RENAME TO {};",
            quote_postgres_identifier(new_table_name)
        ));
    }

    // 2) 主键变化：先丢弃旧约束（列删除可能依赖它），最后再加回新主键。
    let old_primary_key = create_table_primary_key_names(&original.columns);
    let new_primary_key = create_table_primary_key_names(&create.columns);
    let primary_key_changed = old_primary_key != new_primary_key;
    if primary_key_changed && !old_primary_key.is_empty() {
        let name = postgres_primary_key_constraint_name(original_ddl.as_deref(), object.name.trim());
        statements.push(format!(
            "ALTER TABLE {table} DROP CONSTRAINT {};",
            quote_postgres_identifier(&name)
        ));
    }

    // 3) 列的删除 / 新增 / 修改。
    for column in &original.columns {
        if !create.columns.iter().any(|current| current.id == column.id) {
            statements.push(format!(
                "ALTER TABLE {table} DROP COLUMN {};",
                quote_postgres_identifier(column.name.trim())
            ));
        }
    }
    for column in &create.columns {
        let Some(added_line) = create_table_postgres_column_line(column) else {
            continue;
        };
        match original
            .columns
            .iter()
            .find(|original| original.id == column.id)
        {
            None => statements.push(format!("ALTER TABLE {table} ADD COLUMN {added_line};")),
            Some(previous) if previous != column => statements.extend(
                postgres_column_change_statements(&table, previous, column)?,
            ),
            Some(_) => {}
        }
    }

    // 4) 索引：PG 的索引不挂在 ALTER TABLE 上，独立 CREATE/DROP INDEX。
    statements.extend(postgres_index_change_statements(
        create,
        original,
        schema.as_deref(),
    )?);

    // 5) 主键补建。
    if primary_key_changed
        && let Some(line) = create_table_postgres_primary_key_line(&create.columns)
    {
        statements.push(format!("ALTER TABLE {table} ADD {line};"));
    }

    // 6) 外键：PG 的约束统一走 ADD/DROP CONSTRAINT。
    for foreign_key in &original.foreign_keys {
        let unchanged = create
            .foreign_keys
            .iter()
            .any(|current| current.id == foreign_key.id && current == foreign_key);
        if !unchanged && !foreign_key.name.trim().is_empty() {
            statements.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {};",
                quote_postgres_identifier(foreign_key.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_foreign_key_lines(
            &create_table_with_foreign_keys(
                create,
                create_table_changed_items(&original.foreign_keys, &create.foreign_keys),
            ),
            CreateTableSqlDialect::Postgres,
        )?
        .into_iter()
        .map(|line| format!("ALTER TABLE {table} ADD {line};")),
    );

    // 7) CHECK 约束。
    for check in &original.checks {
        let unchanged = create
            .checks
            .iter()
            .any(|current| current.id == check.id && current == check);
        if !unchanged && !check.name.trim().is_empty() {
            statements.push(format!(
                "ALTER TABLE {table} DROP CONSTRAINT {};",
                quote_postgres_identifier(check.name.trim())
            ));
        }
    }
    statements.extend(
        create_table_check_lines(
            &create_table_with_checks(
                create,
                create_table_changed_items(&original.checks, &create.checks),
            ),
            CreateTableSqlDialect::Postgres,
        )?
        .into_iter()
        .map(|line| format!("ALTER TABLE {table} ADD {line};")),
    );

    // 8) 触发器：PG 的 DROP TRIGGER 需要带上所属表。
    for trigger in &original.triggers {
        let unchanged = create
            .triggers
            .iter()
            .any(|current| current.id == trigger.id && current == trigger);
        if !unchanged && !trigger.name.trim().is_empty() {
            statements.push(format!(
                "DROP TRIGGER {} ON {table};",
                quote_postgres_identifier(trigger.name.trim())
            ));
        }
    }
    statements.extend(create_table_postgres_trigger_statements(
        &create_table_with_triggers(
            create,
            create_table_changed_items(&original.triggers, &create.triggers),
        ),
    )?);

    // 9) 表注释。
    if create.comment.trim() != original.comment.trim() {
        statements.push(format!(
            "COMMENT ON TABLE {table} IS {};",
            quote_postgres_string(create.comment.trim())
        ));
    }

    Ok(statements)
}

/// 设计表的 schema：状态里显式填写的优先，其次取原对象路径。
fn postgres_design_schema(create: &CreateTableState, object: &ObjectPath) -> Option<String> {
    let schema = create.schema.trim();
    if !schema.is_empty() {
        return Some(schema.to_string());
    }
    object
        .schema
        .as_deref()
        .map(str::trim)
        .filter(|schema| !schema.is_empty())
        .map(str::to_string)
}

fn postgres_qualified_name(schema: Option<&str>, table: &str) -> String {
    match schema {
        Some(schema) => format!(
            "{}.{}",
            quote_postgres_identifier(schema),
            quote_postgres_identifier(table)
        ),
        None => quote_postgres_identifier(table),
    }
}

/// 原主键约束名。
///
/// 设计快照里的索引列表不含主键（加载时被过滤），故从展示 DDL 里的
/// `CONSTRAINT "name" PRIMARY KEY` 解析；解析不到则回退 PG 的默认命名 `<table>_pkey`。
/// 名字不对时服务端会直接报「constraint does not exist」，比静默改错结构好。
fn postgres_primary_key_constraint_name(ddl: Option<&str>, table_name: &str) -> String {
    if let Some(ddl) = ddl {
        let upper = ddl.to_ascii_uppercase();
        let mut search_from = 0usize;
        while let Some(offset) = upper[search_from..].find("CONSTRAINT") {
            let start = search_from + offset + "CONSTRAINT".len();
            let rest = &ddl[start..];
            let name = rest
                .trim_start()
                .split(|ch: char| ch.is_whitespace() || ch == ';' || ch == ',')
                .next()
                .unwrap_or("")
                .trim_matches('"')
                .trim_matches('`')
                .to_string();
            let after = rest.trim_start();
            let after = after[name.len().min(after.len())..].trim_start();
            if !name.is_empty() && after.to_ascii_uppercase().starts_with("PRIMARY KEY") {
                return name;
            }
            search_from = start;
        }
    }
    format!("{table_name}_pkey")
}

/// 单列差异 → PG 的 ALTER COLUMN 序列（改名/类型/默认值/可空/identity/注释）。
fn postgres_column_change_statements(
    table: &str,
    previous: &CreateTableColumn,
    current: &CreateTableColumn,
) -> Result<Vec<String>, String> {
    let mut statements = Vec::new();
    let previous_name = previous.name.trim();
    let column = quote_postgres_identifier(current.name.trim());

    // 改名先做：后续语句都按新名引用该列。
    if !previous_name.eq_ignore_ascii_case(current.name.trim()) {
        statements.push(format!(
            "ALTER TABLE {table} RENAME COLUMN {} TO {column};",
            quote_postgres_identifier(previous_name)
        ));
    }

    if create_table_postgres_column_type(previous) != create_table_postgres_column_type(current) {
        // 不自动编造 USING：PG 能隐式转换的会成功，需要显式转换时由服务端报错提示用户补写。
        statements.push(format!(
            "ALTER TABLE {table} ALTER COLUMN {column} TYPE {};",
            create_table_postgres_column_type(current)
        ));
    }
    if previous.default_value.trim() != current.default_value.trim() {
        let default_value = current.default_value.trim();
        if default_value.is_empty() {
            statements.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column} DROP DEFAULT;"
            ));
        } else {
            statements.push(format!(
                "ALTER TABLE {table} ALTER COLUMN {column} SET DEFAULT {default_value};"
            ));
        }
    }
    let previous_nullable = previous.nullable && !previous.primary_key;
    let current_nullable = current.nullable && !current.primary_key;
    if previous_nullable != current_nullable {
        let action = if current_nullable {
            "DROP NOT NULL"
        } else {
            "SET NOT NULL"
        };
        statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column} {action};"));
    }
    if previous.auto_increment != current.auto_increment {
        let action = if current.auto_increment {
            "ADD GENERATED BY DEFAULT AS IDENTITY"
        } else {
            "DROP IDENTITY IF EXISTS"
        };
        statements.push(format!("ALTER TABLE {table} ALTER COLUMN {column} {action};"));
    }
    if previous.comment.trim() != current.comment.trim() {
        statements.push(format!(
            "COMMENT ON COLUMN {table}.{column} IS {};",
            quote_postgres_string(current.comment.trim())
        ));
    }
    Ok(statements)
}

/// 索引差异 → DROP INDEX / CREATE INDEX（PG 索引不随 ALTER TABLE 变更）。
fn postgres_index_change_statements(
    create: &CreateTableState,
    original: &CreateTableDesignSnapshot,
    schema: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut statements = Vec::new();
    for index in original.indexes.iter() {
        let unchanged = create
            .indexes
            .iter()
            .any(|current| current.id == index.id && current == index);
        if !unchanged && !index.name.trim().is_empty() {
            statements.push(format!(
                "DROP INDEX {};",
                postgres_qualified_name(schema, index.name.trim())
            ));
        }
    }

    let mut create_with_changed_indexes = create.clone();
    create_with_changed_indexes.indexes = create
        .indexes
        .iter()
        .filter(|index| {
            !original
                .indexes
                .iter()
                .any(|previous| previous.id == index.id && previous == *index)
        })
        .cloned()
        .collect();
    statements.extend(create_table_postgres_index_statements(
        &create_with_changed_indexes,
    )?);
    Ok(statements)
}
