// ER 图导出（§十）：Mermaid / DBML / SVG / JSON。
//
// 均为无副作用纯函数（输入 ErGraphData + 可选的坐标/标题等 -> String），
// 便于单测；UI 侧把生成文本写入剪贴板并提示。JSON 带 `schema_version` 与连接绑定信息，
// 但不含密码/密钥；SVG 为展示型导出（表级连线简化，非 field-precise 路由，见函数注释）。
//
// 无 D1-D9 逻辑关系模型时，导出内容为「结构快照 + 现有物理外键关系」；导入另行按
// schema_version 校验，undo D1-D9 确认后才谈逻辑关系导入。有损项明确标注。

/// 导出的 JSON schema 版本（升级时递增，导入按此兼容）。
pub const ER_EXPORT_SCHEMA_VERSION: u32 = 2;

/// Mermaid erDiagram 导出：实体（列）+ 关系。
/// ```
/// erDiagram
///   CUSTOMERS { bigint id PK, text name }
///   ORDERS { bigint id PK, bigint customer_id FK }
///   ORDERS }o--|| CUSTOMERS : fk_orders_customer
/// ```
pub fn er_export_mermaid(graph: &ErGraphData) -> String {
    let mut out = String::from("erDiagram\n");
    for t in &graph.tables {
        let id = mermaid_id(&t.name);
        out.push_str(&format!("  {id} {{\n"));
        for c in &t.columns {
            let flags = if c.primary_key {
                " PK"
            } else if column_is_fk(graph, t, c.name.as_str()) {
                " FK"
            } else {
                ""
            };
            let ty = c
                .type_name
                .as_deref()
                .map(|s| s.replace('"', "'"))
                .unwrap_or_else(|| "?".to_string());
            out.push_str(&format!("    {ty} \"{}\"{} \n", esc_mermaid(c.name.as_str()), flags));
        }
        out.push_str("  }\n");
    }
    // 关系：按约束名去重（复合键多列同约束只画一条表级关系，标注字段）。
    let mut seen = std::collections::BTreeSet::new();
    for e in &graph.edges {
        let key = (e.name.as_str(), e.from_table.as_str(), e.to_table.as_str());
        if !seen.insert(key) {
            continue;
        }
        let a = mermaid_id(&e.from_table);
        let b = mermaid_id(&e.to_table);
        let label = mermaid_label(&e.name);
        // 简化：去重表级关系，1 对外键方向。
        out.push_str(&format!("  {a} }}o--o| {b} : {label}\n"));
    }
    out
}

/// DBML 导出：Table { ... } + Ref。
pub fn er_export_dbml(graph: &ErGraphData) -> String {
    let mut out = String::new();
    for t in &graph.tables {
        out.push_str(&format!("Table {} {{\n", dbml_ident(&t.name)));
        for c in &t.columns {
            let ty = c.type_name.as_deref().unwrap_or("?");
            let pk = if c.primary_key { " [pk]" } else { "" };
            let notnull = if !c.nullable { " [not null]" } else { "" };
            out.push_str(&format!("  {} {}{}{}\n", dbml_ident(c.name.as_str()), ty, pk, notnull));
        }
        out.push_str("}\n");
    }
    let mut seen = std::collections::BTreeSet::new();
    for e in &graph.edges {
        let key = (e.name.as_str(), e.from_table.as_str(), e.to_table.as_str());
        if !seen.insert(key) {
            continue;
        }
        out.push_str(&format!(
            "Ref: {}.{} > {}.{} [name: {}]\n",
            dbml_ident(&e.from_table),
            dbml_ident(&e.from_column),
            dbml_ident(&e.to_table),
            dbml_ident(&e.to_column),
            dbml_ident(&e.name),
        ));
    }
    out
}

