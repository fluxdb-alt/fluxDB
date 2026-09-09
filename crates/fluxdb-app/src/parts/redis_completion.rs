// Redis Workbench 命令补全的纯逻辑层。
//
// 该模块不依赖 GPUI / lsp_types，只负责三件事：
//   1. 光标上下文解析：把"文本 + 字节光标"还原成"当前命令名 + 当前 token + 替换区间"，
//      覆盖空白、双引号、转义、多行与光标回退/前进到 token 中间的场景。
//   2. 候选生成：按命令树匹配顶层命令 / 子命令 / 参数 token，返回中立补全项模型。
//   3. 参数上下文联动：结合「已输入的参数 token」定位当前应补的 token（见
//      `redis_commands.rs` 的命令树与定位器），让像 `SET k v EX 100 NX` 这样的场景
//      能逐步联想 token。
//
// 命令词典与命令树模型放在 `redis_commands.rs`（见 lib.rs 的 include! 顺序）：
// 本文件只负责「光标位该补命令还是参数、以及从词典里取哪些候选」。
//
// UI 层（fluxdb-desktop）只消费本模块产出的 `QueryCompletionResult`，映射到浮层展示。
// 选择替换区间由 `replace_start / replace_end` 给出，浮层紧贴光标，不改变主输入渲染链路。

/// 光标所在命令片段被解析出的上下文。
#[derive(Clone, Debug, Eq, PartialEq)]
struct RedisCompletionContext {
    /// 此刻应该补哪一类：顶层命令 / 子命令 / 参数。
    kind: RedisCompletionContextKind,
    /// 当前正在输入的 token（统一为大写，用于与目录匹配）。
    prefix: String,
    /// 当前 token 在原文中的起始字节偏移（作为替换区起点）。
    replace_start: usize,
    /// 当前 token 之后的下一个字节偏移（作为替换区终点）。
    replace_end: usize,
    /// 已识别的顶层命令名（大写，未识别为 None）。
    command: Option<String>,
    /// 当前光标所处的参数槽位（0 = 命令后的第一个参数）。
    slot: usize,
    /// 光标前已完整键入的 token（大写），首元素是命令名（已识别时）；
    /// 供参数定位器结合已输入参数决定当前应补的 token。
    before_tokens: Vec<String>,
}

/// 当前应该补全哪一类候选。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedisCompletionContextKind {
    /// 尚未识别出有效顶层命令，补顶层命令名。
    CommandPrefix,
    /// 已识别命令，且光标位于其子命令位/参数位。
    Argument,
}

/// 前缀匹配：prefix 为空时（光标刚落在空白后）返回 true，即展示全部候选。
fn redis_matches(candidate: &str, prefix: &str) -> bool {
    prefix.is_empty() || candidate.starts_with(prefix)
}

/// 把一个行内 token 记录下来（内含 byte 级起止与归一化后的大写值）。
#[derive(Clone, Debug)]
struct RedisToken {
    /// token 起始字节偏移（相对行起点）。
    start: usize,
    /// token 结束字节偏移（相对行起点）。
    end: usize,
    /// 归一大写后的 token 值。
    upper: String,
}

