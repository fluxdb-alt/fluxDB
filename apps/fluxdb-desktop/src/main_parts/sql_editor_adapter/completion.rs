// sql_editor_adapter/completion.rs —— SQL 补全来源注入与候选过滤。
//
// 定义注入用的 `SqlSchemaContext` 与 `SqlCompletionSource` trait object，
// 以及本地候选过滤函数 `filter_items`（纯逻辑，不依赖数据库）。

/// 结构化 schema 上下文：表名与 (表, 列) 列集合。
///
/// 用于向适配器注入表 / 列补全数据，保持适配器自包含、可测试，
/// 避免直接依赖 fluxdb-core 的补全索引或数据库连接。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SqlSchemaContext {
    /// schema 名称列表。
    pub schemas: Vec<String>,
    /// 表名列表。
    pub tables: Vec<String>,
    /// view 名称列表。
    pub views: Vec<String>,
    /// 列列表：(表名, 列名)。
    pub columns: Vec<(String, String)>,
    /// routine 名称列表（函数 / 存储过程）。
    pub routines: Vec<String>,
    /// 用户定义类型名称列表。
    pub types: Vec<String>,
}

impl SqlSchemaContext {
    /// 构造空上下文。
    #[allow(dead_code)] // 接入层公开 API，宿主经 loaded_schema_context+with_data 构造，保留。
    pub fn new() -> Self {
        Self::default()
    }

    /// 带表 / 列数据构造。
    pub fn with_data(tables: Vec<String>, columns: Vec<(String, String)>) -> Self {
        Self {
            schemas: Vec::new(),
            tables,
            views: Vec::new(),
            columns,
            routines: Vec::new(),
            types: Vec::new(),
        }
    }

    /// 注入完整对象元数据；空集合表示该类 metadata 不可用，不进行对应诊断。
    #[allow(dead_code)]
    pub fn with_metadata(
        mut self,
        schemas: Vec<String>,
        views: Vec<String>,
        routines: Vec<String>,
        types: Vec<String>,
    ) -> Self {
        self.schemas = schemas;
        self.views = views;
        self.routines = routines;
        self.types = types;
        self
    }
}

/// 补全来源抽象：任何能提供表 / 列信息的对象均可注入。
///
/// `SqlSchemaContext` 内置实现了该 trait；外部业务也可实现它来
/// 复用 fluxdb-core 的补全索引或连接信息。
pub trait SqlCompletionSource: Send + Sync {
    /// 表名列表。
    fn tables(&self) -> Vec<String>;
    /// 列列表：(表名, 列名)。
    fn columns(&self) -> Vec<(String, String)>;

    /// schema 名称列表；默认空表示 provider 未提供该类 metadata。
    fn schemas(&self) -> Vec<String> {
        Vec::new()
    }

    /// view 名称列表；默认空表示 provider 未提供该类 metadata。
    fn views(&self) -> Vec<String> {
        Vec::new()
    }

    /// routine 名称列表；默认空表示 provider 未提供该类 metadata。
    fn routines(&self) -> Vec<String> {
        Vec::new()
    }

    /// 用户定义类型名称列表；默认空表示 provider 未提供该类 metadata。
    fn types(&self) -> Vec<String> {
        Vec::new()
    }
}

impl SqlCompletionSource for SqlSchemaContext {
    fn tables(&self) -> Vec<String> {
        self.tables.clone()
    }

    fn columns(&self) -> Vec<(String, String)> {
        self.columns.clone()
    }

    fn schemas(&self) -> Vec<String> {
        self.schemas.clone()
    }

    fn views(&self) -> Vec<String> {
        self.views.clone()
    }

    fn routines(&self) -> Vec<String> {
        self.routines.clone()
    }

    fn types(&self) -> Vec<String> {
        self.types.clone()
    }
}

/// 在本地候选中按查询词过滤并排序，返回匹配项。
///
/// 匹配规则与引擎同一套打分（`fluxdb_completion::fuzzy_score`）：子序列命中即保留，
/// 前缀 / 词边界 / 连续命中得分更高。此前这里用 `contains` 子串匹配，与引擎的子序列
/// 语义不同，同一份候选走 resolver 与走本地回退会出现不同结果。
/// 排序：匹配得分优先，其次 priority 高者优先，最后按 label 字典序稳定。
pub fn filter_items(items: &[CompletionItem], query: &str) -> Vec<CompletionItem> {
    let query = query.trim();
    if query.is_empty() {
        let mut sorted = items.to_vec();
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.label.cmp(&b.label)));
        return sorted;
    }
    let mut matched: Vec<(i32, CompletionItem)> = items
        .iter()
        .filter_map(|item| {
            let haystack = if item.filter_text.is_empty() {
                item.label.as_str()
            } else {
                item.filter_text.as_str()
            };
            fluxdb_completion::fuzzy_score(haystack, query).map(|score| (score, item.clone()))
        })
        .collect();
    matched.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.priority.cmp(&a.1.priority))
            .then(a.1.label.cmp(&b.1.label))
    });
    matched.into_iter().map(|(_, item)| item).collect()
}

