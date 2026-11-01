// MongoDB 聚合管道的 stage 卡片模型：管道文本 ↔ stage 数组的纯函数转换。
//
// 管道文本（Extended JSON 数组）始终是唯一真源，卡片模式只是另一种输入法：
// 每次改动都重新序列化写回文本，与条件构建器同一套约定。
//
// 只接受「每个 stage 是单键对象、键以 `$` 开头」的数组。其它写法（多键对象、
// 非运算符键）原样留在文本里、拒绝进入卡片模式——猜着改写会悄悄改变管道语义。

/// 一个聚合 stage：运算符 + 参数文本 + 启用态。
///
/// `body` 是该 stage 值的 JSON 文本（`{"$count": "total"}` 里是 `"total"`）。
/// 停用的 stage 仍留在卡片里，只是不写进执行的管道——与 Compass 的「停用阶段」一致。
#[derive(Clone, Debug, PartialEq)]
struct MongoAggregationStage {
    /// stage 运算符，如 `$match`。
    key: String,
    /// 参数 JSON 文本；空串按 `{}` 处理。
    body: String,
    enabled: bool,
}

/// 卡片数量上限：再多就该直接写文本了，也不至于把界面撑成上千个编辑器。
const MONGO_AGGREGATION_MAX_STAGES: usize = 50;

/// stage 参数编辑器的行高（11px × 1.5），与卡片正文排版共用。
const MONGO_AGGREGATION_LINE_HEIGHT: f32 = 16.5;
/// 参数编辑器上下内边距：`Input::from_state` 的多行分支写死 8px，外部改不掉。
const MONGO_AGGREGATION_EDITOR_PADDING: f32 = 8.;
/// 参数编辑器的行数区间：再高就在编辑器里滚动，卡片不许把下面的阶段顶出视口。
const MONGO_AGGREGATION_BODY_MIN_ROWS: usize = 2;
const MONGO_AGGREGATION_BODY_MAX_ROWS: usize = 8;

/// 卡片区封顶高度：stage 再多也不许把结果区挤没（与条件树的 240px 同一取舍）。
const MONGO_AGGREGATION_CARDS_MAX_HEIGHT: f32 = 320.;

/// 预览卡片的宽度：与 Compass 一样按「一屏放得下两张」定，字段名大都不长，
/// 再宽全是空白，再窄字段值要频繁截断。超出的横向滚动。
const MONGO_AGGREGATION_PREVIEW_CARD_WIDTH: f32 = 520.;

/// 单阶段预览最多取几条。预览是「看一眼这一步输出什么」，不是取全量；
/// 与 Compass 的 `DEFAULT_PREVIEW_LIMIT` 同量级，够看清结构又不至于拉回几千条。
const MONGO_AGGREGATION_PREVIEW_LIMIT: u64 = 10;


/// 参数编辑器高度：按内容行数长高，夹在 2–8 行之间。
fn mongo_aggregation_body_height(body: &str) -> f32 {
    let rows = body.split('\n').count().clamp(
        MONGO_AGGREGATION_BODY_MIN_ROWS,
        MONGO_AGGREGATION_BODY_MAX_ROWS,
    );
    rows as f32 * MONGO_AGGREGATION_LINE_HEIGHT + 2. * MONGO_AGGREGATION_EDITOR_PADDING
}

/// 管道文本 → stage 列表。空文本等价于空管道。
///
/// 解析走与查询条件相同的「先严格 JSON、再 JSON5」规则：严格解析能保住整数类型，
/// 回写时 `{"$limit": 10}` 不会变成 Double。
fn parse_mongo_aggregation_stages(
    text: &str,
) -> Result<Vec<MongoAggregationStage>, SharedString> {
    let error = || tr("app.mongo.aggregation.parse_error");
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let value = parse_mongo_relaxed_json(trimmed).ok_or_else(error)?;
    let serde_json::Value::Array(items) = value else {
        return Err(error());
    };
    if items.len() > MONGO_AGGREGATION_MAX_STAGES {
        return Err(error());
    }
    items
        .into_iter()
        .map(|item| {
            let serde_json::Value::Object(stage) = item else {
                return Err(error());
            };
            // 单键才是「一个 stage」；多键对象在 MongoDB 里会被当成多个 stage，
            // 但卡片上一张卡只该管一个运算符。
            let mut entries = stage.into_iter();
            let (Some((key, body)), None) = (entries.next(), entries.next()) else {
                return Err(error());
            };
            if !key.starts_with('$') {
                return Err(error());
            }
            Ok(MongoAggregationStage {
                key,
                body: mongo_aggregation_body_text(&body),
                enabled: true,
            })
        })
        .collect()
}

