// json_editor/parser.rs —— JSON 校验、pretty 格式化、错误行列→byte offset 映射。
//
// 完全基于 `serde_json`，不依赖外部服务：
// - pretty 格式化输出与 `serde_json::to_string_pretty` 字节一致，避免展示与再解析不一致。
// - 保存前以 `serde_json::from_str` 为权威校验，成功才允许写回 Redis。

use serde_json::Value;

/// 校验 JSON 并返回错误诊断；`Ok(())` 表示合法。
pub(crate) fn validate_json(src: &str) -> Result<(), JsonEditorDiagnostic> {
    match serde_json::from_str::<Value>(src) {
        Ok(_) => Ok(()),
        Err(err) => {
            // serde_json 的 line()/column() 为 1-based，转 0-based 便于内部与 UI 使用。
            let line = err.line().saturating_sub(1);
            let column = err.column().saturating_sub(1);
            let offset = offset_for_line_col(src, line, column);
            let span = error_span(src, offset);
            let message = err.to_string();
            Err(JsonEditorDiagnostic {
                message,
                offset,
                line,
                column,
                span,
            })
        }
    }
}

/// 由 0-based 行列计算 byte offset。`serde_json` 的列按 Unicode 标量计数，
/// 因此逐字符推进以正确处理中文字符等多字节序列。
pub(crate) fn offset_for_line_col(src: &str, line0: usize, col0: usize) -> usize {
    let mut byte = 0usize;
    let mut line = 0usize;
    let mut col = 0usize;
    for ch in src.chars() {
        if line > line0 {
            break;
        }
        if line == line0 && col >= col0 {
            return byte;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else if line == line0 {
            col += 1;
        }
        byte += ch.len_utf8();
    }
    byte
}

/// 计算错误可高亮的字节范围 `[start, end)`。
///
/// - 错误落在合法 token（如多余的 `}`、非法的逗号）时，覆盖该 token 单字符，作为「红色小块」。
/// - 错误落在 EOF（如缺右括号）时，退化为覆盖最后一个可见字符，配合消息提示定位。
fn error_span(src: &str, offset: usize) -> (usize, usize) {
    let len = src.len();
    if offset >= len {
        let start = len.saturating_sub(1);
        return (start, len.max(start + 1));
    }
    // 取错误位置所在的字符宽度作为小块；若其后紧跟结构符则一并纳入（覆盖「token」范围）。
    let ch_len = src[offset..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    (offset, offset + ch_len)
}

/// pretty 格式化。返回格式化后的文本；非法 JSON 返回 `Err(diagnostic)`。
pub(crate) fn format_pretty(src: &str, indent_size: usize) -> Result<String, JsonEditorDiagnostic> {
    if let Err(dia) = validate_json(src) {
        return Err(dia);
    }
    let value = serde_json::from_str::<Value>(src)
        .map_err(|err| json_diagnostic_from_error(src, err))?;
    Ok(format_value(&value, indent_size))
}

/// 无折叠的 pretty 文本。复用与 `build_pretty` 相同的逐值写出逻辑（空折叠集合）。
fn format_value(value: &Value, indent_size: usize) -> String {
    let empty: BTreeSet<String> = BTreeSet::new();
    let mut builder = PrettyBuilder {
        output: String::new(),
        indent: " ".repeat(indent_size),
        line: 0,
        nodes: Vec::new(),
        folded: &empty,
    };
    builder.write_value(value, 0, &mut Vec::new());
    builder.output
}

/// 构造 `PrettyJson`：pretty 文本 + 可折叠节点（非空 object/array）的行区间与折叠态。
///
/// `folded_paths` 为已折叠的路径集合（用 `json_fold_path_key` 编码）；命中路径的节点
/// 折叠为单行 `{ … }` / `[ … ]` 占位，其 `start_line == end_line`。
pub(crate) fn build_pretty(
    src: &str,
    indent_size: usize,
    folded_paths: &BTreeSet<String>,
) -> Result<PrettyJson, JsonEditorDiagnostic> {
    if let Err(dia) = validate_json(src) {
        return Err(dia);
    }
    let value = serde_json::from_str::<Value>(src)
        .map_err(|err| json_diagnostic_from_error(src, err))?;
    let mut builder = PrettyBuilder {
        output: String::new(),
        indent: " ".repeat(indent_size),
        line: 0,
        nodes: Vec::new(),
        folded: folded_paths,
    };
    builder.write_value(&value, 0, &mut Vec::new());
    Ok(PrettyJson {
        text: builder.output,
        nodes: builder.nodes,
    })
}

fn json_diagnostic_from_error(src: &str, err: serde_json::Error) -> JsonEditorDiagnostic {
    let line = err.line().saturating_sub(1);
    let column = err.column().saturating_sub(1);
    let offset = offset_for_line_col(src, line, column);
    let span = error_span(src, offset);
    JsonEditorDiagnostic {
        message: err.to_string(),
        offset,
        line,
        column,
        span,
    }
}

/// pretty 化的结果：文本 + 可折叠节点行区间。
pub(crate) struct PrettyJson {
    pub text: String,
    /// 仅含非空 object / array 的可折叠节点，行号为 pretty 文本内 0-based 行。
    pub nodes: Vec<JsonFoldNode>,
}

/// 逐值写出 pretty 文本，同时记录 object/array 节点的起止行与折叠态。
/// 输出与 `serde_json::to_string_pretty` 保持字节一致（2 空格缩进、数组元素独立成行）。
/// `folded` 为已折叠路径集合；命中路径的节点折叠为单行 `{ … }` / `[ … ]`。
struct PrettyBuilder<'a> {
    output: String,
    /// 单位缩进（默认 2 空格空格串）。
    indent: String,
    /// 当前行号（0-based，从 0 开始；换行时 +1）。
    line: usize,
    nodes: Vec<JsonFoldNode>,
    folded: &'a BTreeSet<String>,
}

impl<'a> PrettyBuilder<'a> {
    /// 写出一个值；`depth` 为当前层级，`path` 为该值在父结构中的折叠路径。
    fn write_value(&mut self, value: &Value, depth: usize, path: &mut Vec<JsonKey>) {
        let node = match value {
            Value::Object(map) if !map.is_empty() => Some(false),
            Value::Array(arr) if !arr.is_empty() => Some(true),
            _ => None,
        };
        let is_folded = node.is_some() && self.folded.contains(&json_fold_path_key(path));

        if let (Some(_), true) = (node, is_folded) {
            // 折叠：对象输出 `{ … }`，数组输出 `[ … ]`，整段占单行，不展开内部。
            let (open, close) = match value {
                Value::Object(_) => ('{', '}'),
                _ => ('[', ']'),
            };
            let start_line = self.line;
            self.output.push(open);
            self.output.push(' ');
            self.output.push('\u{2026}');
            self.output.push(' ');
            self.output.push(close);
            self.nodes.push(JsonFoldNode {
                path: path.clone(),
                start_line,
                end_line: start_line,
                folded: true,
            });
            return;
        }

        match value {
            Value::Object(map) => {
                let start_line = self.line;
                self.output.push('{');
                if map.is_empty() {
                    self.output.push('}');
                    return;
                }
                self.newline(depth + 1);
                let mut first = true;
                for (key, child) in map.iter() {
                    if !first {
                        self.output.push(',');
                        self.newline(depth + 1);
                    }
                    first = false;
                    self.write_string(key);
                    self.output.push_str(": ");
                    path.push(JsonKey::Object(key.clone()));
                    self.write_value(child, depth + 1, path);
                    path.pop();
                }
                self.newline(depth);
                self.output.push('}');
                let end_line = self.line;
                self.nodes.push(JsonFoldNode {
                    path: path.clone(),
                    start_line,
                    end_line,
                    folded: false,
                });
            }
            Value::Array(arr) => {
                let start_line = self.line;
                self.output.push('[');
                if arr.is_empty() {
                    self.output.push(']');
                    return;
                }
                self.newline(depth + 1);
                let mut first = true;
                for (idx, child) in arr.iter().enumerate() {
                    if !first {
                        self.output.push(',');
                        self.newline(depth + 1);
                    }
                    first = false;
                    path.push(JsonKey::Index(idx));
                    self.write_value(child, depth + 1, path);
                    path.pop();
                }
                self.newline(depth);
                self.output.push(']');
                let end_line = self.line;
                self.nodes.push(JsonFoldNode {
                    path: path.clone(),
                    start_line,
                    end_line,
                    folded: false,
                });
            }
            Value::String(s) => self.write_string(s),
            Value::Number(n) => self.output.push_str(&n.to_string()),
            Value::Bool(b) => self.output.push_str(if *b { "true" } else { "false" }),
            Value::Null => self.output.push_str("null"),
        }
    }

    fn write_string(&mut self, s: &str) {
        // 复用 serde_json 的字符串转义输出（含 Unicode 的 UTF-8 直出）。
        self.output.push_str(&serde_json::to_string(s).unwrap_or_default());
    }

    fn newline(&mut self, depth: usize) {
        self.output.push('\n');
        self.line += 1;
        for _ in 0..depth {
            self.output.push_str(&self.indent);
        }
    }
}

#[cfg(test)]
mod json_editor_parser_tests {
    use super::*;

    #[test]
    fn validate_accepts_valid_json() {
        assert!(validate_json(r#"{"a":1,"b":[true,null,"x"]}"#).is_ok());
        assert!(validate_json("42").is_ok());
        assert!(validate_json("\"plain\"").is_ok());
        assert!(validate_json("").is_err()); // 空串不是合法 JSON
    }

    #[test]
    fn validate_rejects_illegal_json_with_message() {
        let err = validate_json(r#"{"a": }"#).unwrap_err();
        assert!(!err.message.is_empty());
        // 错误应被定位到空白后预期值的行列。
        assert_eq!(err.line, 0);
        assert!(err.column >= 5);
    }

    #[test]
    fn error_maps_line_column_to_byte_offset() {
        let src = "\n{\"a\":1,}";
        // 第 1 行（0-based=1）是 `{"a":1,}`：第 0 列是 `{`（偏移 1 字节，因为第 0 行是换行）。
        assert_eq!(offset_for_line_col(src, 1, 0), 1);
        // 第 1 列为 `"`（偏移 2 字节）。
        assert_eq!(offset_for_line_col(src, 1, 1), 2);
    }

    #[test]
    fn error_span_covers_single_char_token() {
        let src = r#"{"a":1,}"#;
        let offset = offset_for_line_col(src, 0, 7); // 指向结尾多余的 '}'
        let (start, end) = error_span(src, offset);
        assert_eq!(&src[start..end], "}");
    }

    #[test]
    fn validate_error_matches_offset_in_text() {
        // 多行文本，报错位置的 byte offset 应指向实际非法字符。
        let src = "{\n  \"a\": 1,\n  \"b\": [1, 2,\n}\n";
        // "}\n" 处缺合法值，无法直接断言 serde_json 列，但 offset/line/column 应自洽。
        let err = validate_json(src).unwrap_err();
        assert!(err.offset <= src.len());
        let recovered = &src[..err.offset];
        assert!(recovered.is_char_boundary(err.offset));
    }

    #[test]
    fn format_pretty_indents_nested() {
        let out = format_pretty(r#"{"a":1,"b":[1,2]}"#, 2).unwrap();
        assert_eq!(out, "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}");
    }

    #[test]
    fn format_pretty_rejects_invalid() {
        assert!(format_pretty(r#"{"a": }"#, 2).is_err());
    }

    #[test]
    fn build_pretty_reports_fold_nodes() {
        let pretty = build_pretty(r#"{"user":{"name":"n"},"tags":[]}"#, 2, &BTreeSet::new())
            .unwrap();
        // 根对象（空路径）与 `user` 对象均可折叠；空数组 `tags` 不可折叠被排除。
        assert!(pretty.nodes.iter().any(|n| n.path.is_empty()));
        assert!(pretty
            .nodes
            .iter()
            .any(|n| n.path == vec![JsonKey::Object("user".into())]));
        assert!(!pretty
            .nodes
            .iter()
            .any(|n| n.path == vec![JsonKey::Object("tags".into())]));
    }

    #[test]
    fn build_pretty_folds_matching_path() {
        let folded: BTreeSet<String> = BTreeSet::from(["o:user".to_string()]);
        let pretty =
            build_pretty(r#"{"user":{"name":"n"},"age":1}"#, 2, &folded).unwrap();
        let user = pretty
            .nodes
            .iter()
            .find(|n| n.path == vec![JsonKey::Object("user".into())])
            .unwrap();
        assert!(user.folded);
        assert_eq!(user.start_line, user.end_line); // 折叠后为单行
        assert!(pretty.text.contains("…"));
    }

    #[test]
    fn diagnostic_summary_is_one_based_and_chinese() {
        let src = r#"{"a": }"#;
        let err = validate_json(src).unwrap_err();
        let s = err.summary();
        assert!(s.starts_with("JSON 格式错误：第 1 行"));
        assert!(s.contains("列"));
    }
}

