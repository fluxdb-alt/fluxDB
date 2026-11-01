// MongoDB 查询/聚合输入框的补全：按光标所在语法位置给候选（对齐 Compass 的行为）。
//
// 位置判定在纯文本上完成，不依赖 tree-sitter：输入框里是「不完整但容错的」JSON 片段，
// 我们只需要三个事实——是否在字符串里、字符串是键还是值、当前对象的键位是否还空着。
// 判定与候选生成都是纯函数，便于单测；`MongoCompletionProvider` 只做接线。
//
// 候选一律是**本应用能解析的形式**：值位给 Extended JSON（`{"$oid": ""}`），
// 不给 `ObjectId(...)` 这类 shell 写法——连接器用 `serde_json` + `Bson::try_from` 解析，
// shell 写法会直接报解析错误，补进去等于制造一次失败查询。

// 补全相关的类型（`CompletionProvider` / `CompletionLsp` / `Rope` / `CompletionItem` 等）
// 已由 `filter_ui/condition_completion.rs` 在 crate 根引入（本仓库的 include! 共享作用域），
// 这里不再重复 use，否则同名导入会冲突。

/// 输入框属于哪种编辑器：查询三框（filter/projection/sort）还是聚合管道。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MongoCompletionMode {
    Query,
    Aggregation,
}

/// 光标处的补全位置。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MongoCompletionKind {
    /// 键位：字段路径 + 查询运算符。
    Field,
    /// 值位：Extended JSON 构造器。
    Value,
    /// 字符串内的值位：文档里出现过的取值。
    StringValue,
    /// 聚合管道的 stage 键位：stage 名。
    Stage,
    /// stage 内部的键位：聚合运算符 + 字段路径。
    StageKey,
    /// 聚合 stage 字符串里以 `$` 开头的值：字段引用。
    StringFieldReference,
}

/// 一次补全的定位结果。
#[derive(Clone, Debug, Eq, PartialEq)]
struct MongoCompletionSite {
    kind: MongoCompletionKind,
    /// 替换起点（字节偏移）：候选插入时覆盖 `replace_start..cursor`。
    replace_start: usize,
    prefix: String,
    /// 值位所属字段的点分路径（`{"status": "pai` → `status`）；非值位时为空。
    ///
    /// 取值补全必须按字段查：全局词表会让 `status` 冒出文档里任何位置出现过的
    /// 手机号、订单号，候选看着像随机数据。
    field_path: String,
}

/// 扫描用的括号栈：只需要区分「对象还在等键」与「其它」。
#[derive(Clone, Debug, Eq, PartialEq)]
enum MongoFrame {
    /// `key` 是这一层最近一个键，用来在值位拼出字段路径。
    Object {
        expect_key: bool,
        key: Option<String>,
    },
    Array,
}