/// stage 值 → 编辑器文本。用缩进格式：卡片里的参数是多行结构，
/// 压成一行既看不清嵌套，改起来也无从下手。
fn mongo_aggregation_body_text(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

/// stage 列表 → 管道文本（Extended JSON 数组，缩进排版）。
///
/// 参数**原样内联**，不先解析再序列化：解析失败时整条管道就是非法 JSON，
/// 由执行时的解析错误如实报出来。若把非法 stage 悄悄跳过，一次「填错了」会变成
/// 一次少了 stage 的查询——结果对不上还找不到原因。
///
/// 唯一的例外是**能解析**的参数：走一遍序列化，把参数框里的说明注释
/// （`/** ... */`）去掉。注释是编辑器里的填写提示，不该跟着管道写进 JSON 模式，
/// 更不该被当成「这条管道的内容」发出去。
fn mongo_aggregation_pipeline_text(stages: &[MongoAggregationStage]) -> String {
    let enabled = stages.iter().filter(|stage| stage.enabled);
    let mut text = String::from("[\n");
    let mut first = true;
    for stage in enabled {
        if !first {
            text.push_str(",\n");
        }
        first = false;
        let body = stage.body.trim();
        let body = if body.is_empty() { "{}" } else { body };
        let normalized;
        let body = match mongo_aggregation_body_value(body) {
            Some(value) => {
                normalized = mongo_aggregation_body_text(&value);
                normalized.as_str()
            }
            None => body,
        };
        // 单行参数（`{}`、`10`、`"total"`）直接收在同一行，多行才展开成块。
        if !body.contains('\n') {
            text.push_str(&format!("  {{ \"{}\": {} }}", stage.key, body));
            continue;
        }
        // 首行接在键后面，后续行整体缩进 4 格，与 `{` 对齐成一块。
        let mut lines = body.split('\n');
        text.push_str(&format!(
            "  {{ \"{}\": {}",
            stage.key,
            lines.next().unwrap_or("")
        ));
        for line in lines {
            text.push_str("\n    ");
            text.push_str(line);
        }
        text.push_str("\n  }");
    }
    // 空管道写成 `[\n]`：只开不闭会多一个空行，解析没问题但看着像丢了内容。
    if first {
        text.push(']');
    } else {
        text.push_str("\n]");
    }
    text
}

/// stage 参数文本 → JSON 值；空文本按 `{}`（还没填参数的 stage）。
fn mongo_aggregation_body_value(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(serde_json::Value::Object(serde_json::Map::new()));
    }
    parse_mongo_relaxed_json(trimmed)
}

/// 新 stage：`$match` + 它自己的默认参数。`$match` 是最常用的起点，`{}` 也是
/// 合法的「不过滤」，新建出来就能直接跑，不用先想参数该写什么形状。
fn mongo_aggregation_new_stage() -> MongoAggregationStage {
    MongoAggregationStage {
        key: "$match".to_string(),
        body: mongo_aggregation_default_body("$match"),
        enabled: true,
    }
}

/// 运算符的默认参数文本：说明注释 + 可直接跑的示例值。
///
/// 注释块抄 Compass 的写法（`/** ... */` 紧跟参数）：参数框里先告诉用户这里要
/// 填什么，再给一个能直接执行的值。注意**不抄它的 `${1:number}` 占位符**——
/// 那是编辑器 snippet 语法，我们没有 snippet 展开，原样落下就是一段非法 JSON，
/// 卡片一创建就标红。
fn mongo_aggregation_default_body(key: &str) -> String {
    let template = mongo_stage_template(key).unwrap_or("{}");
    match mongo_stage_hint(key) {
        Some(hint) => format!("/**\n * {}\n */\n{}", tr(hint), template),
        None => template.to_string(),
    }
}

#[cfg(test)]
mod mongo_aggregation_model_tests {
    use super::*;

    #[test]
    fn empty_text_is_an_empty_pipeline() {
        assert!(parse_mongo_aggregation_stages("  \n ").unwrap().is_empty());
        assert_eq!(parse_mongo_aggregation_stages("[]").unwrap(), Vec::new());
        assert_eq!(mongo_aggregation_pipeline_text(&[]), "[\n]");
    }

