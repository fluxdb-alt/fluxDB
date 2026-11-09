// sql_editor_adapter/statements.rs —— SQL 语句模型与执行状态映射。
//
// 本文件把「语句切分结果」提升为带稳定 id 的 `SqlStatementRun` 列表，并提供
// 执行状态映射 `SqlStatementStatusMap`，供宿主在语句行首展示 Run/Select/Explain
// 按钮状态（Running/Success/Failure）。所有函数均为纯函数，不依赖 GPUI / 数据库，
// 可由单测直接覆盖。
//
// 说明：`SqlStatementRun` / `SqlStatementId` / `SqlStatementStatus` 目前定义于
// crate 根作用域（旧 sql_editor/model.rs，经 include! 展开），因此此处仅使用
// `crate::` 引用，不在本模块重复定义，避免命名冲突。

use std::collections::HashMap;

/// SQL 语句运行结果。每条已切分的语句会被提升为 `SqlStatementRun`，携带稳定 id、
/// 序号、首/尾行号、字节区间与语句文本，供宿主在语句行首渲染 Run/Select/Explain 按钮。
///
/// 说明：这三个类型（`SqlStatementRun` / `SqlStatementId` / `SqlStatementStatus`）
/// 原定义于旧 `sql_editor/model.rs`（crate 根作用域），在移除旧兜编辑器模块后迁移到
/// 本 adapter 模块统一维护，外部宿主通过 `sql_editor_adapter::...` 引用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SqlStatementRun {
    pub id: SqlStatementId,
    pub ordinal: usize,
    pub start_row: usize,
    pub end_row: usize,
    pub range: std::ops::Range<usize>,
    pub text: String,
}

/// 语句唯一 id（元组结构），由「序号 + 语句文本」哈希生成。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct SqlStatementId(pub u64);

/// 语句执行状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SqlStatementStatus {
    Running,
    Success,
    Failure,
}

/// 字节区间别名（`SqlStatementRun::range` 使用 `std::ops::Range<usize>`）。
type ByteRange = std::ops::Range<usize>;

/// 生成可执行的 EXPLAIN SQL 文本。
///
/// 原定义于旧 `sql_editor/execution.rs`，在移除旧兜编辑器模块后迁移到本 adapter 供宿主
/// （loading.rs 的 selected_explain_sql_text 等）与单测使用。仅对 explain / select / with
/// 开头的语句返回可执行文本，其余返回 `None`。
///
/// 只有「`EXPLAIN <sql>` 前缀」方言能这样改写，取 `SqlPolicy::explain_mode` 判定：
/// Oracle 只认 `EXPLAIN PLAN FOR`（会写 PLAN_TABLE，有副作用），SQL Server 要 SHOWPLAN
/// 专用会话，两条路径都无法靠拼前缀得到计划。对它们返回 `None`，让 Explain 按钮禁用，
/// 而不是发出必然失败的 SQL。
///
/// 自动包装优先取结构化输出，供可视化计划解析：
/// MySQL 家族 `EXPLAIN FORMAT=JSON`、PostgreSQL 家族 `EXPLAIN (FORMAT JSON)`、
/// SQLite `EXPLAIN QUERY PLAN`。用户手写的 `EXPLAIN` 原样执行，不二次包装。
pub(crate) fn explain_sql_text(statement: &str, kind: DatabaseKind) -> Option<String> {
    if fluxdb_app::policy_for(kind)?.explain_mode() != fluxdb_app::ExplainMode::Prefix {
        return None;
    }
    let text = statement.trim();
    let keyword = first_sql_keyword(text)?;
    match keyword.as_str() {
        "explain" => Some(text.to_string()),
        "select" | "with" => Some(match kind {
            kind if kind.is_mysql_family() => format!("EXPLAIN FORMAT=JSON {text}"),
            kind if kind.is_postgres_family() => format!("EXPLAIN (FORMAT JSON) {text}"),
            DatabaseKind::Sqlite => format!("EXPLAIN QUERY PLAN {text}"),
            _ => format!("EXPLAIN {text}"),
        }),
        _ => None,
    }
}

/// 语句是否要求 `EXPLAIN ANALYZE`（会真正执行 SQL，不是只看计划）。
///
/// 工具栏「解释」按钮用它避免把实际执行当成预估计划；编辑器 Run 仍尊重用户手写意图。
pub(crate) fn explain_analyze_requested(statement: &str) -> bool {
    let lower = statement.trim_start().to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("explain") else {
        return false;
    };
    let rest = rest.trim_start();
    // 只看 EXPLAIN 后的选项区：`EXPLAIN SELECT 'analyze'` 不应被误判。
    let head = if let Some(options) = rest.strip_prefix('(') {
        options.split(')').next().unwrap_or("")
    } else {
        rest.split_whitespace().next().unwrap_or("")
    };
    head.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|token| token == "analyze")
}

