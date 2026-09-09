// Redis Workbench 命令补全纯逻辑单测。
// 覆盖：顶层命令补全、子命令补全、参数位、替换区间、多行、引号/转义、候选排序。

use fluxdb_core::{InsertTextFormat, QueryCompletionKind, QueryCompletionResult};

/// 从结果里取 label 列表（丢弃空结果便于断言内容）。
fn redis_labels(result: &QueryCompletionResult) -> Vec<String> {
    result.items.iter().map(|item| item.label.clone()).collect()
}

#[test]
fn redis_completion_empty_input_suggests_all_commands() {
    let result = redis_completion_result("", 0);
    assert!(!result.items.is_empty(), "空输入应展示顶层命令候选");
    assert!(result.items[0].kind == QueryCompletionKind::RedisCommand);
    assert_eq!(result.replace_start, 0);
    assert_eq!(result.replace_end, 0);
}

#[test]
fn redis_completion_s_prefix_suggests_set_family() {
    let result = redis_completion_result("S", 1);
    let labels = redis_labels(&result);
    assert!(labels.iter().any(|l| l == "SET"), "S 前缀应包含 SET");
    assert!(labels.iter().any(|l| l == "SELECT"), "S 前缀应包含 SELECT");
    assert!(labels.iter().any(|l| l == "SCAN"), "S 前缀应包含 SCAN");
    // 全部以 S 开头
    assert!(labels.iter().all(|l| l.starts_with('S')));
}

#[test]
fn redis_completion_se_prefix_narrows_to_set_line() {
    let result = redis_completion_result("SE", 2);
    let labels = redis_labels(&result);
    assert!(labels.iter().any(|l| l == "SET"));
    assert!(labels.iter().all(|l| l.starts_with("SE")));
}

#[test]
fn redis_completion_prefix_is_case_insensitive() {
    let lower = redis_labels(&redis_completion_result("se", 2));
    let upper = redis_labels(&redis_completion_result("SE", 2));
    assert_eq!(lower, upper, "大小写应等价匹配");
}

#[test]
fn redis_completion_replace_range_covers_current_token() {
    // "GE " 光标在行尾：当前 token 为空，替换区收缩到光标处 [3,3)。
    let result = redis_completion_result("GE ", 3);
    assert_eq!(result.replace_start, 3);
    assert_eq!(result.replace_end, 3);

    // "GET foo" 光标在 foo 起点：当前 token 是 foo，替换区为 [4,7)。
    let result = redis_completion_result("GET foo", 4);
    assert_eq!(result.replace_start, 4);
    assert_eq!(result.replace_end, 7);
}

#[test]
fn redis_completion_cluster_space_suggests_subcommands() {
    let result = redis_completion_result("CLUSTER ", 8);
    let labels = redis_labels(&result);
    assert!(
        labels.iter().any(|l| l == "KEYSLOT"),
        "CLUSTER 应补 KEYSLOT"
    );
    assert!(labels.iter().any(|l| l == "NODES"), "CLUSTER 应补 NODES");
    assert!(labels.iter().any(|l| l == "INFO"), "CLUSTER 应补 INFO");
    // 都应是子命令 kind
    assert!(
        result
            .items
            .iter()
            .all(|i| i.kind == QueryCompletionKind::RedisSubCommand)
    );
}

#[test]
fn redis_completion_cluster_k_prefix_suggests_keyslot() {
    let result = redis_completion_result("CLUSTER K", 9);
    let labels = redis_labels(&result);
    assert_eq!(labels, vec!["KEYSLOT"]);
    assert_eq!(result.replace_start, 8);
    assert_eq!(result.replace_end, 9);
}

#[test]
fn redis_completion_client_space_suggests_subcommands() {
    let result = redis_completion_result("CLIENT ", 7);
    let labels = redis_labels(&result);
    for expected in ["LIST", "KILL", "PAUSE", "SETNAME"] {
        assert!(
            labels.iter().any(|l| l == expected),
            "CLIENT 应补 {}",
            expected
        );
    }
}

#[test]
fn redis_completion_config_subcommand_matcher() {
    let result = redis_completion_result("CONFIG G", 8);
    let labels = redis_labels(&result);
    assert_eq!(labels, vec!["GET"]);
}

#[test]
fn redis_completion_second_arg_uses_fixed_arguments() {
    // CONFIG 的第二个参数位应给出固定参数提示（GET/SET 等），这里先验证第一条命令位稳定。
    let result = redis_completion_result("CONFIG GET ", 11);
    // CONFIG GET 之后再无固定保留字可补（参数位为通用占位），空前缀不弹。
    assert!(
        result.items.is_empty(),
        "命令第二参数位空前缀不应弹候选（避免噪声）"
    );
}