/// 切分一整行的 token，返回所有 token（含 byte 级 span）与行的字节长度。
///
/// 规则与 Redis CLI 分词一致：
/// - 空白（空格 / 制表符）分隔 token；
/// - 双引号内的空格不拆分，构成一个含空格的复合 token；
/// - 反斜杠 `\` 转义下一字符，两者视为同一个复合 token 的内容。
fn redis_line_tokens(line: &str) -> (Vec<RedisToken>, usize) {
    let n = line.len();
    let mut tokens: Vec<RedisToken> = Vec::new();
    let mut cur_start = 0usize;
    let mut cur_upper = String::new();
    let mut in_quote = false;
    let mut started = false;

    // 以 Unicode 字符为单位遍历（`char_indices` 给出 char 起始字节与值），
    // 避免按字节切多字节字符而把中文等切成半个字符、导致 `upper[..idx]` 越界 panic。
    let mut chars = line.char_indices();
    while let Some((i, ch)) = chars.next() {
        if ch == '\\' {
            // 转义：把 `\x` 视为 token 内容；若 `\` 在行尾则只记录反斜杠本身。
            if !started {
                started = true;
                cur_start = i;
            }
            if let Some((_, next)) = chars.next() {
                cur_upper.push(next.to_ascii_uppercase());
            } else {
                cur_upper.push(ch.to_ascii_uppercase());
            }
            continue;
        }
        if ch == '"' {
            // 双引号：切换引号状态，引号字符本身不计入匹配值；
            // 引号内的空白不作为分隔符，构成一个含空格的复合 token。
            in_quote = !in_quote;
            if !started {
                started = true;
                cur_start = i;
            }
            continue;
        }
        if (ch == ' ' || ch == '\t') && !in_quote {
            if started {
                tokens.push(RedisToken {
                    start: cur_start,
                    end: i,
                    upper: std::mem::take(&mut cur_upper),
                });
                started = false;
            }
            continue;
        }
        // 普通字符 / 引号内内容。
        if !started {
            started = true;
            cur_start = i;
        }
        cur_upper.push(ch.to_ascii_uppercase());
    }
    // 行尾未闭合的 token（引号未闭合 / 无尾随空白）也收进列表。
    if started {
        tokens.push(RedisToken {
            start: cur_start,
            end: n,
            upper: cur_upper,
        });
    }
    (tokens, n)
}

