// 逐表恢复的分桶器：把备份 SQL 按目标表归类，供「覆盖/追加/跳过」过滤重建脚本用。
// 依赖 script.rs 的 for_each_statement 做语句切分，这里只做轻量前缀分类 + 取目标表名。

/// 语句类别。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StmtKind {
    /// DROP/CREATE TABLE、ALTER TABLE ... DISABLE/ENABLE KEYS
    TableDdl,
    /// INSERT INTO
    TableData,
    /// ALTER TABLE ONLY ... ADD CONSTRAINT
    TableConstraint,
    /// CREATE [UNIQUE] INDEX ... ON <table>
    TableIndex,
    /// SELECT pg_catalog.setval(...)
    SequenceSetval,
    /// CREATE SCHEMA
    Schema,
    /// CREATE SEQUENCE
    Sequence,
    /// SET / LOCK / UNLOCK / 其他会话级
    NonTable,
}

/// 单条已分类语句。
#[derive(Clone, Debug)]
pub(crate) struct PStmt {
    /// 可执行语句文本（去注释）。
    pub(crate) raw: String,
    /// 决策匹配键：MySQL=裸表名；PG=「schema.表」或裸表名（兜底）。
    pub(crate) table_key: Option<String>,
    /// 语句目标表的 schema（PG 用于生成 schema 限定的 TRUNCATE）；MySQL/裸名为 None。
    pub(crate) table_schema: Option<String>,
    pub(crate) kind: StmtKind,
}

/// 对单个备份文件做逐表分桶。
///
/// 返回按出现顺序的去重表清单（含各表是否有 DDL/数据/约束的标志）和全部已分类语句。
pub(crate) fn partition_statements(
    path: &Path,
    kind: DatabaseKind,
    cancel: &AtomicBool,
) -> fluxdb_core::Result<(Vec<TablePart>, Vec<PStmt>)> {
    let mut stmts: Vec<PStmt> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut by_key: std::collections::BTreeMap<String, TablePart> = Default::default();
    for_each_statement(path, kind, cancel, &mut |sql| {
        let trimmed = sql.trim();
        let upper = trimmed.to_ascii_uppercase();
        // 切分器会吞掉 COPY 数据行，这里只剩 COPY 头语句；按对象分桶会静默丢行数据，
        // 必须拒绝并引导走整库恢复（整库路径直接管道原始文件，不受影响）。
        if kind == DatabaseKind::Postgres && upper.starts_with("COPY ") {
            return Err(task_error(
                "备份含 COPY 数据块，按对象恢复无法保留其中的行数据；请改用整库恢复",
            ));
        }
        let (k, name, schema) = classify(kind, &upper, trimmed); // name 从原文本取（保大小写）
        let table_key = table_key_for(kind, name.as_deref(), schema.as_deref());
        if let Some(key) = &table_key {
            let entry = by_key
                .entry(key.clone())
                .or_insert_with(|| TablePart {
                    key: key.clone(),
                    name: name.clone().unwrap_or_default(),
                    schema: schema.clone(),
                    has_ddl_drop: false,
                    has_ddl_create: false,
                    has_data: false,
                    has_constraints: false,
                    seen_order: stmts.len(),
                });
            match k {
                StmtKind::TableDdl => {
                    if upper.starts_with("DROP TABLE") {
                        entry.has_ddl_drop = true;
                    } else {
                        entry.has_ddl_create = true;
                    }
                }
                StmtKind::TableData => entry.has_data = true,
                StmtKind::TableConstraint => entry.has_constraints = true,
                _ => {}
            }
            if !order.contains(key) {
                order.push(key.clone());
            }
        }
        stmts.push(PStmt {
            raw: trimmed.to_string(),
            table_schema: schema,
            table_key,
            kind: k,
        });
        Ok(())
    })?;
    // 按首次出现顺序输出表清单（files 里 first appears）。
    let mut tables: Vec<TablePart> = order
        .into_iter()
        .filter_map(|k| by_key.remove(&k))
        .collect();
    tables.sort_by_key(|t| t.seen_order);
    Ok((tables, stmts))
}

/// 汇总表：去重 + 标志。
#[derive(Clone, Debug)]
pub(crate) struct TablePart {
    pub(crate) key: String,
    /// 展示名（裸表名）。
    pub(crate) name: String,
    /// PG schema（非 public 才有值）；MySQL/裸名为 None。
    pub(crate) schema: Option<String>,
    pub(crate) has_ddl_drop: bool,
    pub(crate) has_ddl_create: bool,
    pub(crate) has_data: bool,
    pub(crate) has_constraints: bool,
    pub(crate) seen_order: usize,
}