/// 判定光标处的补全位置；不需要补全时返回 `None`。
fn mongo_completion_site(
    text: &str,
    cursor: usize,
    mode: MongoCompletionMode,
) -> Option<MongoCompletionSite> {
    let cursor = mongo_clamp_boundary(text, cursor);
    let before = &text[..cursor];
    let mut frames: Vec<MongoFrame> = Vec::new();
    let mut in_string = false;
    let mut string_start = 0;
    let mut string_is_value = false;
    let mut last_significant: Option<char> = None;

    let mut chars = before.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        if in_string {
            match character {
                // 转义字符吞掉下一个字符：`\"` 不结束字符串。
                '\\' => {
                    chars.next();
                }
                '"' => {
                    in_string = false;
                    last_significant = Some('"');
                    // 刚闭合的是键：记到当前对象上，值位拼路径时要用。
                    if !string_is_value {
                        let key = before[string_start + 1..index].to_string();
                        if let Some(MongoFrame::Object { key: slot, .. }) = frames.last_mut() {
                            *slot = Some(key);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                string_start = index;
                // 前一个有效字符是 `:` 说明这是值；数组里的字符串没有键，同样算值
                // （`{"tags": ["act` 该补文档里出现过的取值，而不是字段名）。
                string_is_value = last_significant == Some(':')
                    || matches!(frames.last(), Some(MongoFrame::Array));
            }
            '{' => frames.push(MongoFrame::Object {
                expect_key: true,
                key: None,
            }),
            '[' => frames.push(MongoFrame::Array),
            '}' | ']' => {
                frames.pop();
            }
            ',' => {
                if let Some(MongoFrame::Object { expect_key, key }) = frames.last_mut() {
                    *expect_key = true;
                    // 换下一个键：旧键不能再算进新值位的路径。
                    *key = None;
                }
            }
            ':' => {
                if let Some(MongoFrame::Object { expect_key, .. }) = frames.last_mut() {
                    *expect_key = false;
                }
            }
            _ => {}
        }
        if !character.is_whitespace() {
            last_significant = Some(character);
        }
    }

    if in_string {
        let prefix = text[string_start + 1..cursor].to_string();
        let kind = if string_is_value {
            if mode == MongoCompletionMode::Aggregation && prefix.starts_with('$') {
                MongoCompletionKind::StringFieldReference
            } else {
                MongoCompletionKind::StringValue
            }
        } else {
            // JSON 的键本来就写在字符串里，键位判定必须走同一套分层：
            // 管道根数组的元素对象（栈深 2）是 stage 名，更里面才是 stage 内的键。
            mongo_key_kind(frames.len(), mode)
        };
        return Some(MongoCompletionSite {
            kind,
            replace_start: string_start + 1,
            prefix,
            field_path: mongo_frames_path(&frames),
        });
    }

    let (replace_start, prefix) = mongo_word_before(before);
    let expect_key = matches!(
        frames.last(),
        Some(MongoFrame::Object { expect_key: true, .. })
    );
    let kind = if expect_key {
        mongo_key_kind(frames.len(), mode)
    } else {
        MongoCompletionKind::Value
    };
    Some(MongoCompletionSite {
        kind,
        replace_start,
        prefix,
        field_path: mongo_frames_path(&frames),
    })
}

/// 用括号栈里的键拼出点分字段路径；数组层不贡献段。
fn mongo_frames_path(frames: &[MongoFrame]) -> String {
    let mut path = String::new();
    for frame in frames {
        let MongoFrame::Object { key: Some(key), .. } = frame else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(key);
    }
    path
}

/// 键位该给哪类候选：靠括号栈深度区分「管道 stage 名 / stage 内键 / 查询字段」。
///
/// 深度 2 表示「管道根数组 → 元素对象」，也就是 stage 名所在层。
fn mongo_key_kind(frame_depth: usize, mode: MongoCompletionMode) -> MongoCompletionKind {
    match mode {
        MongoCompletionMode::Aggregation if frame_depth == 2 => MongoCompletionKind::Stage,
        MongoCompletionMode::Aggregation => MongoCompletionKind::StageKey,
        MongoCompletionMode::Query => MongoCompletionKind::Field,
    }
}

/// 光标前的半截词：允许字母数字、下划线、`$` 与 `.`（点分路径）。
fn mongo_word_before(before: &str) -> (usize, String) {
    let start = before
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !(character.is_alphanumeric() || matches!(character, '_' | '$' | '.'))
        })
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    (start, before[start..].to_string())
}

fn mongo_clamp_boundary(text: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

/// 生成候选。`fields` 来自已加载文档的字段表（含每个字段自己的取值）。
fn mongo_completions(
    text: &str,
    cursor: usize,
    mode: MongoCompletionMode,
    fields: &[fluxdb_app::MongoField],
) -> Vec<CompletionItem> {
    let cursor = mongo_clamp_boundary(text, cursor);
    let Some(site) = mongo_completion_site(text, cursor, mode) else {
        return Vec::new();
    };
    let prefix = site.prefix.to_ascii_lowercase();
    let in_string = site.replace_start > 0 && text.as_bytes()[site.replace_start - 1] == b'"';
    let mut items = Vec::new();

    match site.kind {
        MongoCompletionKind::Field | MongoCompletionKind::StageKey => {
            for field in fields {
                if !mongo_completion_matches(&field.path, &prefix) {
                    continue;
                }
                let detail = if field.types.is_empty() {
                    tr("app.mongo.completion.field").to_string()
                } else {
                    field.types.join(" | ")
                };
                items.push(mongo_completion_item(
                    &field.path,
                    CompletionItemKind::FIELD,
                    &detail,
                    "0",
                    mongo_key_insert_text(&field.path, in_string),
                ));
            }
            let operators = if site.kind == MongoCompletionKind::StageKey {
                MONGO_STAGE_OPERATORS
            } else {
                MONGO_QUERY_OPERATORS
            };
            for operator in operators {
                if !mongo_completion_matches(operator, &prefix) {
                    continue;
                }
                items.push(mongo_completion_item(
                    operator,
                    CompletionItemKind::OPERATOR,
                    tr("app.mongo.completion.operator"),
                    "1",
                    mongo_key_insert_text(operator, in_string),
                ));
            }
        }
        MongoCompletionKind::Stage => {
            for (stage, _, _) in MONGO_STAGE_NAMES {
                if !mongo_completion_matches(stage, &prefix) {
                    continue;
                }
                items.push(mongo_completion_item(
                    stage,
                    CompletionItemKind::KEYWORD,
                    tr("app.mongo.completion.stage"),
                    "0",
                    mongo_key_insert_text(stage, in_string),
                ));
            }
        }
        MongoCompletionKind::Value => {
            for (label, insert) in MONGO_VALUE_TEMPLATES {
                if !mongo_completion_matches(label, &prefix) {
                    continue;
                }
                items.push(mongo_completion_item(
                    label,
                    CompletionItemKind::VALUE,
                    tr("app.mongo.completion.value"),
                    "0",
                    (*insert).to_string(),
                ));
            }
        }
        MongoCompletionKind::StringValue => {
            // 只给**这个字段**出现过的取值：拿全局词表会让 `status` 冒出手机号。
            let field = fields
                .iter()
                .find(|field| field.path == site.field_path);
            for value in field.map(|field| field.values.as_slice()).unwrap_or_default() {
                if !mongo_completion_matches(value, &prefix) {
                    continue;
                }
                items.push(mongo_completion_item(
                    value,
                    CompletionItemKind::VALUE,
                    tr("app.mongo.completion.word"),
                    "0",
                    value.clone(),
                ));
            }
        }
        MongoCompletionKind::StringFieldReference => {
            for field in fields {
                let label = format!("${}", field.path);
                if !mongo_completion_matches(&label, &prefix) {
                    continue;
                }
                items.push(mongo_completion_item(
                    &label,
                    CompletionItemKind::FIELD,
                    tr("app.mongo.completion.reference"),
                    "0",
                    label.clone(),
                ));
            }
        }
    }

    items.sort_by(|left, right| {
        left.sort_text
            .cmp(&right.sort_text)
            .then_with(|| left.label.cmp(&right.label))
    });
    items.truncate(80);
    // 统一改成一个 text edit 覆盖已输入的部分：弹层本身不负责替换范围。
    for item in &mut items {
        item.text_edit = Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
            range: lsp_types::Range {
                start: mongo_offset_to_position(text, site.replace_start),
                end: mongo_offset_to_position(text, cursor),
            },
            new_text: mongo_item_insert_text(item),
        }));
    }
    items
}

