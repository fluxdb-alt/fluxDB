// sql_editor_adapter/completion.rs —— SQL 补全来源注入与候选过滤。
//
// 定义注入用的 `SqlSchemaContext` 与 `SqlCompletionSource` trait object，
// 以及本地候选过滤函数 `filter_items`（纯逻辑，不依赖数据库）。

use std::borrow::Cow;

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
/// 匹配规则：查询词为空时返回全部；否则按 label 是否包含查询词过滤。
/// 排序：前缀命中优先，其次 priority 高者优先，最后按 label 字典序稳定。
pub fn filter_items(items: &[CompletionItem], query: &str) -> Vec<CompletionItem> {
    if query.trim().is_empty() {
        let mut sorted = items.to_vec();
        sorted.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.label.cmp(&b.label)));
        return sorted;
    }
    let q = query.to_lowercase();
    let mut matched: Vec<(bool, CompletionItem)> = items
        .iter()
        .filter_map(|it| {
            let normalized = if it.filter_text.is_empty() {
                Cow::Owned(it.label.to_lowercase())
            } else {
                Cow::Borrowed(it.filter_text.as_str())
            };
            normalized
                .contains(&q)
                .then(|| (normalized.starts_with(&q), it.clone()))
        })
        .collect();
    matched.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.priority.cmp(&a.1.priority))
            .then(a.1.label.cmp(&b.1.label))
    });
    matched.into_iter().map(|(_, item)| item).collect()
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
    // 取 '.' 前连续标识符字符；标识符内不能夹着字符串/注释片段。
    let mut start = dot;
    for (index, ch) in trimmed[..dot].char_indices().rev() {
        if is_ident_char(ch) {
            start = index;
        } else {
            break;
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
            b'\'' | b'"' | b'`' => {
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

/// 一次线性收集字符串和注释区间，供需要扫描多个 token 的调用方复用。
/// `masked_range_at` 适合单点判断；批量诊断/高亮不能对每个点重复扫描全文。
#[allow(dead_code)]
fn masked_ranges(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut ranges = Vec::new();
    let mut i = 0usize;
    while i < n {
        let start = i;
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[i];
                i += 1;
                while i < n {
                    if bytes[i] == quote {
                        if i + 1 < n && bytes[i + 1] == quote {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'-' if i + 1 < n && bytes[i + 1] == b'-' => {
                i += 2;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'#' => {
                i += 1;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
            }
            _ => {
                i += 1;
            }
        }
        let quoted_or_hash = i > start && matches!(bytes[start], b'\'' | b'"' | b'`' | b'#');
        let line_or_block_comment = i > start + 1
            && ((bytes[start] == b'-' && bytes.get(start + 1) == Some(&b'-'))
                || (bytes[start] == b'/'
                    && matches!(bytes.get(start + 1), Some(&b'*') | Some(&b'/'))));
        if quoted_or_hash || line_or_block_comment {
            ranges.push((start, i));
        }
    }
    ranges
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
mod completion_tests {
    use super::*;

    #[test]
    fn schema_context_default_empty() {
        let ctx = SqlSchemaContext::new();
        assert!(ctx.tables.is_empty());
        assert!(ctx.columns.is_empty());
    }

    #[test]
    fn schema_source_implements_trait() {
        let ctx = SqlSchemaContext::with_data(
            vec!["users".to_string()],
            vec![("users".to_string(), "id".to_string())],
        );
        let src: &dyn SqlCompletionSource = &ctx;
        assert_eq!(src.tables(), vec!["users".to_string()]);
        assert_eq!(src.columns(), vec![("users".to_string(), "id".to_string())]);
    }

    #[test]
    fn filter_items_prefix_first() {
        let items = vec![
            CompletionItem::new("select", CompletionKind::Keyword),
            CompletionItem::new("selection", CompletionKind::Keyword),
            CompletionItem::new("drop", CompletionKind::Keyword),
        ];
        let out = filter_items(&items, "se");
        let labels: Vec<&str> = out.iter().map(|i| i.label.as_str()).collect();
        // select / selection 命中；drop 不含 "se"，被过滤。
        assert!(labels.contains(&"select"));
        assert!(labels.contains(&"selection"));
        assert!(!labels.contains(&"drop"));
    }

    #[test]
    fn filter_items_empty_query_returns_all() {
        let items = vec![
            CompletionItem::new("select", CompletionKind::Keyword),
            CompletionItem::new("from", CompletionKind::Keyword),
        ];
        assert_eq!(filter_items(&items, "").len(), 2);
        assert_eq!(filter_items(&items, "  ").len(), 2);
    }

    // ===== `标识符.` 空前缀触发探测（P0.3）=====

    #[test]
    fn trailing_qualifier_triggers_on_table_dot() {
        assert!(trailing_ident_qualifier("SELECT u."));
        assert!(trailing_ident_qualifier("SELECT users."));
        assert!(trailing_ident_qualifier("select u. "));
        // 前后带其它标识符、可加空格。
        assert!(trailing_ident_qualifier("FROM users u JOIN orders o ON u."));
    }

    #[test]
    fn trailing_qualifier_needs_ident_before_dot() {
        assert!(!trailing_ident_qualifier("."));
        assert!(!trailing_ident_qualifier("SELECT ."));
        assert!(!trailing_ident_qualifier("SELECT 1 + ."));
    }

    #[test]
    fn dot_inside_string_does_not_trigger() {
        assert!(!trailing_ident_qualifier("SELECT 'ui.'"));
        assert!(!trailing_ident_qualifier("SELECT 'a.b' "));
        assert!(!trailing_ident_qualifier("SELECT 'it\\'s.'"));
    }

    #[test]
    fn dot_inside_backtick_identifier_does_not_trigger() {
        assert!(!trailing_ident_qualifier("SELECT `a.b`"));
    }

    #[test]
    fn dot_inside_comment_does_not_trigger() {
        assert!(!trailing_ident_qualifier("SELECT -- foo.x"));
        assert!(!trailing_ident_qualifier("-- comment u."));
        assert!(!trailing_ident_qualifier("SELECT /* comment u. */"));
    }

    #[test]
    fn decimal_and_dot_after_non_dot_do_not_trigger() {
        assert!(!trailing_ident_qualifier("SELECT 3.14"));
        assert!(!trailing_ident_qualifier("price 1.5"));
    }

    #[test]
    fn masked_ranges_collect_all_comment_and_string_spans_once() {
        let text = "SELECT 'a.b', x /* c.d */ FROM t -- e.f\nWHERE t.id = 1";
        let ranges = masked_ranges(text);
        assert_eq!(ranges.len(), 3);
        assert!(ranges.iter().all(|(start, end)| *start < *end));
        assert!(ranges.iter().any(|(start, end)| &text[*start..*end] == "'a.b'"));
        assert!(ranges.iter().any(|(start, end)| text[*start..*end].starts_with("/*")));
        assert!(ranges.iter().any(|(start, end)| text[*start..*end].starts_with("--")));
    }
}
