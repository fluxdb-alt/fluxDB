// SQL 补全遗产清理后的存活工具：标识符引号规则、词法 token 化、DDL 影响面分析。
// 语义补全本体在 `crates/fluxdb-completion`；本文件只保留仍有跨模块消费方的
// 纯工具函数，历史的 is_*_context 启发式、SqlCompletionDialect 与作用域符号收集已删。

/// 判断标识符字符（字母数字、下划线、美元符）。
fn is_sql_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

/// 把 SQL 切成标识符/点号连接的 token 序列（历史分类、DDL 影响面分析用）。
fn sql_identifier_tokens(sql: &str) -> Vec<String> {
    sql.split(|ch: char| !(is_sql_ident_char(ch) || ch == '.'))
        .filter(|token| !token.is_empty())
        .map(|token| token.trim_matches('.'))
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// DDL 语句影响的对象集合：触发表级刷新还是整库刷新。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct SqlDdlImpact {
    tables: BTreeSet<String>,
    database_wide: bool,
}

/// 分析 DDL 语句影响的表；非 DDL 或无法定位对象时返回 None。
fn sql_ddl_impact(sql: &str) -> Option<SqlDdlImpact> {
    let tokens = sql_identifier_tokens(sql);
    let mut impact = SqlDdlImpact::default();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].to_ascii_lowercase();
        match token.as_str() {
            "create" => {
                let Some(kind) = tokens
                    .get(index + 1)
                    .map(|value| value.to_ascii_lowercase())
                else {
                    impact.database_wide = true;
                    break;
                };
                match kind.as_str() {
                    "table" | "view" | "trigger" => {
                        if let Some(table) = ddl_object_name_after(&tokens, index + 2) {
                            impact.tables.insert(table);
                        } else {
                            impact.database_wide = true;
                        }
                    }
                    "index" | "unique" => {
                        if let Some(on_index) = tokens[index + 1..]
                            .iter()
                            .position(|value| value.eq_ignore_ascii_case("on"))
                            .map(|offset| index + 1 + offset)
                        {
                            if let Some(table) = ddl_object_name_after(&tokens, on_index + 1) {
                                impact.tables.insert(table);
                            } else {
                                impact.database_wide = true;
                            }
                        } else {
                            impact.database_wide = true;
                        }
                    }
                    "database" | "schema" => impact.database_wide = true,
                    _ => impact.database_wide = true,
                }
                index += 1;
            }
            "alter" | "drop" | "truncate" => {
                let next_index = index + 1;
                let next = tokens
                    .get(next_index)
                    .map(|value| value.to_ascii_lowercase());
                if matches!(next.as_deref(), Some("database" | "schema")) {
                    impact.database_wide = true;
                } else {
                    let name_index =
                        if matches!(next.as_deref(), Some("table" | "view" | "trigger")) {
                            next_index + 1
                        } else {
                            next_index
                        };
                    if let Some(table) = ddl_object_name_after(&tokens, name_index) {
                        impact.tables.insert(table);
                    } else {
                        impact.database_wide = true;
                    }
                }
                index += 1;
            }
            "rename" => {
                if tokens
                    .get(index + 1)
                    .is_some_and(|value| value.eq_ignore_ascii_case("table"))
                {
                    if let Some(old_table) = ddl_object_name_after(&tokens, index + 2) {
                        impact.tables.insert(old_table);
                    } else {
                        impact.database_wide = true;
                    }
                    if let Some(to_index) = tokens[index + 2..]
                        .iter()
                        .position(|value| value.eq_ignore_ascii_case("to"))
                        .map(|offset| index + 2 + offset)
                    {
                        if let Some(new_table) = ddl_object_name_after(&tokens, to_index + 1) {
                            impact.tables.insert(new_table);
                        }
                    }
                } else {
                    impact.database_wide = true;
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    (impact.database_wide || !impact.tables.is_empty()).then_some(impact)
}

/// 跳过 IF NOT EXISTS / IF EXISTS / TEMPORARY 修饰词后取 DDL 对象名（限定名取末段）。
fn ddl_object_name_after(tokens: &[String], mut index: usize) -> Option<String> {
    while let Some(token) = tokens.get(index) {
        let lower = token.to_ascii_lowercase();
        if matches!(lower.as_str(), "if" | "not" | "exists" | "temporary") {
            index += 1;
            continue;
        }
        return Some(
            token
                .split('.')
                .next_back()
                .unwrap_or(token)
                .to_ascii_lowercase(),
        );
    }
    None
}

/// 标识符是否必须加引号：数字开头、含特殊字符、PG 大写（未加引号会折叠成小写）或保留字。
pub fn identifier_needs_quote(
    name: &str,
    kind: DatabaseKind,
    reserved: impl Fn(&str) -> bool,
) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return true;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return true;
    }
    if kind.is_postgres_family() && name.chars().any(|c| c.is_ascii_uppercase()) {
        return true;
    }
    reserved(name)
}

