fn redis_key_detail_for_row(page: &DataPage, row_index: usize) -> Option<RedisKeyDetail> {
    let row = page.rows.get(row_index)?;
    Some(RedisKeyDetail {
        key: redis_row_text(page, row, "键")?,
        kind: redis_row_text(page, row, "类型")?,
        value: redis_row_text(page, row, "值").unwrap_or_default(),
        ttl: redis_row_text(page, row, "TTL").unwrap_or_else(|| "无 TTL".to_string()),
        size: redis_row_text(page, row, "大小").unwrap_or_default(),
    })
}

/// 在「已过滤」页中按键名反查原始行下标（Folder 模式下详情抽屉跟随叶子键用）。
/// 找不到（键已被删除 / 已过滤掉）返回 None，避免详情指向错误的键。
fn redis_key_page_row_for_key(page: &DataPage, key: &str) -> Option<usize> {
    page.rows
        .iter()
        .position(|row| redis_row_text(page, row, "键").as_deref() == Some(key))
}

fn redis_row_text(page: &DataPage, row: &Row, column: &str) -> Option<String> {
    let index = page
        .columns
        .iter()
        .position(|candidate| candidate.name == column)?;
    row.values.get(index).map(cell_value_label)
}

fn redis_split_view(
    tab_id: TabId,
    page: &DataPage,
    table_state: &Entity<TableState<DataPageTableDelegate>>,
    refreshed_at: Option<Instant>,
    refreshing: bool,
    colors: UiColors,
    this: &mut NavicatMain,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    // 详情面板始终跟随叶子 key：平铺模式走表格「可见行 -> 原行」映射；Folder 模式走叶子键反查，
    // 避免 folder 节点或表格行下标的变动把抽屉带偏（刷新 / 筛选 / 切换模式都不影响 key 反查）。
    let selected_row = if this.redis_key_list_mode(tab_id).is_folder() {
        this.redis_key_list_selected_leaf(tab_id)
            .as_deref()
            .and_then(|key| redis_key_page_row_for_key(page, key))
    } else {
        let table = table_state.read(cx);
        let delegate = table.delegate();
        delegate
            .selected_row
            .or_else(|| delegate.selected_cell.map(|(row, _)| row))
            .and_then(|row| delegate.source_row_indexes.get(row).copied())
    };
    let detail = match selected_row {
        Some(row) => redis_key_detail_drawer(
            tab_id, page, row, refreshed_at, colors, this, window, cx,
        )
        .unwrap_or_else(|| redis_key_empty_detail(colors)),
        // 抽屉无选中行（关闭）时，清理上一个 string key 的缓存，下次进入重新 preview 加载。
        None => {
            this.redis_string_evict_active_cache(tab_id, cx);
            redis_key_empty_detail(colors)
        }
    };

    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .overflow_hidden()
        .bg(colors.panel_bg)
        // 左侧 Key 列表列：顶部是列表自身状态栏（纯图标模式切换 + 结果数/已扫描/Scan more），
        // 下方是可滚动 Key 列表。展示模式切换已从主工具栏（搜索栏）迁到这里。
        .child(
            div()
                .w(px(520.))
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .child(redis_key_list_status_bar(
                    tab_id,
                    page,
                    refreshing,
                    colors,
                    this,
                    cx,
                ))
                .child(redis_key_list(
                    tab_id,
                    page,
                    table_state,
                    refreshed_at,
                    refreshing,
                    colors,
                    this,
                    cx,
                )),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .h_full()
                .overflow_hidden()
                .border_l_1()
                .border_color(colors.border)
                .child(detail),
        )
}