/// 条件构建字段格的补全：格子内容是**裸路径**，不是 JSON 片段。
///
/// 不能复用 [`mongo_completions`]：那套按「键位 / 值位」决定候选，裸文本没有引号
/// 也没有冒号，会被一律判成值位，字段格就只冒出 Extended JSON 构造器。
/// 这里只给字段路径，且原样插入——不加引号、不补冒号。
fn mongo_builder_field_completions(
    text: &str,
    cursor: usize,
    fields: &[fluxdb_app::MongoField],
) -> Vec<CompletionItem> {
    let cursor = mongo_clamp_boundary(text, cursor);
    let (replace_start, prefix) = mongo_word_before(&text[..cursor]);
    let prefix = prefix.to_ascii_lowercase();
    let mut items = Vec::new();
    for field in fields {
        if !mongo_completion_matches(&field.path, &prefix) {
            continue;
        }
        let detail = if field.types.is_empty() {
            tr("app.mongo.completion.field").to_string()
        } else {
            field.types.join(" | ")
        };
        items.push(mongo_completion_item(
            &field.path,
            CompletionItemKind::FIELD,
            detail,
            "0",
            field.path.clone(),
        ));
    }
    finish_mongo_builder_items(&mut items, text, replace_start, cursor);
    items
}

/// 条件构建值格的补全：只给**同一行字段格**所指字段出现过的取值。
///
/// 字段不在当前页的字段目录里（用户手写了目录外的路径）就给空候选：不去全局
/// 词表里凑，凑出来的取值大概率属于别的字段。
fn mongo_builder_value_completions(
    text: &str,
    cursor: usize,
    field_path: &str,
    fields: &[fluxdb_app::MongoField],
) -> Vec<CompletionItem> {
    let cursor = mongo_clamp_boundary(text, cursor);
    let before = &text[..cursor];
    // 值可能已经带引号：正敲的 `"pai`，或整段改写的 `"paid"`。引号算进词里，
    // 否则光标停在收尾引号后会被算成空词，候选追加成 `"paid"active"`——改写变拼接。
    let start = before
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !(character.is_alphanumeric() || matches!(character, '_' | '$' | '.' | '"'))
        })
        .map(|(index, character)| index + character.len_utf8())
        .unwrap_or(0);
    let token = &before[start..];
    let quoted = token.starts_with('"');
    let prefix = token.trim_matches('"').to_ascii_lowercase();
    let mut items = Vec::new();
    let field = fields.iter().find(|field| field.path == field_path.trim());
    for value in field.map(|field| field.values.as_slice()).unwrap_or_default() {
        if !mongo_completion_matches(value, &prefix) {
            continue;
        }
        // 插入时补回收尾引号，否则值格会停在 `"paid` 这半截字符串上。
        let insert = if quoted {
            format!("{value}\"")
        } else {
            value.clone()
        };
        items.push(mongo_completion_item(
            value,
            CompletionItemKind::VALUE,
            tr("app.mongo.completion.word"),
            "0",
            insert,
        ));
    }
    // 开引号原地保留：替换范围从它后面开始。
    let replace_start = if quoted { start + 1 } else { start };
    finish_mongo_builder_items(&mut items, text, replace_start, cursor);
    items
}