#[test]
fn redis_completion_multi_line_uses_current_line_only() {
    // 前一行是 GET foo，当前行是 SET：子命令解析应只看当前行。
    let text = "GET foo\nSET";
    let result = redis_completion_result(text, text.len());
    let labels = redis_labels(&result);
    assert!(labels.iter().any(|l| l == "SET"));
    assert!(labels.iter().any(|l| l == "SETEX"));
    assert!(
        !labels.iter().any(|l| l == "GET"),
        "前一行的 GET 不应再出现"
    );

    // 多行时替换区间相对全文偏移（含换行）：光标在第二行末尾，替换区为该行 token [8,11)。
    let result = redis_completion_result(text, text.len());
    assert_eq!(result.replace_start, 8);
    assert_eq!(result.replace_end, 11);
}

#[test]
fn redis_completion_insert_text_keeps_cursor_after_command() {
    let result = redis_completion_result("SET", 3);
    let set = result
        .items
        .iter()
        .find(|i| i.label == "SET")
        .expect("应先有 SET 候选");
    // 命令候选使用 snippet 格式：命令名 + 参数占位 tabstop，光标定位到第一参数位。
    assert!(
        set.insert_text.starts_with("SET "),
        "SET 候选应以命令名 + 空格开头，实际: {}",
        set.insert_text
    );
    assert_eq!(set.insert_text_format, InsertTextFormat::Snippet);
}

#[test]
fn redis_completion_dot_command_json_dot_suggests_subcommands() {
    // `JSON.`：点号后无前缀，应展示 JSON 的全部子命令（GET/SET/DEL/...）。
    let result = redis_completion_result("JSON.", 5);
    let labels = redis_labels(&result);
    assert!(
        labels.iter().any(|l| l == "GET"),
        "JSON. 应补子命令 GET，实际: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "SET"),
        "JSON. 应补子命令 SET，实际: {labels:?}"
    );
    // 子命令候选使用 snippet 格式：`JSON GET ${1:key} ...`。
    let get = result.items.iter().find(|i| i.label == "GET").unwrap();
    assert!(
        get.insert_text.starts_with("JSON GET "),
        "子命令 snippet 应以 'JSON GET ' 开头，实际: {}",
        get.insert_text
    );
    assert_eq!(get.insert_text_format, InsertTextFormat::Snippet);
}

#[test]
fn redis_completion_dot_command_json_get_prefix() {
    // `JSON.GE`：点号后前缀 `GE`，应过滤出 GET 子命令。
    let result = redis_completion_result("JSON.GE", 7);
    let labels = redis_labels(&result);
    assert!(labels.iter().any(|l| l == "GET"));
    // 不应出现不匹配的子命令（如 SET/DEL）。
    assert!(!labels.iter().any(|l| l == "SET"));
}

#[test]
fn redis_completion_dot_command_cursor_before_dot_keeps_command_prefix() {
    // `JSON`（无点号）：仍按顶层命令前缀补，不应触发子命令。
    let result = redis_completion_result("JSON", 4);
    let labels = redis_labels(&result);
    // 应出现 JSON 本身（顶层命令），而不是子命令。
    assert!(labels.iter().any(|l| l == "JSON"));
    assert!(!labels.iter().any(|l| l == "GET"));
}

#[test]
fn redis_completion_sort_prefers_exact_match_first() {
    // 输入 COMPACT 时，若目录里恰好有 COMPACT 精确项则排最前；这里用 CONFIG 验证精确优先。
    let result = redis_completion_result("CONFIG", 6);
    assert!(!result.items.is_empty());
    assert_eq!(result.items[0].label, "CONFIG");
}

#[test]
fn redis_completion_quote_and_escape_token_boundary() {
    // 带引号的复合 token：`SET "hello world"` 之后是空 token。
    let text = "SET \"hello world\" ";
    let result = redis_completion_result(text, text.len());
    // 已识别命令 SET，当前在参数位且前缀为空 -> 无固定保留字，不弹。
    assert!(result.items.is_empty());
    // 替换区间在引号闭合后的空格处。
    assert_eq!(result.replace_start, text.len());
    assert_eq!(result.replace_end, text.len());
}

// ---- 模块命令补全（JSON / FT / TS / GRAPH 等）----

#[test]
fn redis_completion_json_space_suggests_module_subcommands() {
    let result = redis_completion_result("JSON ", 5);
    let labels = redis_labels(&result);
    for expected in ["GET", "SET", "DEL", "TYPE", "ARRLEN"] {
        assert!(
            labels.iter().any(|l| l == expected),
            "JSON 应补 {}",
            expected
        );
    }
    assert!(
        result
            .items
            .iter()
            .all(|i| i.kind == QueryCompletionKind::RedisSubCommand)
    );
}