/// 生成 JSON 导出（结构快照 + 坐标/固定等视图状态），带 schema_version 与连接绑定。
/// `connection_label`/`database` 仅展示用，不含密码/凭证。
pub fn er_export_json(
    graph: &ErGraphData,
    positions: &std::collections::BTreeMap<String, (f32, f32)>,
    pinned: &std::collections::BTreeSet<String>,
    connection_label: &str,
    connection_id: u64,
    database: &str,
) -> String {
    let tables_json: Vec<String> = graph
        .tables
        .iter()
        .map(|t| {
            let cols: Vec<String> = t
                .columns
                .iter()
                .map(|c| {
                    format!(
                        "{{\"name\":{},\"type\":{},\"pk\":{},\"nullable\":{}}}",
                        json_str(c.name.as_str()),
                        json_str(c.type_name.as_deref().unwrap_or("")),
                        if c.primary_key { "true" } else { "false" },
                        if c.nullable { "true" } else { "false" },
                    )
                })
                .collect();
            let (x, y) = positions.get(&t.name).copied().unwrap_or((0.0, 0.0));
            format!(
                "{{\"name\":{},\"comment\":{},\"x\":{},\"y\":{},\"pinned\":{},\"columns\":[{}]}}",
                json_str(&t.name),
                json_str(t.comment.as_deref().unwrap_or("")),
                fmt_f32(x),
                fmt_f32(y),
                if pinned.contains(&t.name) { "true" } else { "false" },
                cols.join(","),
            )
        })
        .collect();
    let edges_json: Vec<String> = graph
        .edges
        .iter()
        .map(|e| {
            format!(
                "{{\"name\":{},\"from_table\":{},\"from_column\":{},\"to_table\":{},\"to_column\":{}}}",
                json_str(e.name.as_str()),
                json_str(&e.from_table),
                json_str(&e.from_column),
                json_str(&e.to_table),
                json_str(&e.to_column),
            )
        })
        .collect();
    format!(
        "{{\"format\":\"fluxdb-er\",\"schema_version\":{},\"exported_at\":{},\"connection\":{},\"connection_id\":{},\"database\":{},\"tables\":[{}],\"edges\":[{}]}}",
        ER_EXPORT_SCHEMA_VERSION,
        json_str(&er_now_utc()),
        json_str(connection_label),
        connection_id,
        json_str(database),
        tables_json.join(","),
        edges_json.join(","),
    )
}