/// 解析整段 workbench 文本在 byte 光标处的补全上下文。
///
/// 只处理光标所在行（Redis CLI 以换行分隔命令）。先整行分词，
/// 再定位「光标所落在的 token」得到当前前缀与替换区间，
/// 兼容光标位于 token 中间、末尾、或落在空白间隙的多种场景。
fn redis_completion_context(text: &str, cursor: usize) -> RedisCompletionContext {
    let byte_len = text.len();
    // 光标字节位可能落在多字节字符内部（如中文被按字节编辑/删除后），
    // 直接 `text[..cursor]` 会对字符切半 panic；先回退到最近的 char 边界。
    let cursor = std::cmp::min(cursor, byte_len);
    let cursor = (0..=cursor).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
    let line_start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[line_start..]
        .find('\n')
        .map(|i| line_start + i)
        .unwrap_or(byte_len);
    let line = &text[line_start..line_end];
    let lc = cursor - line_start; // 光标在行内的 byte 偏移（0..=行长度）

    let (tokens, _n) = redis_line_tokens(line);

    // 找包含光标的 token；没有则光标落在空白间隙（空 token）。
    let mut current: Option<&RedisToken> = None;
    let mut completed_before = 0usize;
    for token in &tokens {
        if lc >= token.start && lc <= token.end {
            current = Some(token);
            break;
        }
        if token.end < lc {
            completed_before += 1;
        }
    }

    // 点号分隔命令（`JSON.` / `JSON.GET`）处理：当 token 内含 `.`，且 `.` 前是已知命令，
    // 视为「已识别命令 + 子命令前缀」。
    // `current_dot_split`：当前 token 的拆分（光标在 `.` 后时生效）。
    let current_dot_split = current.and_then(|token| {
        let dot_pos = token.upper.find('.')?;
        let dot_byte = token.start + dot_pos; // `.` 在行内的 byte 偏移
        if lc <= dot_byte {
            // 光标在 `.` 前：仍按顶层命令前缀补。
            return None;
        }
        // `.` 后的子命令前缀（大写）。
        let after_dot_upper = &token.upper[dot_pos + 1..];
        let within = (lc - (dot_byte + 1)).min(after_dot_upper.len());
        let sub_prefix = after_dot_upper[..within].to_string();
        Some((token.upper[..dot_pos].to_string(), sub_prefix, dot_byte))
    });

    // `first_dot_cmd`：首 token 含 `.` 时，取 `.` 前作为命令名（供后续参数位查找 spec）。
    let first_dot_cmd = tokens.first().and_then(|token| {
        token
            .upper
            .find('.')
            .map(|dot_pos| token.upper[..dot_pos].to_string())
    });

    let in_command_slot = completed_before == 0;
    let command = if let Some((cmd, _, _)) = &current_dot_split {
        // 当前 token 是点号命令且光标在 `.` 后：识别出命令名，补子命令。
        Some(cmd.clone())
    } else if let Some(cmd) = &first_dot_cmd {
        // 首 token 是点号命令（光标在后续参数位）：用 `.` 前作为命令名。
        Some(cmd.clone())
    } else if in_command_slot && current.is_some() {
        // 光标仍在命令名内，命令尚未确定，继续补顶层命令。
        None
    } else {
        tokens.first().map(|token| token.upper.clone())
    };

    // 当前 token 的索引（0-based），即 slot 计算的基础。
    let token_index = completed_before;
    let slot = if command.is_some() {
        token_index.saturating_sub(1)
    } else {
        0
    };

    // 光标前已完整键入的 token 值（大写），供参数定位器使用。
    // 点号命令（`JSON.GET`）拆为 [`JSON`, `GET`]，让参数定位器能识别子命令。
    let before_tokens = tokens
        .iter()
        .take(completed_before)
        .flat_map(|token| {
            if let Some(dot_pos) = token.upper.find('.') {
                vec![
                    token.upper[..dot_pos].to_string(),
                    token.upper[dot_pos + 1..].to_string(),
                ]
            } else {
                vec![token.upper.clone()]
            }
        })
        .collect::<Vec<_>>();

    let (prefix, replace_start, replace_end) = match &current_dot_split {
        Some((_, sub_prefix, dot_byte)) => {
            // 点号命令：替换从 `.` 后到 token 末尾，前缀为 `.` 后已输入的子命令部分。
            (sub_prefix.clone(), dot_byte + 1, current.unwrap().end)
        }
        None => match current {
            Some(token) => {
                // 光标可能停在多字节字符内部（如中文被按字节删除后），`within` 未必落在
                // char 边界；回退到该字符起点，避免 `upper[..within]` 对多字节字符切半 panic。
                let mut within = (lc - token.start).min(token.upper.len());
                while within > 0 && !token.upper.is_char_boundary(within) {
                    within -= 1;
                }
                (token.upper[..within].to_string(), token.start, token.end)
            }
            None => (String::new(), lc, lc),
        },
    };

    RedisCompletionContext {
        kind: if command.is_some() && (token_index >= 1 || current_dot_split.is_some()) {
            RedisCompletionContextKind::Argument
        } else {
            RedisCompletionContextKind::CommandPrefix
        },
        prefix,
        replace_start: line_start + replace_start,
        replace_end: line_start + replace_end,
        command,
        slot,
        before_tokens,
    }
}

/// 顶层命令补全项，detail 带命令全程 synopsis（如 `SET <key> <value> [...]`）。
///
/// insert_text 为 snippet 格式：命令名 + 参数占位 tabstop（`${1:key} ${2:value} ...`），
/// 选中后光标定位到第一个参数位，Tab 键在占位间切换。
fn redis_command_item(spec: &RedisCommandSpec) -> QueryCompletionItem {
    let snippet = redis_command_snippet(spec);
    QueryCompletionItem {
        label: spec.name.clone(),
        insert_text: snippet,
        kind: QueryCompletionKind::RedisCommand,
        detail: Some(redis_command_synopsis(spec)),
        insert_text_format: InsertTextFormat::Snippet,
        ..Default::default()
    }
}

/// 生成命令的 snippet 插入文本：`CMD ${1:key} ${2:value} ...`。
///
/// 每个参数转为一个 tabstop，占位符为参数的 display 文本（如 `key`、`value`）。
/// 可选参数保留占位（用户可按 Del 跳过），但用 `${N:}` 空占位避免干扰。
fn redis_command_snippet(spec: &RedisCommandSpec) -> String {
    let mut parts: Vec<String> = vec![spec.name.clone()];
    let mut tab_idx = 1usize;
    for arg in &spec.arguments {
        // 跳过纯固定 token 型参数（如 `ALPHA`、`DESC`）：它们不是取值位。
        if arg.optional {
            // 可选参数：用空占位 `${N:}`，用户可快速跳过。
            parts.push(format!("${{{tab_idx}:}}"));
            tab_idx += 1;
        } else {
            let placeholder = arg_placeholder(arg);
            if placeholder.is_empty() {
                parts.push(format!("${{{tab_idx}:}}"));
            } else {
                parts.push(format!("${{{tab_idx}:{placeholder}}}"));
            }
            tab_idx += 1;
        }
    }
    parts.join(" ")
}