/// 排序、截断，并把替换范围写成 `text_edit`（弹层本身不负责替换范围）。
fn finish_mongo_builder_items(
    items: &mut Vec<CompletionItem>,
    text: &str,
    replace_start: usize,
    cursor: usize,
) {
    items.sort_by(|left, right| {
        left.sort_text
            .cmp(&right.sort_text)
            .then_with(|| left.label.cmp(&right.label))
    });
    items.truncate(80);
    for item in items.iter_mut() {
        item.text_edit = Some(lsp_types::CompletionTextEdit::Edit(lsp_types::TextEdit {
            range: lsp_types::Range {
                start: mongo_offset_to_position(text, replace_start),
                end: mongo_offset_to_position(text, cursor),
            },
            new_text: mongo_item_insert_text(item),
        }));
    }
}

/// 候选文本：优先取 `insert_text`（构造器/带冒号的键），否则用标签。
fn mongo_item_insert_text(item: &CompletionItem) -> String {
    item.insert_text
        .clone()
        .unwrap_or_else(|| item.label.clone())
}

fn mongo_completion_item(
    label: &str,
    kind: CompletionItemKind,
    detail: impl Into<String>,
    order: &str,
    insert_text: String,
) -> CompletionItem {
    let mut item = CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.into()),
        filter_text: Some(label.to_ascii_lowercase()),
        insert_text: Some(insert_text),
        ..CompletionItem::default()
    };
    item.sort_text = Some(format!("{order}{}", label.to_ascii_lowercase()));
    item
}

fn mongo_completion_matches(label: &str, prefix: &str) -> bool {
    prefix.is_empty() || label.to_ascii_lowercase().contains(prefix)
}

/// 字段名插入文本：标识符原样，含点/空格等特殊字符的加引号（JSON 转义）。
///
/// 只负责引号；键位还要补冒号，见 [`mongo_key_insert_text`]。
fn mongo_field_insert_text(path: &str) -> String {
    let plain = !path.is_empty()
        && !path.starts_with(|character: char| character.is_ascii_digit())
        && path
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if plain {
        path.to_string()
    } else {
        serde_json::to_string(path).unwrap_or_else(|_| format!("\"{path}\""))
    }
}

/// 键位插入文本：字符串里补 `$gt": `（收尾引号 + 冒号，正好落到值位）；
/// 字符串外补 `"$gt": `（JSON 键必须带引号）。
fn mongo_key_insert_text(key: &str, in_string: bool) -> String {
    if in_string {
        return format!("{key}\": ");
    }
    format!("{}: ", mongo_field_insert_text(key))
}

fn mongo_offset_to_position(text: &str, offset: usize) -> lsp_types::Position {
    let offset = mongo_clamp_boundary(text, offset);
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map(|index| index + 1).unwrap_or(0);
    let character = prefix[line_start..].encode_utf16().count() as u32;
    lsp_types::Position::new(line, character)
}

/// 查询里可用的键位运算符（过滤 + 投影 + 数组/位运算）。
const MONGO_QUERY_OPERATORS: &[&str] = &[
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$in",
    "$nin",
    "$exists",
    "$type",
    "$regex",
    "$options",
    "$mod",
    "$size",
    "$all",
    "$elemMatch",
    "$not",
    "$and",
    "$or",
    "$nor",
    "$expr",
    "$text",
    "$where",
    "$comment",
    "$jsonSchema",
    "$slice",
    "$meta",
    "$bitsAllSet",
    "$bitsAnySet",
    "$bitsAllClear",
    "$bitsAnyClear",
    "$near",
    "$nearSphere",
    "$geoWithin",
    "$geoIntersects",
    "$maxDistance",
    "$minDistance",
];