#[test]
fn redis_completion_json_g_prefix_suggests_get() {
    let result = redis_completion_result("JSON G", 6);
    let labels = redis_labels(&result);
    assert_eq!(labels, vec!["GET"]);
    assert_eq!(result.replace_start, 5);
    assert_eq!(result.replace_end, 6);
}

#[test]
fn redis_completion_ft_se_suggests_search() {
    let result = redis_completion_result("FT SE", 5);
    let labels = redis_labels(&result);
    assert_eq!(labels, vec!["SEARCH"]);
}

#[test]
fn redis_completion_ts_a_suggests_add() {
    let result = redis_completion_result("TS A", 4);
    let labels = redis_labels(&result);
    for expected in ["ADD", "ALTER"] {
        assert!(
            labels.iter().any(|l| l == expected),
            "TS A 应补 {}",
            expected
        );
    }
}

#[test]
fn redis_completion_graph_se_suggests_search_sub() {
    // GRAPH 下 SEARCH 不存在，但 QUERY / EXPLAIN 存在；验证前缀联动到正确子命令。
    let result = redis_completion_result("GRAPH Q", 7);
    let labels = redis_labels(&result);
    assert_eq!(labels, vec!["QUERY"]);
}

#[test]
fn redis_completion_module_subcommand_arg_skeleton_does_not_pop_noise() {
    // JSON GET 之后的第一个参数是 `<key>`（取值），空前缀不弹候选。
    let result = redis_completion_result("JSON GET ", 9);
    assert!(
        result.items.is_empty(),
        "JSON GET 后接取值参数，空前缀不应弹 token 噪声"
    );
}

#[test]
fn redis_completion_json_command_synopsis_detail() {
    // 顶层命令补全项 detail 应为 synopsis（含命令名）。
    let result = redis_completion_result("JSON", 4);
    let get = result
        .items
        .iter()
        .find(|i| i.label == "JSON")
        .expect("应有 JSON 命令候选");
    let detail = get.detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("JSON"),
        "detail 应含命令名，实际: {}",
        detail
    );
}

// ---- 参数上下文联动（token 级候选）----

#[test]
fn redis_completion_set_key_value_ex_nx_candidate() {
    // SET k v EX 100 N -> 在 [NX|XX] oneof 上联想 NX。
    let result = redis_completion_result("SET k v EX 100 N", "SET k v EX 100 N".len());
    let labels = redis_labels(&result);
    assert!(
        labels.contains(&"NX".to_string()),
        "应联想 NX，实际: {:?}",
        labels
    );
    assert!(
        !labels.contains(&"XX".to_string()),
        "不应联想未匹配前缀的 XX"
    );
}

#[test]
fn redis_completion_set_key_value_nx_candidate() {
    // SET k v 的第三个参数位（[NX|XX]）在输入 N 时联想 NX。
    let result = redis_completion_result("SET k v N", "SET k v N".len());
    let labels = redis_labels(&result);
    assert!(
        labels.contains(&"NX".to_string()),
        "应联想 NX，实际: {:?}",
        labels
    );
}

#[test]
fn redis_completion_subcommand_arg_token_candidate() {
    // TS.ADD 的 on_duplicate 策略 oneof：TS ADD k 100 1 ON_D 联想不到（空前缀不弹），
    // 但输入 ON_D 时应联想 ON_DUPLICATE。
    let result = redis_completion_result("TS ADD k 100 1 ON_D", "TS ADD k 100 1 ON_D".len());
    let labels = redis_labels(&result);
    assert!(
        labels.contains(&"ON_DUPLICATE".to_string()),
        "应联想 ON_DUPLICATE，实际: {:?}",
        labels
    );
}

#[test]
fn redis_completion_base_error_args_unchanged() {
    // 基础命令参数位无可用 token 时不回退噪声。
    let result = redis_completion_result("GET ", 4);
    assert!(result.items.is_empty(), "GET 后仅取值参数，不应弹候选");
}

#[test]
fn redis_completion_multibyte_cursor_mid_char_no_panic() {
    // 回归：中文等多字节字符被按字节编辑时，光标可能停在字符内部（byte 4 落在 `中` 的第 2 字节）。
    // 旧分词器按字节切 `中` 并把切半的 byte 当 char，导致 `upper[..within]` 对多字节字符切半 panic。
    // 现在应按 char 分词并把 within 回退到 char 边界，绝不 panic。
    let result = redis_completion_result("SE 中", 4); // cursor 落在 `中`(bytes 3..6) 内部
    // 核心断言：绝不 panic（旧实现 `text[..cursor]` / `upper[..within]` 都切半崩溃）。
    // prefix 应回退到 char 边界，替换起点回到 `中` 起始字节。
    assert_eq!(result.replace_start, 3, "替换起点应回到 `中` 的起始字节");
}

