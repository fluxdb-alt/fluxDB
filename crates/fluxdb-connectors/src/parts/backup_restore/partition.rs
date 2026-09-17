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
        let (k, name, schema) = classify(kind, &upper, trimmed); // name 从原文本取（保大小写）
        let table_key = table_key_for(kind, name.as_deref(), schema.as_deref());
        if let Some(key) = &table_key {
            let entry = by_key
                .entry(key.clone())
                .or_insert_with(|| TablePart {
                    key: key.clone(),
                    name: name.clone().unwrap_or_default(),
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
    pub(crate) has_ddl_drop: bool,
    pub(crate) has_ddl_create: bool,
    pub(crate) has_data: bool,
    pub(crate) has_constraints: bool,
    pub(crate) seen_order: usize,
}

/// 取出「CREATE TABLE <name>」等语句后的第一个标识符（去引号），返回 (name, schema)。
fn classify(
    kind: DatabaseKind,
    upper: &str,
    raw: &str,
) -> (StmtKind, Option<String>, Option<String>) {
    // 语句前缀到类别的映射
    // 标识符从 `raw`（原文）读取以保留大小写；类别判定用 `upper`。
    if upper.starts_with("DROP TABLE") || upper.starts_with("CREATE TABLE") {
        return (StmtKind::TableDdl, read_first_ident(raw, 2), None);
    }
    if upper.starts_with("INSERT INTO") {
        return (StmtKind::TableData, read_first_ident(raw, 2), None);
    }
    if upper.starts_with("ALTER TABLE") {
        if upper.contains("ADD CONSTRAINT") {
            return (StmtKind::TableConstraint, read_first_ident(raw, 2), None);
        }
        if upper.ends_with("DISABLE KEYS") || upper.ends_with("ENABLE KEYS") {
            return (StmtKind::TableDdl, read_first_ident(raw, 2), None);
        }
        return (StmtKind::TableConstraint, read_first_ident(raw, 2), None);
    }
    if upper.starts_with("CREATE UNIQUE INDEX") || upper.starts_with("CREATE INDEX") {
        return (StmtKind::TableIndex, read_index_target(raw), None);
    }
    if upper.starts_with("SELECT PG_CATALOG.SETVAL") || upper.starts_with("SELECT SETVAL") {
        return (StmtKind::SequenceSetval, None, None);
    }
    if upper.starts_with("CREATE SCHEMA") {
        return (StmtKind::Schema, None, None);
    }
    if upper.starts_with("CREATE SEQUENCE") {
        return (
            StmtKind::Sequence,
            read_first_ident(raw, 2),
            Some("public".to_string()),
        );
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

/// 读取「CREATE TABLE / INSERT INTO / ALTER TABLE」后的第一个标识符（可带 schema.表 或 `引号`）。
/// 返回裸表名（保原大小写）。
fn read_first_ident(raw: &str, skip_words: usize) -> Option<String> {
    // 去掉前 skip_words 个词（CREATE TABLE / INSERT INTO / ALTER TABLE / DROP TABLE / CREATE SEQUENCE）
    let mut words = raw.split_whitespace().skip(skip_words);
    let mut tok = words.next()?;
    // 跳过可选的 ONLY / IF EXISTS（不区分大小写；IF 后可能跟 EXISTS）
    loop {
        let up = tok.to_ascii_uppercase();
        if up == "ONLY" || up == "IF" || up == "EXISTS" {
            tok = words.next()?;
        } else {
            break;
        }
    }
    // 去掉反引号/双引号
    let cleaned = tok
        .trim_start_matches('`')
        .trim_end_matches('`')
        .trim_start_matches('"')
        .trim_end_matches('"');
    // 取最后一段（schema.表 取表名；裸名取自身）
    cleaned.rsplit('.').next().map(|s| s.to_string())
}

/// 从「CREATE INDEX <idx> ON <table>」里取表名（不区分大小写定位，返回原大小写）。
fn read_index_target(raw: &str) -> Option<String> {
    let upper = raw.to_ascii_uppercase();
    let idx = upper.find(" ON ")?;
    let rest = &raw[idx + 4..];
    let tok = rest.split_whitespace().next()?;
    let cleaned = tok
        .trim_start_matches('`')
        .trim_end_matches('`')
        .trim_start_matches('"')
        .trim_end_matches('"');
    Some(cleaned.rsplit('.').next()?.to_string())
}

/// 依决策重建可执行脚本。返回 (sql, 追加的警告)。
///
/// 规则：
/// - Overwrite(t)：输出 t 的全部语句（DROP+CREATE+data+constraint+index）。
/// - Append(t)：只输出 t 的数据语句（INSERT）；目标需已有同名表。
/// - Skip(t)：不输出 t 的任何语句（含其约束/索引/序列 setval）。
/// - 非表/会话级语句（SET、LOCK、Schema、Sequence）恒输出，保证 DDL 可解析。
/// - FK 修剪：约束语句若其引用表未被 Overwrite，则剔除该约束并告警（避免引用缺失表）。
pub(crate) fn reconstruct_sql(
    stmts: &[PStmt],
    decisions: &[PerTableDecision],
) -> (String, Vec<String>) {
    use std::collections::HashMap;
    let action = |key: &str| -> Option<RestoreTableAction> {
        decisions
            .iter()
            .find(|d| d.table == key)
            .map(|d| d.action)
    };
    let mut by_key: HashMap<&str, RestoreTableAction> = decisions
        .iter()
        .map(|d| (d.table.as_str(), d.action))
        .collect();
    let _ = &mut by_key;
    let mut out = String::new();
    let mut warnings = Vec::new();
    for s in stmts {
        match s.kind {
            StmtKind::NonTable | StmtKind::Schema | StmtKind::Sequence => {
                out.push_str(&s.raw);
                out.push_str(";\n");
            }
            StmtKind::TableDdl | StmtKind::TableData | StmtKind::TableIndex => {
                let Some(key) = s.table_key.as_deref() else {
                    continue;
                };
                match action(key) {
                    Some(RestoreTableAction::Overwrite) => {
                        out.push_str(&s.raw);
                        out.push_str(";\n");
                    }
                    Some(RestoreTableAction::Append) if s.kind == StmtKind::TableData => {
                        out.push_str(&s.raw);
                        out.push_str(";\n");
                    }
                    _ => {}
                }
            }
            StmtKind::TableConstraint => {
                let Some(key) = s.table_key.as_deref() else {
                    continue;
                };
                let Some(act) = action(key) else {
                    continue;
                };
                if act != RestoreTableAction::Overwrite {
                    continue;
                }
                // FK 修剪：REFERENCES 引用的表若非 Overwrite，剔除该约束。
                let refs = referenced_tables(&s.raw);
                let all_overwrite = refs
                    .iter()
                    .all(|r| action(r) == Some(RestoreTableAction::Overwrite));
                if refs.is_empty() || all_overwrite {
                    out.push_str(&s.raw);
                    out.push_str(";\n");
                } else {
                    warnings.push(format!(
                        "表 {key} 的部分外键约束因引用表未覆盖（追加/跳过）而未恢复"
                    ));
                }
            }
            StmtKind::SequenceSetval => {
                // 序列 setval：归属表被跳过才剔除；否则保留（目标序列已建）。
                let keep = match setval_table(&s.raw).and_then(|t| action(&t)) {
                    Some(RestoreTableAction::Skip) => false,
                    _ => true,
                };
                if keep {
                    out.push_str(&s.raw);
                    out.push_str(";\n");
                }
            }
        }
    }
    (out, warnings)
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

