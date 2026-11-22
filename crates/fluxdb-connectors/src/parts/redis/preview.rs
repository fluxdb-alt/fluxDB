fn redis_query_looks_like_glob(query: &str) -> bool {
    query.chars().any(|ch| matches!(ch, '*' | '?' | '[' | ']'))
}

/// 每种类型取预览需要的命令；返回顺序与 `redis_key_preview_from` 的解析顺序一一对应。
fn redis_key_preview_commands(key: &str, raw_type: &str) -> Vec<Vec<String>> {
    let key = key.to_string();
    match raw_type.to_ascii_lowercase().as_str() {
        "string" => vec![vec!["GET".to_string(), key]],
        "hash" => vec![vec!["HGETALL".to_string(), key]],
        "list" => vec![vec![
            "LRANGE".to_string(),
            key,
            "0".to_string(),
            "9".to_string(),
        ]],
        "set" => vec![
            vec!["SCARD".to_string(), key.clone()],
            vec!["SMEMBERS".to_string(), key],
        ],
        "zset" => vec![vec![
            "ZRANGE".to_string(),
            key,
            "0".to_string(),
            "9".to_string(),
            "WITHSCORES".to_string(),
        ]],
        "stream" => vec![
            vec!["XLEN".to_string(), key.clone()],
            vec![
                "XREVRANGE".to_string(),
                key,
                "+".to_string(),
                "-".to_string(),
                "COUNT".to_string(),
                "5".to_string(),
            ],
        ],
        "rejson-rl" | "json" => vec![vec!["JSON.GET".to_string(), key]],
        // 未知类型不取预览，保持与旧实现一致（值列留空）。
        _ => Vec::new(),
    }
}

/// 把 `redis_key_preview_commands` 的响应翻译成 (展示用类型名, 预览文本)。
fn redis_key_preview_from(raw_type: &str, values: Vec<RedisValue>) -> (String, String) {
    let mut values = values.into_iter().map(Some).collect::<Vec<_>>();
    let mut next = |index: usize| values.get_mut(index).and_then(Option::take);
    match raw_type.to_ascii_lowercase().as_str() {
        // 类型只认服务端 `TYPE`，不按值内容改标：普通 string 里存了 JSON 文本也仍显示 STRING，
        // 否则用户改值后键类型会“动态变化”，并误入 JSON 面板（非法 JSON 文本会被截断展示）。
        "string" => {
            let value = redis_bulk_text_opt(next(0)).unwrap_or_default();
            (raw_type.to_string(), redis_preview(value))
        }
        "hash" => (raw_type.to_string(), redis_preview_pairs(next(0))),
        "list" => (raw_type.to_string(), redis_preview_items(next(0))),
        "set" => (
            raw_type.to_string(),
            redis_set_preview(next(0), next(1)),
        ),
        "zset" => (raw_type.to_string(), redis_preview_pairs(next(0))),
        "stream" => (
            raw_type.to_string(),
            redis_stream_preview(next(0), next(1)),
        ),
        "rejson-rl" | "json" => (
            "json".to_string(),
            redis_bulk_text_opt(next(0)).unwrap_or_default(),
        ),
        _ => (raw_type.to_string(), String::new()),
    }
}

fn redis_preview_items(value: Option<RedisValue>) -> String {
    let items = redis_array_text(value);
    if items.is_empty() {
        String::new()
    } else {
        redis_preview(format!("[{}]", items.join(", ")))
    }
}

fn redis_preview_pairs(value: Option<RedisValue>) -> String {
    let items = redis_array_text(value);
    if items.is_empty() {
        return String::new();
    }
    let pairs = items
        .chunks(2)
        .map(|pair| match pair {
            [field, value] => format!("{field}: {value}"),
            [field] => field.clone(),
            _ => String::new(),
        })
        .collect::<Vec<_>>();
    redis_preview(format!("{{{}}}", pairs.join(", ")))
}

fn redis_array_text(value: Option<RedisValue>) -> Vec<String> {
    match value {
        Some(RedisValue::Array(items)) => items.into_iter().map(redis_value_text).collect(),
        _ => Vec::new(),
    }
}

fn redis_preview(mut value: String) -> String {
    const LIMIT: usize = 200;
    if value.chars().count() <= LIMIT {
        return value;
    }
    value = value.chars().take(LIMIT).collect();
    value.push_str("...");
    value
}

#[cfg(test)]
mod redis_preview_tests {
    use super::*;

    fn bulk(text: &str) -> RedisValue {
        RedisValue::Bulk(Some(text.as_bytes().to_vec()))
    }

    #[test]
    fn string_type_is_not_relabelled_by_json_content() {
        // 普通 string 即使内容是 JSON 文本，类型也必须保持 STRING：
        // 按内容改标会让类型“动态变化”，并把非法 JSON 文本送进 JSON 面板。
        let (kind, preview) = redis_key_preview_from("string", vec![bulk("{\"a\": 1}")]);
        assert_eq!(kind, "string");
        assert_eq!(preview, "{\"a\": 1}");

        let (kind, _) = redis_key_preview_from("string", vec![bulk("{a=1}")]);
        assert_eq!(kind, "string");
    }

    #[test]
    fn rejson_type_still_reported_as_json() {
        let (kind, preview) = redis_key_preview_from("rejson-rl", vec![bulk("{\"a\": 1}")]);
        assert_eq!(kind, "json");
        assert_eq!(preview, "{\"a\": 1}");
    }
}