/// 取出「CREATE TABLE <name>」等语句后的第一个（可能带 schema 限定的）标识符，返回 (类别, 表名, schema)。
fn classify(
    kind: DatabaseKind,
    upper: &str,
    raw: &str,
) -> (StmtKind, Option<String>, Option<String>) {
    // 语句前缀到类别的映射
    // 标识符从 `raw`（原文）读取以保留大小写；类别判定用 `upper`。
    if upper.starts_with("DROP TABLE") || upper.starts_with("CREATE TABLE") {
        let (name, schema) = read_qualified_ident(raw, 2);
        return (StmtKind::TableDdl, name, schema);
    }
    if upper.starts_with("INSERT INTO") {
        let (name, schema) = read_qualified_ident(raw, 2);
        return (StmtKind::TableData, name, schema);
    }
    if upper.starts_with("ALTER TABLE") {
        let (name, schema) = read_qualified_ident(raw, 2);
        if upper.contains("ADD CONSTRAINT") {
            return (StmtKind::TableConstraint, name, schema);
        }
        if upper.ends_with("DISABLE KEYS") || upper.ends_with("ENABLE KEYS") {
            return (StmtKind::TableDdl, name, schema);
        }
        return (StmtKind::TableConstraint, name, schema);
    }
    if upper.starts_with("CREATE UNIQUE INDEX") || upper.starts_with("CREATE INDEX") {
        let (name, schema) = read_index_target(raw);
        return (StmtKind::TableIndex, name, schema);
    }
    if upper.starts_with("SELECT PG_CATALOG.SETVAL") || upper.starts_with("SELECT SETVAL") {
        return (StmtKind::SequenceSetval, None, None);
    }
    if upper.starts_with("CREATE SCHEMA") {
        return (StmtKind::Schema, None, None);
    }
    if upper.starts_with("CREATE SEQUENCE") {
        let (name, schema) = read_qualified_ident(raw, 2);
        return (StmtKind::Sequence, name, schema.or_else(|| Some("public".to_string())));
    }
    let _ = kind;
    (StmtKind::NonTable, None, None)
}

/// 决策匹配键：PG 用 "schema.name"（或裸名兜底），其余用裸名。
fn table_key_for(kind: DatabaseKind, name: Option<&str>, schema: Option<&str>) -> Option<String> {
    let name = name?;
    if kind == DatabaseKind::Postgres {
        if let Some(s) = schema {
            if !s.is_empty() && s != "public" {
                return Some(format!("{}.{}", s, name));
            }
        }
        return Some(name.to_string());
    }
    Some(name.to_string())
}

/// 读取「CREATE TABLE / INSERT INTO / ALTER TABLE」后第一个（可能 schema 限定的）标识符。
/// 返回 (裸表名, schema)。基于 char 解析以正确处理反引号/双引号与非 ASCII 名称。
fn read_qualified_ident(raw: &str, skip_words: usize) -> (Option<String>, Option<String>) {
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    // 跳过前 skip_words 个词。
    for _ in 0..skip_words {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
    }
    // 跳过可选的 ONLY / IF / EXISTS 关键字。
    loop {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        let cleaned = word
            .trim_end_matches(['(', ';', ','])
            .to_ascii_uppercase();
        if cleaned == "ONLY" || cleaned == "IF" || cleaned == "EXISTS" {
            continue;
        }
        i = start;
        break;
    }
    parse_dotted_ident(&chars, &mut i)
}

/// 从 `chars[i]` 起解析一个（可能带引号、可能 `schema.name` 限定的）标识符，返回 (裸名, schema)。
fn parse_dotted_ident(chars: &[char], i: &mut usize) -> (Option<String>, Option<String>) {
    let mut parts: Vec<String> = Vec::new();
    loop {
        while *i < chars.len() && chars[*i].is_whitespace() {
            *i += 1;
        }
        let mut seg = String::new();
        if *i < chars.len() && (chars[*i] == '`' || chars[*i] == '"') {
            let quote = chars[*i];
            *i += 1;
            while *i < chars.len() && chars[*i] != quote {
                seg.push(chars[*i]);
                *i += 1;
            }
            if *i < chars.len() {
                *i += 1; // 闭合引号
            }
        } else {
            while *i < chars.len() {
                let c = chars[*i];
                if c.is_alphanumeric() || c == '_' || c == '$' {
                    seg.push(c);
                    *i += 1;
                } else {
                    break;
                }
            }
        }
        if seg.is_empty() {
            break;
        }
        parts.push(seg);
        // 点号直接跟随则有 schema/限定续段。
        if *i < chars.len() && chars[*i] == '.' {
            *i += 1;
            continue;
        }
        break;
    }
    if parts.is_empty() {
        return (None, None);
    }
    if parts.len() == 1 {
        (Some(parts.remove(0)), None)
    } else {
        let name = parts.pop().unwrap_or_default();
        let schema = parts.pop();
        (Some(name), schema)
    }
}

