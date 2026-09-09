// sql_editor_adapter/execution.rs —— SQL 语句切分与执行单元。
//
// 提供纯函数语句切分器 `split_statements` 与 `statement_around`，
// 均可被 fluxdb-editor-core 的 `ExecutionAdapter` 复用；不执行任何 SQL。

/// 把 SQL 文本按 `;` 切分成语句区间（字节偏移，半开区间 `[start, end)`）。
///
/// 切分时正确跳过字符串（`'`、`"`、反引号）与注释（`--`、`#`、`/* */`）。
/// 返回的区间已去除首尾空白，但不包含结尾分号。
pub fn split_statements(text: &str) -> Vec<Range> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut statements = Vec::new();
    let mut i = 0;
    // 当前语句首个非空白字符偏移；None 表示尚未开始。
    let mut start_ptr: Option<usize> = None;
    // 当前字符串引号（None 表示不在字符串内）。
    let mut quote: Option<u8> = None;
    let mut in_block = false;
    let mut in_line = false;

    while i < n {
        let b = bytes[i];
        // 字符串优先于注释识别：`'--'`、`'/*'` 和 `';'` 都只是字符串内容。
        if let Some(q) = quote {
            if b == b'\\' && i + 1 < n {
                i += 2;
                continue;
            }
            if b == q {
                if i + 1 < n && bytes[i + 1] == q {
                    i += 2;
                } else {
                    quote = None;
                    i += 1;
                }
                continue;
            }
            i += 1;
            continue;
        }
        if in_line {
            if b == b'\n' {
                in_line = false;
            }
            i += 1;
            continue;
        }
        if in_block {
            if b == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                in_block = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        // 注释起始
        if b == b'-' && i + 1 < n && bytes[i + 1] == b'-' {
            in_line = true;
            i += 2;
            continue;
        }
        if b == b'#' {
            in_line = true;
            i += 1;
            continue;
        }
        // 行注释 `//`（兼容代码编辑器输入）。
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            in_line = true;
            i += 2;
            continue;
        }
        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            in_block = true;
            i += 2;
            continue;
        }
        // 进入字符串
        if b == b'\'' || b == b'"' || b == b'`' {
            quote = Some(b);
            i += 1;
            continue;
        }
        // 语句分隔符
        if b == b';' {
            if let Some(s) = start_ptr {
                let mut end = i;
                while end > s && bytes[end - 1].is_ascii_whitespace() {
                    end -= 1;
                }
                statements.push(Range::new(s, end));
                start_ptr = None;
            }
            i += 1;
            continue;
        }
        // 记录语句起点（忽略空白）
        if start_ptr.is_none() && !b.is_ascii_whitespace() {
            start_ptr = Some(i);
        }
        i += 1;
    }
    // 收尾：最后一段语句
    if let Some(s) = start_ptr {
        let mut end = n;
        while end > s && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if end > s {
            statements.push(Range::new(s, end));
        }
    }
    statements
}

/// 在不可变编辑器快照上切分语句，不物化全文字符串。
///
/// 该扫描器与 `split_statements` 保持同一语义，但每次只读取一个 Rope 字节，
/// 适合折叠、诊断索引和执行定位等只需要范围的路径。
pub fn split_statement_ranges_snapshot(
    snapshot: &fluxdb_editor_core::BufferSnapshot,
) -> Vec<Range> {
    let n = snapshot.len();
    let mut statements = Vec::new();
    let mut bytes = snapshot
        .text_chunks_in_range(Range::new(0, n))
        .flat_map(|chunk| chunk.iter().copied())
        .enumerate()
        .peekable();
    let mut start_ptr = None;
    let mut quote = None;
    let mut in_block = false;
    let mut in_line = false;

    while let Some((i, b)) = bytes.next() {
        if let Some(q) = quote {
            if b == b'\\' && bytes.peek().is_some() {
                bytes.next();
                continue;
            }
            if b == q {
                if bytes.peek().is_some_and(|(_, next)| *next == q) {
                    bytes.next();
                } else {
                    quote = None;
                }
                continue;
            }
            continue;
        }
        if in_line {
            if b == b'\n' {
                in_line = false;
            }
            continue;
        }
        if in_block {
            if b == b'*' && bytes.peek().is_some_and(|(_, next)| *next == b'/') {
                in_block = false;
                bytes.next();
            }
            continue;
        }
        if b == b'\'' || b == b'"' || b == b'`' {
            quote = Some(b);
            continue;
        }
        if (b == b'-' && bytes.peek().is_some_and(|(_, next)| *next == b'-'))
            || b == b'#'
            || (b == b'/' && bytes.peek().is_some_and(|(_, next)| *next == b'/'))
        {
            in_line = true;
            if b != b'#' {
                bytes.next();
            }
            continue;
        }
        if b == b'/' && bytes.peek().is_some_and(|(_, next)| *next == b'*') {
            in_block = true;
            bytes.next();
            continue;
        }
        if b == b';' {
            if let Some(s) = start_ptr {
                let mut end = i;
                while end > s
                    && snapshot
                        .byte_at(end.saturating_sub(1))
                        .is_some_and(|byte| byte.is_ascii_whitespace())
                {
                    end -= 1;
                }
                if end > s {
                    statements.push(Range::new(s, end));
                }
                start_ptr = None;
            }
            continue;
        }
        if start_ptr.is_none() && !b.is_ascii_whitespace() {
            start_ptr = Some(i);
        }
    }
    if let Some(s) = start_ptr {
        let mut end = n;
        while end > s
            && snapshot
                .byte_at(end.saturating_sub(1))
                .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            end -= 1;
        }
        if end > s {
            statements.push(Range::new(s, end));
        }
    }
    statements
}