/// SVG 导出（展示型）：卡片 + 表级连线。旋转到左上为正的 bounding box。
/// 有损说明：连线为表级简化（非 field-precise 路由），布局/坐标/主题不保证与画布像素一致。
pub fn er_export_svg(
    graph: &ErGraphData,
    positions: &std::collections::BTreeMap<String, (f32, f32)>,
    title: &str,
) -> String {
    // 计算 bbox 与偏移。
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut h_of: std::collections::BTreeMap<&str, f32> = std::collections::BTreeMap::new();
    for t in &graph.tables {
        let (x, y) = positions.get(&t.name).copied().unwrap_or((0.0, 0.0));
        let h = card_height(t.status, t.columns.len());
        h_of.insert(t.name.as_str(), h);
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + NODE_WIDTH);
        max_y = max_y.max(y + h);
    }
    if !min_x.is_finite() {
        min_x = 0.0;
        min_y = 0.0;
        max_x = 300.0;
        max_y = 200.0;
    }
    let pad = 24.0;
    let w = (max_x - min_x + pad * 2.0).ceil() as u32;
    let h = (max_y - min_y + pad * 2.0).ceil() as u32;
    let off_x = pad - min_x;
    let off_y = pad - min_y;
    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\" font-family=\"sans-serif\">\n",
        w, h, w, h
    ));
    out.push_str(&format!("  <text x=\"{}\" y=\"20\" font-size=\"14\" font-weight=\"bold\">{}</text>\n", pad, svg_esc(title)));
    // 边（表级连线：按两端 x 左右侧出/入，含自关联回路说明）。
    let mut seen = std::collections::BTreeSet::new();
    for e in &graph.edges {
        let key = (e.name.as_str(), e.from_table.as_str(), e.to_table.as_str());
        if !seen.insert(key) {
            continue;
        }
        let (f_from, f_to) = (e.from_table.as_str(), e.to_table.as_str());
        let (a, b) = (positions.get(&e.from_table).copied().unwrap_or((0.0, 0.0)), positions.get(&e.to_table).copied().unwrap_or((0.0, 0.0)));
        let a_h = h_of.get(f_from).copied().unwrap_or(100.0);
        let b_h = h_of.get(f_to).copied().unwrap_or(100.0);
        let (x1, y1, x2, y2) = if f_from == f_to {
            // 自关联：右侧小回路。
            (a.0 + NODE_WIDTH + off_x, a.1 + 12.0 + off_y, a.0 + NODE_WIDTH + off_x, a.1 + 30.0 + off_y)
        } else if b.0 >= a.0 + NODE_WIDTH {
            // b 在右：a 右出 → b 左入。
            (a.0 + NODE_WIDTH + off_x, a.1 + a_h / 2.0 + off_y, b.0 + off_x, b.1 + b_h / 2.0 + off_y)
        } else if a.0 >= b.0 + NODE_WIDTH {
            (b.0 + NODE_WIDTH + off_x, b.1 + b_h / 2.0 + off_y, a.0 + off_x, a.1 + a_h / 2.0 + off_y)
        } else {
            // 横向重叠：并列两端各出。
            (a.0 + NODE_WIDTH + off_x, a.1 + 8.0 + off_y, b.0 + NODE_WIDTH + off_x, b.1 + 8.0 + off_y)
        };
        out.push_str(&format!(
            "  <path d=\"M {:.1} {:.1} L {:.1} {:.1}\" stroke=\"#888\" stroke-width=\"1\" fill=\"none\"/>\n",
            x1, y1, x2, y2
        ));
        let mx = (x1 + x2) / 2.0;
        let my = (y1 + y2) / 2.0 - 4.0;
        out.push_str(&format!(
            "  <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#666\" text-anchor=\"middle\">{}</text>\n",
            mx, my, svg_esc(&format!("{}.{}", e.from_column, e.to_column))
        ));
    }
    // 卡片。
    for t in &graph.tables {
        let (x, y) = positions.get(&t.name).copied().unwrap_or((0.0, 0.0));
        let chh = h_of.get(t.name.as_str()).copied().unwrap_or(100.0);
        let cx = x + off_x;
        let cy = y + off_y;
        out.push_str(&format!(
            "  <rect x=\"{:.1}\" y=\"{:.1}\" width=\"{}\" height=\"{:.1}\" rx=\"6\" fill=\"#f7f7f8\" stroke=\"#c9c9cf\"/>\n",
            cx, cy, NODE_WIDTH, chh
        ));
        out.push_str(&format!(
            "  <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\" font-weight=\"bold\">{}</text>\n",
            cx + 8.0, cy + 16.0, svg_esc(&t.name)
        ));
        let mut ry = cy + 28.0;
        for c in &t.columns {
            let mark = if c.primary_key { "🔑 " } else if column_is_fk(graph, t, c.name.as_str()) { "🔗 " } else { "" };
            let ty = c.type_name.as_deref().unwrap_or("?");
            out.push_str(&format!(
                "  <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"9\" fill=\"#333\">{}</text>\n",
                cx + 8.0, ry, svg_esc(&format!("{mark}{} {}", c.name, ty))
            ));
            ry += 12.0;
        }
    }
    out.push_str("</svg>\n");
    out
}

fn column_is_fk(graph: &ErGraphData, t: &fluxdb_core::ErTableNode, col: &str) -> bool {
    graph
        .edges
        .iter()
        .any(|e| e.from_table == t.name && e.from_column == col)
}

// ---- 转义/格式化辅助 ----

fn mermaid_id(name: &str) -> String {
    // Mermaid 实体名用 {type}name{type}；这里统一用引号包裹的安全名。
    format!("[{name}]")
}

fn mermaid_label(s: &str) -> String {
    esc_mermaid(s)
}

fn esc_mermaid(s: &str) -> String {
    s.replace('"', "'")
}


fn dbml_ident(s: &str) -> String {
    // 仅当可能是保留字/特殊字符时加双引号。
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('"', "\\\""))
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn svg_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn fmt_f32(v: f32) -> String {
    if v.fract().abs() < 1e-4 {
        format!("{:.0}", v)
    } else {
        format!("{:.2}", v)
    }
}

/// 导出时间戳（UTC，稳定文本）。
fn er_now_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 可读 UTC。
    let days = secs / 86400;
    let rem = secs % 86400;
    format!("{days}d{rem:07}sZ")
}

// ---- 导入（与上面的 JSON 导出互为逆操作）----