/// 聚合 stage 名 + 说明文案 key + 参数模板。补全、卡片下拉、新卡片初值都用这一份来源。
///
/// 说明是语言包 key（`app.mongo.stage.*`），不是中文字面量：卡头和下拉都会显示它，
/// 字面量会让英文界面里冒出中文。
///
/// 模板是「选中这个运算符后参数框里该写什么」的最小骨架：用户不必去查文档，
/// 也知道这里要的是整数、字符串还是对象。参数框已有内容时不覆盖（见
/// `set_mongo_aggregation_stage_key`），避免误点下拉冲掉手写的东西。
pub(crate) const MONGO_STAGE_NAMES: &[(&str, &str, &str)] = &[
    ("$match", "app.mongo.stage.match", "{}"),
    ("$project", "app.mongo.stage.project", "{ \"field\": 1 }"),
    ("$group", "app.mongo.stage.group", "{ \"_id\": null }"),
    ("$sort", "app.mongo.stage.sort", "{ \"field\": 1 }"),
    ("$limit", "app.mongo.stage.limit", "10"),
    ("$skip", "app.mongo.stage.skip", "0"),
    ("$unwind", "app.mongo.stage.unwind", "\"$array\""),
    (
        "$lookup",
        "app.mongo.stage.lookup",
        "{ \"from\": \"other\", \"localField\": \"a\", \"foreignField\": \"b\", \"as\": \"joined\" }",
    ),
    ("$addFields", "app.mongo.stage.add_fields", "{ \"field\": 1 }"),
    ("$set", "app.mongo.stage.set", "{ \"field\": 1 }"),
    ("$unset", "app.mongo.stage.unset", "\"field\""),
    ("$count", "app.mongo.stage.count", "\"total\""),
    ("$facet", "app.mongo.stage.facet", "{}"),
    (
        "$bucket",
        "app.mongo.stage.bucket",
        "{ \"groupBy\": \"$field\", \"boundaries\": [0, 10, 20] }",
    ),
    (
        "$bucketAuto",
        "app.mongo.stage.bucket_auto",
        "{ \"groupBy\": \"$field\", \"buckets\": 5 }",
    ),
    ("$replaceRoot", "app.mongo.stage.replace_root", "{ \"newRoot\": {} }"),
    ("$replaceWith", "app.mongo.stage.replace_with", "{}"),
    ("$sample", "app.mongo.stage.sample", "{ \"size\": 10 }"),
    ("$sortByCount", "app.mongo.stage.sort_by_count", "\"$field\""),
    (
        "$graphLookup",
        "app.mongo.stage.graph_lookup",
        "{ \"from\": \"other\", \"startWith\": \"$a\", \"connectFromField\": \"a\", \"connectToField\": \"b\", \"as\": \"path\" }",
    ),
    ("$unionWith", "app.mongo.stage.union_with", "\"other\""),
    ("$out", "app.mongo.stage.out", "\"target\""),
    ("$merge", "app.mongo.stage.merge", "{ \"into\": \"target\" }"),
    ("$indexStats", "app.mongo.stage.index_stats", "{}"),
    ("$collStats", "app.mongo.stage.coll_stats", "{ \"storageStats\": {} }"),
    (
        "$geoNear",
        "app.mongo.stage.geo_near",
        "{ \"near\": { \"type\": \"Point\", \"coordinates\": [0, 0] }, \"distanceField\": \"distance\" }",
    ),
    ("$redact", "app.mongo.stage.redact", "\"$$KEEP\""),
    (
        "$densify",
        "app.mongo.stage.densify",
        "{ \"field\": \"field\", \"range\": { \"step\": 1, \"unit\": \"day\" } }",
    ),
    ("$fill", "app.mongo.stage.fill", "{}"),
    ("$documents", "app.mongo.stage.documents", "[]"),
    ("$setWindowFields", "app.mongo.stage.set_window_fields", "{}"),
];

/// 运算符的说明文案 key（卡片参数框的占位提示、卡头说明、下拉第二行）。
pub(crate) fn mongo_stage_hint(key: &str) -> Option<&'static str> {
    MONGO_STAGE_NAMES
        .iter()
        .find(|(stage, _, _)| *stage == key)
        .map(|(_, hint, _)| *hint)
}

/// 运算符的参数模板（新建卡片 / 切换运算符时填进参数框）。
pub(crate) fn mongo_stage_template(key: &str) -> Option<&'static str> {
    MONGO_STAGE_NAMES
        .iter()
        .find(|(stage, _, _)| *stage == key)
        .map(|(_, _, template)| *template)
}

/// stage 内部键位可用的运算符：累加器 + 常用表达式运算符。
const MONGO_STAGE_OPERATORS: &[&str] = &[
    // 累加器（$group）
    "$sum",
    "$avg",
    "$min",
    "$max",
    "$push",
    "$addToSet",
    "$first",
    "$last",
    "$count",
    "$stdDevPop",
    "$stdDevSamp",
    "$mergeObjects",
    // 表达式
    "$add",
    "$subtract",
    "$multiply",
    "$divide",
    "$mod",
    "$abs",
    "$ceil",
    "$floor",
    "$round",
    "$concat",
    "$substrCP",
    "$toLower",
    "$toUpper",
    "$split",
    "$strLenCP",
    "$ifNull",
    "$cond",
    "$switch",
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$and",
    "$or",
    "$not",
    "$in",
    "$size",
    "$arrayElemAt",
    "$filter",
    "$map",
    "$reduce",
    "$literal",
    "$let",
    "$type",
    "$convert",
    "$toString",
    "$toInt",
    "$toDouble",
    "$toDate",
    "$toObjectId",
    "$dateToString",
    "$dateFromString",
    "$year",
    "$month",
    "$dayOfMonth",
    "$hour",
    "$minute",
    "$second",
    "$isArray",
    "$objectToArray",
    "$arrayToObject",
];