/// 取参数的占位显示名：Value 取 `<key>` 内文本，Block 用子参数名拼接，其余用 display。
fn arg_placeholder(arg: &RedisArg) -> String {
    match arg.rtype {
        RedisArgType::Value => {
            // `<key>` -> `key`
            arg.display
                .strip_prefix('<')
                .and_then(|s| s.strip_suffix('>'))
                .unwrap_or(&arg.display)
                .to_string()
        }
        RedisArgType::Block => arg
            .children
            .iter()
            .map(arg_placeholder)
            .collect::<Vec<_>>()
            .join(" "),
        RedisArgType::OneOf => arg
            .children
            .iter()
            .map(arg_placeholder)
            .collect::<Vec<_>>()
            .join("|"),
        RedisArgType::Token => arg.display.clone(),
    }
}

/// 子命令补全项，detail 带父子命令包的 synopsis（如 `JSON GET <key> [path]`）。
fn redis_subcommand_item(spec: &RedisCommandSpec, sub: &RedisSubcommand) -> QueryCompletionItem {
    // 子命令 snippet：`CMD SUBCMD ${1:key} ${2:value} ...`，光标定位到子命令后第一参数位。
    let mut snippet = format!("{} {}", spec.name, sub.name);
    let mut tab_idx = 1usize;
    for arg in &sub.arguments {
        let placeholder = arg_placeholder(arg);
        if placeholder.is_empty() {
            snippet.push_str(&format!(" ${{{tab_idx}:}}"));
        } else {
            snippet.push_str(&format!(" ${{{tab_idx}:{placeholder}}}"));
        }
        tab_idx += 1;
    }
    QueryCompletionItem {
        label: sub.name.clone(),
        insert_text: snippet,
        kind: QueryCompletionKind::RedisSubCommand,
        detail: Some(redis_subcommand_synopsis(spec, sub)),
        insert_text_format: InsertTextFormat::Snippet,
        ..Default::default()
    }
}

/// 参数/固定 token 补全项，detail 标注其所在命令，便于区分。
fn redis_argument_item(owner: &str, token: &str) -> QueryCompletionItem {
    QueryCompletionItem {
        label: token.to_string(),
        insert_text: token.to_string(),
        kind: QueryCompletionKind::RedisArgument,
        detail: Some(format!("{} 参数", owner)),
        ..Default::default()
    }
}

/// 按"前缀精确一致优先、长度短优先、字典序"排序，保证候选稳定不抖动。
fn sort_redis_completion_items(items: &mut [QueryCompletionItem], prefix: &str) {
    items.sort_by(|a, b| {
        let a_exact = a.label.eq_ignore_ascii_case(prefix);
        let b_exact = b.label.eq_ignore_ascii_case(prefix);
        b_exact
            .cmp(&a_exact)
            .then_with(|| a.label.len().cmp(&b.label.len()))
            .then_with(|| a.label.cmp(&b.label))
    });
}