/// 一次导入解析结果：结构（表/列/边）+ 视图坐标/固定 + 与当前图的差异预览。
/// 纯解析校验，不写数据库、不改当前模型；「导入失败不破坏现有模型」由调用方保证
/// （先解析校验，全部通过后才允许应用到画布/持久化）。
#[derive(Debug)]
pub struct ErImportReport {
    /// 解析出的表（名字 -> 列）。
    pub tables: std::collections::BTreeMap<String, Vec<fluxdb_core::ErColumn>>,
    /// 边的展示名索引（from_table.from_column -> to_table.to_column）。
    pub edges: Vec<(String, String, String, String)>,
    /// 视图坐标（表名 -> (x, y)）。
    pub positions: std::collections::BTreeMap<String, (f32, f32)>,
    pub pinned: std::collections::BTreeSet<String>,
    /// 导入文件声明的 database（连接绑定校验用）。
    pub database: String,
    /// v2 连接身份；旧版无此字段只可预览，不能应用到另一个连接。
    pub connection_id: Option<u64>,
    /// 差异预览（相对当前图）：新增表、缺失表、未解析边（两端/列不在图内）。
    pub added_tables: Vec<String>,
    pub missing_tables: Vec<String>,
    pub unresolved_edges: Vec<String>,
}

/// 解析并校验一段 FluxDB ER JSON 导出（§十）。
///
/// 校验：`format == "fluxdb-er"`、`schema_version <= ER_EXPORT_SCHEMA_VERSION`、
/// 结构引用完整（边两端表/列都能在 tables 里找到）。不做相似名猜测、不自动补缺。
/// 失败返回中文错误；调用方据此提示「导入失败，未改动当前模型」。
pub fn er_import_parse(
    json: &str,
    current_graph: &fluxdb_core::ErGraphData,
    _current_database: &str,
) -> Result<ErImportReport, String> {
    if json.len() > 16 * 1024 * 1024 {
        return Err("ER JSON 超过 16 MiB 上限".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("JSON 解析失败：{e}"))?;
    let fmt = value
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if fmt != "fluxdb-er" {
        return Err(format!("不是 FluxDB ER 导出文件（format={fmt:?}）"));
    }
    let version = value.get("schema_version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version == 0 || version > ER_EXPORT_SCHEMA_VERSION as u64 {
        return Err(format!(
            "不支持的 schema_version={version}（当前支持 ≤{ER_EXPORT_SCHEMA_VERSION}）"
        ));
    }
    let connection_id = value.get("connection_id").and_then(|v| v.as_u64());
    if version >= 2 && connection_id.is_none() {
        return Err("ER JSON 缺少连接身份，不能安全应用".into());
    }
    let database = value
        .get("database")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    // 解析表 + 列。
    let mut tables: std::collections::BTreeMap<String, Vec<fluxdb_core::ErColumn>> = Default::default();
    if let Some(arr) = value.get("tables").and_then(|v| v.as_array()) {
        for t in arr {
            let Some(name) = t.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let mut cols = Vec::new();
            if let Some(cs) = t.get("columns").and_then(|v| v.as_array()) {
                for c in cs {
                    cols.push(fluxdb_core::ErColumn {
                        name: c.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        type_name: {
                            let ty = c.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            if ty.is_empty() { None } else { Some(ty.to_string()) }
                        },
                        primary_key: c.get("pk").and_then(|v| v.as_bool()).unwrap_or(false),
                        nullable: c.get("nullable").and_then(|v| v.as_bool()).unwrap_or(false),
                        stable: None,
                    });
                }
            }
            if tables.insert(name.to_string(), cols).is_some() {
                return Err(format!("ER JSON 包含重复表名：{name}"));
            }
        }
    }
    // 解析边，并校验两端表/列引用存在（缺失记入 unresolved_edges，不静默丢弃）。
    let mut edges: Vec<(String, String, String, String)> = Vec::new();
    let mut unresolved_edges: Vec<String> = Vec::new();
    if let Some(arr) = value.get("edges").and_then(|v| v.as_array()) {
        for e in arr {
            let f = e.get("from_table").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let fc = e.get("from_column").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let t = e.get("to_table").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let tc = e.get("to_column").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let f_cols = tables.get(&f).map(|c| c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>());
            let t_cols = tables.get(&t).map(|c| c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>());
            let f_ok = f_cols.as_ref().is_some_and(|cs| cs.contains(&fc.as_str()));
            let t_ok = t_cols.as_ref().is_some_and(|cs| cs.contains(&tc.as_str()));
            if !f_ok || !t_ok {
                unresolved_edges.push(format!("{f}.{fc} → {t}.{tc}"));
                continue;
            }
            edges.push((f, fc, t, tc));
        }
    }
    // 视图像素坐标 / 固定。
    let mut positions: std::collections::BTreeMap<String, (f32, f32)> = Default::default();
    let mut pinned: std::collections::BTreeSet<String> = Default::default();
    if let Some(arr) = value.get("tables").and_then(|v| v.as_array()) {
        for t in arr {
            let Some(name) = t.get("name").and_then(|v| v.as_str()) else { continue; };
            let x = t.get("x").and_then(|v| v.as_f64()).ok_or_else(|| format!("表 {name} 缺少有效 x 坐标"))?;
            let y = t.get("y").and_then(|v| v.as_f64()).ok_or_else(|| format!("表 {name} 缺少有效 y 坐标"))?;
            if !x.is_finite() || !y.is_finite() || x.abs() > 1e9 || y.abs() > 1e9 {
                return Err(format!("表 {name} 坐标超出可用范围"));
            }
            let (x, y) = (x as f32, y as f32);
            positions.insert(name.to_string(), (x, y));
            if t.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false) {
                pinned.insert(name.to_string());
            }
        }
    }
    // 差异预览。
    let current_names: std::collections::BTreeSet<String> =
        current_graph.tables.iter().map(|t| t.name.clone()).collect();
    let imported_names: std::collections::BTreeSet<String> = tables.keys().cloned().collect();
    let added_tables: Vec<String> = imported_names.difference(&current_names).cloned().collect();
    let missing_tables: Vec<String> = current_names.difference(&imported_names).cloned().collect();

    Ok(ErImportReport {
        tables,
        edges,
        positions,
        pinned,
        database,
        connection_id,
        added_tables,
        missing_tables,
        unresolved_edges,
    })
}