impl SignatureProvider for SqlAdapter {
    /// 光标落在 `NAME(...)` 内时给出该函数的参数骨架。
    ///
    /// 调用点定位（含嵌套取最内层、schema 限定取末段）与 active 参数下标由
    /// `fluxdb_editor_core::signature_at` 完成；本适配器只按函数名向宿主解析器取骨架文本。
    /// `parameter_ranges` 留空：SQL 函数骨架用 `()` + 逗号分隔，渲染层按既有规则切分。
    fn signature(&self, snapshot: &BufferSnapshot, position: Point) -> Option<SignatureInfo> {
        let resolver = self.signature_resolver.as_ref()?;
        let call = fluxdb_editor_core::signature_at(snapshot, position)?;
        let label = resolver(call.name.as_str())?;
        Some(SignatureInfo {
            label,
            documentation: String::new(),
            active_parameter: Some(call.active_parameter),
            parameter_ranges: Vec::new(),
        })
    }
}

/// 判断 `<cursor>` 前的文本是否以「标识符.」结尾（如 `SELECT u.`），用于空前缀补全触发。
///
/// 扫描时跳过字符串、反引号标识符、单/多行注释，避免点在字符串、小数或注释中误触发
/// 数据库请求。仅做纯文本判断，不依赖数据库 / fluxdb-app（SQL 决策收敛在本接入层）。
///
/// 返回 `true` 表示光标紧跟在 `标识符.` 之后（光标后可为空，此时应触发该表的列补全）。
pub(crate) fn trailing_ident_qualifier(before: &str) -> bool {
    // 先定位光标前末端的 `.`（忽略紧邻空白）。
    let trimmed = before.trim_end_matches([' ', '\t']);
    let Some(dot) = trimmed.rfind('.') else {
        return false;
    };
    // 若 '.' 落在字符串/注释内部则不触发；且其后到光标之间只能有空白。
    if masked_range_at(trimmed, dot).is_some() {
        return false;
    }
    // '.' 之后到末尾（除空白）不应再有其它字符。
    if !trimmed[dot + 1..].trim_matches([' ', '\t']).is_empty() {
        return false;
    }
    // 取 '.' 前的一段标识符；支持 `"schema".` / `` `schema`. `` / `[schema].`。
    // 标识符内不能夹着字符串/注释片段。
    // 从 '.' 前一位向左取连续标识符；必须排除 '.' 本身，否则循环首个字符即非标识符，
    // `start` 停留在 `dot` 并被下面的 `start >= dot` 判为无前缀（原先只有带引号的
    // 标识符靠后续引号回退巧合绕开）。
    let mut start = dot;
    for (index, ch) in trimmed[..dot].char_indices().rev() {
        if is_ident_char(ch) {
            start = index;
        } else {
            break;
        }
    }
    if start > 0 {
        let before = trimmed[..start].chars().next_back();
        if before.is_some_and(|ch| matches!(ch, '"' | '`' | '[')) {
            start -= before.unwrap().len_utf8();
        } else if trimmed[..start].ends_with(']')
            && let Some(open) = trimmed[..start - 1].rfind('[')
        {
            start = open;
        }
    }
    if start >= dot {
        return false;
    }
    // 标识符段 [start, dot) 必须完全位于字符串/注释之外。
    !segment_is_masked(trimmed, start, dot)
}

/// F001：空前缀空格自动触发补全。判断 `<cursor>` 前刚输入空白（空格/tab/换行），
/// 且该空白不在字符串/注释内部、其前还存在至少一个 SQL token 时返回 `true`。
///
/// 命中后交由 app 层按 SQL 意图决定具体候选（`FROM |` 返表、`users |` 返 alias/JOIN/WHERE、
/// `WHERE |` 返列），本函数只判断「这个空前缀位置是否值得请求 provider」，避免裸空格
/// 在字符串/注释/小数或空文档中误触发数据库请求。仅做纯文本判断，SQL 决策收敛在本接入层。
pub(crate) fn sql_space_trigger_context(before: &str) -> bool {
    // 末字符必须是刚输入的空白。
    if !before.ends_with([' ', '\t', '\n', '\r']) {
        return false;
    }
    // 空白之前的文本需非空，且末尾空白位置不在字符串/注释内。
    let trimmed = before.trim_end_matches([' ', '\t', '\n', '\r']);
    if trimmed.is_empty() {
        return false;
    }
    let last = before.len().saturating_sub(1);
    if masked_range_at(before, last).is_some() {
        return false;
    }
    // 空白之前还必须存在至少一个「实词」SQL token（说明确实在写 SQL 而非纯注释/空白）。
    // `sql_tokens` 会把数字字面量的每一位当标识符（如 `3.14` → `3`+`14`），故须排除纯数字
    // token：`SELECT 3.14 ` / `x + 1 ` 这种小数/表达式空格不应触发补全。
    if let Some(last_token) = sql_tokens(trimmed).last()
        && last_token.word.bytes().any(|b| b.is_ascii_alphabetic())
    {
        return true;
    }
    false
}