fn redis_key_detail_drawer(
    tab_id: TabId,
    page: &DataPage,
    row_index: usize,
    refreshed_at: Option<Instant>,
    colors: UiColors,
    this: &mut NavicatMain,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Option<Div> {
    let detail = redis_key_detail_for_row(page, row_index)?;
    this.sync_redis_set_member_search_input(tab_id, &detail, window, cx);
    this.sync_redis_key_value_input(tab_id, &detail, window, cx);
    this.sync_redis_key_meta_inputs(tab_id, &detail, window, cx);
    this.sync_redis_set_member_inputs(tab_id, &detail, window, cx);
    this.sync_redis_string_value(tab_id, &detail, cx);
    this.sync_redis_json_editor(tab_id, &detail, window, cx);
    let input = this.redis_key_value_input_for_kind(&detail.kind);
    let applying = this
        ._redis_key_value_apply_tasks
        .contains_key(&redis_key_value_apply_task_id(tab_id));
    let key_refreshing = this
        ._data_load_tasks
        .contains_key(&redis_key_refresh_task_id(tab_id));
    let meta_dirty = this.redis_key_meta_dirty(tab_id, &detail);
    let value_dirty = this.redis_key_value_dirty(tab_id, &detail, &input, cx);
    let dirty = meta_dirty || value_dirty;
    let drawer_bg = if colors.is_dark {
        rgb(0x171b21)
    } else {
        rgb(0xfbfcfe)
    };

    Some(
        div()
            .relative()
            .size_full()
            .w_full()
            .bg(drawer_bg)
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(redis_key_detail_header(
                tab_id,
                &detail,
                this.redis_key_name_display(tab_id, &detail),
                refreshed_at,
                key_refreshing,
                colors,
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .relative()
                    .child(redis_key_detail_meta_panel(
                        tab_id, &detail, dirty, applying, this, colors, window, cx,
                    ))
                    .child(redis_key_detail_value_panel(
                        tab_id, &detail, input, applying, this, window, colors, cx,
                    ))
                    .when(key_refreshing, |this| this.child(redis_key_detail_loading_overlay(colors))),
            ),
    )
}

fn redis_key_empty_detail(colors: UiColors) -> Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .text_color(colors.muted)
        .child("请选择 Redis Key")
}

fn redis_detail_panel(colors: UiColors) -> Div {
    div()
        .h_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_bg)
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
}

fn redis_detail_panel_title(label: &'static str, colors: UiColors) -> Div {
    div()
        .flex_none()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(label)
}

fn redis_detail_meta_label(label: &'static str, colors: UiColors) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.muted)
        .child(label)
}

fn redis_detail_meta_field_shell(
    label: &'static str,
    key_for_copy: Option<String>,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let mut label_row = div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .child(redis_detail_meta_label(label, colors));
    if let Some(key) = key_for_copy {
        label_row = label_row.child(redis_detail_copy_key_button(key, colors, cx));
    }

    div()
        .h_full()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .px_3()
        .py_2()
        .flex()
        .flex_col()
        .justify_start()
        .gap_3()
        .child(label_row)
}