    #[test]
    fn stages_round_trip_without_changing_numbers() {
        // 严格 JSON 先解析：`$limit: 10` 必须还是整数，不能被 JSON5 读成 Double。
        let stages =
            parse_mongo_aggregation_stages(r#"[{"$limit": 10}, {"$count": "total"}]"#).unwrap();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].key, "$limit");
        assert_eq!(stages[0].body, "10");
        assert_eq!(stages[1].body, "\"total\"");
        let text = mongo_aggregation_pipeline_text(&stages);
        let reparsed = parse_mongo_aggregation_stages(&text).unwrap();
        assert_eq!(reparsed, stages);
    }

    #[test]
    fn nested_body_is_indented_under_its_key() {
        let stages =
            parse_mongo_aggregation_stages(r#"[{"$match": {"a": 1, "b": {"c": 2}}}]"#).unwrap();
        let text = mongo_aggregation_pipeline_text(&stages);
        assert_eq!(
            text,
            "[\n  { \"$match\": {\n      \"a\": 1,\n      \"b\": {\n        \"c\": 2\n      }\n    }\n  }\n]"
        );
        assert_eq!(parse_mongo_aggregation_stages(&text).unwrap(), stages);
    }

    #[test]
    fn disabled_stages_are_kept_but_not_executed() {
        let mut stages = parse_mongo_aggregation_stages(r#"[{"$match":{}},{"$limit":5}]"#).unwrap();
        stages[1].enabled = false;
        assert_eq!(
            mongo_aggregation_pipeline_text(&stages),
            "[\n  { \"$match\": {} }\n]"
        );
        // 卡片仍持有停用的那一张，切回来能重新启用。
        assert_eq!(stages.len(), 2);
    }

    #[test]
    fn invalid_body_is_kept_verbatim() {
        let stages = vec![MongoAggregationStage {
            key: "$match".into(),
            body: "{\"a\": ".into(),
            enabled: true,
        }];
        assert!(mongo_aggregation_body_value(&stages[0].body).is_none());
        // 原样内联：执行时由解析错误如实报出，而不是静默少一个 stage。
        let text = mongo_aggregation_pipeline_text(&stages);
        assert!(text.contains("{\"a\": "), "{text}");
        assert!(parse_mongo_aggregation_stages(&text).is_err());
    }

    #[test]
    fn stage_templates_and_hints_cover_every_operator() {
        // 下拉菜单、新卡片初值、参数框占位都读这张表：任何一个运算符缺模板或
        // 缺说明，卡片模式就有一处是空白的。
        for (stage, hint, template) in MONGO_STAGE_NAMES {
            assert!(!hint.trim().is_empty(), "{stage} needs a hint");
            assert!(
                mongo_aggregation_body_value(template).is_some(),
                "{stage} template must be valid JSON: {template}"
            );
        }
        assert_eq!(mongo_stage_template("$limit"), Some("10"));
        assert_eq!(mongo_stage_hint("$limit"), Some("app.mongo.stage.limit"));
        assert_eq!(mongo_stage_template("$notAStage"), None);
    }

    #[test]
    fn every_stage_hint_key_exists_in_both_packs() {
        // 说明是语言包 key：漏一条，卡片卡头和下拉就显示 key 本身。
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales");
        for locale in ["en", "zh-CN"] {
            let path = root.join(locale).join("app.json");
            let value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).expect("read bundled locale pack"))
                    .expect("parse bundled locale pack");
            for (stage, hint, _) in MONGO_STAGE_NAMES {
                let found = hint
                    .split('.')
                    .skip(1)
                    .try_fold(&value, |node, key| node.get(key));
                assert!(
                    found.is_some_and(|text| text.as_str().is_some_and(|t| !t.is_empty())),
                    "{locale} is missing the hint for {stage}: {hint}"
                );
            }
        }
    }

    #[test]
    fn default_body_is_a_comment_plus_a_runnable_value() {
        // 抄 Compass 的默认参数写法：说明注释 + 能直接跑的值。它有 snippet
        // 占位符（`number`），我们没有 snippet 展开，落下来必须是合法 JSON。
        let body = mongo_aggregation_default_body("$limit");
        assert!(body.starts_with("/**"), "{body}");
        assert!(body.ends_with('}') || body.ends_with("10"), "{body}");
        assert!(mongo_aggregation_body_value(&body).is_some(), "{body}");

        // 注释只活在编辑器里：写进管道的只有值。
        let stages = vec![MongoAggregationStage {
            key: "$limit".into(),
            body,
            enabled: true,
        }];
        assert_eq!(
            mongo_aggregation_pipeline_text(&stages),
            "[\n  { \"$limit\": 10 }\n]"
        );
    }

    #[test]
    fn non_stage_shapes_are_rejected() {
        for text in [
            r#"{"$match":{}}"#,
            r#"[{"match":{}}]"#,
            r#"[{"$match":{}, "$limit":1}]"#,
            r#"["$match"]"#,
            r#"[1]"#,
        ] {
            assert!(
                parse_mongo_aggregation_stages(text).is_err(),
                "{text} must be rejected"
            );
        }
    }

    #[test]
    fn body_editor_height_clamps_between_two_and_eight_rows() {
        let two = 2. * MONGO_AGGREGATION_LINE_HEIGHT + 2. * MONGO_AGGREGATION_EDITOR_PADDING;
        assert_eq!(mongo_aggregation_body_height("{}"), two);
        let tall = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10";
        assert_eq!(
            mongo_aggregation_body_height(tall),
            8. * MONGO_AGGREGATION_LINE_HEIGHT + 2. * MONGO_AGGREGATION_EDITOR_PADDING
        );
    }
}