/// 取命令需要被按参数定位器消费的「命令后已输入 token」与「待匹配的参数骨架」。
///
/// 对子命令型命令（如 `JSON`、`CLUSTER`）：若第一个已输入 token 是已知子命令，则
/// 定位器消费该子命令之后的 token、匹配该子命令自己的参数骨架；否则（处于子命令
/// 位或尚未输入子命令）返回 None。
///
/// 返回 `(参数骨架, 定位器要消费的 token 切片)`。
fn redis_active_arguments<'s, 't>(
    spec: &'s RedisCommandSpec,
    typed_args: &'t [String],
) -> Option<(&'s [RedisArg], &'t [String])> {
    if spec.subcommands.is_empty() {
        // 无子命令：直接用顶层参数骨架，消费全部命令后 token。
        return Some((&spec.arguments, typed_args));
    }
    if typed_args.is_empty() {
        // 有子命令但还没输入：处于子命令位，未到参数定位阶段。
        return None;
    }
    let first = &typed_args[0];
    if let Some(sub) = spec.subcommands.iter().find(|s| &s.name == first) {
        Some((&sub.arguments, &typed_args[1..]))
    } else {
        // 第一个 token 不是已知子命令（可能是拼写中间态），落到顶层参数。
        Some((&spec.arguments, typed_args))
    }
}