/// 值位模板：Extended JSON 形式，粘贴即可解析。
const MONGO_VALUE_TEMPLATES: &[(&str, &str)] = &[
    ("ObjectId", "{\"$oid\": \"\"}"),
    ("Date", "{\"$date\": \"\"}"),
    ("NumberInt", "{\"$numberInt\": \"\"}"),
    ("NumberLong", "{\"$numberLong\": \"\"}"),
    ("NumberDecimal", "{\"$numberDecimal\": \"\"}"),
    ("Regex", "{\"$regularExpression\": {\"pattern\": \"\", \"options\": \"\"}}"),
    ("Binary", "{\"$binary\": {\"base64\": \"\", \"subType\": \"\"}}"),
    ("Timestamp", "{\"$timestamp\": {\"t\": 0, \"i\": 0}}"),
    ("MinKey", "{\"$minKey\": 1}"),
    ("MaxKey", "{\"$maxKey\": 1}"),
];

/// 补全挂在哪个输入框上。
#[derive(Clone)]
enum MongoCompletionTarget {
    /// 查询/聚合编辑器：按 JSON 语法位置给候选。
    Query(MongoCompletionMode),
    /// 条件构建的字段格：只给字段路径（裸文本）。
    BuilderField,
    /// 条件构建的值格：只给同一行字段格所指字段的取值，所以要拿到字段格实体。
    BuilderValue {
        field: gpui::WeakEntity<EditorState>,
    },
}

/// 给编辑器装上补全：字段表与取值词表在每次补全时从标签页状态读取。
///
/// 不在创建时把目录拷进 provider：抽样与翻页都会往目录里加字段，拷贝出去的那份
/// 会立刻过期，用户就得先切一次标签页才能补到新字段。
pub(crate) fn configure_mongo_completion(
    editor: &mut EditorState,
    mode: MongoCompletionMode,
    view: gpui::WeakEntity<NavicatMain>,
    tab_id: TabId,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<EditorState>,
) {
    install_mongo_completion(
        editor,
        MongoCompletionTarget::Query(mode),
        view,
        tab_id,
        window,
        cx,
    );
}

/// 给条件构建的字段格装上补全。
///
/// 格子里是裸字段路径：打字即出候选，选中直接填空。右侧那个「浏览字段目录」的
/// 下拉箭头已去掉——补全占了同一个用途，多一个入口只会分叉。
pub(crate) fn configure_mongo_builder_field_completion(
    editor: &mut EditorState,
    view: gpui::WeakEntity<NavicatMain>,
    tab_id: TabId,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<EditorState>,
) {
    install_mongo_completion(
        editor,
        MongoCompletionTarget::BuilderField,
        view,
        tab_id,
        window,
        cx,
    );
}

/// 给条件构建的值格装上补全。
///
/// `field` 是同一行的字段格：值候选按它指的字段查，否则 `status` 会冒出文档里
/// 任何位置出现过的手机号。字段为空或不在目录里时给空候选。
pub(crate) fn configure_mongo_builder_value_completion(
    editor: &mut EditorState,
    field: gpui::WeakEntity<EditorState>,
    view: gpui::WeakEntity<NavicatMain>,
    tab_id: TabId,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<EditorState>,
) {
    install_mongo_completion(
        editor,
        MongoCompletionTarget::BuilderValue { field },
        view,
        tab_id,
        window,
        cx,
    );
}

fn install_mongo_completion(
    editor: &mut EditorState,
    target: MongoCompletionTarget,
    view: gpui::WeakEntity<NavicatMain>,
    tab_id: TabId,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<EditorState>,
) {
    // 查询片段没有 fold 的意义，关掉以免行号槽出现可点击的折叠箭头；
    // 行号同理——面板里是 30px 高的工具条，不是代码文件。
    editor.set_folding(false, window, cx);
    editor.set_line_number(false, window, cx);
    // 条件构建的格子是裸路径 / 裸标量，不是 JSON 片段：套 JSON 高亮会把
    // `customer.tier`、`paid` 当成非法 token 染色，反而不如纯文本。
    if matches!(target, MongoCompletionTarget::Query(_)) {
        editor.set_highlighter(crate::JSON_HIGHLIGHT_LANGUAGE, cx);
    }
    let mut lsp = CompletionLsp::default();
    lsp.completion_provider = Some(Rc::new(MongoCompletionProvider {
        target,
        view,
        tab_id,
    }));
    *editor.lsp_mut() = lsp;
}

struct MongoCompletionProvider {
    target: MongoCompletionTarget,
    view: gpui::WeakEntity<NavicatMain>,
    tab_id: TabId,
}

