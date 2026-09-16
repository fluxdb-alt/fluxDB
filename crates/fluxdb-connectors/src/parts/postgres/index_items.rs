/// 从完整索引定义提取键项列表（只读顶层括号内的逗号分隔片段，尊重嵌套括号/引号）。
fn pg_split_index_keys(definition: &str) -> Vec<String> {
    let Some(open) = definition.find('(') else {
        return Vec::new();
    };
    // 找到与该开括号配对的闭括号（CREATE INDEX 的键列列表）。
    let mut depth = 0usize;
    let mut in_squote = false;
    let mut close = None;
    for (i, ch) in definition[open..].char_indices() {
        let c = ch;
        if c == '\'' && !in_squote {
            in_squote = true;
        } else if c == '\'' && in_squote {
            in_squote = false;
        } else if !in_squote {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
        }
    }
    let Some(close) = close else {
        return Vec::new();
    };
    let body = &definition[open + 1..open + close];
    split_top_level_commas(body)
}

/// 按顶层逗号切分（不进嵌套括号）。
fn split_top_level_commas(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut in_squote = false;
    for (i, c) in body.char_indices() {
        match c {
            '\'' => in_squote = !in_squote,
            _ if !in_squote && c == '(' => depth += 1,
            _ if !in_squote && c == ')' => depth -= 1,
            _ if !in_squote && c == ',' && depth == 0 => {
                parts.push(body[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(body[start..].trim().to_string());
    parts.retain(|p| !p.is_empty());
    parts
}

/// 解析单条键项文本：剥离 ` DESC`/` ASC`/` NULLS LAST|FIRST` 方向后缀。
/// `is_expression` 来自 `indkey` 的 attnum==0（表达式键项），不靠括号猜测。
fn pg_parse_index_item(item: &str, is_expression: bool) -> IndexColumnItem {
    let mut text = item.trim();
    let mut descending = false;
    let mut nulls_first = false;
    // 尾部顺序修饰循环剥到无修饰为止。
    loop {
        if let Some(rest) = strip_suffix_ci(text, " DESC") {
            descending = true;
            text = rest;
        } else if let Some(rest) = strip_suffix_ci(text, " ASC") {
            text = rest;
        } else if let Some(rest) = strip_suffix_ci(text, " NULLS LAST") {
            text = rest;
            nulls_first = false;
        } else if let Some(rest) = strip_suffix_ci(text, " NULLS FIRST") {
            text = rest;
            nulls_first = true;
        } else {
            break;
        }
    }
    IndexColumnItem {
        column: (!is_expression).then(|| text.to_string()),
        expression: is_expression.then(|| text.to_string()),
        descending,
        nulls_first,
    }
}

fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let s = s.trim_end();
    s.strip_suffix(suffix).map(|r| r.trim_end())
}

/// 读取索引：键项（列或表达式）+ INCLUDE + predicate + 方法 + 有效状态 + 完整定义。
async fn pg_load_indexes(
    client: &tokio_postgres::Client,
    rel_oid: u32,
) -> fluxdb_core::Result<Vec<IndexMeta>> {
    let rows = client
        .query(
            "SELECT ic.relname AS index_name, i.indisunique, i.indisprimary, i.indisvalid, \
                    am.amname, pg_catalog.pg_get_expr(i.indpred, i.indrelid) AS predicate, \
                    pg_catalog.pg_get_indexdef(i.indexrelid) AS def, \
                    i.indnkeyatts::int4, i.indkey::int2[] \
             FROM pg_catalog.pg_index i \
             JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
             JOIN pg_catalog.pg_class tc ON tc.oid = i.indrelid \
             JOIN pg_catalog.pg_am am ON am.oid = ic.relam \
             WHERE i.indrelid = $1 ORDER BY ic.relname",
            &[&rel_oid],
        )
        .await
        .map_err(pg_error)?;

    let mut indexes = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.get(0);
        let is_unique: bool = row.get(1);
        let is_primary: bool = row.get(2);
        let valid: bool = row.get(3);
        let index_type: String = row.get(4);
        let predicate: Option<String> = row.get(5);
        let definition: String = row.get(6);
        let indnkeyatts: usize = row.get::<_, i32>(7) as usize;
        let indkey: Vec<i16> = row.get(8);

        // 键项：从完整定义按序取该位置的原始片段（列名或表达式）；
        // 表达式身份以 `indkey` 的 attnum==0 为准（不靠括号猜测），方向后缀一并剥离。
        let key_fragments = pg_split_index_keys(&definition);
        let mut columns: Vec<IndexColumnItem> = Vec::new();
        for (pos, attnum) in indkey.iter().enumerate().take(indnkeyatts) {
            let fragment = key_fragments.get(pos).cloned().unwrap_or_default();
            columns.push(pg_parse_index_item(&fragment, *attnum == 0));
        }

        // INCLUDE 列：键项之后的 attnum 全部为命名列。
        let mut include_columns: Vec<String> = Vec::new();
        for attnum in indkey.iter().skip(indnkeyatts) {
            if *attnum > 0 {
                if let Some(col) = pg_attnum_name(client, rel_oid, *attnum as i32).await? {
                    include_columns.push(col);
                }
            }
        }

        indexes.push(IndexMeta {
            name,
            columns,
            include_columns,
            is_unique,
            is_primary,
            index_type: Some(index_type).filter(|t| !t.is_empty()),
            predicate: predicate.filter(|p| !p.trim().is_empty()),
            valid,
            definition,
        });
    }
    Ok(indexes)
}

async fn pg_attnum_name(
    client: &tokio_postgres::Client,
    rel_oid: u32,
    attnum: i32,
) -> fluxdb_core::Result<Option<String>> {
    let row = client
        .query_opt(
            "SELECT a.attname FROM pg_catalog.pg_attribute a \
             WHERE a.attrelid = $1 AND a.attnum = $2 AND a.attnum > 0 AND NOT a.attisdropped",
            &[&rel_oid, &attnum],
        )
        .await
        .map_err(pg_error)?;
    Ok(row.map(|r| r.get(0)))
}