/// 在快照上定位光标所在语句，避免为执行单条 SQL 复制全文。
pub fn statement_around_snapshot(
    snapshot: &fluxdb_editor_core::BufferSnapshot,
    offset: usize,
) -> Option<(usize, usize)> {
    split_statement_ranges_snapshot(snapshot)
        .into_iter()
        .find(|range| offset >= range.start && offset <= range.end)
        .map(|range| (range.start, range.end))
}

/// 返回包含给定字节偏移的语句区间 `(start, end)`。
///
/// 若 offset 位于语句边界处（等于某结束端），归入该语句；否则返回
/// `None` 供调用方决定回退策略。
pub fn statement_around(text: &str, offset: usize) -> Option<(usize, usize)> {
    let statements = split_statements(text);
    for stmt in &statements {
        if offset >= stmt.start && offset <= stmt.end {
            return Some((stmt.start, stmt.end));
        }
    }
    None
}

#[cfg(test)]
mod execution_tests {
    use super::*;

    #[test]
    fn splits_simple_statements() {
        let stmts = split_statements("select 1; select 2;");
        assert_eq!(stmts.len(), 2);
        assert_eq!(&gcd_text("select 1; select 2;", stmts[0]), "select 1");
    }

    #[test]
    fn ignores_semicolon_in_strings() {
        let text = "insert into t values('a;b'); select 1";
        let stmts = split_statements(text);
        assert_eq!(stmts.len(), 2);
        assert_eq!(&gcd_text(text, stmts[0]), "insert into t values('a;b')");
    }

    #[test]
    fn ignores_comment_markers_and_escaped_quotes_in_strings() {
        let text = "select '-- not a comment; /* still text */', 'it''s;ok'; select 2";
        let stmts = split_statements(text);
        assert_eq!(stmts.len(), 2);
        assert_eq!(&text[stmts[0].start..stmts[0].end], "select '-- not a comment; /* still text */', 'it''s;ok'");
    }

    #[test]
    fn snapshot_scanner_matches_string_scanner() {
        let text = "-- header\nselect a from t; /* x; */ select 'a;b'";
        let snapshot = fluxdb_editor_core::EditorBuffer::new_from(text).snapshot();
        assert_eq!(split_statement_ranges_snapshot(&snapshot), split_statements(text));
        assert_eq!(statement_around_snapshot(&snapshot, text.find("select 'a").unwrap()), Some((split_statements(text)[1].start, split_statements(text)[1].end)));
    }

    #[test]
    fn ignores_semicolon_in_comments() {
        let text = "select 1 -- foo; bar\n; select 2";
        let stmts = split_statements(text);
        assert_eq!(stmts.len(), 2);
        assert_eq!(&gcd_text(text, stmts[1]), "select 2");
    }

    #[test]
    fn ignores_semicolon_in_slash_comment() {
        let text = "select 1; // foo; bar\nselect 2";
        let stmts = split_statements(text);
        assert_eq!(stmts.len(), 2);
        assert_eq!(&gcd_text(text, stmts[1]), "select 2");
    }

    #[test]
    fn handles_block_comment() {
        let text = "/* a; b */ select 1;";
        let stmts = split_statements(text);
        // 块注释中的分号不切分；语句范围不含前导注释，仅保留可执行语句。
        assert_eq!(stmts.len(), 1);
        assert_eq!(&gcd_text(text, stmts[0]), "select 1");
    }

    #[test]
    fn strips_leading_trailing_whitespace() {
        let text = "  select 1 ;  ";
        let stmts = split_statements(text);
        assert_eq!(stmts.len(), 1);
        assert_eq!(&gcd_text(text, stmts[0]), "select 1");
    }

    #[test]
    fn statement_around_finds_containing() {
        let text = "select 1; select 22; select 333";
        // offset 命中第二条语句。
        let (s, e) = statement_around(text, "select 1; ".len()).unwrap();
        assert_eq!(&text[s..e], "select 22");
        // 光标在语句结尾，归入该语句。
        let (s2, e2) = statement_around(text, 0).unwrap();
        assert_eq!(&text[s2..e2], "select 1");
    }

    /// 测试辅助：取文本子串。
    fn gcd_text(text: &str, range: Range) -> String {
        text[range.start..range.end].to_string()
    }
}