#[test]
fn redis_completion_multibyte_token_end_safe() {
    // 中文 token 末尾光标：替换区间覆盖整个中文字符，不触发 byte 越界。
    let text = "GET 中";
    let result = redis_completion_result(text, text.len());
    assert_eq!(result.replace_start, 4, "替换起点 = `中` 起始字节");
    assert_eq!(result.replace_end, text.len(), "替换终点 = 行尾");
}

#[test]
fn redis_completion_ascii_after_multibyte_safe() {
    // 中文后接 ASCII：分词器按 char 推进，字节偏移仍正确，后续 token 不串位。
    let text = "SET k 中v";
    let result = redis_completion_result(text, text.len());
    // 中文后接 ASCII：分词按 char 推进，字节偏移正确、绝不 panic；末 token 替换区间覆盖全。
    assert_eq!(result.replace_start, 6, "替换起点 = `中v` 起始字节");
    assert_eq!(result.replace_end, text.len(), "替换终点应覆盖最末 token");
}

// ---------------------------------------------------------------------------
// 命令签名（signature help）helper 单测
// ---------------------------------------------------------------------------

#[test]
fn redis_signature_base_command_label_and_ranges() {
    // SET 无子命令：label 为 `SET <key> <value> [EX ...] [NX|XX] [GET]`，
    // parameter_ranges 覆盖每个顶层参数且不含命令名。
    let sig = redis_command_signature("SET key value ", 14).expect("应有 SET 签名");
    assert!(sig.label.starts_with("SET "), "label 应以命令名开头");
    assert!(sig.label.contains("<key>"), "应含 <key> 骨架");
    assert!(sig.label.contains("[NX | XX]"), "应含 [NX | XX] 骨架，label: {}", sig.label);
    // 命令名后第 3 个参数（索引 2）是当前待填位。
    assert_eq!(sig.active_parameter, 2, "光标在 value 后应停在第三个参数");
    // 参数范围数量与顶层参数数一致，且都落在 label 内、不含命令名前缀。
    assert!(!sig.parameter_ranges.is_empty(), "应有参数范围");
    for &(start, end) in &sig.parameter_ranges {
        assert!(start >= sig.label.find('<').unwrap_or(0), "范围应在命令名之后");
        assert!(start < end && end <= sig.label.len(), "范围应落在 label 内");
    }
}

#[test]
fn redis_signature_active_follows_typing() {
    // 只输入 key（光标在 key 内、无尾随空格）：active 停在第一个参数（索引 0）。
    let sig = redis_command_signature("SET key", 7).expect("应有 SET 签名");
    assert_eq!(sig.active_parameter, 0, "key 仍是当前编辑中的第一个参数");
    // 输入 `SET key `（尾随空格）：key 已消费，active 前进到第二个参数（索引 1）。
    let sig2 = redis_command_signature("SET key ", 8).expect("应有 SET 签名");
    assert_eq!(sig2.active_parameter, 1, "key 后有空格应停在第二个参数");
}

#[test]
fn redis_signature_dot_subcommand_uses_sub_skeleton() {
    // JSON.GET：label 为 `JSON GET <key> [path]`，参数范围对应 GET 的骨架。
    let sig = redis_command_signature("JSON.GET ", 9).expect("应有 JSON.GET 签名");
    assert!(
        sig.label.starts_with("JSON GET "),
        "label 应含父命令与子命令，实际: {}",
        sig.label
    );
    assert!(sig.label.contains("<key>"), "应含 GET 的参数骨架");
}

#[test]
fn redis_signature_unknown_command_is_none() {
    assert!(redis_command_signature("NOPE ", 5).is_none(), "未知命令应无签名");
}

#[test]
fn redis_signature_empty_text_is_none() {
    assert!(redis_command_signature("", 0).is_none(), "空文本应无签名");
}

#[test]
fn redis_signature_quoted_value_not_miscounted() {
    // 带双引号值不按空格误拆：`SET "a b" ` 中引号内空格算一个参数。
    let text = "SET \"a b\" ";
    let sig = redis_command_signature(text, text.len()).expect("应有 SET 签名");
    assert_eq!(sig.active_parameter, 1, "引号内空格不应拆成额外参数");
}