fn redis_detail_editable_meta_field(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    field: RedisKeyMetaField,
    label: &'static str,
    input: Entity<InputState>,
    value: String,
    mono: bool,
    editing: bool,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let focus_border = if colors.is_dark {
        rgb(0x8ab4ff)
    } else {
        rgb(0x111111)
    };
    let copy_value = value.clone();
    let field_shell = redis_detail_meta_field_shell(
        label,
        (field == RedisKeyMetaField::KeyName).then_some(copy_value),
        colors,
        cx,
    )
    .cursor_pointer()
    .when(mono, |this| this.gap_2());
    if editing {
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        return field_shell
            .cursor_text()
            .child(
                div()
                    .w_full()
                    .min_w(px(0.))
                    .h(px(if mono { 40. } else { 32. }))
                    .rounded(colors.radius)
                    .border_1()
                    .border_color(if focused { focus_border } else { colors.border })
                    .bg(colors.input_bg)
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(
                        Input::new(&input)
                            .appearance(false)
                            .focus_bordered(false)
                            .w_full()
                            .h_full()
                            .px_2()
                            .text_size(px(if mono { 16. } else { 13. }))
                            .when(mono, |this| this.line_height(px(24.)))
                            .when(mono, |this| this.font_family("Menlo")),
                    ),
            );
    }

    let detail_for_edit = detail.clone();
    field_shell
        .on_mouse_down(
            MouseButton::Left,
            cx.listener({
                let key = detail.key.clone();
                move |this, _, window, cx| {
                    this.redis_key_meta_editing = Some(field);
                    this.sync_redis_key_meta_inputs(tab_id, &detail_for_edit, window, cx);
                    let input = match field {
                        RedisKeyMetaField::KeyName => this.redis_key_name_input.clone(),
                        RedisKeyMetaField::Ttl => this.redis_key_ttl_input.clone(),
                    };
                    input.read(cx).focus_handle(cx).focus(window, cx);
                    this.redis_key_meta_active = Some((tab_id, key.clone()));
                    cx.notify();
                    cx.stop_propagation();
                }
            }),
        )
        .hover(move |style| style.bg(colors.hover).border_color(colors.border))
        .child(
            div()
                .min_h(px(if mono { 36. } else { 30. }))
                .flex()
                .items_center()
                .w_full()
                .min_w(px(0.))
                .overflow_hidden()
                // 无过期时间时渲染成「永不超时」徽章，避免留白；其余情况走常规文本。
                .when(redis_ttl_is_none(&value), |this| {
                    this.child(redis_detail_no_ttl_badge(colors))
                })
                .when(!redis_ttl_is_none(&value), |this| {
                    this.whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(if mono { 16. } else { 14. }))
                        .line_height(px(if mono { 24. } else { 18. }))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .when(mono, |this| this.font_family("Menlo"))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(redis_ellipsis_text(&value, if mono { 26 } else { 18 })),
                        )
                })
        )
}

/// 无过期时间的展示徽章：圆角浅色胶囊 + CircleSlash 图标 + 「No TTL」英文小字
/// （与列表页 `(No TTL)` 口径一致），明暗主题均取 UiColors 中性配色。
fn redis_detail_no_ttl_badge(colors: UiColors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_1()
        .h(px(22.))
        .px_2()
        .rounded_full()
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .child(app_icon(AppIcon::CircleSlash, 13., colors.muted))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.muted)
                .whitespace_nowrap()
                .child("No TTL"),
        )
}

fn redis_ellipsis_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    format!("{}...", value.chars().take(keep).collect::<String>())
}

fn redis_detail_copy_key_button(
    key: String,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> impl IntoElement {
    Button::new((
        gpui::ElementId::Name(format!("redis-detail-copy-key-{key}").into()),
        key.clone(),
    ))
        .ghost()
        .xsmall()
        .flex_none()
        .h(px(26.))
        .min_w(px(26.))
        .p_0()
        .tooltip("复制键名称")
        .child(app_icon(AppIcon::Copy, 14., colors.muted))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(key.clone()));
            this.show_message("键名称已复制", AppMessageKind::Success, cx);
            cx.stop_propagation();
        }))
}

fn redis_detail_toolbar_button(
    label: &'static str,
    icon: AppIcon,
    enabled: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(24.))
        .px_1()
        .rounded(colors.radius * 0.5)
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .text_color(if enabled { colors.text } else { colors.muted })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .child(app_icon(
            icon,
            13.,
            if enabled { colors.muted } else { colors.border },
        ))
        .child(label)
}

fn redis_key_refresh_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 63)
}

fn redis_key_value_apply_task_id(tab_id: TabId) -> u64 {
    tab_id.0 | (1_u64 << 62)
}

// String 详情值加载（preview / 完整）任务 id：与保存任务区分，互不阻塞

fn redis_key_value_editable(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "string" | "json" | "rejson-rl"
    )
}

/// 连接器对非 UTF-8 内容返回的占位符前缀（U+FFFC）。
/// 见 `crates/fluxdb-connectors/src/parts/redis.rs` 的 REDIS_BINARY_MARKER。
const REDIS_BINARY_MARKER: char = '\u{FFFC}';

/// 值是二进制占位符时不允许编辑：回写占位符会毁掉原始字节。
fn redis_value_is_binary(value: &str) -> bool {
    value.starts_with(REDIS_BINARY_MARKER)
}