/// 是否属于「本连接当前 database」的导入（连接绑定校验，§十）。跨库导入由调用方提示，
/// 不自动重绑另一连接。
pub fn er_import_database_matches(report: &ErImportReport, current_database: &str) -> bool {
    report.database.is_empty() || report.database == current_database
}

/// 应用导入布局必须双重校验连接与库；旧版无身份只能预览，不能按名称猜接。
pub fn er_import_scope_matches(report: &ErImportReport, connection_id: u64, database: &str) -> bool {
    report.connection_id == Some(connection_id) && report.database == database
}


/// 导入仅改变当前图中同名表的位置/固定，未匹配的表与关系保持原状。
pub fn er_import_layout_merge(
    report: &ErImportReport,
    graph: &fluxdb_core::ErGraphData,
    current_positions: &std::collections::BTreeMap<String, (f32, f32)>,
    current_pinned: &std::collections::BTreeSet<String>,
) -> (std::collections::BTreeMap<String, (f32, f32)>, std::collections::BTreeSet<String>, usize) {
    let mut positions = current_positions.clone();
    let mut pinned = current_pinned.clone();
    let mut count = 0;
    for table in &graph.tables {
        let Some(&position) = report.positions.get(&table.name) else { continue; };
        positions.insert(table.name.clone(), position);
        if report.pinned.contains(&table.name) { pinned.insert(table.name.clone()); }
        else { pinned.remove(&table.name); }
        count += 1;
    }
    (positions, pinned, count)
}

#[cfg(test)]
mod export_tests {
    use super::*;

