/// 表 DDL：视图/物化视图走 `pg_get_viewdef`；普通表由 TableStructure 组装重建。
fn pg_table_ddl(config: &ConnectionConfig, path: &ObjectPath) -> fluxdb_core::Result<String> {
    let database = pg_request_database(config, path.database.as_deref());
    let schema = path.schema.as_deref().filter(|s| !s.is_empty())
        .ok_or_else(|| Error::new(ErrorKind::Query, "缺少 schema 上下文"))?;
    pg_runtime().block_on(async {
        let session = pg_connect(config, &database).await?;
        pg_catalog_table_ddl(&session.client, &database, schema, &path.name).await
    })
}

/// 单列定义行：`"name" type [DEFAULT x] [ident/generated] [NOT NULL]`。
fn pg_column_line(col: &ColumnMeta) -> String {
    let mut line = format!(
        "{} {}",
        fluxdb_core::PostgresDialect.quote_identifier(&col.name),
        col.data_type
    );
    if !col.is_identity && !col.is_generated {
        if let Some(default) = col.default_expr.as_deref().filter(|d| !d.is_empty()) {
            line.push_str(&format!(" DEFAULT {default}"));
        }
    }
    if col.is_identity {
        let generation = if col.identity_generation.as_deref() == Some("a") {
            "ALWAYS"
        } else {
            "BY DEFAULT"
        };
        line.push_str(&format!(" GENERATED {generation} AS IDENTITY"));
    } else if col.is_generated {
        let expression = col.default_expr.as_deref().unwrap_or("");
        line.push_str(&format!(" GENERATED ALWAYS AS ({expression}) STORED"));
    }
    if !col.nullable {
        line.push_str(" NOT NULL");
    }
    line
}