/// 从「CREATE INDEX <idx> ON <table>」里取表名与 schema（返回原大小写）。
fn read_index_target(raw: &str) -> (Option<String>, Option<String>) {
    let upper = raw.to_ascii_uppercase();
    let Some(idx) = upper.find(" ON ") else {
        return (None, None);
    };
    let chars: Vec<char> = raw.chars().collect();
    // 将字节偏移换算为 char 偏移。
    let byte_off = idx + 4;
    let mut char_off = raw[..byte_off].chars().count();
    parse_dotted_ident(&chars, &mut char_off)
}

/// 依决策重建可执行脚本。返回 (sql, 追加的警告)。
///
/// 动作语义（设计文档 §6.4）：
/// - Create / Recreate：输出该表全部语句（DROP+CREATE+data+index+constraint）。二者执行前
///   由决策校验区分（Create 要求目标不存在，Recreate 要求存在），重建脚本一致。
/// - TruncateAndLoad：保留目标结构，仅在该表首条数据语句前插入清空语句（PG `TRUNCATE ...
///   RESTART IDENTITY`，MySQL `DELETE`），随后输出数据；不输出 DDL/index/constraint。
/// - Append：只输出数据语句（INSERT），不改结构。
/// - Skip：不输出该表任何语句（含约束/索引/序列 setval）。
/// - 非表/会话级语句（SET、LOCK、Schema、Sequence）恒输出，保证 DDL 可解析。
/// - FK 修剪：约束语句仅在归属表动作为 Create/Recreate 且其全部引用表都在 `structure_keys`
///   （恢复后具有可用结构）时保留，否则剔除并告警，避免引用缺失表。
/// - 序列 setval：Append 时剔除并告警（追加不得把现有序列倒退）；Skip 时剔除；其余保留。
pub(crate) fn reconstruct_sql(
    kind: DatabaseKind,
    stmts: &[PStmt],
    decisions: &[PerTableDecision],
    structure_keys: &std::collections::BTreeSet<String>,
) -> (String, Vec<String>) {
    use std::collections::HashSet;
    let action = |key: &str| -> Option<RestoreTableAction> {
        decisions
            .iter()
            .find(|d| d.table == key)
            .map(|d| d.action)
    };
    let mut body = String::new();
    let mut warnings = Vec::new();
    let mut truncated: HashSet<String> = HashSet::new();
    let any_truncate = decisions
        .iter()
        .any(|d| d.action == RestoreTableAction::TruncateAndLoad);
    for s in stmts {
        match s.kind {
            StmtKind::NonTable | StmtKind::Schema | StmtKind::Sequence => {
                body.push_str(&s.raw);
                body.push_str(";\n");
            }
            StmtKind::TableDdl | StmtKind::TableData | StmtKind::TableIndex => {
                let Some(key) = s.table_key.as_deref() else {
                    continue;
                };
                let act = action(key);
                let emit_data = |body: &mut String| {
                    body.push_str(&s.raw);
                    body.push_str(";\n");
                };
                match act {
                    Some(RestoreTableAction::Create) | Some(RestoreTableAction::Recreate) => {
                        emit_data(&mut body);
                    }
                    Some(RestoreTableAction::TruncateAndLoad) => {
                        if s.kind != StmtKind::TableData {
                            continue; // 结构与索引保留目标现状，不重放 DDL/index。
                        }
                        if truncated.insert(key.to_string()) {
                            body.push_str(&truncate_statement(kind, s.table_schema.as_deref(), key));
                            body.push('\n');
                        }
                        emit_data(&mut body);
                    }
                    Some(RestoreTableAction::Append) => {
                        if s.kind == StmtKind::TableData {
                            emit_data(&mut body);
                        }
                    }
                    _ => {}
                }
            }
            StmtKind::TableConstraint => {
                let Some(key) = s.table_key.as_deref() else {
                    continue;
                };
                let act = action(key);
                // 仅 Create/Recreate 会重建结构，才有必要（且安全）重放约束。
                if !matches!(
                    act,
                    Some(RestoreTableAction::Create) | Some(RestoreTableAction::Recreate)
                ) {
                    continue;
                }
                let refs = referenced_tables(&s.raw);
                let all_present = refs.iter().all(|r| structure_keys.contains(r));
                if refs.is_empty() || all_present {
                    body.push_str(&s.raw);
                    body.push_str(";\n");
                } else {
                    warnings.push(format!(
                        "表 {key} 的部分外键约束因引用表未恢复结构（追加/清空/跳过）而未恢复"
                    ));
                }
            }
            StmtKind::SequenceSetval => {
                let owner = setval_table(&s.raw);
                match owner.as_deref().and_then(action) {
                    Some(RestoreTableAction::Skip) => {}
                    Some(RestoreTableAction::Append) => {
                        if let Some(t) = &owner {
                            warnings.push(format!("表 {t} 追加数据，未回放序列 setval（避免现有序列倒退）"));
                        }
                    }
                    _ => {
                        body.push_str(&s.raw);
                        body.push_str(";\n");
                    }
                }
            }
        }
    }
    // MySQL：存在清空后导入时用 FOREIGN_KEY_CHECKS 包裹，避免 DELETE 触发外键失败。
    if any_truncate && kind != DatabaseKind::Postgres {
        let mut sql = String::from("SET FOREIGN_KEY_CHECKS=0;\n");
        sql.push_str(&body);
        sql.push_str("SET FOREIGN_KEY_CHECKS=1;\n");
        (sql, warnings)
    } else {
        (body, warnings)
    }
}

