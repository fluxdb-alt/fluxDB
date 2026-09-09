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
/// （loading.rs 的 selected_explain_sql_text 等）与单测使用。逻辑保持不变：仅对
/// explain / select / with 开头的语句返回可执行文本，其余返回 `None`。
pub(crate) fn explain_sql_text(statement: &str) -> Option<String> {
    let text = statement.trim();
    let keyword = first_sql_keyword(text)?;
    match keyword.as_str() {
        "explain" => Some(text.to_string()),
        "select" | "with" => Some(format!("EXPLAIN {text}")),
        _ => None,
    }
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
pub fn build_statement_runs(text: &str) -> Vec<SqlStatementRun> {
    let bytes = text.as_bytes();
    let mut scan_offset = 0;
    let mut row = 0;
    split_statements(text)
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

/// 计算字节偏移 `offset` 所在的 0 起始行号（统计其前的换行数）。
#[allow(dead_code)] // 保留给按偏移查询行号的备用调用与单测。
pub fn row_for_offset(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())]
        .chars()
        .filter(|ch| *ch == '\n')
        .count()
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
    pub fn status_for_row(&self, text: &str, row: usize) -> Option<SqlStatementStatus> {
        build_statement_runs(text)
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
) -> Option<ParameterReplacementTarget> {
    let original = original.trim();
    let selection_text = text.get(selection.clone()).map(str::trim);
    if selection_text == Some(original) {
        return Some(ParameterReplacementTarget::Selected(selection));
    }
    if text.trim() == original {
        return Some(ParameterReplacementTarget::All);
    }
    let (start, end) = statement_around(text, selection.start)?;
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
    let start = range.start.min(text.len());
    let end = range.end.min(text.len());
    let mut out = String::with_capacity(text.len() + replacement.len());
    out.push_str(&text[..start]);
    out.push_str(replacement);
    out.push_str(&text[end..]);
    out
}

#[cfg(test)]
mod statements_tests {
    use super::*;

    /// 从语句区间取原始文本。
    fn text_at(text: &str, run: &SqlStatementRun) -> String {
        text[run.range.clone()].to_string()
    }

    #[test]
    fn builds_runs_with_ordinal_rows_and_ids() {
        let text = "select 1;\nselect 2\n; -- 注释\nselect 3;";
        let runs = build_statement_runs(text);
        // 三句可执行语句（注释行不会成句）。
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].ordinal, 0);
        assert_eq!(runs[1].ordinal, 1);
        assert_eq!(runs[2].ordinal, 2);
        // 首句位于第 0 行。
        assert_eq!(runs[0].start_row, 0);
        assert_eq!(runs[0].end_row, 0);
        // 第二句跨第 1 行与第 2 行（分号在第 2 行）。
        assert_eq!(runs[1].start_row, 1);
        assert_eq!(runs[1].end_row, 1);
        // 语句文本正确。
        assert_eq!(text_at(text, &runs[0]), "select 1");
        assert_eq!(text_at(text, &runs[1]), "select 2");
        // id 唯一且对同一语句稳定。
        assert_ne!(runs[0].id, runs[1].id);
        let again = build_statement_runs(text);
        assert_eq!(again[1].id, runs[1].id);
    }

    #[test]
    fn ids_differ_when_text_changes() {
        let text = "select 1;";
        let before = build_statement_runs(text);
        let changed = build_statement_runs("select 999;");
        // 同一 ordinal 但文本不同 -> id 不同（供状态衰减判定）。
        assert_ne!(before[0].id, changed[0].id);
    }

    #[test]
    fn skips_strings_and_comments() {
        let text = "insert into t values(';'); /* a; */ select 1;";
        let runs = build_statement_runs(text);
        assert_eq!(runs.len(), 2);
        assert_eq!(text_at(text, &runs[0]), "insert into t values(';')");
        assert_eq!(text_at(text, &runs[1]), "select 1");
    }

    #[test]
    fn row_for_offset_counts_newlines() {
        assert_eq!(row_for_offset("a\nb\nc", 0), 0);
        assert_eq!(row_for_offset("a\nb\nc", 2), 1);
        assert_eq!(row_for_offset("a\nb\nc", 4), 2);
        // 越界安全。
        assert_eq!(row_for_offset("", 5), 0);
    }

    #[test]
    fn status_map_tracks_and_decays_runs() {
        let text = "select 1;\nselect 2;";
        let runs = build_statement_runs(text);
        let mut map = SqlStatementStatusMap::default();
        assert_eq!(map.status_for_run(&runs[0]), None);

        map.set_status(&runs[0], Some(SqlStatementStatus::Running));
        assert_eq!(map.status_for_run(&runs[0]), Some(SqlStatementStatus::Running));
        // 未设置状态的第二条语句仍为 None。
        assert_eq!(map.status_for_run(&runs[1]), None);

        // 按行查询：第 1 行属于第二条语句。
        assert_eq!(map.status_for_row(text, 1), None);
        assert_eq!(map.status_for_row(text, 0), Some(SqlStatementStatus::Running));

        // 语句被编辑后 id 变化 -> 状态自然衰减为 None。
        let edited_runs = build_statement_runs("select 999;\nselect 2;");
        assert_eq!(map.status_for_run(&edited_runs[0]), None);
        // 未编辑的第二条语句仍保留状态（若已设置）。
        map.set_status(&edited_runs[1], Some(SqlStatementStatus::Success));
        assert_eq!(map.status_for_run(&edited_runs[1]), Some(SqlStatementStatus::Success));
    }

    #[test]
    fn status_map_clear_via_none() {
        let text = "select 1;";
        let runs = build_statement_runs(text);
        let mut map = SqlStatementStatusMap::default();
        map.set_status(&runs[0], Some(SqlStatementStatus::Running));
        map.set_status(&runs[0], None);
        assert_eq!(map.status_for_run(&runs[0]), None);
    }

    #[test]
    fn parameter_target_matches_selection() {
        let text = "select 1;";
        // 选区文本（trim）等于 original -> 选中。
        let sel = 0..8; // "select 1"
        assert_eq!(
            parameter_replacement_target(text, sel.clone(), "select 1"),
            Some(ParameterReplacementTarget::Selected(sel))
        );
        // 选区空白 -> 不匹配文本，回退到全文/语句判断。
        let sel2 = 0..0;
        assert_ne!(
            parameter_replacement_target(text, sel2.clone(), "select 1"),
            Some(ParameterReplacementTarget::Selected(sel2))
        );
    }

    #[test]
    fn parameter_target_matches_all_or_statement() {
        // 全文匹配（整篇文本 trim 后等于 original）。
        let text = "select 1";
        assert_eq!(
            parameter_replacement_target(text, 0..0, "select 1"),
            Some(ParameterReplacementTarget::All)
        );
        // 全文带分号且仅一句：整篇不等 -> 命中语句落点。
        let text3 = "select 1;";
        assert_eq!(
            parameter_replacement_target(text3, 0..0, "select 1"),
            Some(ParameterReplacementTarget::Statement(0..8))
        );
        // 光标在首语句内且语句文本匹配 -> Statement。
        let text2 = "select 999; select 2;";
        let sel = 5..5; // 处于首语句 "select 999"
        assert_eq!(
            parameter_replacement_target(text2, sel.clone(), "select 999"),
            Some(ParameterReplacementTarget::Statement(0..10))
        );
        // 均不匹配 -> None。
        assert_eq!(
            parameter_replacement_target(text2, sel.clone(), "nope"),
            None
        );
    }

    #[test]
    fn replace_text_in_range_replaces_segment() {
        assert_eq!(
            replace_text_in_range("select 1; select 2;", 0..8, "select 9"),
            "select 9; select 2;"
        );
        // 越界安全。
        assert_eq!(
            replace_text_in_range("abc", 0..99, "XY"),
            "XY"
        );
    }
}