impl CompletionProvider for MongoCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: lsp_types::CompletionContext,
        _: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> gpui::Task<anyhow::Result<CompletionResponse>> {
        let text = text.to_string();
        // 主视图已关闭（补全弹层还挂着）时给空候选，不 panic。
        //
        // 候选直接借字段目录算，不先 `clone()` 一份：目录含每个字段的取值，
        // 大集合上一份拷贝就是几百个 `String`，而这里每次按键都会走一次。
        let items = self.view.upgrade().map_or_else(Vec::new, |view| {
            let state = view.read(cx);
            let Some(collection) = mongo_collection_state(state.controller.state(), self.tab_id)
            else {
                return Vec::new();
            };
            let fields = &collection.field_catalog.fields;
            match &self.target {
                MongoCompletionTarget::Query(mode) => mongo_completions(&text, offset, *mode, fields),
                MongoCompletionTarget::BuilderField => {
                    mongo_builder_field_completions(&text, offset, fields)
                }
                MongoCompletionTarget::BuilderValue { field } => {
                    let path = field
                        .upgrade()
                        .map(|editor| editor.read(cx).value().trim().to_string())
                        .unwrap_or_default();
                    mongo_builder_value_completions(&text, offset, &path, fields)
                }
            }
        });
        gpui::Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(&self, _: usize, _: &str, _: &mut gpui::App) -> bool {
        true
    }
}

#[cfg(test)]
mod mongo_completion_tests {
    use super::*;
    use fluxdb_app::MongoField;

    fn fields() -> Vec<MongoField> {
        vec![
            MongoField {
                path: "_id".into(),
                types: vec!["ObjectId".into()],
                values: Vec::new(),
            },
            MongoField {
                path: "status".into(),
                types: vec!["String".into()],
                values: vec!["active".into(), "paid".into()],
            },
            MongoField {
                // 手机号挂在别的字段上：写 `status` 的值时不该出现。
                path: "phone".into(),
                types: vec!["String".into()],
                values: vec!["13900000001".into()],
            },
            MongoField {
                path: "address.city".into(),
                types: vec!["String".into()],
                values: Vec::new(),
            },
        ]
    }

    fn labels(items: &[CompletionItem]) -> Vec<String> {
        items.iter().map(|item| item.label.clone()).collect()
    }

    fn completions(text: &str, mode: MongoCompletionMode) -> Vec<CompletionItem> {
        mongo_completions(text, text.len(), mode, &fields())
    }

    #[test]
    fn key_position_offers_fields_and_query_operators() {
        let items = completions("{\"sta", MongoCompletionMode::Query);
        assert!(labels(&items).contains(&"status".to_string()));
        // 字符串里的键位补上收尾引号和冒号，正好落到值位；否则用户得自己再敲两个字符。
        let status = items
            .iter()
            .find(|item| item.label == "status")
            .expect("status candidate");
        assert_eq!(mongo_item_insert_text(status), "status\": ");
    }

    #[test]
    fn value_position_offers_extended_json_not_shell_constructors() {
        let items = completions("{\"createdAt\": {\"$o", MongoCompletionMode::Query);
        // `{"$o` 是嵌套对象的键位，仍是字段/运算符位；候选取值位要看外层值。
        assert!(labels(&items).contains(&"$or".to_string()));

        let items = completions("{\"createdAt\": ", MongoCompletionMode::Query);
        let labels = labels(&items);
        assert!(labels.contains(&"ObjectId".to_string()));
        // shell 写法（`ObjectId("…")`）本应用解析不了，不能作为候选。
        assert!(!labels.iter().any(|label| label.contains('(')));
        let oid = items
            .iter()
            .find(|item| item.label == "ObjectId")
            .expect("ObjectId template");
        assert_eq!(mongo_item_insert_text(oid), "{\"$oid\": \"\"}");
    }

    #[test]
    fn field_paths_with_dots_are_quoted_outside_strings() {
        let items = completions("{addr", MongoCompletionMode::Query);
        let city = items
            .iter()
            .find(|item| item.label == "address.city")
            .expect("nested field candidate");
        assert_eq!(mongo_item_insert_text(city), "\"address.city\": ");
    }

    #[test]
    fn builder_field_completes_bare_paths() {
        let items = mongo_builder_field_completions("addr", 4, &fields());
        assert_eq!(labels(&items), vec!["address.city".to_string()]);
        // 裸路径原样插入：不加引号、不补冒号，否则字段格会变成 `"address.city": `。
        assert_eq!(mongo_item_insert_text(&items[0]), "address.city");

        // 只给字段，不给运算符 / Extended JSON 构造器：这一格不是 JSON 片段。
        let items = mongo_builder_field_completions("", 0, &fields());
        let labels = labels(&items);
        assert!(labels.contains(&"status".to_string()));
        assert!(!labels.iter().any(|label| label.starts_with('$')));
        assert!(!labels.contains(&"ObjectId".to_string()));
    }