/// 生成清空目标表的语句：PG 用 `TRUNCATE ... RESTART IDENTITY`（schema 限定，因 pg_dump 会重置
/// search_path），MySQL/TiDB 用 `DELETE FROM`（配合脚本级 FOREIGN_KEY_CHECKS=0）。
fn truncate_statement(kind: DatabaseKind, schema: Option<&str>, key: &str) -> String {
    if kind == DatabaseKind::Postgres {
        let schema = schema.unwrap_or("public");
        let bare = key
            .strip_prefix(&format!("{schema}."))
            .unwrap_or(key);
        format!("TRUNCATE TABLE \"{schema}\".\"{bare}\" RESTART IDENTITY;")
    } else {
        format!("DELETE FROM `{key}`;")
    }
}

/// 从约束/建表语句里收集 `REFERENCES <表>` 的表名（决策键 = 裸表名）。
fn referenced_tables(sql: &str) -> Vec<String> {
    let upper = sql.to_ascii_uppercase();
    let mut out = Vec::new();
    let mut rest = upper.as_str();
    while let Some(pos) = rest.find("REFERENCES") {
        rest = &rest[pos + "REFERENCES".len()..];
        // 跳过空白与分隔符（避免 split 前导空串），再取标识符。
        let start = rest
            .find(|c: char| !c.is_whitespace() && c != '(' && c != ')' && c != ',')
            .unwrap_or_else(|| rest.len());
        let mut tok = &rest[start..];
        let end = tok
            .find(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
            .unwrap_or(tok.len());
        tok = &tok[..end];
        let bare = tok
            .trim_start_matches('`')
            .trim_end_matches('`')
            .trim_start_matches('"')
            .trim_end_matches('"');
        let bare = bare.rsplit('.').next().unwrap_or(bare).to_string();
        if !bare.is_empty() {
            out.push(bare);
        }
        // 推进到当前片段之后，避免死循环。
        let advance = start + tok.len();
        rest = &rest[advance.min(rest.len())..];
    }
    out
}

/// 从 `SELECT pg_catalog.setval('seq', ...)` 提取序列名对应的表（决策键简化：返回裸名）。
fn setval_table(sql: &str) -> Option<String> {
    // 形如 SELECT pg_catalog.setval('public.seq_name'...
    let start = sql.find(['(', '\''])?;
    let after = sql.get(start + 1..)?;
    let name = after.strip_prefix('\'')?.split('\'').next()?.to_string();
    Some(name.rsplit('.').next().unwrap_or(&name).to_string())
}