/// 取语句的首个 ASCII 小写关键字（如 select / with / explain），无字母前缀时返回 `None`。
pub(crate) fn first_sql_keyword(statement: &str) -> Option<String> {
    let keyword = statement
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    (!keyword.is_empty()).then_some(keyword)
}

// 注：`split_statements` 与 `Range` 均定义于同一 `sql_editor_adapter` 模块
// （execution.rs / mod.rs，经 include! 合并），故直接引用即可，无需 `use`。

/// 根据 SQL 全文切分出带稳定 id 的语句列表。
///
/// 复用 `split_statements` 的切分语义（跳过字符串/注释），为每条语句分配
/// ordinal 与基于「序号 + 语句文本」哈希的 id，并换算首/尾行号与字节区间。
/// 语句的 `start_row`/`end_row` 为 0 起始行号（`end_row` 为语句结束行，闭区间）。
pub fn build_statement_runs(text: &str, kind: DatabaseKind) -> Vec<SqlStatementRun> {
    let bytes = text.as_bytes();
    let mut scan_offset = 0;
    let mut row = 0;
    split_statements(text, kind)
        .into_iter()
        .enumerate()
        .map(|(ordinal, range)| {
            // Range 按文档顺序排列；沿文本向前推进一次，避免对每条语句重复统计前缀换行。
            while scan_offset < range.start {
                if bytes[scan_offset] == b'\n' {
                    row += 1;
                }
                scan_offset += 1;
            }
            let start_row = row;
            while scan_offset < range.end {
                if bytes[scan_offset] == b'\n' {
                    row += 1;
                }
                scan_offset += 1;
            }
            let statement_text = text[range.start..range.end].to_string();
            SqlStatementRun {
                id: statement_id(ordinal, statement_text.trim()),
                ordinal,
                start_row,
                end_row: row,
                range: ByteRange {
                    start: range.start,
                    end: range.end,
                },
                text: statement_text,
            }
        })
        .collect()
}

/// 从已解析缓存的语句字节范围构造运行状态索引，避免为每次布局把整篇快照拼成一个 String。
pub fn build_statement_runs_from_snapshot(
    snapshot: &fluxdb_editor_core::BufferSnapshot,
    ranges: &[fluxdb_editor_core::Range],
) -> Vec<SqlStatementRun> {
    ranges
        .iter()
        .copied()
        .enumerate()
        .map(|(ordinal, range)| {
            let range = fluxdb_editor_core::Range::new(
                range.start.min(snapshot.len()),
                range.end.min(snapshot.len()),
            );
            let text = snapshot.text_in_range(range);
            let start_row = snapshot.offset_to_point(range.start).row;
            let end_row = snapshot.offset_to_point(range.end.saturating_sub(1)).row;
            SqlStatementRun {
                id: statement_id(ordinal, text.trim()),
                ordinal,
                start_row,
                end_row: end_row.max(start_row),
                range: range.start..range.end,
                text,
            }
        })
        .collect()
}

/// 生成语句唯一 id：对「序号 + 语句文本」做一次默认哈希。
fn statement_id(ordinal: usize, sql: &str) -> SqlStatementId {
    use std::hash::Hash;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    ordinal.hash(&mut hasher);
    sql.hash(&mut hasher);
    SqlStatementId(std::hash::Hasher::finish(&hasher))
}

/// 执行状态映射：以语句 id 为键保存 Run/Select/Explain 的执行状态。
///
/// 由于 `SqlStatementId` 由「序号 + 语句文本」哈希生成，当某条语句被编辑后其
/// id 会改变，从而「自然退化为无状态」——这正是 gutter 状态随编辑衰减所需的行为：
/// 查询某区间语句状态时，先用当前文本重新算出该语句的 id，再查表，匹配不到即无状态。
#[derive(Clone, Debug, Default)]
pub struct SqlStatementStatusMap {
    /// id -> 执行状态。
    statuses: HashMap<SqlStatementId, SqlStatementStatus>,
}

impl SqlStatementStatusMap {
    /// 是否不含任何语句状态（未执行过任何语句）。
    pub fn is_empty(&self) -> bool {
        self.statuses.is_empty()
    }