    #[test]
    fn builder_value_completes_only_its_row_field_values() {
        let items = mongo_builder_value_completions("", 0, "status", &fields());
        assert_eq!(labels(&items), vec!["active".to_string(), "paid".to_string()]);

        // 字段不在当前页目录里（用户手写了别的路径）就没有候选，不去全局词表凑。
        assert!(mongo_builder_value_completions("", 0, "unknown.field", &fields()).is_empty());
        assert!(mongo_builder_value_completions("", 0, "", &fields()).is_empty());

        // 目录里有这个字段、但这一页没采到值时同样为空。
        assert!(mongo_builder_value_completions("", 0, "address.city", &fields()).is_empty());
    }

    #[test]
    fn builder_value_replaces_quoted_token_including_its_closing_quote() {
        // 光标停在收尾引号后：整段 `"pai"` 都算替换范围，否则候选会被追加成
        // `"paid"active"`——改写旧值变成拼接。
        let items = mongo_builder_value_completions("\"pai\"", 5, "status", &fields());
        assert_eq!(labels(&items), vec!["paid".to_string()]);
        let edit = items[0].text_edit.as_ref().expect("replace range");
        let lsp_types::CompletionTextEdit::Edit(edit) = edit else {
            panic!("builder value uses a plain text edit");
        };
        assert_eq!((edit.range.start.character, edit.range.end.character), (1, 5));
        // 开引号留在原地、插入补回收尾引号，整格仍是 `"paid"`。
        assert_eq!(edit.new_text, "paid\"");

        // 半截字符串：开引号留在原地，插入时补回收尾引号。
        let items = mongo_builder_value_completions("\"ac", 3, "status", &fields());
        let edit = items[0].text_edit.as_ref().expect("replace range");
        let lsp_types::CompletionTextEdit::Edit(edit) = edit else {
            panic!("builder value uses a plain text edit");
        };
        assert_eq!((edit.range.start.character, edit.range.end.character), (1, 3));
        assert_eq!(edit.new_text, "active\"");
    }

    #[test]
    fn strings_complete_only_the_enclosing_field_values() {
        let items = completions("{\"status\": \"ac", MongoCompletionMode::Query);
        assert_eq!(labels(&items), vec!["active".to_string()]);

        // 取值按字段分开：`status` 的值位不能冒出 `phone` 上的手机号。
        let status_items = completions("{\"status\": \"", MongoCompletionMode::Query);
        let status_labels = labels(&status_items);
        assert!(status_labels.contains(&"paid".to_string()), "{status_labels:?}");
        assert!(
            !status_labels.iter().any(|label| label.starts_with("139")),
            "status 不该拿到别的字段的取值: {status_labels:?}"
        );

        // 嵌套字段按点分路径查：`address.city` 没有采集到取值时给空。
        let nested = completions("{\"address\": {\"city\": \"", MongoCompletionMode::Query);
        assert!(nested.is_empty(), "{:?}", labels(&nested));

        // 键名字符串里给字段，不给取值。
        let key_items = completions("{\"sta", MongoCompletionMode::Query);
        assert!(!labels(&key_items).contains(&"active".to_string()));
    }

    #[test]
    fn aggregation_stage_keys_and_dollar_references() {
        let items = completions("[{\"$m", MongoCompletionMode::Aggregation);
        assert!(labels(&items).contains(&"$match".to_string()));
        let stage = items
            .iter()
            .find(|item| item.label == "$match")
            .expect("stage candidate");
        // 数组元素键位：开头的引号用户已经敲了，只补名字、收尾引号和冒号。
        assert_eq!(mongo_item_insert_text(stage), "$match\": ");

        let items = completions(
            "[{\"$group\": {\"_id\": \"$sta",
            MongoCompletionMode::Aggregation,
        );
        assert!(
            labels(&items).contains(&"$status".to_string()),
            "stage string starting with $ offers field references: {:?}",
            labels(&items)
        );
    }

    #[test]
    fn stage_body_offers_accumulators_and_fields() {
        let items = completions(
            "[{\"$group\": {\"_id\": null, \"total\": {\"$su",
            MongoCompletionMode::Aggregation,
        );
        assert!(labels(&items).contains(&"$sum".to_string()));
    }

    #[test]
    fn unicode_text_keeps_byte_offsets_valid() {
        let text = "{\"备注\": \"完成\", \"sta";
        let items = completions(text, MongoCompletionMode::Query);
        let status = items
            .iter()
            .find(|item| item.label == "status")
            .expect("status candidate");
        let edit = match status.text_edit.as_ref().expect("text edit") {
            lsp_types::CompletionTextEdit::Edit(edit) => edit,
            _ => panic!("expected plain edit"),
        };
        assert_eq!(edit.range.start, mongo_offset_to_position(text, text.len() - 3));
    }
}