/// 检查字符串值里是否含有「非法控制字符」，返回第一个命中字符。
///
/// 对齐 RedisInsight 的「不要把二进制垃圾 / 异常控制字符写进 key」守卫：文本值编辑器应
/// 拒绝无法被正常表示或会造成数据损坏的控制字符。文本中合法的制表符 / 换行 / 回车
/// （`\t` `\n` `\r`）予以放行，其余 C0/C1/DEL 等控制字符（含 NUL、ESC）一律拦截。
/// 返回 `None` 表示值合法。
fn redis_value_illegal_control_char(value: &str) -> Option<char> {
    value
        .chars()
        .find(|ch| ch.is_control() && !matches!(ch, '\t' | '\n' | '\r'))
}

/// 类型可编辑 且 内容不是二进制，才允许编辑值。
fn redis_key_value_editable_for(kind: &str, value: &str) -> bool {
    redis_key_value_editable(kind) && !redis_value_is_binary(value)
}

/// 判断键类型是否为 Redis JSON 值（ReJSON / 原生 JSON）。
///
/// 展示分支、编辑输入选择、保存准备、下载扩展名等统一使用本判断，保证各处一致。
/// trim 与大小写均不敏感，兼容 Redis 返回的 `ReJSON-RL` 及 `json` / `rejson` 等变体。
pub(crate) fn redis_key_value_is_json_kind(kind: &str) -> bool {
    let kind = kind.trim();
    kind.eq_ignore_ascii_case("json")
        || kind.eq_ignore_ascii_case("rejson")
        || kind.eq_ignore_ascii_case("rejson-rl")
}

fn redis_pretty_json(value: &str) -> Result<String, serde_json::Error> {
    serde_json::from_str::<serde_json::Value>(value).and_then(|json| serde_json::to_string_pretty(&json))
}

fn redis_ttl_input_value(ttl: &str) -> String {
    ttl.chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>()
}

/// 判断给定的 TTL 展示值是否表示「永不超时」（无过期时间）。
/// 兼容列表页的 `(No TTL)`、Hash 字段的 `无 TTL` 以及空串三种来源。
fn redis_ttl_is_none(ttl: &str) -> bool {
    let t = ttl.trim();
    t.is_empty() || t == "(No TTL)" || t == "无 TTL"
}

fn redis_detail_action_button(
    label: &'static str,
    primary: bool,
    enabled: bool,
    colors: UiColors,
) -> Div {
    div()
        .h(px(26.))
        .min_w(px(72.))
        .px_3()
        .rounded(colors.radius)
        .border_1()
        .border_color(if !enabled {
            colors.border
        } else if primary {
            rgb(0x2c6bed)
        } else {
            colors.border
        })
        .bg(if !enabled {
            colors.panel_alt
        } else if primary {
            if colors.is_dark {
                rgb(0x173766)
            } else {
                rgb(0xe8f0ff)
            }
        } else {
            colors.panel_alt
        })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if !enabled {
            colors.muted
        } else if primary {
            rgb(0x1677ff)
        } else {
            colors.text
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
        })
        .child(label)
}

#[cfg(test)]
mod redis_json_kind_judge_tests {
    use super::*;

    #[test]
    fn json_kind_dispatch_covers_all_variants_case_and_trim_insensitive() {
        // 展示、保存、输入选择、下载扩展名共用同一判断；所有 JSON 变体（含大小写、前后空格）都应命中。
        for kind in [
            "json",
            "JSON",
            "Json",
            "rejson",
            "ReJSON",
            "REJSON",
            "rejson-rl",
            "ReJSON-RL",
            "  json  ",
            "\trejson-rl\n",
        ] {
            assert!(redis_key_value_is_json_kind(kind), "应命中 JSON：{kind:?}");
        }
        // 其它类型不命中，仍走 String / 其它分支。
        for kind in ["string", "hash", "list", "set", "zset", "stream", ""] {
            assert!(!redis_key_value_is_json_kind(kind), "不应命中 JSON：{kind:?}");
        }
    }
}
