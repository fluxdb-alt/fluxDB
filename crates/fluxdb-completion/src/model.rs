//! 补全引擎的输入输出模型：请求、候选、词法上下文。

/// 候选项类别。与宿主 `QueryCompletionKind` 一一对应，由宿主负责转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    Keyword,
    Snippet,
    Schema,
    Table,
    View,
    Column,
    Function,
    Procedure,
}

/// 一条补全候选。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub label: String,
    pub insert_text: String,
    pub kind: CandidateKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub filter_text: Option<String>,
    /// 排序键：形如 `{score}:{...}`，由引擎按槽位语义生成。
    pub sort_text: Option<String>,
    /// 候选对象所属 schema，宿主用于详情面板与歧义消解。
    pub schema: Option<String>,
    /// 插入时是否按方言补标识符引号。CTE / 派生表的列名来自语句内写法，
    /// 补引号会改变大小写折叠语义（Oracle 的 `"item_total"` ≠ `ITEM_TOTAL`）。
    pub quote_identifier: bool,
}

impl Candidate {
    /// 构造一条基础候选，detail/documentation/sort_text/schema 留空。
    pub fn new(
        label: impl Into<String>,
        insert_text: impl Into<String>,
        kind: CandidateKind,
    ) -> Self {
        Self {
            label: label.into(),
            insert_text: insert_text.into(),
            kind,
            detail: None,
            documentation: None,
            filter_text: None,
            sort_text: None,
            schema: None,
            quote_identifier: true,
        }
    }

    /// 语句内推导出的名字（CTE / 派生表列、投影别名）按原样插入，不补引号。
    pub fn without_identifier_quotes(mut self) -> Self {
        self.quote_identifier = false;
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    pub fn with_sort_text(mut self, sort_text: impl Into<String>) -> Self {
        self.sort_text = Some(sort_text.into());
        self
    }
}

/// 一次补全请求。`text` 为光标前后窗口文本，`cursor` 是窗口内相对字节偏移。
#[derive(Debug, Clone)]
pub struct CompletionRequest<'a> {
    pub text: &'a str,
    /// 窗口内相对字节偏移，落在 UTF-8 字符边界上。
    pub cursor: usize,
    pub explicit: bool,
    /// 连接级作用域：当前数据库。
    pub database: Option<&'a str>,
    /// 连接级作用域：当前 schema（PG search_path 首项、达梦默认 schema 等）。
    pub schema: Option<&'a str>,
}

impl<'a> CompletionRequest<'a> {
    pub fn new(text: &'a str, cursor: usize) -> Self {
        Self {
            text,
            cursor,
            explicit: false,
            database: None,
            schema: None,
        }
    }
}

/// 补全输出。`replace_start..replace_end` 是需要被插入文本替换的区间。
#[derive(Debug, Clone)]
pub struct CompletionOutput {
    pub replace_start: usize,
    pub replace_end: usize,
    pub items: Vec<Candidate>,
}

/// 词法层光标上下文（等价 DBeaver SQLWordPartDetector）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordContext {
    /// 光标所在词已输入的部分，不含限定符。
    pub word_part: String,
    /// 词法扫描起点（含限定符 `u.` 的起点），用于掩码与 AST 定位。
    pub replace_start: usize,
    /// 实际插入时应替换的起点：不含限定符（`u.na` → `na` 的起点），
    /// 引号标识符则含开引号。
    pub prefix_start: usize,
    /// 需要替换的区间终点（不含光标后的词，保留后缀由宿主决定）。
    pub replace_end: usize,
    /// 引号标识符（`"x"` / `` `x` ``）→ 大小写敏感匹配。
    pub quoted: bool,
    /// 前置限定符：`u.` 的 `u`（多级为 `schema.table` 时只取最后一段之外的全串）。
    pub qualifier: Option<String>,
    /// 光标前的词序列（小写化，最多 8 个，最近的在末尾）。
    pub prev_words: Vec<String>,
    /// 向前扫到的最近 SQL 关键字（大写原文）。
    pub prev_keyword: Option<String>,
}

impl WordContext {
    /// 光标所在词的全文（限定符 + 已输入部分），用于过滤与展示。
    pub fn full_word(&self) -> String {
        match &self.qualifier {
            Some(qualifier) => format!("{qualifier}.{}", self.word_part),
            None => self.word_part.clone(),
        }
    }
}
