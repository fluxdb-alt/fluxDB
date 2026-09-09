
fn redis_key_detail_header(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    key_name: String,
    refreshed_at: Option<Instant>,
    refreshing: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .h(px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap_2()
                .child(redis_type_tag(detail.kind.clone(), colors.is_dark))
                // 键大小：紧凑只读数字（如 `12.5 KB`），紧跟类型 tag 之后、key 名之前；
                // 无大小（MEMORY USAGE 未返回）时不显示。
                .when(!detail.size.is_empty(), |this| {
                    this.child(
                        div()
                            .flex_none()
                            .px_2()
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .rounded(colors.radius * 0.5)
                            .bg(colors.panel_alt)
                            .font_family("Menlo")
                            .text_size(px(11.))
                            .text_color(colors.muted)
                            .child(detail.size.clone()),
                    )
                })
                .child(
                    div()
                        .min_w(px(0.))
                        .w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_family("Menlo")
                        .text_size(px(12.))
                        .text_color(colors.muted)
                        .child(redis_ellipsis_text(&key_name, 64)),
                ),
        )
        .child(redis_key_detail_toolbar(
            tab_id,
            detail.key.clone(),
            detail.refresh_kind(),
            refreshed_at,
            refreshing,
            colors,
            cx,
        ))
}

fn redis_key_detail_toolbar(
    tab_id: TabId,
    key: String,
    refresh_kind: RedisKeyDetailRefreshKind,
    refreshed_at: Option<Instant>,
    refreshing: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(12.))
        .text_color(colors.muted)
        .child(redis_key_refresh_label(refreshed_at))
        .child(
            Button::new(("redis-key-detail-refresh", tab_id.0))
                .ghost()
                .xsmall()
                .h(px(24.))
                .flex_none()
                .disabled(refreshing)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_1()
                        .child(if refreshing {
                            loading_spinner_with_color(13., colors.muted).into_any_element()
                        } else {
                            app_icon(AppIcon::Refresh, 13., colors.muted).into_any_element()
                        })
                        .child("刷新"),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.request_redis_key_refresh(tab_id, key.clone(), refresh_kind, cx);
                    cx.stop_propagation();
                })),
        )
}

fn redis_key_refresh_label(refreshed_at: Option<Instant>) -> String {
    let Some(refreshed_at) = refreshed_at else {
        return "上次刷新: 未刷新".to_string();
    };
    let elapsed = refreshed_at.elapsed();
    if elapsed < Duration::from_secs(60) {
        "上次刷新: <1m".to_string()
    } else if elapsed < Duration::from_secs(60 * 60) {
        format!("上次刷新: {}m", elapsed.as_secs() / 60)
    } else {
        format!("上次刷新: {}h", elapsed.as_secs() / 3600)
    }
}

fn redis_key_detail_meta_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    dirty: bool,
    applying: bool,
    this: &mut NavicatMain,
    colors: UiColors,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) -> Div {
    let show_actions = redis_key_meta_actions_visible(dirty, applying, this.redis_key_meta_editing);
    let actions_enabled = redis_key_meta_actions_enabled(dirty, applying, this.redis_key_meta_editing);
    redis_detail_panel(colors)
        .h(px(142.))
        .flex_none()
        .child(
            div()
                .h(px(28.))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .child(redis_detail_panel_title("基础信息", colors))
                .when(show_actions, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(("redis-key-detail-save", tab_id.0))
                                    .label("保存")
                                    .small()
                                    .primary()
                                    .h(px(26.))
                                    .min_w(px(72.))
                                    .disabled(!actions_enabled)
                                    .on_click(cx.listener({
                                        let detail = detail.clone();
                                        move |this, _, _, cx| {
                                            this.request_redis_key_value_apply(
                                                tab_id,
                                                detail.clone(),
                                                cx,
                                            );
                                            cx.stop_propagation();
                                        }
                                    })),
                            )
                            .child(
                                Button::new(("redis-key-detail-discard", tab_id.0))
                                    .label("放弃")
                                    .small()
                                    .outline()
                                    .h(px(26.))
                                    .min_w(px(72.))
                                    .disabled(!actions_enabled)
                                    .on_click(cx.listener({
                                        let detail = detail.clone();
                                        move |this, _, window, cx| {
                                            this.discard_redis_key_drafts(
                                                tab_id,
                                                detail.clone(),
                                                window,
                                                cx,
                                            );
                                            cx.stop_propagation();
                                        }
                                    })),
                            ),
                    )
                }),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(0.))
                .flex()
                .gap_2()
                .child(
                    redis_detail_editable_meta_field(
                        tab_id,
                        detail,
                        RedisKeyMetaField::KeyName,
                        "键名称",
                        this.redis_key_name_input.clone(),
                        this.redis_key_name_display(tab_id, detail),
                        true,
                        this.redis_key_meta_editing == Some(RedisKeyMetaField::KeyName),
                        colors,
                        window,
                        cx,
                    )
                    .flex_1()
                    .min_w(px(0.)),
                )
                .child(
                    redis_detail_editable_meta_field(
                        tab_id,
                        detail,
                        RedisKeyMetaField::Ttl,
                        "TTL",
                        this.redis_key_ttl_input.clone(),
                        this.redis_key_ttl_display(tab_id, detail),
                        false,
                        this.redis_key_meta_editing == Some(RedisKeyMetaField::Ttl),
                        colors,
                        window,
                        cx,
                    )
                    .w(px(132.))
                    .flex_none(),
                ),
        )
}

fn redis_key_meta_actions_visible(
    dirty: bool,
    applying: bool,
    editing: Option<RedisKeyMetaField>,
) -> bool {
    dirty || applying || editing.is_some()
}

fn redis_key_meta_actions_enabled(
    dirty: bool,
    applying: bool,
    editing: Option<RedisKeyMetaField>,
) -> bool {
    !applying && (dirty || editing.is_some())
}

fn redis_key_detail_value_panel(
    tab_id: TabId,
    detail: &RedisKeyDetail,
    input: Entity<InputState>,
    applying: bool,
    this: &mut NavicatMain,
    window: &mut Window,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Div {
    if detail.kind.eq_ignore_ascii_case("stream") {
        return redis_key_detail_stream_panel(tab_id, detail, applying, this, window, colors, cx);
    }
    if detail.kind.eq_ignore_ascii_case("set") {
        return redis_key_detail_set_panel(tab_id, detail, applying, this, window, colors, cx);
    }
    if detail.kind.eq_ignore_ascii_case("hash") {
        return redis_key_detail_hash_panel(tab_id, detail, applying, this, window, colors, cx);
    }
    if detail.kind.eq_ignore_ascii_case("zset") {
        return redis_key_detail_zset_panel(tab_id, detail, applying, this, window, colors, cx);
    }
    if detail.kind.eq_ignore_ascii_case("list") {
        return redis_key_detail_list_panel(tab_id, detail, applying, this, window, colors, cx);
    }
    // 展示分支与保存分支共用同一套 JSON 类型判断（json / rejson / rejson-rl）。
    if redis_key_value_is_json_kind(&detail.kind) {
        return redis_json_value_panel(tab_id, detail, applying, this, colors, cx);
    }
    redis_string_value_panel(tab_id, detail, input, applying, this, colors, cx)
}