    /// 设置某条语句的执行状态。`None` 表示清除该语句状态。
    #[allow(dead_code)] // 与 set_id_status 等价、按 run 定位的变体，宿主采用 id 版，保留备用。
    pub fn set_status(&mut self, run: &SqlStatementRun, status: Option<SqlStatementStatus>) {
        match status {
            Some(status) => {
                self.statuses.insert(run.id, status);
            }
            None => {
                self.statuses.remove(&run.id);
            }
        }
    }

    /// 设置某条已执行语句的状态，供宿主在语句执行前/后调用。
    pub fn set_id_status(&mut self, id: SqlStatementId, status: SqlStatementStatus) {
        self.statuses.insert(id, status);
    }

    /// 查询包含 `range`（语句号即 range.start 所在行）对应语句的状态。
    ///
    /// 返回 `None` 表示该语句当前无执行状态（未执行，或语句已被编辑而失效）。
    pub fn status_for_run(&self, run: &SqlStatementRun) -> Option<SqlStatementStatus> {
        self.statuses.get(&run.id).copied()
    }

    /// 依据当前文本与光标行，查询该行所属语句的执行状态。
    ///
    /// row 为 0 起始行号；找不到对应语句或该语句无状态时返回 `None`。
    #[allow(dead_code)] // 供宿主按行查询语句状态，当前装饰由 decorations() 直接消费，保留备用。
    pub fn status_for_row(
        &self,
        text: &str,
        row: usize,
        kind: DatabaseKind,
    ) -> Option<SqlStatementStatus> {
        build_statement_runs(text, kind)
            .into_iter()
            .find(|run| row >= run.start_row && row <= run.end_row)
            .and_then(|run| self.statuses.get(&run.id).copied())
    }
}

/// 参数替换的落点：用一条 SQL 替换选中文本 / 全文 / 当前语句所对应的区间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParameterReplacementTarget {
    /// 替换选区文本（字节区间）。
    Selected(ByteRange),
    /// 替换整篇文本。
    All,
    /// 替换光标所在语句（字节区间）。
    Statement(ByteRange),
}

/// 依据选区与原文，判定参数替换应落在哪个区间（纯函数版本）。
///
/// 语义与旧版参数替换流程对齐：
/// 1. 选区文本（trim 后）等于 `original` -> 替换选区；
/// 2. 否则整篇文本（trim 后）等于 `original` -> 替换全文；
/// 3. 否则光标所在语句（trim 后）等于 `original` -> 替换该语句；
/// 4. 均不匹配 -> `None`（宿主无需替换）。
///
/// `selection` 为当前光标/选区；光标非空选区时即为选区区间。仅判断与返回目标，
/// 实际替换文本由宿主写入编辑器 buffer。
pub fn parameter_replacement_target(
    text: &str,
    selection: ByteRange,
    original: &str,
    kind: DatabaseKind,
) -> Option<ParameterReplacementTarget> {
    let original = original.trim();
    let selection_text = text.get(selection.clone()).map(str::trim);
    if selection_text == Some(original) {
        return Some(ParameterReplacementTarget::Selected(selection));
    }
    if text.trim() == original {
        return Some(ParameterReplacementTarget::All);
    }
    let (start, end) = statement_around(text, selection.start, kind)?;
    let stmt_text = text[start..end].trim();
    if stmt_text == original {
        Some(ParameterReplacementTarget::Statement(ByteRange { start, end }))
    } else {
        None
    }
}

/// 在给定文本上把 `range` 区间替换为 `replacement`，返回新全文。
///
/// 这是参数替换的纯文本变换：宿主拿到新文本后可整体写入编辑器 buffer，
/// 由编辑器统一维护撤销/重做与补全状态。`range` 越界时安全退化（截断）。
#[allow(dead_code)] // 参数替换流程暂未接入宿主，保留纯文本变换工具。
pub fn replace_text_in_range(text: &str, range: ByteRange, replacement: &str) -> String {
    let mut start = range.start.min(text.len());
    let mut end = range.end.min(text.len());
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..start]);
    out.push_str(replacement);
    out.push_str(&text[end..]);
    out
}

#[cfg(test)]
mod statement_multibyte_safety_tests {
    use super::*;
    use fluxdb_editor_core::EditorBuffer;