/// 线性扫描 `text`，返回覆盖字节位置 `pos` 的字符串/注释区间（若有）。
///
/// 返回 `Some((start, end))` 表示 `pos ∈ [start, end)` 落在被屏蔽范围内。
fn masked_range_at(text: &str, pos: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    while i < n {
        match bytes[i] {
            b'\'' => {
                let quote = bytes[i];
                let start = i;
                i += 1;
                while i < n {
                    if bytes[i] == quote {
                        // 转义：两个连续引号视为一个。
                        if i + 1 < n && bytes[i + 1] == quote {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                let end = i;
                if start <= pos && pos < end {
                    return Some((start, end));
                }
            }
            b'-' if i + 1 < n && bytes[i + 1] == b'-' => {
                let start = i;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                let end = i;
                if start <= pos && pos < end {
                    return Some((start, end));
                }
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'/' => {
                i += 2;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'#' => {
                let start = i;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                let end = i;
                if start <= pos && pos < end {
                    return Some((start, end));
                }
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                let start = i;
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                let end = i;
                if start <= pos && pos < end {
                    return Some((start, end));
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

/// 判断字节区间 `[start, end)` 是否存在任何被字符串/注释屏蔽的子区间。
fn segment_is_masked(text: &str, start: usize, end: usize) -> bool {
    let mut i = start;
    while i < end {
        if let Some((_, m_end)) = masked_range_at(text, i) {
            if m_end > i {
                return true;
            }
        }
        let next = text[i..].char_indices().nth(1).map(|(j, _)| i + j).unwrap_or(end);
        i = next;
    }
    false
}

/// 标识符字符（与 fluxdb-app 的 `is_sql_ident_char` 对齐）。
fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

#[cfg(test)]
mod trigger_tests {
    use super::trailing_ident_qualifier;

    #[test]
    fn quoted_schema_prefix_triggers_completion() {
        for before in [
            r#"SELECT * FROM "fluxdb_pr46_acceptance"."#,
            "SELECT * FROM `fluxdb_pr46_acceptance`.",
            "SELECT * FROM [fluxdb_pr46_acceptance].",
        ] {
            assert!(trailing_ident_qualifier(before), "{before}");
        }
    }
}

#[cfg(test)]
mod filter_items_tests {
    use super::*;
    use fluxdb_editor_core::{CompletionKind, CompletionItem};

    fn item(label: &str, priority: i32) -> CompletionItem {
        let mut item = CompletionItem::new(label, CompletionKind::Column);
        item.priority = priority;
        item
    }

    fn labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|item| item.label.as_str()).collect()
    }

    #[test]
    fn filter_items_matches_subsequence_like_the_engine() {
        let items = vec![item("customer_id", 0), item("id", 0), item("name", 0)];
        // `cid` 只是 customer_id 的子序列；id / name 都不含 c，必须被过滤掉。
        assert_eq!(labels(&filter_items(&items, "cid")), vec!["customer_id"]);
    }

    #[test]
    fn filter_items_ranks_match_quality_before_priority() {
        // 前缀命中（id）比中段子序列命中（index）分高，即使后者 priority 更高。
        let items = vec![item("index", 5), item("id", 0)];
        assert_eq!(labels(&filter_items(&items, "id")), vec!["id", "index"]);
    }

    #[test]
    fn filter_items_breaks_ties_by_priority_then_label() {
        // `a_col` 与 `b_col` 的 `col` 匹配得分相同：priority 高者在前，同 priority 按 label 字典序。
        let items = vec![item("b_col", 1), item("a_col", 9)];
        assert_eq!(labels(&filter_items(&items, "col")), vec!["a_col", "b_col"]);
        let items = vec![item("b_col", 1), item("a_col", 1)];
        assert_eq!(labels(&filter_items(&items, "col")), vec!["a_col", "b_col"]);
    }

    #[test]
    fn filter_items_empty_query_sorts_by_priority_then_label() {
        // 空查询不做匹配过滤，只按 priority 降序 + label 升序。
        let items = vec![item("b_col", 1), item("a_col", 1), item("c_col", 9)];
        assert_eq!(
            labels(&filter_items(&items, "")),
            vec!["c_col", "a_col", "b_col"]
        );
    }

    #[test]
    fn trailing_qualifier_recognizes_quoted_identifiers() {
        // SQL Server 方括号标识符闭合后，点号必须触发补全。
        for text in [
            "SELECT * FROM [dbo].",
            "SELECT * FROM [a]]b].",
        ] {
            assert!(trailing_ident_qualifier(text), "{text}");
        }
        assert!(!trailing_ident_qualifier("SELECT * FROM ."));
        assert!(!trailing_ident_qualifier("SELECT 'x.'"));
        assert!(
            trailing_ident_qualifier("SELECT * FROM completion_lab."),
            "plain ident qualifier"
        );
    }
}