/// 按方言给标识符加引号并对内部引号做转义（P1.6）。
///
/// 规则：MySQL/TiDB 用反引号；SQL Server 用方括号；其余用双引号。
/// 普通安全标识符（非保留字、无特殊字符）原样返回，避免候选过度带引号。
pub fn quote_identifier(name: &str, kind: DatabaseKind, reserved: impl Fn(&str) -> bool) -> String {
    if !identifier_needs_quote(name, kind, &reserved) {
        return name.to_string();
    }
    match kind {
        DatabaseKind::MySql | DatabaseKind::TiDb | DatabaseKind::MariaDb => {
            fluxdb_core::MySqlDialect.quote_identifier(name)
        }
        DatabaseKind::SqlServer => fluxdb_core::SqlServerDialect.quote_identifier(name),
        _ => fluxdb_core::PostgresDialect.quote_identifier(name),
    }
}

/// 单调毫秒时间戳（补全个性化排序 / 历史记录去重用）。
fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// T071：可关闭的轻量个性化（recency/frequency 加分）。
///
/// 记录用户近期采纳哪些补全候选（仅 label 的匿名统计），在确定性语义分数
/// 之后对其小幅加分。安全：不记录完整 SQL、密码、DSN 或敏感值，只记
/// `label -> (count, last_seen_ts)`；默认 `enabled == false`（关闭），关闭时
/// `score` 恒 0，排序与确定性基线完全一致。
#[derive(Debug)]
pub struct RecencyFrequency {
    enabled: bool,
    /// label -> (采纳次数, 最近采纳时间戳秒)
    counts: std::collections::HashMap<String, (u32, u64)>,
}

impl RecencyFrequency {
    pub fn new() -> Self {
        Self {
            enabled: false,
            counts: std::collections::HashMap::new(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 记录一次候选采纳（仅匿名 label）。关闭时不记录。
    pub fn record_accept(&mut self, label: &str, ts: u64) {
        if !self.enabled {
            return;
        }
        let entry = self
            .counts
            .entry(label.to_ascii_lowercase())
            .or_insert((0, ts));
        entry.0 += 1;
        entry.1 = ts;
    }

    /// 个性化分数：frequency 为主、recency 衰减系数为次。关闭或未记录时 0。
    pub fn score(&self, label: &str) -> i32 {
        if !self.enabled {
            return 0;
        }
        match self.counts.get(&label.to_ascii_lowercase()) {
            Some((count, last)) => {
                // 轻量 recency 加分：60s 内最近采纳的对象额外 +1。
                let now = now_ts();
                let recency = if *last + 60 >= now { 1 } else { 0 };
                i32::try_from(*count).unwrap_or(i32::MAX) + recency
            }
            None => 0,
        }
    }
}

impl Default for RecencyFrequency {
    fn default() -> Self {
        Self::new()
    }
}

/// 排序解释摘要（T024）：给出候选 top-N 的（type:label），用于 debug/perf 日志；
/// 不输出完整 SQL，避免泄露敏感内容。
fn completion_top_candidates(items: &[QueryCompletionItem], limit: usize) -> String {
    let mut out = String::new();
    for (index, item) in items.iter().take(limit).enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{:?}:{}", item.kind, item.label));
    }
    out
}