    #[test]
    fn statement_lookup_snaps_offsets_inside_chinese_text() {
        let text = "SELECT '等等';\nSELECT 2;";
        let offset_on_first_statement = text.find('等').expect("Chinese literal") + 1;
        assert!(!text.is_char_boundary(offset_on_first_statement));

        let (start, end) = statement_around(text, offset_on_first_statement, DatabaseKind::MySql)
            .expect("first statement");
        assert_eq!(&text[start..end], "SELECT '等等'");

        let buffer = EditorBuffer::new_from(text);
        let (start, end) =
            statement_around_snapshot(&buffer.snapshot(), offset_on_first_statement, DatabaseKind::MySql)
                .expect("snapshot statement");
        assert_eq!(&text[start..end], "SELECT '等等'");
    }

    /// 编辑器里按 Run 走的是 `build_statement_runs`；Oracle 匿名块与 `q'…'`
    /// 必须整条执行，否则驱动收到的是半截 PL/SQL。
    #[test]
    fn oracle_block_and_q_quote_stay_one_statement() {
        let text = "BEGIN NULL; END;\nSELECT q'[a;b]' FROM dual;";
        let runs = build_statement_runs(text, DatabaseKind::Oracle);
        assert_eq!(runs.len(), 2, "runs = {runs:#?}");
        // 块语句保留结尾分号，普通 SQL 不保留（与执行侧一致）。
        assert_eq!(runs[0].text, "BEGIN NULL; END;");
        assert_eq!(runs[1].text, "SELECT q'[a;b]' FROM dual");

        // 光标落在块内部的 `;` 上仍归属整块。
        let cursor = text.find("NULL;").expect("inner semicolon");
        let (start, end) = statement_around(text, cursor, DatabaseKind::Oracle).expect("block");
        assert_eq!(&text[start..end], "BEGIN NULL; END;");

        // 同一段文本按 MySQL 切分会多出一条：MySQL 没有匿名 PL/SQL 块，`BEGIN NULL` 与
        // `END` 是两个独立语句。方言参数确实生效，不是所有库都套一套规则。
        assert_eq!(build_statement_runs(text, DatabaseKind::MySql).len(), 3);
    }

    #[test]
    fn parameter_replacement_snaps_range_inside_chinese_text() {
        let text = "SELECT '等等';";
        let offset_inside = text.find('等').expect("Chinese literal") + 1;
        let replaced =
            replace_text_in_range(text, offset_inside..offset_inside, "X");
        assert_eq!(replaced, "SELECT 'X等等';");
    }

    /// Oracle / SQL Server 没有 `EXPLAIN <sql>` 前缀写法：拼出来必然是解析失败，
    /// 因此不许生成；两者各自的计划入口（`EXPLAIN PLAN FOR` / SHOWPLAN）另行实现。
    #[test]
    fn explain_is_not_prefixed_for_dialects_without_prefix_support() {
        for kind in [DatabaseKind::Oracle, DatabaseKind::SqlServer] {
            assert_eq!(explain_sql_text("SELECT 1 FROM DUAL", kind), None, "{kind:?}");
            assert_eq!(explain_sql_text("WITH x AS (SELECT 1) SELECT * FROM x", kind), None);
        }
        assert_eq!(
            explain_sql_text("SELECT 1", DatabaseKind::MySql).as_deref(),
            Some("EXPLAIN FORMAT=JSON SELECT 1")
        );
        assert_eq!(
            explain_sql_text("SELECT 1", DatabaseKind::Postgres).as_deref(),
            Some("EXPLAIN (FORMAT JSON) SELECT 1")
        );
        assert_eq!(
            explain_sql_text("SELECT 1", DatabaseKind::Sqlite).as_deref(),
            Some("EXPLAIN QUERY PLAN SELECT 1")
        );
        // 已手写 EXPLAIN 的语句原样执行，不做二次包装。
        assert_eq!(
            explain_sql_text("EXPLAIN SELECT 1", DatabaseKind::MySql).as_deref(),
            Some("EXPLAIN SELECT 1")
        );
        assert_eq!(explain_sql_text("INSERT INTO t VALUES (1)", DatabaseKind::MySql), None);
    }

    #[test]
    fn explain_analyze_is_detected_but_not_rewritten() {
        assert!(explain_analyze_requested("EXPLAIN ANALYZE SELECT 1"));
        assert!(explain_analyze_requested("  explain (analyze, format json) select 1"));
        assert!(!explain_analyze_requested("EXPLAIN SELECT 1"));
        assert!(!explain_analyze_requested("EXPLAIN SELECT 'analyze'"));
        assert!(!explain_analyze_requested("SELECT 1"));
        assert_eq!(
            explain_sql_text("EXPLAIN ANALYZE SELECT 1", DatabaseKind::MySql).as_deref(),
            Some("EXPLAIN ANALYZE SELECT 1")
        );
    }
}