/// 入口：给定文本与 byte 光标，返回命令补全结果。
///
/// 纯逻辑、无副作用，只在 cursor / text 变化时由调用方触发；
/// 返回的中立结果由 UI 层映射到浮层展示。
///
/// `pub` 供桌面层（fluxdb-desktop）的 completion provider 桥接调用；
/// `items` 为空即表示当前光标位没有候选，可用于判断是否触发浮层。
pub fn redis_completion_result(text: &str, cursor: usize) -> QueryCompletionResult {
    let ctx = redis_completion_context(text, cursor);
    let mut items = Vec::new();

    match ctx.kind {
        RedisCompletionContextKind::CommandPrefix => {
            for spec in redis_commands() {
                if redis_matches(&spec.name, &ctx.prefix) {
                    items.push(redis_command_item(spec));
                }
            }
        }
        RedisCompletionContextKind::Argument => {
            // 已识别命令；取出命令后已完整键入的参数 token（不含当前光标 token）。
            if let Some(command) = &ctx.command
                && let Some(spec) = redis_command_spec(command)
            {
                // 去掉命令 token 本身，得到「命令后的参数 token」。
                let typed_args = if ctx.before_tokens.is_empty() {
                    Vec::new()
                } else {
                    ctx.before_tokens[1..].to_vec()
                };

                if ctx.slot == 0 && !spec.subcommands.is_empty() {
                    // 命令的第一个参数位：补子命令（空前缀也展示全部，满足 `JSON ` 体验）。
                    for sub in &spec.subcommands {
                        if redis_matches(&sub.name, &ctx.prefix) {
                            items.push(redis_subcommand_item(spec, sub));
                        }
                    }
                } else {
                    // 其余参数位：先在「不会再回退的基础 Redis 命令体验」上保持
                    // 「空前缀不弹噪声」，再按参数定位器联想 token。
                    if !ctx.prefix.is_empty()
                        && let Some((args, walk_tokens)) = redis_active_arguments(spec, &typed_args)
                    {
                        // 从前沿（一次可容纳的多个待补参数）收集所有可补 token，再按前缀过滤。
                        for arg in redis_locate_args(args, walk_tokens) {
                            for cand in redis_arg_candidates(arg) {
                                if redis_matches(&cand, &ctx.prefix) {
                                    items.push(redis_argument_item(command, &cand));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    sort_redis_completion_items(&mut items, &ctx.prefix);
    QueryCompletionResult {
        replace_start: ctx.replace_start,
        replace_end: ctx.replace_end,
        items,
    }
}

// ---------------------------------------------------------------------------
// 命令签名（signature help）
// ---------------------------------------------------------------------------

/// 一条 Redis 命令的签名提示：label（synopsis）+ 当前应填参数索引 + 每参数精确 byte 范围。
///
/// `parameter_ranges` 为 label 内每个顶层参数（不含命令名/子命令名）的 `[start, end)`
/// UTF-8 byte 区间，供 UI 精确高亮 active 参数；不使用「按空格切词数 token」的近似。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedisSignatureInfo {
    /// 完整 synopsis 文本（命令名 + 参数骨架），用于签名浮层展示。
    pub label: String,
    /// 光标当前应填的顶层参数索引（0 = 命令后的第一个参数）。
    pub active_parameter: usize,
    /// label 内每个顶层参数的 `[start, end)` byte 范围。
    pub parameter_ranges: Vec<(usize, usize)>,
}

/// 给定整段 workbench 文本与 byte 光标，生成当前命令的签名提示。
///
/// 复用 `redis_completion_context` 解析命令名与光标前已输入参数，不重复分词；
/// 支持点号命令（`JSON.GET`）。未知命令 / 未识别命令返回 `None`。
pub fn redis_command_signature(text: &str, cursor: usize) -> Option<RedisSignatureInfo> {
    let ctx = redis_completion_context(text, cursor);
    let before = &ctx.before_tokens;
    // before_tokens 首元素是命令名（大写）；为空表示尚未输入任何 token。
    let main = before.first()?;
    let spec = redis_command_spec(main)?;

    // 取参数骨架与定位器要消费的 token：
    //   - 无子命令命令（如 SET）：顶层参数骨架，消费命令名之后全部 token。
    //   - 模块命令（如 JSON）：首参若是已知子命令 → 用子命令骨架，消费其后的 token；
    //     否则停在子命令位（未选子命令，无参数可标）。
    let (label_prefix, args, walk): (String, &[RedisArg], &[String]) =
        if spec.subcommands.is_empty() {
            (main.clone(), &spec.arguments, &before[1..])
        } else {
            match before
                .get(1)
                .and_then(|s| spec.subcommands.iter().find(|sc| &sc.name == s))
            {
                Some(sub_spec) => (
                    format!("{} {}", main, sub_spec.name),
                    &sub_spec.arguments,
                    &before[2..],
                ),
                None => {
                    return Some(RedisSignatureInfo {
                        label: main.clone(),
                        active_parameter: 0,
                        parameter_ranges: Vec::new(),
                    });
                }
            }
        };

    let active_parameter = redis_active_arg_index(args, walk);
    let (label, parameter_ranges) = build_signature_label(label_prefix, args);
    Some(RedisSignatureInfo {
        label,
        active_parameter,
        parameter_ranges,
    })
}

/// 计算「光标当前应填的顶层参数在骨架中的索引」：从 args 起始用 `redis_arg_steps`
/// 消费 tokens，返回首个未完全消费的顶层参数 index（0 = 命令后的第一个参数）。
///
/// 语义与参数定位器 `redis_locate_args` 对齐：可选参数未触发则跳过；必填 token
/// 未匹配则停在此参数；参数配齐后停在最后（可重复参数可继续补）。
fn redis_active_arg_index(args: &[RedisArg], tokens: &[String]) -> usize {
    let mut ti = 0usize;
    let mut last_multiple: Option<usize> = None;
    for (i, arg) in args.iter().enumerate() {
        if ti >= tokens.len() {
            // 所有已输入 token 消费完：当前参数是待填前沿。
            return i;
        }
        if let Some(nti) = redis_arg_steps(arg, tokens, ti) {
            ti = nti;
            if arg.multiple {
                last_multiple = Some(i);
            }
            continue;
        }
        if arg.optional {
            // 可选且未触发：跳过，继续看后续参数。
            continue;
        }
        // 必填参数未匹配：应停留在它上面。
        return i;
    }
    // 参数遍历完仍有剩余 token：若存在可重复参数则停留在其上，否则已配齐停在最后参数。
    if ti < tokens.len() {
        last_multiple.unwrap_or(args.len().saturating_sub(1))
    } else {
        args.len().saturating_sub(1)
    }
}

/// 拼接签名 label（前缀 + 各参数 synopsis），同时返回每个顶层参数的 `[start, end)` 范围。
fn build_signature_label(prefix: String, args: &[RedisArg]) -> (String, Vec<(usize, usize)>) {
    let mut label = prefix;
    let mut ranges = Vec::new();
    let mut offset = label.len();
    for arg in args {
        let syn = redis_arg_synopsis(arg);
        if offset > 0 {
            label.push(' ');
            offset += 1;
        }
        ranges.push((offset, offset + syn.len()));
        label.push_str(&syn);
        offset += syn.len();
    }
    (label, ranges)
}