    fn mk_graph() -> fluxdb_core::ErGraphData {
        let mk = |name: &str, cols: &[(&str, bool, bool)]| fluxdb_core::ErTableNode {
            name: name.to_string(),
            reference: fluxdb_core::ErTableRef {
                database: "db".into(),
                schema: None,
                name: name.to_string(),
            },
            comment: None,
            stable: None,
            status: fluxdb_core::ErLoadStatus::Loaded,
            columns: cols
                .iter()
                .map(|(n, pk, nullable)| fluxdb_core::ErColumn {
                    name: n.to_string(),
                    type_name: Some("bigint".to_string()),
                    primary_key: *pk,
                    nullable: *nullable,
                    stable: None,
                })
                .collect(),
        };
        let edge = |name: &str, f: &str, fc: &str, t: &str, tc: &str| fluxdb_core::ErForeignKeyEdge {
            name: name.to_string(),
            from_table: f.to_string(),
            from_column: fc.to_string(),
            to_table: t.to_string(),
            to_column: tc.to_string(),
            from_reference: fluxdb_core::ErTableRef { database: "db".into(), schema: None, name: f.into() },
            to_reference: fluxdb_core::ErTableRef { database: "db".into(), schema: None, name: t.into() },
        };
        fluxdb_core::ErGraphData {
            tables: vec![
                mk("customers", &[("id", true, false), ("name", false, true)]),
                mk("orders", &[("id", true, false), ("customer_id", false, false)]),
            ],
            edges: vec![edge("fk_orders_customer", "orders", "customer_id", "customers", "id")],
            relation_status: fluxdb_core::ErLoadStatus::Loaded,
        }
    }

    #[test]
    fn mermaid_contains_tables_and_relation() {
        let s = er_export_mermaid(&mk_graph());
        assert!(s.contains("[customers]")); // mermaid 用括号实体名
        assert!(s.contains("id"));
        assert!(s.contains("fk_orders_customer"));
    }

    #[test]
    fn dbml_has_tables_and_ref() {
        let s = er_export_dbml(&mk_graph());
        assert!(s.contains("Table customers"));
        assert!(s.contains("customer_id"));
        assert!(s.contains("Ref: orders.customer_id > customers.id"));
    }

    #[test]
    fn json_has_schema_version_and_no_fields_concatenated() {
        let pos = std::collections::BTreeMap::from([
            ("customers".to_string(), (10.0f32, 10.0f32)),
            ("orders".to_string(), (300.0, 10.0)),
        ]);
        let pinned = std::collections::BTreeSet::from(["orders".to_string()]);
        let s = er_export_json(&mk_graph(), &pos, &pinned, "conn", 7, "db");
        assert!(s.contains("\"schema_version\":2"));
        assert!(s.contains("\"pinned\":true"));
        // 每列独立对象，不把 PK/类型拼进列名。
        assert!(s.contains("\"columns\":[{\"name\":\"id\""));
    }

    #[test]
    fn svg_escapes_and_has_card() {
        let pos = std::collections::BTreeMap::from([
            ("customers".to_string(), (10.0f32, 10.0f32)),
            ("orders".to_string(), (300.0, 10.0)),
        ]);
        let s = er_export_svg(&mk_graph(), &pos, "demo & <er>");
        assert!(s.starts_with("<svg"));
        assert!(s.contains("&amp;"));
        assert!(s.contains("<rect"));
        assert!(s.contains("customer_id"));
    }

    #[test]
    fn json_export_import_roundtrip_recovers_tables_edges_and_views() {
        let pos = std::collections::BTreeMap::from([
            ("customers".to_string(), (10.0f32, 10.0f32)),
            ("orders".to_string(), (300.0, 10.0)),
        ]);
        let pinned = std::collections::BTreeSet::from(["orders".to_string()]);
        let s = er_export_json(&mk_graph(), &pos, &pinned, "conn", 7, "db");
        // 导入到空图（无当前结构），应完整还原表/列/边/坐标/固定，无未解析边。
        let empty = fluxdb_core::ErGraphData {
            tables: Vec::new(),
            edges: Vec::new(),
            relation_status: fluxdb_core::ErLoadStatus::NotLoaded,
        };
        let report = er_import_parse(&s, &empty, "db").expect("round-trip 可解析");
        assert_eq!(report.database, "db");
        assert!(er_import_database_matches(&report, "db"));
        assert!(er_import_scope_matches(&report, 7, "db"));
        assert!(!er_import_scope_matches(&report, 8, "db"));
        assert!(!er_import_scope_matches(&report, 7, "other"));
        assert!(report.tables.contains_key("customers"));
        assert!(report.tables.contains_key("orders"));
        assert_eq!(report.tables["orders"].len(), 2);
        // 边完整还原。
        assert_eq!(report.edges, vec![("orders".to_string(), "customer_id".to_string(), "customers".to_string(), "id".to_string())]);
        assert!(report.unresolved_edges.is_empty());
        // 坐标/固定还原。
        assert_eq!(report.positions.get("orders"), Some(&(300.0_f32, 10.0_f32)));
        assert!(report.pinned.contains("orders"));
    }