/// 保留 pg_catalog 的完整定义；图形编辑器不能表达的约束/种类也不能在 DDL 中消失。
async fn pg_catalog_table_ddl(client: &tokio_postgres::Client, database: &str, schema: &str, name: &str) -> fluxdb_core::Result<String> {
    let relation = pg_qualified_relation(schema, name);
    let meta = client.query_opt(
        "SELECT c.oid, c.relkind::text, c.relpersistence::text, c.relispartition,
                pg_get_partkeydef(c.oid), pg_get_expr(c.relpartbound, c.oid),
                c.relrowsecurity, c.relforcerowsecurity, c.reloptions,
                (SELECT format('%I.%I', n.nspname,p.relname) FROM pg_inherits i JOIN pg_class p ON p.oid=i.inhparent JOIN pg_namespace n ON n.oid=p.relnamespace WHERE i.inhrelid=c.oid ORDER BY i.inhseqno LIMIT 1),
                t.spcname
         FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
         LEFT JOIN pg_tablespace t ON t.oid=c.reltablespace WHERE n.nspname=$1 AND c.relname=$2",
        &[&schema, &name]).await.map_err(pg_error)?
        .ok_or_else(|| Error::new(ErrorKind::Query, "关系不存在"))?;
    let oid: u32 = meta.get(0); let kind: String = meta.get(1);
    let persistence: String = meta.get(2); let partition: bool = meta.get(3);
    let partition_key: Option<String> = meta.get(4); let partition_bound: Option<String> = meta.get(5);
    let rls: bool = meta.get(6); let force_rls: bool = meta.get(7);
    let options: Option<Vec<String>> = meta.get(8); let parent: Option<String> = meta.get(9);
    let tablespace: Option<String> = meta.get(10);
    let structure = pg_load_table_structure(client, database, schema, name).await?;
    let mut before = Vec::new(); let mut after = Vec::new();
    let mut columns = structure.columns.iter().map(|col| (col.name.clone(), pg_column_line(col))).collect::<BTreeMap<_,_>>();
    let mut constraints = Vec::new(); let mut backing_indexes = std::collections::HashSet::new();
    for row in client.query("SELECT conname, pg_get_constraintdef(oid, true), conindid, contype::text FROM pg_constraint WHERE conrelid=$1 AND conparentid=0 ORDER BY conname", &[&oid]).await.map_err(pg_error)? {
        let constraint_name: String = row.get(0); let definition: String = row.get(1);
        let index_oid: u32 = row.get(2); let constraint_kind: String = row.get(3);
        if constraint_kind != "f" && index_oid != 0 { backing_indexes.insert(index_oid); }
        constraints.push(format!("CONSTRAINT {} {}", fluxdb_core::PostgresDialect.quote_identifier(&constraint_name), definition));
    }
    // identity/serial 的生成器属性来自 pg_sequence；serial 必须先建序列，再建列默认值。
    for row in client.query(
        // Kingbase 的 pg_sequence 把 start/increment/min/max 报成 int2（值仍是 int8），
        // tokio-postgres 按声明类型解码会失败；统一转 bigint 后再按 i64 读取。
        "SELECT a.attname, a.attidentity::text, sn.nspname, sc.relname,
                format_type(s.seqtypid,NULL), s.seqstart::bigint, s.seqincrement::bigint,
                s.seqmin::bigint, s.seqmax::bigint, s.seqcache::bigint, s.seqcycle
         FROM pg_attribute a JOIN pg_depend d ON d.refobjid=a.attrelid AND d.refobjsubid=a.attnum
              AND d.classid='pg_class'::regclass AND d.refclassid='pg_class'::regclass AND d.deptype IN ('a','i')
         JOIN pg_class sc ON sc.oid=d.objid AND sc.relkind='S'
         JOIN pg_namespace sn ON sn.oid=sc.relnamespace JOIN pg_sequence s ON s.seqrelid=sc.oid
         WHERE a.attrelid=$1 ORDER BY a.attnum", &[&oid]).await.map_err(pg_error)? {
        let col: String = row.get(0); let identity: String = row.get(1);
        let seq_schema: String = row.get(2); let seq_name: String = row.get(3);
        let seq = pg_qualified_relation(&seq_schema, &seq_name);
        let seq_type: String = row.get(4);
        let seq_options = format!("START WITH {} INCREMENT BY {} MINVALUE {} MAXVALUE {} CACHE {} {}CYCLE",
            row.get::<_, i64>(5), row.get::<_, i64>(6), row.get::<_, i64>(7), row.get::<_, i64>(8), row.get::<_, i64>(9), if row.get::<_, bool>(10) { "" } else { "NO " });
        if identity.is_empty() {
            before.push(format!("CREATE SEQUENCE {seq} AS {seq_type} {seq_options};"));
            after.push(format!("ALTER SEQUENCE {seq} OWNED BY {relation}.{};", fluxdb_core::PostgresDialect.quote_identifier(&col)));
        } else if let Some(line) = columns.get_mut(&col) {
            *line = line.replace("AS IDENTITY", &format!("AS IDENTITY (SEQUENCE NAME {seq} {seq_options})"));
        }
    }
    for row in client.query("SELECT a.attname, a.attgenerated::text, CASE WHEN a.attcollation <> t.typcollation THEN format('%I.%I', n.nspname,c.collname) END FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid LEFT JOIN pg_collation c ON c.oid=a.attcollation LEFT JOIN pg_namespace n ON n.oid=c.collnamespace WHERE a.attrelid=$1 AND a.attnum>0 AND NOT a.attisdropped", &[&oid]).await.map_err(pg_error)? {
        let col: String = row.get(0); let generated: String = row.get(1); let collation: Option<String> = row.get(2);
        if let Some(line) = columns.get_mut(&col) {
            if generated == "v" { *line = line.replace(" STORED", " VIRTUAL"); }
            if let Some(collation) = collation { line.push_str(&format!(" COLLATE {collation}")); }
        }
    }
    let storage = if persistence == "u" { "UNLOGGED " } else if persistence == "t" { "TEMPORARY " } else { "" };
    let mut create = if matches!(kind.as_str(), "v" | "m") {
        let definition: String = client.query_one("SELECT pg_get_viewdef($1::oid, true)", &[&oid]).await.map_err(pg_error)?.get(0);
        let view_type = if kind == "m" { "MATERIALIZED VIEW" } else { "VIEW" };
        format!("CREATE {view_type} {relation} AS\n{}{}", definition.trim_end_matches(';'), if kind == "m" { " WITH NO DATA" } else { "" })
    } else if partition {
        format!("CREATE {storage}TABLE {relation} PARTITION OF {} {}", parent.ok_or_else(|| Error::new(ErrorKind::Query, "分区缺少父表"))?, partition_bound.unwrap_or_default())
    } else {
        let mut body = structure.columns.iter().filter_map(|c| columns.get(&c.name).cloned()).collect::<Vec<_>>();
        body.extend(constraints);
        let table_kind = if kind == "f" { "FOREIGN TABLE" } else { "TABLE" };
        let mut ddl = format!("CREATE {storage}{table_kind} {relation} (\n  {}\n)", body.join(",\n  "));
        if let Some(parent) = parent { ddl.push_str(&format!(" INHERITS ({parent})")); }
        ddl
    };
    if let Some(key) = partition_key { create.push_str(&format!(" PARTITION BY {key}")); }
    if kind == "f" {
        let row = client.query_one("SELECT s.srvname, f.ftoptions FROM pg_foreign_table f JOIN pg_foreign_server s ON s.oid=f.ftserver WHERE f.ftrelid=$1", &[&oid]).await.map_err(pg_error)?;
        create.push_str(&format!(" SERVER {}", fluxdb_core::PostgresDialect.quote_identifier(row.get::<_, String>(0).as_str())));
        if let Some(options) = row.get::<_, Option<Vec<String>>>(1) {
            let options = options.iter().filter_map(|s| s.split_once('=')).map(|(k,v)| format!("{} {}", fluxdb_core::PostgresDialect.quote_identifier(k), pg_sql_string_literal(v))).collect::<Vec<_>>();
            if !options.is_empty() { create.push_str(&format!(" OPTIONS ({})", options.join(", "))); }
        }
    }
    if !matches!(kind.as_str(), "v" | "m") {
        if let Some(options) = options { if !options.is_empty() { create.push_str(&format!(" WITH ({})", options.join(", "))); } }
        if let Some(space) = tablespace { create.push_str(&format!(" TABLESPACE {}", fluxdb_core::PostgresDialect.quote_identifier(&space))); }
    }
    before.push(format!("{create};"));
    for row in client.query("SELECT indexrelid, pg_get_indexdef(indexrelid) FROM pg_index WHERE indrelid=$1 ORDER BY indexrelid", &[&oid]).await.map_err(pg_error)? {
        if !backing_indexes.contains(&row.get::<_, u32>(0)) { after.push(format!("{};", row.get::<_, String>(1))); }
    }
    for trigger in &structure.triggers {
        after.push(format!("{};", trigger.definition.trim_end_matches(';')));
        if !trigger.enabled { after.push(format!("ALTER TABLE {relation} DISABLE TRIGGER {};", fluxdb_core::PostgresDialect.quote_identifier(&trigger.name))); }
    }
    if rls { after.push(format!("ALTER TABLE {relation} ENABLE ROW LEVEL SECURITY;")); }
    if force_rls { after.push(format!("ALTER TABLE {relation} FORCE ROW LEVEL SECURITY;")); }
    if let Some(comment) = &structure.comment {
        let object_type = match kind.as_str() { "v" => "VIEW", "m" => "MATERIALIZED VIEW", "f" => "FOREIGN TABLE", _ => "TABLE" };
        after.push(format!("COMMENT ON {object_type} {relation} IS {};", pg_sql_string_literal(comment)));
    }
    for col in &structure.columns {
        if let Some(comment) = &col.comment { after.push(format!("COMMENT ON COLUMN {relation}.{} IS {};", fluxdb_core::PostgresDialect.quote_identifier(&col.name), pg_sql_string_literal(comment))); }
    }
    before.extend(after);
    Ok(before.join("\n\n"))
}