    #[test]
    fn apply_import_layout_only_updates_current_tables() {
        let graph = mk_graph();
        let exported = er_export_json(&graph, &std::collections::BTreeMap::from([
            ("orders".into(), (400.0, 20.0)), ("ghost".into(), (1.0, 2.0)),
        ]), &std::collections::BTreeSet::new(), "c", 7, "db");
        let report = er_import_parse(&exported, &graph, "db").unwrap();
        let existing = std::collections::BTreeMap::from([
            ("orders".into(), (10.0, 10.0)), ("customers".into(), (50.0, 50.0)),
        ]);
        let pins = std::collections::BTreeSet::from(["orders".into(), "customers".into()]);
        let (positions, pinned, count) = er_import_layout_merge(&report, &graph, &existing, &pins);
        assert_eq!(count, 2);
        assert_eq!(positions["orders"], (400.0, 20.0));
        assert_eq!(positions["customers"], (0.0, 0.0));
        assert!(!positions.contains_key("ghost"));
        assert!(pinned.is_empty());
    }

    #[test]
    fn import_missing_edge_reference_marked_unresolved_not_dropped() {
        // 手工构造引用了不存在列的边：导入应保留为 unresolved 边（不静默丢弃）。
        let json = r#"{
          "format":"fluxdb-er","schema_version":1,"exported_at":"1d0000000Z",
          "connection":"c","database":"db",
          "tables":[{"name":"orders","comment":"","x":0,"y":0,"pinned":false,"columns":[{"name":"id","type":"bigint","pk":true,"nullable":false}]}],
          "edges":[{"name":"fk","from_table":"orders","from_column":"ghost_col","to_table":"nonexistent","to_column":"id"}]
        }"#;
        let empty = fluxdb_core::ErGraphData {
            tables: Vec::new(),
            edges: Vec::new(),
            relation_status: fluxdb_core::ErLoadStatus::NotLoaded,
        };
        let report = er_import_parse(json, &empty, "db").expect("可解析");
        assert!(report.edges.is_empty());
        assert_eq!(report.unresolved_edges.len(), 1);
        assert!(report.unresolved_edges[0].contains("ghost_col"));
        assert!(!er_import_scope_matches(&report, 7, "db"), "旧版仅预览，不按连接名猜接");
    }

    #[test]
    fn import_rejects_duplicate_tables_invalid_coordinates_and_oversize() {
        let graph = mk_graph();
        let json = r#"{"format":"fluxdb-er","schema_version":2,"connection_id":7,"database":"db","tables":[{"name":"t","x":0,"y":0},{"name":"t","x":1,"y":1}]}"#;
        assert!(er_import_parse(json, &graph, "db").unwrap_err().contains("重复"));
        let json = r#"{"format":"fluxdb-er","schema_version":2,"connection_id":7,"database":"db","tables":[{"name":"t","x":1e100,"y":0}]}"#;
        assert!(er_import_parse(json, &graph, "db").unwrap_err().contains("坐标"));
        assert!(er_import_parse(&"x".repeat(16 * 1024 * 1024 + 1), &graph, "db").is_err());
    }

    #[test]
    fn import_rejects_wrong_format_and_unsupported_version() {
        let empty = fluxdb_core::ErGraphData {
            tables: Vec::new(),
            edges: Vec::new(),
            relation_status: fluxdb_core::ErLoadStatus::NotLoaded,
        };
        assert!(er_import_parse("not json", &empty, "db").is_err());
        assert!(er_import_parse(r#"{"format":"other"}"#, &empty, "db").is_err());
        // 未来版本拒绝（不静默降级）。
        let future = r#"{"format":"fluxdb-er","schema_version":99,"tables":[],"edges":[]}"#;
        let err = er_import_parse(future, &empty, "db").unwrap_err();
        assert!(err.contains("schema_version"));
    }
}
